//! Protocol boundary for message-level TRAE account pooling.
//! Keep account-specific model details outside persisted conversation history.
use serde_json::{json, Value};
use std::io::{BufRead, Read};

const MAX_FRAME: usize = 1024 * 1024;

/// Resolve the exact model for a particular account and function. Internal edit
/// models must never masquerade as chat models or be used as silent fallbacks.
pub(crate) fn select_model<'a>(catalog: &'a Value, name: &str) -> Result<&'a Value, String> {
    if name.is_empty() || matches!(name, "fast_apply" | "fast_apply_new" | "context_selection") {
        return Err("请选择账号实际提供的对话模型".into());
    }
    catalog
        .get("config_info_list")
        .and_then(Value::as_array)
        .ok_or("模型目录格式无效")?
        .iter()
        .find(|m| {
            m["config_name"].as_str() == Some(name)
                && m["is_custom_model"].as_bool() != Some(true)
                && !m
                    .get("custom_models")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty())
        })
        .ok_or_else(|| "该账号未提供所选模型，未替换为其他模型".into())
}

/// Build a fresh native request from the entire caller-owned history. On a
/// rotation, call this again with the *new account's* model configuration.
pub(crate) fn build_request(
    input: &Value,
    model: &Value,
    function: &str,
    session: &str,
) -> Result<Value, String> {
    if session.is_empty() || function.is_empty() {
        return Err("缺少会话或模型功能标识".into());
    }
    let messages = input
        .get("messages")
        .and_then(Value::as_array)
        .filter(|m| !m.is_empty())
        .ok_or("消息历史不能为空")?;
    let mut history = Vec::with_capacity(messages.len());
    for original in messages {
        let mut item = original.as_object().cloned().ok_or("消息格式无效")?;
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .ok_or("消息缺少角色")?;
        if !matches!(role, "system" | "developer" | "user" | "assistant" | "tool") {
            return Err("上游不支持该消息角色，未丢弃或改写历史".into());
        }
        if role == "developer" {
            item.insert("role".into(), json!("system"));
        }
        let content = match item.get("content") {
            Some(Value::String(text)) => json!([{"type":"text", "text":text}]),
            Some(Value::Array(parts)) => Value::Array(parts.clone()),
            None | Some(Value::Null) => json!([]),
            _ => return Err("消息内容必须是文本或内容块数组".into()),
        };
        item.insert("content".into(), content);
        // tool_calls, tool_call_id, names and reasoning history remain intact.
        history.push(Value::Object(item));
    }
    let detail = model
        .get("model_detail_list")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .ok_or("所选模型缺少原生参数")?;
    let name = model
        .get("config_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("模型缺少配置名")?;
    let backend = detail
        .get("model_name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or("模型缺少后端标识")?;
    let parameters = detail
        .get("encrypted_model_params")
        .filter(|p| !p.is_null())
        .ok_or("该账号模型缺少加密参数，未伪造请求")?;
    let mut body = json!({"config_name":name, "model_name":backend, "messages":history,
        "encrypted_model_params":parameters, "session_id":session, "conversation_id":session,
        "raw_chat_function":function, "config_source":1, "is_custom_model":false, "stream":true});
    for key in [
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "reasoning_effort",
        "reasoning_effort_level",
    ] {
        if let Some(value) = input.get(key) {
            body[key] = value.clone();
        }
    }
    Ok(body)
}

/// The verified SOLO endpoint uses a smaller envelope than raw v2. Its tool
/// schemas are JSON strings and tool history names `function_call` explicitly.
pub(crate) fn build_solo_request(input: &Value, model: &str) -> Result<Value, String> {
    let messages = input
        .get("messages")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or("消息历史不能为空")?;
    let mut history = Vec::with_capacity(messages.len());
    let mut call_ids = std::collections::HashSet::new();
    for message in messages {
        let mut item = message.as_object().cloned().ok_or("消息格式无效")?;
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .ok_or("消息缺少角色")?
            .to_string();
        if !matches!(
            role.as_str(),
            "system" | "developer" | "user" | "assistant" | "tool"
        ) {
            return Err("消息角色不受支持".into());
        }
        if role == "developer" {
            item.insert("role".into(), json!("system"));
        }
        if let Some(content) = item.get_mut("content") {
            if let Value::String(text) = content {
                *content = json!([{"type":"text", "text":text}]);
            } else if !content.is_null() && !content.is_array() {
                return Err("消息内容格式无效".into());
            }
        }
        if let Some(calls) = item.get_mut("tool_calls") {
            let calls = calls.as_array_mut().ok_or("工具调用列表格式无效")?;
            for call in calls {
                let call = call.as_object_mut().ok_or("工具调用格式无效")?;
                let id = call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or("工具调用缺少关联 ID")?
                    .to_string();
                call_ids.insert(id);
                let function = call
                    .remove("function")
                    .or_else(|| call.remove("function_call"))
                    .ok_or("工具调用缺少函数")?;
                if function
                    .get("name")
                    .and_then(Value::as_str)
                    .is_none_or(str::is_empty)
                {
                    return Err("工具调用缺少函数名".into());
                }
                call.insert("function_call".into(), function);
            }
        }
        if role == "tool" {
            let id = item
                .get("tool_call_id")
                .and_then(Value::as_str)
                .ok_or("工具结果缺少关联 ID")?;
            if !call_ids.contains(id) {
                return Err("工具结果缺少对应调用，未丢弃历史".into());
            }
        }
        history.push(Value::Object(item));
    }
    let mut out = json!({"messages":history, "function":"solo_work_lite", "stream":true, "config_name":model,"model":model});
    if let Some(tools) = input.get("tools") {
        let mut converted = tools.as_array().cloned().ok_or("工具列表格式无效")?;
        for tool in &mut converted {
            let function = tool
                .get_mut("function")
                .and_then(Value::as_object_mut)
                .ok_or("仅支持函数工具")?;
            if let Some(parameters) = function.get_mut("parameters") {
                if parameters.is_object() {
                    *parameters = json!(parameters.to_string());
                } else if !parameters.is_string() {
                    return Err("工具参数必须是 JSON schema 对象".into());
                }
            }
        }
        out["tools"] = json!(converted);
    }
    if let Some(choice) = input.get("tool_choice") {
        out["tool_choice"] = match choice {
            Value::String(_) => choice.clone(),
            Value::Object(_) if choice["type"] == "function" => choice
                .pointer("/function/name")
                .cloned()
                .ok_or("缺少指定工具名称")?,
            _ => return Err("工具选择格式不受支持".into()),
        };
        if out["tool_choice"] == "none" {
            out.as_object_mut().unwrap().remove("tools");
            out.as_object_mut().unwrap().remove("tool_choice");
        }
    }
    for key in [
        "temperature",
        "top_p",
        "max_tokens",
        "presence_penalty",
        "frequency_penalty",
        "seed",
        "n",
    ] {
        if let Some(value) = input.get(key) {
            if !value.is_number() {
                return Err(format!("{key} 必须是数字"));
            }
            out[key] = value.clone();
        }
    }
    if !out.get("max_tokens").is_some() {
        if let Some(value) = input.get("max_completion_tokens") {
            out["max_tokens"] = value.clone();
        }
    }
    for key in ["stop", "reasoning_effort"] {
        if let Some(value) = input.get(key) {
            out[key] = value.clone();
        }
    }
    if input.get("response_format").is_some() {
        return Err("当前通道不支持 response_format，请使用文本输出".into());
    }
    Ok(out)
}

/// Convert one native output delta without altering IDs or argument fragments.
pub(crate) fn solo_delta(value: &Value) -> Result<Value, String> {
    let mut out = serde_json::Map::new();
    for (native, target) in [
        ("response", "content"),
        ("reasoning_content", "reasoning_content"),
    ] {
        if let Some(text) = value
            .get(native)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            out.insert(target.into(), json!(text));
        }
    }
    if let Some(calls) = value.get("tool_calls").filter(|v| !v.is_null()) {
        let calls = calls
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![calls.clone()]);
        let mut normalized = Vec::new();
        for (position, call) in calls.into_iter().enumerate() {
            let mut call = call.as_object().cloned().ok_or("上游工具调用格式无效")?;
            call.entry("index").or_insert(json!(position));
            for key in ["id", "type"] {
                if call
                    .get(key)
                    .and_then(Value::as_str)
                    .is_some_and(str::is_empty)
                {
                    call.remove(key);
                }
            }
            if let Some(function) = call.remove("function_call") {
                call.insert("function".into(), function);
            }
            if let Some(function) = call.get_mut("function").and_then(Value::as_object_mut) {
                function.remove("namespace");
                function.remove("partial_arguments");
                if let Some(arguments) = function.get_mut("arguments") {
                    if !arguments.is_string() {
                        *arguments = json!(arguments.to_string());
                    }
                }
            }
            normalized.push(Value::Object(call));
        }
        if !normalized.is_empty() {
            out.insert("tool_calls".into(), json!(normalized));
        }
    }
    Ok(Value::Object(out))
}

#[derive(Debug, PartialEq)]
pub(crate) struct NativeFrame {
    pub event: String,
    pub data: String,
}

/// Incremental, bounded SSE parsing. BufRead retains split UTF-8 bytes until a
/// whole event is available. EOF must be handled as a failure unless `done` was
/// previously observed; a dropped TCP connection is never successful output.
pub(crate) fn read_frame(reader: &mut impl BufRead) -> Result<Option<NativeFrame>, String> {
    let mut event = String::new();
    let mut parts = Vec::new();
    let mut total = 0;
    loop {
        let mut line = Vec::new();
        let size = (&mut *reader)
            .take((MAX_FRAME - total + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| "读取上游响应失败".to_string())?;
        total += size;
        if total > MAX_FRAME {
            return Err("上游事件过大，未截断响应".into());
        }
        if size == 0 {
            return if event.is_empty() && parts.is_empty() {
                Ok(None)
            } else {
                Err("上游事件尚未完整返回就已断流".into())
            };
        }
        let line = String::from_utf8(line).map_err(|_| "上游事件包含无效编码")?;
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            if parts.is_empty() {
                event.clear();
                continue;
            }
            return Ok(Some(NativeFrame {
                event,
                data: parts.join("\n"),
            }));
        }
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event = value.trim_start_matches(' ').into();
        }
        if let Some(value) = line.strip_prefix("data:") {
            parts.push(value.strip_prefix(' ').unwrap_or(value).to_string());
        }
    }
}

#[derive(Default)]
pub(crate) struct StreamGuard {
    pub output_started: bool,
    pub done: bool,
    failed: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) enum FrameAction {
    Metadata,
    Output(Value),
    Usage(Value),
    Done(String),
    Rotate,
    Reject,
}

impl StreamGuard {
    pub fn inspect(&mut self, frame: &NativeFrame) -> Result<FrameAction, String> {
        if self.failed {
            return Err("上游已返回错误，不再接受成功结束事件".into());
        }
        if self.done {
            return Err("上游结束事件之后又返回了数据".into());
        }
        if frame.data == "[DONE]" {
            self.done = true;
            return Ok(FrameAction::Done("stop".into()));
        }
        let data: Value = serde_json::from_str(&frame.data).map_err(|_| "上游事件不是有效 JSON")?;
        let event = if frame.event.is_empty() {
            data.get("event").and_then(Value::as_str).unwrap_or("")
        } else {
            &frame.event
        };
        if event == "error"
            || data.get("error").is_some()
            || data
                .get("code")
                .and_then(Value::as_i64)
                .is_some_and(|v| v != 0)
        {
            self.failed = true;
            let message = data
                .get("message")
                .or_else(|| data.pointer("/error/message"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let lower = message.to_ascii_lowercase();
            let quota = matches!(data.get("code").and_then(Value::as_i64), Some(1005 | 4008))
                || message.contains("积分不足")
                || message.contains("额度不足")
                || lower.contains("insufficient credit")
                || lower.contains("insufficient quota");
            return Ok(if quota && !self.output_started {
                FrameAction::Rotate
            } else {
                FrameAction::Reject
            });
        }
        match event {
            "done" => {
                self.done = true;
                Ok(FrameAction::Done(
                    data.get("finish_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("stop")
                        .into(),
                ))
            }
            "token_usage" => Ok(FrameAction::Usage(data)),
            "text_delta" | "reasoning_delta" | "translated_text" | "output" | "accumulated"
            | "tool_calls" => {
                if event == "output" && solo_delta(&data)?.as_object().is_some_and(|m| m.is_empty())
                {
                    return Ok(FrameAction::Metadata);
                }
                self.output_started = true;
                Ok(FrameAction::Output(data))
            }
            _ => Ok(FrameAction::Metadata),
        }
    }

    pub fn finish(&self) -> Result<(), String> {
        if self.done && !self.failed {
            Ok(())
        } else {
            Err("上游返回错误或提前断流，未自动重放当前请求".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn model(params: &str) -> Value {
        json!({"config_name":"exact-model", "model_detail_list":[{"model_name":"backend-id", "encrypted_model_params":params}]})
    }

    #[test]
    fn rotation_preserves_whole_history_and_rebinds_account_model_parameters() {
        let input = json!({"messages":[{"role":"developer","content":"system rules"},
            {"role":"user","content":"question"},
            {"role":"assistant","content":null,"tool_calls":[{"id":"call1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"x\"}"}}]},
            {"role":"tool","tool_call_id":"call1","content":"file result"}],
            "tools":[{"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}],"tool_choice":"auto"});
        let a = build_request(&input, &model("account-a"), "solo_agent", "session").unwrap();
        let b = build_request(&input, &model("account-b"), "solo_agent", "session").unwrap();
        assert_eq!(a["messages"], b["messages"]);
        assert_eq!(a["messages"].as_array().unwrap().len(), 4);
        assert_eq!(
            b["messages"][2]["tool_calls"],
            input["messages"][2]["tool_calls"]
        );
        assert_eq!(b["messages"][3]["tool_call_id"], "call1");
        assert_eq!(b["tools"], input["tools"]);
        assert_eq!(b["encrypted_model_params"], "account-b");
        assert_eq!(input["messages"][0]["role"], "developer");
    }

    #[test]
    fn missing_or_internal_model_is_not_substituted() {
        let catalog = json!({"config_info_list":[model("a")]});
        assert!(select_model(&catalog, "exact-model").is_ok());
        assert!(select_model(&catalog, "wrong-case").is_err());
        assert!(select_model(&catalog, "fast_apply").is_err());
        assert!(build_request(&json!({"messages":[]}), &model("a"), "solo_agent", "s").is_err());
    }

    #[test]
    fn sse_handles_split_utf8_crlf_multiline_and_heartbeats() {
        let mut reader = BufReader::with_capacity(1, Cursor::new(": ping\r\n\r\nevent: text_delta\r\ndata: {\"text\":\"你好\",\r\ndata: \"other\":1}\r\n\r\n".as_bytes()));
        let frame = read_frame(&mut reader).unwrap().unwrap();
        assert_eq!(frame.event, "text_delta");
        assert_eq!(
            serde_json::from_str::<Value>(&frame.data).unwrap()["text"],
            "你好"
        );
        assert!(read_frame(&mut reader).unwrap().is_none());
    }

    fn frame(event: &str, data: Value) -> NativeFrame {
        NativeFrame {
            event: event.into(),
            data: data.to_string(),
        }
    }

    #[test]
    fn quota_can_rotate_after_queue_but_never_after_text_reasoning_or_tool_output() {
        let quota = frame("error", json!({"code":4028,"message":"积分不足"}));
        let mut guard = StreamGuard::default();
        assert_eq!(
            guard
                .inspect(&frame("queue", json!({"position":2})))
                .unwrap(),
            FrameAction::Metadata
        );
        assert_eq!(guard.inspect(&quota).unwrap(), FrameAction::Rotate);
        assert!(guard.inspect(&frame("done", json!({}))).is_err());
        for event in ["text_delta", "reasoning_delta", "tool_calls"] {
            let mut guard = StreamGuard::default();
            guard
                .inspect(&frame(event, json!({"text":"started"})))
                .unwrap();
            assert_eq!(guard.inspect(&quota).unwrap(), FrameAction::Reject);
        }
    }

    #[test]
    fn unknown_errors_and_disconnects_are_never_silent_success() {
        let mut guard = StreamGuard::default();
        assert_eq!(
            guard
                .inspect(&frame(
                    "error",
                    json!({"code":4001,"message":"invalid params"})
                ))
                .unwrap(),
            FrameAction::Reject
        );
        assert!(guard.finish().is_err());
        assert!(guard
            .inspect(&frame("done", json!({"finish_reason":"stop"})))
            .is_err());
        assert!(guard.finish().is_err());
        let mut successful = StreamGuard::default();
        successful
            .inspect(&frame("done", json!({"finish_reason":"length"})))
            .unwrap();
        assert!(successful.finish().is_ok());
        assert!(successful
            .inspect(&frame("text_delta", json!("late")))
            .is_err());
        let mut broken = Cursor::new(b"event:text_delta\ndata:{\"text\":\"unfinished\"}");
        assert!(read_frame(&mut broken).is_err());
    }

    #[test]
    fn solo_tool_schema_and_results_survive_wire_conversion() {
        let input = json!({"messages":[{"role":"user","content":"use echo"},{"role":"assistant","content":null,"tool_calls":[{"id":"a","type":"function","function":{"name":"echo","arguments":"{\"value\":1}"}}]},{"role":"tool","tool_call_id":"a","content":"1"}],"tools":[{"type":"function","function":{"name":"echo","parameters":{"type":"object"}}}],"tool_choice":{"type":"function","function":{"name":"echo"}}});
        let wire = build_solo_request(&input, "model").unwrap();
        assert_eq!(wire["function"], "solo_work_lite");
        assert_eq!(
            wire["tools"][0]["function"]["parameters"],
            "{\"type\":\"object\"}"
        );
        assert_eq!(
            wire["messages"][1]["tool_calls"][0]["function_call"],
            input["messages"][1]["tool_calls"][0]["function"]
        );
        assert_eq!(wire["messages"][2]["tool_call_id"], "a");
        assert_eq!(wire["tool_choice"], "echo");
        let delta=solo_delta(&json!({"tool_calls":[{"index":0,"id":"a","function_call":{"name":"echo","arguments":"{","namespace":"x"}}]})).unwrap();
        assert_eq!(delta["tool_calls"][0]["function"]["arguments"], "{");
        assert!(delta["tool_calls"][0]["function"]
            .get("namespace")
            .is_none());
    }

    #[test]
    fn empty_output_does_not_prevent_quota_rotation() {
        let mut guard = StreamGuard::default();
        assert_eq!(
            guard
                .inspect(&frame("output", json!({"response":"","tool_calls":null})))
                .unwrap(),
            FrameAction::Metadata
        );
        assert_eq!(
            guard
                .inspect(&frame(
                    "error",
                    json!({"code":4008,"message":"plan unavailable"})
                ))
                .unwrap(),
            FrameAction::Rotate
        );
    }
}

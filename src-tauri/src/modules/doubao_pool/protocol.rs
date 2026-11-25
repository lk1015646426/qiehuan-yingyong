//! Independent adapter informed by the public samantha protocol documented in
//! https://github.com/wangchuxiaoji-oss/doubao2api and Bitsea1/doubao-free-api.
//! No upstream code, signatures or fabricated security tokens are embedded.
use serde_json::{json, Value};
use std::io::{self, BufRead, Read};

pub fn cookie_value<'a>(cookie: &'a str, name: &str) -> Option<&'a str> {
    cookie
        .split(';')
        .filter_map(|p| p.trim().split_once('='))
        .find_map(|(k, v)| (k == name && !v.is_empty()).then_some(v))
}

pub fn normalize_cookie(input: &str) -> Result<String, String> {
    let input = input.trim();
    if input.is_empty()
        || input.len() > 32768
        || !input.is_ascii()
        || input.bytes().any(|b| b < 32 || b == 127)
    {
        return Err("Cookie 为空、过长或含无效字符".into());
    }
    let mut parts = Vec::new();
    for part in input.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (name, value) = part
            .split_once('=')
            .ok_or("请粘贴完整 Cookie，需包含 sessionid=，不要填写方舟 API Key")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            || value.bytes().any(|b| b == b' ' || b == b'"' || b == b',')
        {
            return Err("Cookie 格式无效，请复制请求中的 Cookie 值".into());
        }
        if parts
            .iter()
            .any(|p: &String| p.starts_with(&format!("{name}=")))
        {
            return Err("Cookie 包含重复字段，请重新复制".into());
        }
        parts.push(format!("{name}={value}"));
    }
    let normalized = parts.join("; ");
    if cookie_value(&normalized, "sessionid").is_none() {
        return Err("未找到 sessionid，请先登录豆包；方舟 API Key 不能用于网页登录账号池".into());
    }
    Ok(normalized)
}

pub fn payload(body: &Value, model: &str) -> Result<Value, String> {
    if !matches!(model, "doubao" | "doubao-think") {
        return Err("豆包网页版模式无效".into());
    }
    if body
        .get("tools")
        .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
        || body
            .get("functions")
            .is_some_and(|v| v.as_array().is_none_or(|a| !a.is_empty()))
        || body.get("tool_choice").is_some_and(|v| v != "none")
    {
        return Err("豆包网页版账号池暂不支持外部工具调用，请使用普通对话".into());
    }
    let messages = body["messages"]
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("messages 不能为空")?;
    let mut history = Vec::new();
    let mut has_user = false;
    for message in messages {
        let role = message["role"].as_str().ok_or("消息缺少 role")?;
        if !matches!(role, "system" | "developer" | "user" | "assistant")
            || message.get("tool_calls").is_some()
        {
            return Err("暂不支持工具消息".into());
        }
        let text = if let Some(s) = message["content"].as_str() {
            s.to_string()
        } else if let Some(parts) = message["content"].as_array() {
            let mut texts = Vec::new();
            for part in parts {
                if part["type"] != "text" {
                    return Err("豆包网页版账号池当前仅支持文本消息".into());
                }
                texts.push(part["text"].as_str().ok_or("文本消息格式无效")?);
            }
            texts.join("\n")
        } else {
            return Err("消息 content 必须为文本".into());
        };
        has_user |= role == "user";
        history.push(format!("[{role}]\n{text}"));
    }
    if !has_user {
        return Err("至少需要一条 user 消息".into());
    }
    // Each request creates an isolated conversation; replay text history explicitly.
    Ok(json!({
        "messages":[{"content":json!({"text":history.join("\n\n")}).to_string(),"content_type":2001,"attachments":[],"references":[]}],
        "completion_option":{"is_regen":false,"with_suggest":false,"need_create_conversation":true,
            "launch_stage":1,"is_replace":false,"is_delete":false,"message_from":0,
            "use_deep_think":model == "doubao-think","use_auto_cot":false,"resend_for_regen":false},
        "evaluate_option":{"web_ab_params":""},"conversation_id":"0",
        "local_conversation_id":format!("local_{}",uuid::Uuid::new_v4().simple()),
        "local_message_id":uuid::Uuid::new_v4().to_string()
    }))
}

#[derive(Debug)]
pub struct UpstreamError {
    pub code: i64,
}
impl UpstreamError {
    pub fn message(&self) -> String {
        let reason = match self.code {
            401 | 403 | 710012001 => "登录会话失效，请重新登录并导入账号",
            429 | 710022002 => "账号受限或需要验证，请在豆包登录窗口完成验证后重新导入",
            _ => "豆包返回异常，请在豆包网页确认账号状态后重试",
        };
        format!("{reason}（code={}）", self.code)
    }
    pub fn cooldown(&self) -> Option<std::time::Duration> {
        match self.code {
            401 | 403 | 710012001 => Some(std::time::Duration::from_secs(300)),
            429 | 710022002 => Some(std::time::Duration::from_secs(60)),
            _ => None,
        }
    }
}

fn object(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| value.clone())
}

/// Converts incrementally, never declares a truncated upstream response complete.
pub struct Stream<R> {
    reader: R,
    pending: io::Cursor<Vec<u8>>,
    finished: bool,
    id: String,
    model: String,
    created: i64,
    pub has_content: bool,
}
impl<R: BufRead> Stream<R> {
    pub fn new(reader: R, model: String) -> Self {
        Self {
            reader,
            pending: io::Cursor::new(Vec::new()),
            finished: false,
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4()),
            model,
            created: chrono::Utc::now().timestamp(),
            has_content: false,
        }
    }
    fn chunk(&self, delta: Value, finish: Value) -> String {
        format!(
            "data: {}\n\n",
            json!({"id":self.id,"object":"chat.completion.chunk",
            "created":self.created,"model":self.model,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})
        )
    }
    pub fn prime(&mut self) -> Result<(), UpstreamError> {
        loop {
            let mut event = String::new();
            let mut data = Vec::new();
            let mut size = 0;
            loop {
                let mut line = String::new();
                let n = self
                    .reader
                    .by_ref()
                    .take((1024 * 1024 - size + 1) as u64)
                    .read_line(&mut line)
                    .map_err(|_| UpstreamError { code: -2 })?;
                size += n;
                if n == 0 || size > 1024 * 1024 {
                    return Err(UpstreamError { code: -2 });
                }
                if line.trim_end_matches(['\r', '\n']).is_empty() {
                    break;
                }
                if let Some(v) = line.strip_prefix("event:") {
                    event = v.trim().to_string();
                }
                if let Some(v) = line.strip_prefix("data:") {
                    data.push(v.trim().to_string());
                }
            }
            if data.is_empty() {
                continue;
            }
            let value: Value =
                serde_json::from_str(&data.join("\n")).map_err(|_| UpstreamError { code: -3 })?;
            let code = value["code"]
                .as_i64()
                .or_else(|| value["error_code"].as_i64())
                .unwrap_or(0);
            if event == "gateway-error"
                || event == "STREAM_ERROR"
                || code != 0
                || value["event_type"] == 2005
            {
                let detail = object(&value["event_data"]);
                return Err(UpstreamError {
                    code: if code != 0 {
                        code
                    } else {
                        detail["code"].as_i64().unwrap_or(-1)
                    },
                });
            }
            let detail = object(&value["event_data"]);
            if value["event_type"] == 2003 || detail["is_finish"] == true {
                if !self.has_content {
                    return Err(UpstreamError { code: -4 });
                }
                self.finished = true;
                self.pending = io::Cursor::new(
                    format!("{}data: [DONE]\n\n", self.chunk(json!({}), json!("stop")))
                        .into_bytes(),
                );
                return Ok(());
            }
            if value["event_type"] != 2001 {
                continue;
            }
            let message = object(&detail["message"]);
            let content = object(&message["content"]);
            let kind = message["content_type"].as_i64().unwrap_or(0);
            let mut delta = json!({"role":"assistant"});
            if matches!(kind, 2001 | 2003 | 10000 | 2008) {
                if let Some(t) = content["text"].as_str().filter(|t| !t.is_empty()) {
                    delta["content"] = json!(t);
                }
            }
            if matches!(kind, 2008 | 10040) {
                if let Some(t) = content["think"].as_str().filter(|t| !t.is_empty()) {
                    delta["reasoning_content"] = json!(t);
                }
            }
            if delta.as_object().unwrap().len() == 1 {
                continue;
            }
            self.has_content = true;
            self.pending = io::Cursor::new(self.chunk(delta, Value::Null).into_bytes());
            return Ok(());
        }
    }
}
impl<R: BufRead> Read for Stream<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let n = self.pending.read(buf)?;
        if n > 0 || self.finished {
            return Ok(n);
        }
        self.prime()
            .map_err(|e| io::Error::new(io::ErrorKind::UnexpectedEof, e.message()))?;
        self.pending.read(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wire(kind: i64, content: Value) -> String {
        format!(
            "data: {}\r\n\r\n",
            json!({"event_type":2001,"event_data":json!({"message":{"content_type":kind,"content":content.to_string()}}).to_string()})
        )
    }
    #[test]
    fn cookies_require_web_session_and_reject_injection() {
        assert!(normalize_cookie("ark-key").is_err());
        assert!(normalize_cookie("sessionid=test\r\nX: secret").is_err());
        assert!(normalize_cookie("ttwid=x").is_err());
        assert!(normalize_cookie("sessionid=a; sessionid=b").is_err());
        let c = normalize_cookie(" sessionid=abc; msToken=real==; ").unwrap();
        assert_eq!(cookie_value(&c, "msToken"), Some("real=="));
    }
    #[test]
    fn text_history_is_preserved_and_unsupported_inputs_rejected() {
        let body = json!({"messages":[{"role":"system","content":"rule"},{"role":"user","content":"first"},{"role":"assistant","content":"answer"},{"role":"user","content":"next"}]});
        let p = payload(&body, "doubao-think").unwrap();
        let text = object(&p["messages"][0]["content"])["text"]
            .as_str()
            .unwrap()
            .to_string();
        for word in ["rule", "first", "answer", "next"] {
            assert!(text.contains(word));
        }
        assert_eq!(p["completion_option"]["use_deep_think"], true);
        assert!(payload(&json!({"messages":body["messages"],"tools":[{}]}), "doubao").is_err());
        assert!(payload(
            &json!({"messages":[{"role":"user","content":[{"type":"image_url"}]}]}),
            "doubao"
        )
        .is_err());
    }
    #[test]
    fn converts_text_and_reasoning_and_requires_finish() {
        let input = format!(
            "{}{}data: {{\"event_type\":2003}}\n\n",
            wire(2008, json!({"think":"reason"})),
            wire(2001, json!({"text":"你好"}))
        );
        let stream = Stream::new(io::Cursor::new(input), "doubao-pool-doubao".into());
        let result =
            crate::modules::model_service::collect_completion(io::BufReader::new(stream)).unwrap();
        assert_eq!(result["choices"][0]["message"]["content"], "你好");
        assert_eq!(
            result["choices"][0]["message"]["reasoning_content"],
            "reason"
        );
        let partial = Stream::new(
            io::Cursor::new(wire(2001, json!({"text":"partial"}))),
            "test".into(),
        );
        assert!(
            crate::modules::model_service::collect_completion(io::BufReader::new(partial)).is_err()
        );
    }
    #[test]
    fn gateway_errors_are_classified_without_echoing_credentials() {
        let mut s = Stream::new(io::Cursor::new("event: gateway-error\ndata: {\"code\":710012001,\"message\":\"secret-cookie\"}\n\n"),"test".into());
        let error = s.prime().unwrap_err();
        assert!(error.cooldown().is_some());
        assert!(!error.message().contains("secret-cookie"));
        let mut empty = Stream::new(
            io::Cursor::new("data: {\"event_type\":2003}\n\n"),
            "test".into(),
        );
        assert!(empty.prime().is_err());
    }
}

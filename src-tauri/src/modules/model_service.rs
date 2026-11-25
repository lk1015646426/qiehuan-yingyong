//! Client-neutral model service boundary. Never launches a desktop agent.
use serde_json::{json, Value};
use std::{collections::VecDeque, io::BufRead, sync::Mutex};

static RECORDS: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());

fn read_existing_connection_key(path: &std::path::Path) -> Result<String, String> {
    let key = std::fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "尚未生成本地 API Key，请先启用账号池"
        } else {
            "无法读取本地 API Key"
        }
    })?;
    let key = key.trim();
    uuid::Uuid::parse_str(key).map_err(|_| "本地 API Key 格式异常，未修改")?;
    Ok(key.to_owned())
}

/// Filename lookup is pure so tests never depend on process-wide env overrides.
pub(crate) fn key_filename(pool: &str) -> Result<&'static str, String> {
    match pool {
        "workbuddy" => Ok("workbuddy-pool-local.key"),
        "trae" => Ok("trae-model-pool.key"),
        "doubao" => Ok("doubao-pool-local.key"),
        _ => Err("账号池类型无效".into()),
    }
}

#[tauri::command]
pub fn get_pool_connection_key(pool: String) -> Result<String, String> {
    let filename = key_filename(&pool)?;
    read_existing_connection_key(&super::config::get_data_dir()?.join(filename))
}
const RECORDS_PER_POOL: usize = 50;
// Stable result labels. The UI and the summary must never parse free-form text.
pub(crate) const RESULT_COMPLETE: &str = "响应完成";
pub(crate) const RESULT_STREAM_ENDED: &str = "流式转发结束";
pub(crate) const RESULT_SEND_FAILED: &str = "响应发送失败";
pub(crate) const RESULT_FAILED: &str = "请求失败";
pub(crate) const RESULT_ENTRY_REJECTED: &str = "入口过载拒绝";
pub(crate) const RESULT_STOPPING: &str = "服务正在停止";
fn provider_prefix(model: &str) -> Option<&'static str> {
    ["workbuddy-pool-", "trae-pool-", "doubao-pool-"]
        .into_iter()
        .find(|prefix| model.starts_with(*prefix))
}
fn push_record(rows: &mut VecDeque<Value>, row: Value) {
    let Some(prefix) = row["model"].as_str().and_then(provider_prefix) else {
        return;
    };
    if rows
        .iter()
        .filter(|r| r["model"].as_str().is_some_and(|m| m.starts_with(prefix)))
        .count()
        >= RECORDS_PER_POOL
    {
        if let Some(index) = rows
            .iter()
            .position(|r| r["model"].as_str().is_some_and(|m| m.starts_with(prefix)))
        {
            rows.remove(index);
        }
    }
    rows.push_back(row);
}
pub(crate) fn record(model: &str, stream: bool, status: u16, result: &str) {
    record_timed(model, stream, status, result, None);
}
fn record_row(
    model: &str,
    stream: bool,
    status: u16,
    result: &str,
    relay_ms: Option<u64>,
) -> Value {
    json!({"time":chrono::Utc::now().to_rfc3339(),"model":model,"stream":stream,"status":status,"result":result,"relayMs":relay_ms})
}
pub(crate) fn record_timed(
    model: &str,
    stream: bool,
    status: u16,
    result: &str,
    relay_ms: Option<u64>,
) {
    if let Ok(mut rows) = RECORDS.lock() {
        push_record(
            &mut rows,
            record_row(model, stream, status, result, relay_ms),
        );
    }
}
fn summarize_records(rows: &VecDeque<Value>, prefix: &str) -> (Vec<Value>, Value) {
    let records: Vec<Value> = rows
        .iter()
        .filter(|row| {
            row["model"]
                .as_str()
                .is_some_and(|model| model.starts_with(prefix))
        })
        .cloned()
        .collect();
    let nonstream_success = records
        .iter()
        .filter(|r| r["stream"] == false && r["status"] == 200)
        .count();
    let stream_relay_ok = records
        .iter()
        .filter(|r| r["stream"] == true && r["status"] == 200)
        .count();
    let limited = records.iter().filter(|r| r["status"] == 429).count();
    // Entry saturation and per-account saturation are both 429; only the former is admission control.
    let entry_rejected = records
        .iter()
        .filter(|r| r["status"] == 429 && r["result"] == RESULT_ENTRY_REJECTED)
        .count();
    let failed = records.len() - nonstream_success - stream_relay_ok - limited;
    let mut times: Vec<u64> = records
        .iter()
        .filter(|r| r["stream"] == false && r["status"] == 200)
        .filter_map(|r| r["relayMs"].as_u64())
        .collect();
    let average = if times.is_empty() {
        None
    } else {
        Some(times.iter().map(|n| *n as f64).sum::<f64>() / times.len() as f64)
    };
    // Nearest rank percentile of successful non-stream relay samples only.
    times.sort_unstable();
    let p95 = if times.is_empty() {
        None
    } else {
        Some(times[((times.len() * 95 + 99) / 100).saturating_sub(1)])
    };
    let summary = json!({"recordCount":records.len(),"nonstreamSuccess":nonstream_success,"streamRelayOk":stream_relay_ok,"limited":limited,"entryRejected":entry_rejected,"failed":failed,"timingSamples":times.len(),"meanRelayMs":average,"p95RelayMs":p95});
    (records, summary)
}

/// Aggregate OpenAI SSE without dropping tool calls, reasoning, usage or choices.
/// A missing DONE marker is a failure, never a successful partial completion.
pub(crate) fn collect_completion(mut reader: impl BufRead) -> Result<Value, String> {
    use std::collections::BTreeMap;
    let mut choices: BTreeMap<u64, Value> = BTreeMap::new();
    let mut result = json!({"object":"chat.completion","choices":[]});
    let mut total = 0;
    loop {
        let mut line = String::new();
        let n = std::io::Read::take(&mut reader, 16 * 1024 * 1024 - total + 1)
            .read_line(&mut line)
            .map_err(|_| "上游流读取失败")?;
        total += n as u64;
        if total > 16 * 1024 * 1024 {
            return Err("非流式响应超过 16 MiB".into());
        }
        if n == 0 {
            return Err("上游流提前中断，未返回完整结果".into());
        }
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            if choices.is_empty() || choices.values().any(|c| c["finish_reason"].is_null()) {
                return Err("上游缺少完成状态".into());
            }
            result["choices"] = Value::Array(choices.into_values().collect());
            return Ok(result);
        }
        if data.is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(data).map_err(|_| "上游 SSE JSON 无效")?;
        if event.get("error").is_some() {
            return Err("上游在响应过程中返回错误".into());
        }
        for key in ["id", "model", "created", "usage", "system_fingerprint"] {
            if let Some(v) = event.get(key) {
                result[key] = v.clone();
            }
        }
        for item in event["choices"].as_array().into_iter().flatten() {
            let index = item["index"].as_u64().ok_or("上游 choice 缺少 index")?;
            let choice = choices.entry(index).or_insert_with(|| json!({"index":index,"message":{"role":"assistant","content":null},"finish_reason":null}));
            if !item["finish_reason"].is_null() {
                choice["finish_reason"] = item["finish_reason"].clone();
            }
            if let Some(delta) = item["delta"].as_object() {
                for (key, value) in delta {
                    if key == "tool_calls" {
                        if choice["message"].get("tool_calls").is_none() {
                            choice["message"]["tool_calls"] = json!([]);
                        }
                        let calls = choice["message"]["tool_calls"].as_array_mut().unwrap();
                        for call in value.as_array().ok_or("tool_calls 不是数组")? {
                            let i = call["index"].as_u64().ok_or("tool call 缺少 index")? as usize;
                            if i > 1024 {
                                return Err("tool call index 超限".into());
                            }
                            while calls.len() <= i {
                                calls.push(json!({"type":"function","function":{"name":"","arguments":""}}));
                            }
                            if let Some(id) = call.get("id") {
                                calls[i]["id"] = id.clone();
                            }
                            for field in ["name", "arguments"] {
                                if let Some(part) = call["function"][field].as_str() {
                                    let old = calls[i]["function"][field].as_str().unwrap_or("");
                                    calls[i]["function"][field] = json!(format!("{old}{part}"));
                                }
                            }
                        }
                    } else if key == "role" {
                        choice["message"][key] = value.clone();
                    } else if let Some(part) = value.as_str() {
                        let old = choice["message"][key].as_str().unwrap_or("");
                        choice["message"][key] = json!(format!("{old}{part}"));
                    }
                }
            }
        }
    }
}

#[tauri::command]
pub async fn start_harness_model_service(pool: String) -> Result<Value, String> {
    match pool.as_str() {
        "trae" => {
            tauri::async_runtime::spawn_blocking(super::trae_model_pool::start)
                .await
                .map_err(|_| "TRAE 服务启动失败")??;
        }
        "workbuddy" => {
            super::workbuddy_pool::start_model_service().await?;
        }
        "doubao" => {
            super::doubao_pool::start_doubao_pool().await?;
        }
        _ => return Err("账号池类型无效".into()),
    }
    get_harness_model_service(pool)
}

#[tauri::command]
pub fn get_harness_model_service(pool: String) -> Result<Value, String> {
    let (status, port, prefix) = match pool.as_str() {
        "trae" => (
            serde_json::to_value(super::trae_model_pool::status()?).map_err(|_| "状态转换失败")?,
            18790,
            "trae-pool-",
        ),
        "workbuddy" => (
            serde_json::to_value(super::workbuddy_pool::get_workbuddy_pool_status()?)
                .map_err(|_| "状态转换失败")?,
            18789,
            "workbuddy-pool-",
        ),
        "doubao" => (
            serde_json::to_value(super::doubao_pool::get_doubao_pool_status()?)
                .map_err(|_| "状态转换失败")?,
            18791,
            "doubao-pool-",
        ),
        _ => return Err("账号池类型无效".into()),
    };
    let (records, summary) = {
        let rows = RECORDS.lock().map_err(|_| "记录锁异常")?;
        summarize_records(&rows, prefix)
    };
    Ok(
        json!({"pool":status,"harness":super::harness_config::detect()?,"records":records,"recordSummary":summary,"baseUrl":format!("http://127.0.0.1:{port}/v1")}),
    )
}

#[tauri::command]
pub async fn configure_harness_model_service(model: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || super::harness_config::configure(&model))
        .await
        .map_err(|_| "配置任务失败")?
}

#[tauri::command]
pub async fn test_harness_model_service(
    model: String,
    stream: bool,
    pool: String,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let client = reqwest::blocking::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(180)).build().map_err(|_| "测试客户端创建失败")?;
        let (port, key) = match pool.as_str() {
            "trae" if model.starts_with("trae-pool-") => (18790, super::trae_model_pool::key()?),
            "workbuddy" if model.starts_with("workbuddy-pool-") => (18789, super::workbuddy_pool::load_pool_key()?),
            "doubao" if model.starts_with("doubao-pool-") => (18791, super::doubao_pool::load_pool_key()?),
            _ => return Err("模型与账号池不匹配".into()),
        };
        let started = std::time::Instant::now();
        let response = client.post(format!("http://127.0.0.1:{port}/v1/chat/completions")).bearer_auth(key).json(&json!({"model":model,"stream":stream,"max_tokens":64,"messages":[{"role":"user","content":"Reply with OK only."}]})).send().map_err(|_| "服务连接失败")?;
        let status = response.status().as_u16();
        let headers_ms = started.elapsed().as_millis() as u64;
        let body = response.text().map_err(|_| "响应读取失败")?;
        let total_ms = started.elapsed().as_millis() as u64;
        let response_bytes = body.len();
        if status != 200 { return Err(format!("调用失败 HTTP {status}（耗时 {total_ms} ms）")); }
        let timing = json!({"headersMs":headers_ms,"totalMs":total_ms,"responseBytes":response_bytes});
        if stream {
            let value = collect_completion(std::io::Cursor::new(body))?;
            Ok(json!({"status":status,"stream":true,"timing":timing,"completion":value}))
        } else {
            let value: Value = serde_json::from_str(&body).map_err(|_| "非流式 JSON 无效")?;
            Ok(json!({"status":status,"stream":false,"timing":timing,"completion":value}))
        }
    }).await.map_err(|_| "测试任务失败")?
}

/// Run a small, explicit two-round tool protocol probe without executing model-supplied code.
#[tauri::command]
pub async fn test_harness_tool_roundtrip(pool: String, model: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (port, key) = match pool.as_str() {
            "trae" if model.starts_with("trae-pool-") => (18790, super::trae_model_pool::key()?),
            "workbuddy" if model.starts_with("workbuddy-pool-") => (18789, super::workbuddy_pool::load_pool_key()?),
            "doubao" if model.starts_with("doubao-pool-") => (18791, super::doubao_pool::load_pool_key()?),
            _ => return Err("模型与账号池不匹配".to_string()),
        };
        let client = reqwest::blocking::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(180)).build().map_err(|_| "测试客户端创建失败")?;
        let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
        let tools = json!([{ "type":"function", "function": {"name":"read_probe_value", "description":"Return the current test value.", "parameters":{"type":"object","properties":{},"additionalProperties":false}} }]);
        let mut messages = vec![json!({"role":"system","content":"Use the available function when the user asks you to."}), json!({"role":"user","content":"Call read_probe_value, then reply with the returned value exactly."})];
        let send = |messages: &[Value]| -> Result<Value,String> {
            let response = client.post(&url).bearer_auth(&key).json(&json!({"model":model,"stream":false,"max_tokens":512,"messages":messages,"tools":tools,"tool_choice":"auto"})).send().map_err(|_| "测试连接中断，未自动重放")?;
            let status = response.status();
            if !status.is_success() { return Err(format!("工具协议测试失败：HTTP {}", status.as_u16())); }
            response.json::<Value>().map_err(|_| "工具协议测试响应不是有效 JSON".to_string())
        };
        let first = send(&messages)?;
        let assistant = first.pointer("/choices/0/message").and_then(Value::as_object).ok_or("模型未返回 assistant 消息")?;
        let calls = assistant.get("tool_calls").and_then(Value::as_array).ok_or("模型未发出工具调用")?;
        if calls.len() != 1 || calls[0].pointer("/function/name").and_then(Value::as_str) != Some("read_probe_value") { return Err("模型未按预期调用测试工具".into()); }
        let call_id = calls[0]["id"].as_str().filter(|id| !id.is_empty()).ok_or("工具调用缺少 ID")?.to_string();
        let marker = format!("CHECK_{}", uuid::Uuid::new_v4().simple());
        messages.push(Value::Object(assistant.clone()));
        messages.push(json!({"role":"tool","tool_call_id":call_id,"content":marker}));
        let second = send(&messages)?;
        let answer = second.pointer("/choices/0/message/content").and_then(Value::as_str).unwrap_or("");
        if !answer.contains(&marker) { return Err("模型未在第二轮正确使用工具返回值".into()); }
        Ok(json!({"ok":true,"rounds":2,"model":model,"pool":pool,"toolCall":true,"toolResultUsed":true}))
    }).await.map_err(|_| "工具协议测试任务失败")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_key_read_never_creates_or_repairs_credentials() {
        let dir = std::env::temp_dir().join(format!("pool-key-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("key");
        assert!(read_existing_connection_key(&path).is_err());
        assert!(!path.exists());
        std::fs::write(&path, "invalid").unwrap();
        assert!(read_existing_connection_key(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "invalid");
        let value = uuid::Uuid::new_v4().to_string();
        std::fs::write(&path, format!("{value}\n")).unwrap();
        assert_eq!(read_existing_connection_key(&path).unwrap(), value);
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn connection_keys_resolve_distinct_files_per_pool() {
        // Pure filename mapping: no process-wide env override, so this cannot race other tests
        // that set QIEHUAN_YINGYONG_TEST_DATA_DIR / COCKPIT_TOOLS_TEST_DATA_DIR concurrently.
        let dir = std::env::temp_dir().join(format!("pool-key-map-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let workbuddy = uuid::Uuid::new_v4().to_string();
        let trae = uuid::Uuid::new_v4().to_string();
        let doubao = uuid::Uuid::new_v4().to_string();
        std::fs::write(dir.join(key_filename("workbuddy").unwrap()), &workbuddy).unwrap();
        std::fs::write(dir.join(key_filename("trae").unwrap()), &trae).unwrap();
        std::fs::write(dir.join(key_filename("doubao").unwrap()), &doubao).unwrap();
        for (pool, expected) in [
            ("workbuddy", &workbuddy),
            ("trae", &trae),
            ("doubao", &doubao),
        ] {
            assert_eq!(
                read_existing_connection_key(&dir.join(key_filename(pool).unwrap())).unwrap(),
                *expected
            );
        }
        assert_eq!(
            key_filename("workbuddy").unwrap(),
            "workbuddy-pool-local.key"
        );
        assert_eq!(key_filename("trae").unwrap(), "trae-model-pool.key");
        assert_eq!(key_filename("doubao").unwrap(), "doubao-pool-local.key");
        assert!(key_filename("invalid").is_err());
        assert!(key_filename("").is_err());
        assert!(key_filename("Trae").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
    #[test]
    fn record_rows_carry_no_prompt_or_credential_fields() {
        let row = record_row(
            "workbuddy-pool-glm-5.3-flash",
            false,
            200,
            RESULT_COMPLETE,
            Some(120),
        );
        let mut keys: Vec<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["model", "relayMs", "result", "status", "stream", "time"]
        );
        let text = row.to_string();
        for forbidden in [
            "prompt",
            "messages",
            "Authorization",
            "Bearer",
            "apiKey",
            "api_key",
            "cookie",
        ] {
            assert!(!text.contains(forbidden), "record leaked {forbidden}");
        }
        assert_eq!(row["relayMs"], 120);
    }
    #[test]
    fn concurrent_writes_keep_the_window_bounded_per_pool() {
        let rows = std::sync::Arc::new(Mutex::new(VecDeque::new()));
        let mut threads = Vec::new();
        for pool in ["workbuddy-pool-", "trae-pool-", "doubao-pool-"] {
            for worker in 0..4 {
                let rows = rows.clone();
                threads.push(std::thread::spawn(move || {
                    for i in 0..200u64 {
                        let mut guard = rows.lock().unwrap();
                        push_record(
                            &mut guard,
                            record_row(
                                &format!("{pool}m{worker}"),
                                false,
                                200,
                                RESULT_COMPLETE,
                                Some(i),
                            ),
                        );
                    }
                }));
            }
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let rows = rows.lock().unwrap();
        assert_eq!(rows.len(), 150);
        for prefix in ["workbuddy-pool-", "trae-pool-", "doubao-pool-"] {
            let (records, summary) = summarize_records(&rows, prefix);
            assert_eq!(records.len(), RECORDS_PER_POOL);
            assert_eq!(summary["recordCount"], RECORDS_PER_POOL);
            assert_eq!(summary["timingSamples"], RECORDS_PER_POOL);
            assert!(records
                .iter()
                .all(|r| r["model"].as_str().is_some_and(|m| m.starts_with(prefix))));
        }
    }
    #[test]
    fn entry_rejection_is_counted_separately_from_account_saturation() {
        let mut rows = VecDeque::new();
        push_record(
            &mut rows,
            record_row("trae-pool-", false, 429, RESULT_ENTRY_REJECTED, None),
        );
        push_record(
            &mut rows,
            record_row("trae-pool-", false, 429, RESULT_FAILED, None),
        );
        push_record(
            &mut rows,
            record_row("trae-pool-", false, 503, RESULT_STOPPING, None),
        );
        push_record(
            &mut rows,
            record_row("trae-pool-x", false, 200, RESULT_COMPLETE, Some(80)),
        );
        let (_, summary) = summarize_records(&rows, "trae-pool-");
        assert_eq!(summary["recordCount"], 4);
        assert_eq!(summary["limited"], 2);
        assert_eq!(summary["entryRejected"], 1);
        assert_eq!(summary["failed"], 1);
        assert_eq!(summary["nonstreamSuccess"], 1);
    }
    #[test]
    fn mean_relay_only_covers_completed_nonstream_calls() {
        let mut rows = VecDeque::new();
        push_record(
            &mut rows,
            record_row("doubao-pool-a", false, 200, RESULT_COMPLETE, Some(100)),
        );
        push_record(
            &mut rows,
            record_row("doubao-pool-a", false, 200, RESULT_COMPLETE, Some(300)),
        );
        push_record(
            &mut rows,
            record_row("doubao-pool-a", true, 200, RESULT_STREAM_ENDED, Some(900)),
        );
        push_record(
            &mut rows,
            record_row("doubao-pool-a", false, 502, RESULT_SEND_FAILED, Some(700)),
        );
        let (_, summary) = summarize_records(&rows, "doubao-pool-");
        assert_eq!(summary["timingSamples"], 2);
        assert_eq!(summary["meanRelayMs"], 200.0);
        assert_eq!(summary["p95RelayMs"], 300);
        assert_eq!(summary["streamRelayOk"], 1);
        assert_eq!(summary["failed"], 1);
    }
    #[test]
    fn records_are_bounded_and_isolated_by_provider() {
        let mut rows = VecDeque::new();
        for i in 0..55 {
            push_record(
                &mut rows,
                json!({"model":"workbuddy-pool-demo","stream":false,"status":200,"relayMs":i}),
            );
        }
        push_record(
            &mut rows,
            json!({"model":"trae-pool-","stream":false,"status":429}),
        );
        push_record(
            &mut rows,
            json!({"model":"trae-pool-demo","stream":true,"status":200,"relayMs":20}),
        );
        push_record(
            &mut rows,
            json!({"model":"doubao-pool-","stream":false,"status":503}),
        );
        push_record(&mut rows, json!({"model":"","stream":false,"status":401}));
        let (work, stats) = summarize_records(&rows, "workbuddy-pool-");
        assert_eq!(work.len(), 50);
        assert_eq!(work[0]["relayMs"], 5);
        assert_eq!(stats["timingSamples"], 50);
        assert_eq!(stats["meanRelayMs"], 29.5);
        let (_, trae) = summarize_records(&rows, "trae-pool-");
        assert_eq!(trae["recordCount"], 2);
        assert_eq!(trae["limited"], 1);
        assert_eq!(trae["streamRelayOk"], 1);
        assert_eq!(trae["timingSamples"], 0);
        assert!(trae["meanRelayMs"].is_null());
        assert!(trae["p95RelayMs"].is_null());
        let (_, doubao) = summarize_records(&rows, "doubao-pool-");
        assert_eq!(doubao["failed"], 1);
        assert_eq!(rows.len(), 53);
    }
    #[test]
    fn aggregates_content_reasoning_tools_and_usage() {
        let s = "data: {\"id\":\"x\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"O\",\"reasoning_content\":\"think\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\"}}]}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"K\",\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"total_tokens\":5}}\n\ndata: [DONE]\n\n";
        let v = collect_completion(std::io::Cursor::new(s)).unwrap();
        assert_eq!(v["choices"][0]["message"]["content"], "OK");
        assert_eq!(v["choices"][0]["message"]["reasoning_content"], "think");
        assert_eq!(
            v["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"],
            "{}"
        );
        assert_eq!(v["usage"]["total_tokens"], 5);
    }
    #[test]
    fn rejects_interrupted_and_error_streams() {
        for s in [
            "data: {}\n",
            "data: [DONE]\n",
            "data: {\"error\":{}}\n",
            "data: broken\n",
        ] {
            assert!(collect_completion(std::io::Cursor::new(s)).is_err());
        }
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    /// One real call through the already-running local pool. Requires an explicit opt-in so no
    /// credits are spent by accident; never prints prompts, responses or credentials.
    #[test]
    #[ignore = "uses one real model call and real quota; run explicitly with QIEHUAN_LIVE_CONFIRM=1"]
    fn live_single_call_latency() {
        assert!(
            std::env::var_os("QIEHUAN_LIVE_CONFIRM").is_some(),
            "set QIEHUAN_LIVE_CONFIRM=1 to authorise one real call"
        );
        let pool = std::env::var("QIEHUAN_LIVE_POOL")
            .expect("set QIEHUAN_LIVE_POOL to trae/workbuddy/doubao");
        let model = std::env::var("QIEHUAN_LIVE_MODEL")
            .expect("set QIEHUAN_LIVE_MODEL to a model already selected in that pool");
        let full = format!("{pool}-pool-{model}");
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let wall = std::time::Instant::now();
        let result = runtime
            .block_on(test_harness_model_service(
                full.clone(),
                false,
                pool.clone(),
            ))
            .unwrap();
        let wall_ms = wall.elapsed().as_millis() as u64;
        let timing = &result["timing"];
        println!(
            "live_single_call pool={pool} model={full} status={} headers_ms={} total_ms={} response_bytes={} wall_ms={wall_ms}",
            result["status"], timing["headersMs"], timing["totalMs"], timing["responseBytes"]
        );
        assert_eq!(result["status"], 200);
        assert!(timing["totalMs"].as_u64().is_some_and(|ms| ms > 0));
    }
    #[test]
    #[ignore = "uses real credits and running local account pools"]
    fn live_page_tool_probe_both_pools() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        for (pool, model) in [
            ("trae", "trae-pool-Doubao-Seed-2.1-Turbo"),
            ("workbuddy", "workbuddy-pool-glm-5.3-flash"),
        ] {
            let result = runtime
                .block_on(test_harness_tool_roundtrip(pool.into(), model.into()))
                .unwrap();
            assert_eq!(result["ok"], true);
            assert_eq!(result["toolResultUsed"], true);
            assert_eq!(result["rounds"], 2);
        }
    }
    #[test]
    #[ignore = "uses real TRAE credits and a running local TRAE pool"]
    fn real_trae_harness_config() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.tmp")
            .join(format!("trae-config-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("profiles/desktop")).unwrap();
        std::fs::write(root.join("profiles/desktop/cordis.patch.yml"), "[]\n").unwrap();
        std::fs::write(root.join("proof.txt"), "TRAECONFIG_493").unwrap();
        std::env::set_var("DSH_HOME", &root);
        super::super::harness_config::configure("trae-pool-Doubao-Seed-2.1-Turbo").unwrap();
        std::env::remove_var("DSH_HOME");
        let output = std::process::Command::new("D:\\deepseek harness\\DeepSeek Harness.exe")
            .env("ELECTRON_RUN_AS_NODE", "1").env("DSH_HOME", &root).current_dir(&root)
            .args(["--expose-internals", "D:\\deepseek harness\\resources\\app.asar\\dsh\\node_modules\\@deepseek-ai\\dsh-desktop-host\\lib\\cli.js", "--profile", "headless", "--patch"])
            .arg(root.join("profiles/desktop/cordis.patch.yml")).arg("Use the read tool to read proof.txt and return its content.")
            .output().unwrap();
        let out = String::from_utf8_lossy(&output.stdout);
        let err = String::from_utf8_lossy(&output.stderr);
        std::fs::write(
            root.join("result.json"),
            json!({"exitCode":output.status.code(),"stdout":out,"stderr":err}).to_string(),
        )
        .unwrap();
        assert!(
            output.status.success() && out.contains("TRAECONFIG_493"),
            "Harness returned failure; inspect isolated result"
        );
    }
    #[test]
    #[ignore = "uses real account credits; run explicitly"]
    fn real_pool_acceptance() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(super::super::workbuddy_pool::start_model_service())
            .unwrap();
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .unwrap();
        let key = super::super::workbuddy_pool::load_pool_key().unwrap();
        let base = "http://127.0.0.1:18791/v1";
        let catalog: Value = client
            .get(format!("{base}/models"))
            .bearer_auth(&key)
            .send()
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .unwrap();
        let mut evidence = vec![json!({"check":"models","response":catalog})];
        let mut ids: Vec<_> = catalog["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_owned())
            .collect();
        assert!(!ids.is_empty());
        if let Ok(model) = std::env::var("QIEHUAN_LIVE_MODEL") {
            assert!(ids.contains(&model), "model not selected in existing pool");
            ids.retain(|id| id == &model);
        }
        let chat_count = if std::env::var_os("QIEHUAN_AGENT_ONLY").is_some() {
            0
        } else {
            2
        };
        for id in ids.iter().take(chat_count) {
            for streaming in [true, false] {
                let response = client.post(format!("{base}/chat/completions")).bearer_auth(&key).json(&json!({"model":id,"stream":streaming,"max_tokens":128,"messages":[{"role":"user","content":"Remember the code ORANGE739. Reply only READY."},{"role":"assistant","content":"READY"},{"role":"user","content":"What was the code? Reply only the code."}]})).send().unwrap();
                let status = response.status().as_u16();
                let body = response.text().unwrap();
                let parsed = if streaming {
                    collect_completion(std::io::Cursor::new(body.clone()))
                } else {
                    serde_json::from_str(&body).map_err(|_| "invalid JSON".into())
                };
                evidence.push(json!({"check":"real-conversation","model":id,"stream":streaming,"status":status,"completion":parsed.as_ref().ok(),"error":parsed.as_ref().err(),"rawOnError":if status != 200 {Some(body)} else {None}}));
            }
        }
        for (check, body, auth) in [
            ("missing-auth", json!({}), false),
            (
                "bad-model",
                json!({"model":"bad","messages":[],"stream":true}),
                true,
            ),
            (
                "bad-stream",
                json!({"model":ids[0],"messages":[],"stream":"yes"}),
                true,
            ),
        ] {
            let mut request = client.post(format!("{base}/chat/completions")).json(&body);
            if auth {
                request = request.bearer_auth(&key);
            }
            let status = request.send().unwrap().status().as_u16();
            evidence.push(json!({"check":check,"status":status}));
        }
        // Exercise the installed Harness itself, in a disposable home and headless session.
        let home =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.tmp/harness-cli-home");
        std::fs::create_dir_all(home.join("profiles/desktop")).unwrap();
        std::fs::write(home.join("profiles/desktop/cordis.patch.yml"), "[]\n").unwrap();
        std::env::set_var("DSH_HOME", &home);
        let configured = super::super::harness_config::configure(&ids[0]);
        std::env::remove_var("DSH_HOME");
        configured.unwrap();
        let patch_path = home.join("profiles/desktop/cordis.patch.yml");
        let mut patch = std::fs::read_to_string(&patch_path)
            .unwrap()
            .replace("18789", "18791");
        if let Ok(max) = std::env::var("QIEHUAN_LIVE_MAX_TOKENS") {
            let max: u64 = max.parse().unwrap();
            let mut output = String::new();
            for line in patch.lines() {
                if let Some(row) = line.strip_prefix("- {") {
                    let mut row: Value = serde_json::from_str(&format!("{{{row}")).unwrap();
                    if row["id"] == "llm-pi-ai" {
                        for model in row["config"]["providers"]["qiehuan-workbuddy"]["models"]
                            .as_array_mut()
                            .unwrap()
                        {
                            model["maxTokens"] = json!(max);
                            model["contextWindow"] = json!(32768);
                        }
                    }
                    output.push_str(&format!("- {row}\n"));
                } else {
                    output.push_str(line);
                    output.push('\n');
                }
            }
            patch = output;
        }
        std::fs::write(&patch_path, patch).unwrap();
        let output = std::process::Command::new("D:\\deepseek harness\\DeepSeek Harness.exe")
            .env("ELECTRON_RUN_AS_NODE", "1").env("DSH_HOME", &home)
            .args(["--expose-internals", "D:\\deepseek harness\\resources\\app.asar\\dsh\\node_modules\\@deepseek-ai\\dsh-desktop-host\\lib\\cli.js", "--profile", "headless", "--patch"])
            .arg(&patch_path).arg("Reply with HARNESS_CONNECTED only. Do not use tools.").output().unwrap();
        evidence.push(json!({"check":"installed-harness-headless","exitCode":output.status.code(),"stdout":String::from_utf8_lossy(&output.stdout),"stderr":String::from_utf8_lossy(&output.stderr)}));
        let report = json!({"time":chrono::Utc::now().to_rfc3339(),"evidence":evidence});
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../.tmp/harness-real-acceptance-{}.json", ids[0])),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .unwrap();
        assert_eq!(
            report["evidence"].as_array().unwrap().last().unwrap()["exitCode"],
            0,
            "Harness must complete successfully; inspect evidence"
        );
        assert!(
            report["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["check"] == "real-conversation")
                .all(|r| r["status"] == 200
                    && r["error"].is_null()
                    && r["completion"]["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or("")
                        .contains("ORANGE739")),
            "inspect .tmp/harness-real-acceptance.json"
        );
    }
}

//! Loopback OpenAI-compatible pool for the verified SOLO message endpoint.
//! The official client owns conversation history and executes its own tools.
use super::trae_pool_protocol::{self as protocol, FrameAction, StreamGuard};
use crate::models::trae::TraeAccount;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, Read};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

pub(crate) const PORT: u16 = 18790;
const ORIGIN: &str = "https://trae-api-cn.mchost.guru";
const CATALOG_PATH: &str = "/api/ide/v1/get_detail_param";
const CHAT_PATH: &str = "/api/agent/v3/llm_utils_chat";
const PREFIX: &str = "trae-pool-";
const BODY_LIMIT: usize = 64 * 1024 * 1024;
static SERVICE: LazyLock<Mutex<Option<Arc<Pool>>>> = LazyLock::new(|| Mutex::new(None));
static CONFIG_LOCK: Mutex<()> = Mutex::new(());
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Selection {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    model_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub name: String,
    pub multimodal: bool,
    pub tools: Option<bool>,
    pub input_limit: Option<u64>,
    pub output_limit: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsView {
    pub models: Vec<Model>,
    pub selected_model_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationResult {
    pub registered: Vec<String>,
    pub already_registered: Vec<String>,
    pub endpoint: String,
}

fn registration_payload(model: &Model, local_key: &str) -> Value {
    let mut limits = serde_json::Map::new();
    if let Some(n) = model.input_limit.filter(|n| *n > 0) {
        limits.insert("prompt_max_tokens".into(), json!(n));
    }
    if let Some(n) = model.output_limit.filter(|n| *n > 0) {
        limits.insert("max_tokens".into(), json!(n));
    }
    json!({"provider":"custom_openai_compatible","model_name":format!("{PREFIX}{}",model.id),"is_custom":true,"auth_type":0,"ak":local_key,
        "base_url":format!("http://127.0.0.1:{PORT}/v1/chat/completions"),"display_name":format!("TRAE 账号池 · {}",model.name),
        "config_detail":{"multimodal":model.multimodal,"model_hyper_params":limits,"max_turn":500}})
}

fn manager_models(
    account: &TraeAccount,
    http: &reqwest::blocking::Client,
) -> Result<HashMap<String, Value>, String> {
    let response = http
        .get(format!("{ORIGIN}/api/ide/v1/model_list?type=llm_raw_chat"))
        .headers(headers(account, false)?)
        .send()
        .map_err(|_| "无法读取 TRAE 自定义模型列表")?;
    if !response.status().is_success() {
        return Err(format!(
            "读取 TRAE 自定义模型失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    let body: Value = response.json().map_err(|_| "TRAE 自定义模型列表格式无效")?;
    if body
        .get("code")
        .and_then(Value::as_i64)
        .is_some_and(|c| c != 0)
    {
        return Err("TRAE 拒绝读取自定义模型列表，请重新登录".into());
    }
    fn visit(value: &Value, out: &mut HashMap<String, Value>, depth: usize) {
        if depth > 20 {
            return;
        }
        match value {
            Value::Object(object) => {
                let name = object
                    .get("model_name")
                    .or_else(|| object.get("name"))
                    .and_then(Value::as_str);
                if let Some(name) = name.filter(|s| s.starts_with(PREFIX)) {
                    if object.get("provider").and_then(Value::as_str)
                        == Some("custom_openai_compatible")
                    {
                        out.insert(name.into(), value.clone());
                    }
                }
                for child in object.values() {
                    visit(child, out, depth + 1);
                }
            }
            Value::Array(items) => {
                for child in items {
                    visit(child, out, depth + 1);
                }
            }
            _ => {}
        }
    }
    let mut found = HashMap::new();
    visit(&body, &mut found, 0);
    Ok(found)
}

/// Called only by an explicit registration action in the settings UI.
/// Account credentials stay in memory; only a loopback-only API key is registered.
pub fn register_models(account_id: &str) -> Result<RegistrationResult, String> {
    let _lock = CONFIG_LOCK.lock().map_err(|_| "模型注册锁异常")?;
    let current = status()?;
    if !current.running || current.stopping {
        return Err("请先保存模型并启动账号池".into());
    }
    let account = accounts()?
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or("注册目标账号不存在")?;
    let http = client()?;
    let available = account_models(&account, &http, ORIGIN)?;
    let config = selection()?;
    let local_key = key()?;
    let mut remote = manager_models(&account, &http)?;
    let receipt_path = data_path("trae-model-pool-registration.json")?;
    let mut receipts: HashMap<String, String> = match std::fs::read_to_string(&receipt_path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|_| "模型注册记录损坏，未覆盖")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(_) => return Err("无法读取模型注册记录".into()),
    };
    let mut result = RegistrationResult {
        registered: Vec::new(),
        already_registered: Vec::new(),
        endpoint: current.endpoint,
    };
    for id in config.model_ids {
        let model = available
            .iter()
            .find(|m| m.id == id)
            .ok_or("所选模型不属于当前注册账号，请刷新目录")?;
        let mut payload = registration_payload(model, &local_key);
        let digest = format!("{:x}", Sha256::digest(payload.to_string().as_bytes()));
        let receipt_key = format!("{}:{id}", account.id);
        let wire_name = format!("{PREFIX}{id}");
        let path = if let Some(existing) = remote.get(&wire_name) {
            if receipts.get(&receipt_key) == Some(&digest) {
                result.already_registered.push(id);
                continue;
            }
            let existing_id = existing
                .get("custom_model_id")
                .or_else(|| existing.get("id"))
                .filter(|v| v.is_number() || v.is_string())
                .ok_or("已存在同名账号池模型，但无法识别其 ID，未重复创建")?;
            let base = existing
                .get("base_url")
                .and_then(Value::as_str)
                .unwrap_or("");
            if !receipts.contains_key(&receipt_key)
                && base != payload["base_url"].as_str().unwrap_or("")
            {
                return Err("存在不属于本机的同名模型，未覆盖；请在 TRAE 中检查该模型".into());
            }
            payload["id"] = existing_id.clone();
            payload["action"] = json!("update");
            "/api/ide/v1/update_custom_model"
        } else {
            "/api/ide/v1/add_custom_model"
        };
        let response = http
            .post(format!("{ORIGIN}{path}"))
            .headers(headers(&account, false)?)
            .json(&payload)
            .send()
            .map_err(|_| "模型注册请求未确认，请刷新 TRAE 模型列表后重试")?;
        if !response.status().is_success() {
            return Err(format!(
                "TRAE 模型注册失败（HTTP {}）",
                response.status().as_u16()
            ));
        }
        let response: Value = response
            .json()
            .map_err(|_| "模型注册响应无效，请刷新模型列表确认")?;
        if response
            .get("code")
            .and_then(Value::as_i64)
            .is_some_and(|c| c != 0)
            || response.get("error").is_some()
        {
            return Err("TRAE 拒绝注册模型，请检查账号权限和模型配置".into());
        }
        remote = manager_models(&account, &http)?;
        if !remote.contains_key(&wire_name) {
            return Err("注册请求已提交，但云端模型列表尚未确认该条目，请稍后刷新后重试".into());
        }
        receipts.insert(receipt_key, digest);
        super::atomic_write::write_string_atomic(
            &receipt_path,
            &serde_json::to_string_pretty(&receipts).map_err(|_| "注册记录序列化失败")?,
        )?;
        result.registered.push(id);
    }
    Ok(result)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub running: bool,
    pub stopping: bool,
    pub endpoint: String,
    pub requests: u64,
    pub rotations: u64,
    pub active_requests: usize,
    pub cooling_accounts: usize,
    pub selected_model_ids: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Default)]
struct Runtime {
    requests: u64,
    rotations: u64,
    preferred: Option<String>,
    cooldowns: HashMap<String, Instant>,
    last_error: Option<String>,
}

struct Pool {
    server: Arc<Server>,
    key: String,
    selected: Mutex<Vec<String>>,
    state: Mutex<Runtime>,
    stopping: AtomicBool,
    active: AtomicUsize,
}

struct ActiveRequest(Arc<Pool>);
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

fn data_path(name: &str) -> Result<std::path::PathBuf, String> {
    Ok(super::config::get_data_dir()?.join(name))
}
fn selection() -> Result<Selection, String> {
    let path = data_path("trae-model-pool.json")?;
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|_| "TRAE 账号池配置损坏，未覆盖".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Selection::default()),
        Err(_) => Err("无法读取 TRAE 账号池配置".into()),
    }
}
fn save_selection(value: &Selection) -> Result<(), String> {
    super::atomic_write::write_string_atomic(
        &data_path("trae-model-pool.json")?,
        &serde_json::to_string_pretty(value).map_err(|_| "账号池设置序列化失败")?,
    )
}
fn key() -> Result<String, String> {
    let path = data_path("trae-model-pool.key")?;
    match std::fs::read_to_string(&path) {
        Ok(value) if uuid::Uuid::parse_str(value.trim()).is_ok() => Ok(value.trim().into()),
        Ok(_) => Err("本地账号池密钥格式无效，未覆盖".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let value = uuid::Uuid::new_v4().to_string();
            super::atomic_write::write_string_atomic(&path, &value)?;
            Ok(value)
        }
        Err(_) => Err("无法读取本地账号池密钥".into()),
    }
}

fn accounts() -> Result<Vec<TraeAccount>, String> {
    Ok(super::trae_account::list_accounts_checked()?
        .into_iter()
        .filter(|a| {
            super::trae_account::resolve_account_platform_kind(a).is_cn()
                && !a.access_token.trim().is_empty()
        })
        .collect())
}

fn headers(account: &TraeAccount, stream: bool) -> Result<reqwest::header::HeaderMap, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let mut h = HeaderMap::new();
    let token = account.access_token.as_str();
    for (k, v) in [
        ("authorization", format!("Cloud-IDE-JWT {token}")),
        ("x-cloudide-token", token.into()),
        ("x-ide-token", token.into()),
        ("content-type", "application/json".into()),
        (
            "accept",
            if stream {
                "text/event-stream"
            } else {
                "application/json"
            }
            .into(),
        ),
        ("user-agent", "Trae/0.1.61".into()),
        ("x-app-id", "6eefa01c-1036-4c7e-9ca5-d891f63bfcd8".into()),
        ("x-app-version", "default".into()),
        ("x-ide-version", "0.1.61".into()),
        ("x-ide-version-code", "20260820".into()),
        ("x-app-version-code", "20260820".into()),
        ("x-ide-version-type", "stable".into()),
        ("x-device-type", "windows".into()),
        ("request-traffic-type", "prod".into()),
        (
            "x-device-id",
            account.auth_device_id.clone().unwrap_or_default(),
        ),
        (
            "x-machine-id",
            account.machine_id.clone().unwrap_or_default(),
        ),
        ("x-uid", account.user_id.clone().unwrap_or_default()),
    ] {
        if !v.is_empty() {
            h.insert(
                HeaderName::from_bytes(k.as_bytes()).map_err(|_| "请求头名称无效")?,
                HeaderValue::from_str(&v).map_err(|_| "账号身份字段包含无效字符")?,
            );
        }
    }
    Ok(h)
}

fn client() -> Result<reqwest::blocking::Client, String> {
    client_for(ORIGIN)
}
fn client_for(origin: &str) -> Result<reqwest::blocking::Client, String> {
    let url = reqwest::Url::parse(origin).map_err(|_| "上游地址无效")?;
    crate::utils::http::create_domestic_blocking_client(
        180,
        url.host_str().ok_or("上游缺少主机名")?,
    )
}

fn parse_models(data: &Value) -> Result<Vec<Model>, String> {
    let rows = data
        .get("config_info_list")
        .and_then(Value::as_array)
        .ok_or("TRAE 模型目录格式无效")?;
    let mut models = Vec::new();
    for row in rows {
        let Some(id) = row
            .get("config_name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if row["is_invisible_to_user"].as_bool() == Some(true)
            || row["config_switch"].as_bool() == Some(false)
        {
            continue;
        }
        let Some(name) = row
            .pointer("/display_config/display_name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        else {
            continue;
        };
        if id.starts_with("custom_model_")
            || matches!(
                id,
                "fast_apply" | "fast_apply_new" | "context_selection" | "summary"
            )
        {
            continue;
        }
        let extra = row
            .get("extra_config")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str::<Value>(s).ok());
        let output_limit = row
            .get("model_detail_list")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| d.get("max_tokens").and_then(Value::as_u64))
            .filter(|n| *n > 0)
            .min();
        if models.iter().any(|m: &Model| m.id == id) {
            continue;
        }
        models.push(Model {
            id: id.into(),
            name: name.into(),
            multimodal: row
                .pointer("/display_config/multimodal")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            tools: extra
                .as_ref()
                .and_then(|v| v.get("native_function_call"))
                .and_then(Value::as_bool),
            input_limit: row
                .pointer("/context_window_tokens/dev")
                .and_then(Value::as_u64),
            output_limit,
        });
    }
    Ok(models)
}

fn account_models(
    account: &TraeAccount,
    http: &reqwest::blocking::Client,
    origin: &str,
) -> Result<Vec<Model>, String> {
    let response=http.post(format!("{origin}{CATALOG_PATH}")).headers(headers(account,false)?)
        .json(&json!({"function":"solo_work_lite","config_names":null,"need_prompt":false,"current_config_info":null,"poly_prompt":true,"mode_type":null,"agent_type":null}))
        .send().map_err(|_|"无法连接 TRAE 模型目录")?;
    if !response.status().is_success() {
        return Err(format!(
            "模型目录请求失败（HTTP {}）",
            response.status().as_u16()
        ));
    }
    let data: Value = response.json().map_err(|_| "模型目录未返回有效 JSON")?;
    parse_models(&data)
}

pub fn models() -> Result<ModelsView, String> {
    let http = client()?;
    let mut found = Vec::<Model>::new();
    for account in accounts()? {
        if let Ok(items) = account_models(&account, &http, ORIGIN) {
            for item in items {
                if !found.iter().any(|m| m.id == item.id) {
                    found.push(item)
                }
            }
        }
    }
    if found.is_empty() {
        return Err("账号未返回可用对话模型，请检查网络或重新登录".into());
    }
    Ok(ModelsView {
        models: found,
        selected_model_ids: selection()?.model_ids,
    })
}

pub fn save_models(ids: Vec<String>) -> Result<ModelsView, String> {
    let _lock = CONFIG_LOCK.lock().map_err(|_| "账号池配置锁异常")?;
    let catalog = models()?;
    let mut unique = Vec::new();
    for id in ids {
        if !catalog.models.iter().any(|m| m.id == id) {
            return Err("所选模型已不在当前账号目录，未保存".into());
        }
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    let previous = selection()?;
    save_selection(&Selection {
        enabled: previous.enabled && !unique.is_empty(),
        model_ids: unique.clone(),
    })?;
    if let Some(pool) = SERVICE.lock().map_err(|_| "账号池状态锁异常")?.as_ref() {
        *pool.selected.lock().map_err(|_| "模型选择锁异常")? = unique.clone();
    }
    Ok(ModelsView {
        models: catalog.models,
        selected_model_ids: unique,
    })
}

pub fn status() -> Result<Status, String> {
    let service = SERVICE.lock().map_err(|_| "账号池状态锁异常")?;
    let endpoint = format!("http://127.0.0.1:{PORT}/v1");
    if let Some(pool) = service.as_ref() {
        let state = pool.state.lock().map_err(|_| "账号池状态锁异常")?;
        Ok(Status {
            running: true,
            stopping: pool.stopping.load(Ordering::SeqCst),
            endpoint,
            requests: state.requests,
            rotations: state.rotations,
            active_requests: pool.active.load(Ordering::SeqCst),
            cooling_accounts: state
                .cooldowns
                .values()
                .filter(|t| **t > Instant::now())
                .count(),
            selected_model_ids: pool.selected.lock().map_err(|_| "模型选择锁异常")?.clone(),
            last_error: state.last_error.clone(),
        })
    } else {
        Ok(Status {
            running: false,
            stopping: false,
            endpoint,
            requests: 0,
            rotations: 0,
            active_requests: 0,
            cooling_accounts: 0,
            selected_model_ids: selection()?.model_ids,
            last_error: LAST_ERROR.lock().ok().and_then(|e| e.clone()),
        })
    }
}

fn reply(request: Request, status: u16, message: &str) {
    let _ = request.respond(
        Response::from_string(
            json!({"error":{"type":"trae_pool_error","message":message}}).to_string(),
        )
        .with_status_code(StatusCode(status))
        .with_header(
            Header::from_bytes("Content-Type", "application/json").expect("static header"),
        ),
    );
}

pub fn start() -> Result<Status, String> {
    let _config = CONFIG_LOCK.lock().map_err(|_| "账号池配置锁异常")?;
    let config = selection()?;
    if config.model_ids.is_empty() {
        return Err("请先添加并保存账号池模型".into());
    }
    if accounts()?.is_empty() {
        return Err("请先导入 TRAE 国内版账号".into());
    }
    let mut service = SERVICE.lock().map_err(|_| "账号池状态锁异常")?;
    if let Some(pool) = service.as_ref() {
        if pool.stopping.load(Ordering::SeqCst) {
            return Err("账号池正在等待请求结束，请稍后启动".into());
        }
        drop(service);
        return status();
    }
    let server = Arc::new(
        Server::http(("127.0.0.1", PORT))
            .map_err(|_| format!("本地端口 {PORT} 已被占用，未修改客户端"))?,
    );
    let pool = Arc::new(Pool {
        server,
        key: key()?,
        selected: Mutex::new(config.model_ids.clone()),
        state: Mutex::new(Runtime::default()),
        stopping: AtomicBool::new(false),
        active: AtomicUsize::new(0),
    });
    save_selection(&Selection {
        enabled: true,
        ..config
    })?;
    *service = Some(pool.clone());
    drop(service);
    let dispatcher = std::thread::Builder::new()
        .name("trae-model-pool".into())
        .spawn(move || {
            while !pool.stopping.load(Ordering::SeqCst) {
                match pool.server.recv_timeout(Duration::from_millis(250)) {
                    Ok(Some(request)) => {
                        if pool.stopping.load(Ordering::SeqCst) {
                            reply(request, 503, "账号池正在停止");
                            break;
                        }
                        if pool.active.load(Ordering::SeqCst) >= 8 {
                            reply(request, 429, "本地账号池并发已满，请稍后重试");
                            continue;
                        }
                        pool.active.fetch_add(1, Ordering::SeqCst);
                        let worker = pool.clone();
                        let active = ActiveRequest(pool.clone());
                        let _ = std::thread::Builder::new()
                            .name("trae-model-request".into())
                            .spawn(move || {
                                let _active = active;
                                let outcome =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        handle(request, &worker)
                                    }));
                                if outcome.is_err() {
                                    if let Ok(mut state) = worker.state.lock() {
                                        state.last_error = Some("请求处理异常，未自动重放".into());
                                    }
                                }
                            });
                    }
                    Ok(None) => {}
                    Err(_) => {
                        if let Ok(mut error) = LAST_ERROR.lock() {
                            *error = Some("账号池监听退出，请重新启动".into());
                        }
                        break;
                    }
                }
            }
            while pool.active.load(Ordering::SeqCst) > 0 {
                std::thread::sleep(Duration::from_millis(100));
            }
            if let Ok(mut slot) = SERVICE.lock() {
                if slot.as_ref().is_some_and(|p| Arc::ptr_eq(p, &pool)) {
                    slot.take();
                }
            }
        });
    if dispatcher.is_err() {
        if let Ok(mut slot) = SERVICE.lock() {
            slot.take();
        }
        let mut config = selection()?;
        config.enabled = false;
        save_selection(&config)?;
        return Err("无法创建账号池工作线程，未继续启动".into());
    }
    if let Ok(mut error) = LAST_ERROR.lock() {
        *error = None;
    }
    status()
}

pub fn stop() -> Result<Status, String> {
    let _config = CONFIG_LOCK.lock().map_err(|_| "账号池配置锁异常")?;
    let mut config = selection()?;
    config.enabled = false;
    save_selection(&config)?;
    if let Some(pool) = SERVICE.lock().map_err(|_| "账号池状态锁异常")?.as_ref() {
        pool.stopping.store(true, Ordering::SeqCst);
        pool.server.unblock();
    }
    status()
}

pub async fn restore() {
    if selection().is_ok_and(|s| s.enabled) {
        if let Ok(Err(error)) = tauri::async_runtime::spawn_blocking(start).await {
            if let Ok(mut last) = LAST_ERROR.lock() {
                *last = Some(format!("账号池自动恢复失败：{error}"));
            }
        }
    }
}

fn handle(mut request: Request, pool: &Pool) {
    if !request.headers().iter().any(|h| {
        h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {}", pool.key)
    }) {
        reply(request, 401, "本地账号池密钥无效");
        return;
    }
    if request.method() == &Method::Get && matches!(request.url(), "/v1/models" | "/models") {
        let ids = match pool.selected.lock() {
            Ok(s) => s.clone(),
            Err(_) => {
                reply(request, 503, "模型选择不可用");
                return;
            }
        };
        let data:Vec<_>=ids.iter().map(|id|json!({"id":format!("{PREFIX}{id}"),"object":"model","owned_by":"local-trae-pool"})).collect();
        let _ = request.respond(
            Response::from_string(json!({"object":"list","data":data}).to_string()).with_header(
                Header::from_bytes("Content-Type", "application/json").expect("static header"),
            ),
        );
        return;
    }
    if request.method() != &Method::Post
        || !matches!(request.url(), "/v1/chat/completions" | "/chat/completions")
    {
        reply(request, 404, "接口不存在");
        return;
    }
    let mut bytes = Vec::new();
    if request
        .as_reader()
        .take((BODY_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        reply(request, 400, "读取请求失败");
        return;
    }
    if bytes.len() > BODY_LIMIT {
        reply(request, 413, "请求超过 64 MiB，未截断上下文");
        return;
    }
    let input: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            reply(request, 400, "请求不是有效 JSON");
            return;
        }
    };
    let requested = input.get("model").and_then(Value::as_str).unwrap_or_default();
    let normalized = requested.rsplit_once("//").map_or(requested, |(_, tail)| tail);
    let Some(model) = normalized.strip_prefix(PREFIX).map(str::to_owned) else {
        reply(
            request,
            400,
            &format!("请选择 trae-pool- 账号池模型（收到：{requested}）"),
        );
        return;
    };
    if !pool.selected.lock().is_ok_and(|s| s.contains(&model)) {
        reply(request, 400, "该模型未添加到账号池");
        return;
    }
    let body = match protocol::build_solo_request(&input, &model) {
        Ok(v) => v,
        Err(e) => {
            reply(request, 400, &e);
            return;
        }
    };
    let available = match accounts() {
        Ok(v) => v,
        Err(_) => {
            reply(request, 503, "无法读取加密账号库");
            return;
        }
    };
    relay(request, pool, &input, body, &model, available, ORIGIN);
}

fn relay(
    request: Request,
    pool: &Pool,
    input: &Value,
    body: Value,
    model: &str,
    mut candidates: Vec<TraeAccount>,
    origin: &str,
) {
    let http = match client_for(origin) {
        Ok(c) => c,
        Err(_) => {
            reply(request, 503, "无法创建上游连接");
            return;
        }
    };
    let preferred = pool.state.lock().ok().and_then(|s| s.preferred.clone());
    candidates.sort_by_key(|a| preferred.as_deref() != Some(&a.id));
    if let Ok(mut s) = pool.state.lock() {
        s.requests += 1;
    }
    for account in candidates {
        if pool.stopping.load(Ordering::SeqCst) {
            reply(request, 503, "账号池正在停止，未继续轮换");
            return;
        }
        if pool.state.lock().is_ok_and(|s| {
            s.cooldowns
                .get(&account.id)
                .is_some_and(|t| *t > Instant::now())
        }) {
            continue;
        }
        if crate::utils::jwt::parse_jwt_exp(&account.access_token)
            .is_some_and(|exp| exp <= chrono::Utc::now().timestamp())
        {
            continue;
        }
        let catalog = match account_models(&account, &http, origin) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Some(capability) = catalog.iter().find(|m| m.id == model) else {
            continue;
        };
        if capability.tools == Some(false)
            && input
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty())
        {
            continue;
        }
        let response = match http
            .post(format!("{origin}{CHAT_PATH}"))
            .headers(match headers(&account, true) {
                Ok(h) => h,
                Err(_) => continue,
            })
            .json(&body)
            .send()
        {
            Ok(r) => r,
            Err(_) => {
                reply(request, 502, "上游连接失败，未自动重放可能已处理的请求");
                return;
            }
        };
        let status = response.status().as_u16();
        if status == 401 || status == 402 || status == 429 {
            cool(pool, &account.id, if status == 429 { 60 } else { 300 });
            continue;
        }
        if !response.status().is_success() {
            reply(request, status, "上游拒绝请求，未换号重放");
            return;
        }
        let mut reader = BufReader::new(response);
        let mut guard = StreamGuard::default();
        let mut prefix = None;
        let mut blocked = false;
        loop {
            let frame = match protocol::read_frame(&mut reader) {
                Ok(Some(f)) => f,
                Ok(None) => {
                    reply(request, 502, "上游未输出内容就已断流，未重放");
                    return;
                }
                Err(e) => {
                    reply(request, 502, &e);
                    return;
                }
            };
            match guard.inspect(&frame) {
                Ok(FrameAction::Rotate) => {
                    cool(pool, &account.id, 6 * 3600);
                    blocked = true;
                    break;
                }
                Ok(FrameAction::Reject) => {
                    reply(
                        request,
                        502,
                        "TRAE 返回模型或请求错误，未自动换号；请刷新模型目录",
                    );
                    return;
                }
                Ok(FrameAction::Output(v)) => {
                    prefix = Some(v);
                    break;
                }
                Ok(FrameAction::Done(_)) => {
                    reply(request, 502, "上游返回空结果，未伪装成功或自动重放");
                    return;
                }
                Ok(_) => {}
                Err(e) => {
                    reply(request, 502, &e);
                    return;
                }
            }
        }
        if blocked {
            continue;
        }
        if let Ok(mut s) = pool.state.lock() {
            s.preferred = Some(account.id.clone());
            s.last_error = None;
        }
        let stream = SoloReader::new(
            reader,
            guard,
            format!("{PREFIX}{model}"),
            prefix.unwrap_or(Value::Null),
            input.get("stream").and_then(Value::as_bool) != Some(true)
                || input
                    .pointer("/stream_options/include_usage")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
        );
        if input.get("stream").and_then(Value::as_bool) == Some(true) {
            let _ = request.respond(Response::new(
                StatusCode(200),
                vec![
                    Header::from_bytes("Content-Type", "text/event-stream").expect("static header"),
                    Header::from_bytes("Cache-Control", "no-cache").expect("static header"),
                ],
                stream,
                None,
                None,
            ));
        } else {
            match collect(stream) {
                Ok(body) => {
                    let _ = request.respond(
                        Response::from_string(body.to_string()).with_header(
                            Header::from_bytes("Content-Type", "application/json")
                                .expect("static header"),
                        ),
                    );
                }
                Err(e) => reply(request, 502, &e),
            }
        }
        return;
    }
    if let Ok(mut s) = pool.state.lock() {
        s.last_error = Some("全部账号暂不可用、额度不足或没有所选模型".into());
    }
    reply(
        request,
        503,
        "全部账号暂不可用、额度不足或没有所选模型，请刷新积分或重新登录",
    );
}

fn cool(pool: &Pool, id: &str, seconds: u64) {
    if let Ok(mut s) = pool.state.lock() {
        s.cooldowns
            .insert(id.into(), Instant::now() + Duration::from_secs(seconds));
        s.rotations += 1;
    }
}

struct SoloReader<R: std::io::BufRead> {
    reader: R,
    guard: StreamGuard,
    model: String,
    id: String,
    created: i64,
    pending: std::io::Cursor<Vec<u8>>,
    finished: bool,
    failed: bool,
    tools: bool,
    include_usage: bool,
    tool_ids: HashMap<u64, String>,
}
impl<R: std::io::BufRead> SoloReader<R> {
    fn new(
        reader: R,
        guard: StreamGuard,
        model: String,
        first: Value,
        include_usage: bool,
    ) -> Self {
        let mut s = Self {
            reader,
            guard,
            model,
            id: format!("chatcmpl-{}", uuid::Uuid::new_v4().simple()),
            created: chrono::Utc::now().timestamp(),
            pending: std::io::Cursor::new(Vec::new()),
            finished: false,
            failed: false,
            tools: false,
            include_usage,
            tool_ids: HashMap::new(),
        };
        let mut initial = s.chunk(json!({"role":"assistant"}), None);
        match s.delta(&first) {
            Ok(delta) => {
                s.tools = delta.get("tool_calls").is_some();
                initial.extend(s.chunk(delta, None));
            }
            Err(_) => {
                s.failed = true;
            }
        }
        s.pending = std::io::Cursor::new(initial);
        s
    }
    fn chunk(&self, delta: Value, finish: Option<&str>) -> Vec<u8> {
        format!("data: {}\n\n",json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,"choices":[{"index":0,"delta":delta,"finish_reason":finish}]})).into_bytes()
    }
    fn delta(&mut self, value: &Value) -> Result<Value, String> {
        let mut delta = protocol::solo_delta(value)?;
        if let Some(calls) = delta.get_mut("tool_calls").and_then(Value::as_array_mut) {
            for call in calls {
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                let first = !self.tool_ids.contains_key(&index);
                let id = self
                    .tool_ids
                    .entry(index)
                    .or_insert_with(|| {
                        call.get("id")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("call_{}", uuid::Uuid::new_v4().simple()))
                    })
                    .clone();
                if first {
                    call["id"] = json!(id);
                    call["type"] = json!("function");
                } else if let Some(object) = call.as_object_mut() {
                    object.remove("id");
                    object.remove("type");
                }
            }
        }
        Ok(delta)
    }
    fn next(&mut self) -> Result<Vec<u8>, String> {
        loop {
            let Some(frame) = protocol::read_frame(&mut self.reader)? else {
                return Err("上游提前断流，当前请求未重放".into());
            };
            match self.guard.inspect(&frame)? {
                FrameAction::Output(v) => {
                    let delta = self.delta(&v)?;
                    self.tools |= delta.get("tool_calls").is_some();
                    if delta.as_object().is_some_and(|o| !o.is_empty()) {
                        return Ok(self.chunk(delta, None));
                    }
                }
                FrameAction::Usage(v) => {
                    if self.include_usage {
                        let mut usage = serde_json::Map::new();
                        for k in ["prompt_tokens", "completion_tokens", "total_tokens"] {
                            if let Some(n) = v.get(k).and_then(Value::as_u64) {
                                usage.insert(k.into(), json!(n));
                            }
                        }
                        if let Some(n) = v.get("cache_read_input_tokens").and_then(Value::as_u64) {
                            usage
                                .insert("prompt_tokens_details".into(), json!({"cached_tokens":n}));
                        }
                        return Ok(format!("data: {}\n\n",json!({"id":self.id,"object":"chat.completion.chunk","created":self.created,"model":self.model,"choices":[],"usage":usage})).into_bytes());
                    }
                }
                FrameAction::Done(reason) => {
                    self.guard.finish()?;
                    self.finished = true;
                    let reason = if self.tools {
                        "tool_calls"
                    } else {
                        match reason.as_str() {
                            "max_tokens" => "length",
                            "end_turn" => "stop",
                            other => other,
                        }
                    };
                    let mut bytes = self.chunk(json!({}), Some(reason));
                    bytes.extend_from_slice(b"data: [DONE]\n\n");
                    return Ok(bytes);
                }
                FrameAction::Reject | FrameAction::Rotate => {
                    return Err("TRAE 输出后返回错误，当前请求未重放".into())
                }
                FrameAction::Metadata => {}
            }
        }
    }
}
impl<R: std::io::BufRead> Read for SoloReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            let n = self.pending.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            if self.failed {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "TRAE stream failed",
                ));
            }
            if self.finished {
                return Ok(0);
            }
            match self.next() {
                Ok(bytes) => self.pending = std::io::Cursor::new(bytes),
                Err(_) => {
                    self.failed = true;
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "TRAE stream failed; request was not replayed",
                    ));
                }
            }
        }
    }
}

fn collect(reader: impl Read) -> Result<Value, String> {
    let mut reader = BufReader::new(reader);
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut calls = BTreeMap::<u64, Value>::new();
    let mut result = json!({"object":"chat.completion","choices":[]});
    let mut finish = None;
    while let Some(frame) = protocol::read_frame(&mut reader)? {
        if frame.data == "[DONE]" {
            break;
        }
        let chunk: Value = serde_json::from_str(&frame.data).map_err(|_| "无效的本地流响应")?;
        for k in ["id", "created", "model", "usage"] {
            if let Some(v) = chunk.get(k) {
                result[k] = v.clone();
            }
        }
        if let Some(choice) = chunk.pointer("/choices/0") {
            if let Some(s) = choice.pointer("/delta/content").and_then(Value::as_str) {
                text.push_str(s);
            }
            if let Some(s) = choice
                .pointer("/delta/reasoning_content")
                .and_then(Value::as_str)
            {
                reasoning.push_str(s);
            }
            if let Some(items) = choice
                .pointer("/delta/tool_calls")
                .and_then(Value::as_array)
            {
                for item in items {
                    let index = item.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let target = calls.entry(index).or_insert_with(
                        || json!({"type":"function","function":{"name":"","arguments":""}}),
                    );
                    for key in ["id", "type"] {
                        if let Some(v) = item
                            .get(key)
                            .filter(|v| v.as_str().is_some_and(|s| !s.is_empty()))
                        {
                            target[key] = v.clone();
                        }
                    }
                    for key in ["name", "arguments"] {
                        if let Some(s) = item
                            .pointer(&format!("/function/{key}"))
                            .and_then(Value::as_str)
                        {
                            let old = target["function"][key].as_str().unwrap_or("");
                            target["function"][key] = json!(format!("{old}{s}"));
                        }
                    }
                }
            }
            if let Some(s) = choice.get("finish_reason").and_then(Value::as_str) {
                finish = Some(s.to_string());
            }
        }
    }
    let reason = finish.ok_or("对话未正常完成，未返回伪造成功结果")?;
    let mut message = json!({"role":"assistant","content":text});
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    if !calls.is_empty() {
        message["tool_calls"] = json!(calls.into_values().collect::<Vec<_>>());
    }
    result["choices"] = json!([{"index":0,"message":message,"finish_reason":reason}]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_catalog_excludes_internal_and_placeholder_models() {
        let data = json!({"config_info_list":[{"config_name":"a","display_config":{"display_name":"A","multimodal":true},"extra_config":"{\"native_function_call\":true}","model_detail_list":[{"max_tokens":64}]},{"config_name":"secret","is_invisible_to_user":true,"display_config":{"display_name":"Secret"}},{"config_name":"custom_model_placeholder","display_config":{"display_name":"Template"}}]});
        let models = parse_models(&data).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "a");
        assert_eq!(models[0].tools, Some(true));
        assert_eq!(models[0].output_limit, Some(64));
    }
    #[test]
    fn native_registration_targets_only_loopback_and_preserves_model_identity() {
        let model = Model {
            id: "example/Model".into(),
            name: "Example".into(),
            multimodal: true,
            tools: Some(true),
            input_limit: Some(32000),
            output_limit: Some(4096),
        };
        let payload = registration_payload(&model, "local-key-fixture");
        assert_eq!(payload["model_name"], "trae-pool-example/Model");
        assert_eq!(
            payload["base_url"],
            "http://127.0.0.1:18790/v1/chat/completions"
        );
        assert_eq!(
            payload["config_detail"]["model_hyper_params"]["max_tokens"],
            4096
        );
        assert!(payload.get("access_token").is_none());
        assert_eq!(payload["provider"], "custom_openai_compatible");
    }
    #[test]
    fn streamed_utf8_text_usage_and_finish_are_valid_openai() {
        let input="event:output\ndata:{\"response\":\"世界\"}\n\nevent:token_usage\ndata:{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}\n\nevent:done\ndata:{\"finish_reason\":\"stop\"}\n\n";
        let mut guard = StreamGuard::default();
        guard.output_started = true;
        let reader = SoloReader::new(
            BufReader::with_capacity(1, std::io::Cursor::new(input.as_bytes())),
            guard,
            "model".into(),
            json!({"response":"你好"}),
            true,
        );
        let result = collect(reader).unwrap();
        assert_eq!(result["choices"][0]["message"]["content"], "你好世界");
        assert_eq!(result["usage"]["total_tokens"], 5);
        assert_eq!(result["choices"][0]["finish_reason"], "stop");
    }
    #[test]
    fn disconnected_stream_cannot_be_collected_as_success() {
        let reader = SoloReader::new(
            BufReader::new(std::io::Cursor::new(Vec::<u8>::new())),
            StreamGuard::default(),
            "model".into(),
            json!({"response":"partial"}),
            false,
        );
        assert!(collect(reader).is_err());
    }
    #[test]
    fn empty_later_tool_ids_cannot_destroy_call_result_correlation() {
        let tail="event:output\ndata:{\"tool_calls\":[{\"index\":0,\"id\":\"\",\"type\":\"\",\"function_call\":{\"arguments\":\":1}\"}}]}\n\nevent:done\ndata:{\"finish_reason\":\"tool_calls\"}\n\n";
        let first = json!({"tool_calls":[{"index":0,"id":"call-a","type":"function","function_call":{"name":"echo","arguments":"{\"value\""}}]});
        let reader = SoloReader::new(
            BufReader::new(std::io::Cursor::new(tail.as_bytes())),
            StreamGuard::default(),
            "model".into(),
            first,
            false,
        );
        let result = collect(reader).unwrap();
        let call = &result["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], "call-a");
        assert_eq!(call["type"], "function");
        assert_eq!(call["function"]["arguments"], "{\"value\":1}");
    }

    fn fixture_account(id: &str) -> TraeAccount {
        serde_json::from_value(json!({"id":id,"email":"test@example.invalid","access_token":id,"created_at":0,"last_used":0})).unwrap()
    }
    fn fixture_pool() -> Arc<Pool> {
        Arc::new(Pool {
            server: Arc::new(Server::http("127.0.0.1:0").unwrap()),
            key: "fixture-key".into(),
            selected: Mutex::new(vec!["model".into()]),
            state: Mutex::new(Runtime::default()),
            stopping: AtomicBool::new(false),
            active: AtomicUsize::new(0),
        })
    }
    fn fixture_catalog() -> Value {
        json!({"config_info_list":[{"config_name":"model","display_config":{"display_name":"Model"}}]})
    }

    #[test]
    fn exhausted_account_rotates_same_complete_request_to_second_account() {
        let upstream = Server::http("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", upstream.server_addr());
        let upstream_worker = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for (id, quota) in [("a", true), ("b", false)] {
                let catalog = upstream
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(catalog.url(), CATALOG_PATH);
                catalog
                    .respond(Response::from_string(fixture_catalog().to_string()))
                    .unwrap();
                let mut chat = upstream
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(chat.url(), CHAT_PATH);
                assert!(chat.headers().iter().any(|h| h.field.equiv("Authorization")
                    && h.value.as_str() == format!("Cloud-IDE-JWT {id}")));
                let mut raw = String::new();
                chat.as_reader().read_to_string(&mut raw).unwrap();
                bodies.push(serde_json::from_str::<Value>(&raw).unwrap());
                let response = if quota {
                    "event:error\ndata:{\"code\":4008,\"message\":\"plan quota\"}\n\n"
                } else {
                    "event:output\ndata:{\"response\":\"OK\"}\n\nevent:done\ndata:{\"finish_reason\":\"stop\"}\n\n"
                };
                chat.respond(Response::from_string(response)).unwrap();
            }
            assert_eq!(bodies[0], bodies[1]);
            assert_eq!(bodies[1]["messages"].as_array().unwrap().len(), 4);
            assert_eq!(bodies[1]["messages"][3]["tool_call_id"], "tool1");
        });
        let pool = fixture_pool();
        let address = format!("http://{}", pool.server.server_addr());
        let runner = pool.clone();
        let worker = std::thread::spawn(move || {
            let request = runner.server.recv().unwrap();
            let input = json!({"stream":false,"messages":[{"role":"system","content":"rules"},{"role":"user","content":"task"},{"role":"assistant","content":null,"tool_calls":[{"id":"tool1","type":"function","function":{"name":"echo","arguments":"{}"}}]},{"role":"tool","tool_call_id":"tool1","content":"answer"}]});
            let body = protocol::build_solo_request(&input, "model").unwrap();
            relay(
                request,
                &runner,
                &input,
                body,
                "model",
                vec![fixture_account("a"), fixture_account("b")],
                &origin,
            );
        });
        let result: Value = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(address)
            .body("{}")
            .send()
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(result["choices"][0]["message"]["content"], "OK");
        worker.join().unwrap();
        upstream_worker.join().unwrap();
        assert_eq!(pool.state.lock().unwrap().rotations, 1);
    }

    #[test]
    fn quota_after_partial_output_never_replays_to_another_account() {
        let upstream = Server::http("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", upstream.server_addr());
        let upstream_worker = std::thread::spawn(move || {
            upstream
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap()
                .respond(Response::from_string(fixture_catalog().to_string()))
                .unwrap();
            upstream.recv_timeout(Duration::from_secs(5)).unwrap().unwrap().respond(Response::from_string("event:output\ndata:{\"response\":\"partial\"}\n\nevent:error\ndata:{\"code\":4008,\"message\":\"quota\"}\n\n")).unwrap();
            assert!(upstream
                .recv_timeout(Duration::from_millis(300))
                .unwrap()
                .is_none());
        });
        let pool = fixture_pool();
        let address = format!("http://{}", pool.server.server_addr());
        let runner = pool.clone();
        let worker = std::thread::spawn(move || {
            let request = runner.server.recv().unwrap();
            let input = json!({"messages":[{"role":"user","content":"task"}]});
            relay(
                request,
                &runner,
                &input,
                protocol::build_solo_request(&input, "model").unwrap(),
                "model",
                vec![fixture_account("a"), fixture_account("b")],
                &origin,
            );
        });
        let result = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(address)
            .body("{}")
            .send()
            .unwrap();
        assert_eq!(result.status().as_u16(), 502);
        worker.join().unwrap();
        upstream_worker.join().unwrap();
        assert_eq!(pool.state.lock().unwrap().rotations, 0);
    }

    #[test]
    #[ignore = "Explicit live acceptance harness; uses the user's already imported account"]
    fn live_pool_validation_server() {
        let catalog = models().unwrap();
        println!(
            "Acceptance account directory: {:?}; models: {:?}",
            super::super::config::get_data_dir().unwrap(),
            catalog.models.iter().map(|m| &m.id).collect::<Vec<_>>()
        );
        let chosen = catalog
            .models
            .iter()
            .find(|m| m.tools == Some(true))
            .or_else(|| catalog.models.first())
            .expect("live account has selectable models")
            .id
            .clone();
        let old = selection().unwrap();
        let mut ids: Vec<_> = old
            .model_ids
            .into_iter()
            .filter(|id| catalog.models.iter().any(|m| &m.id == id))
            .collect();
        if !ids.contains(&chosen) {
            ids.push(chosen);
        }
        save_models(ids).unwrap();
        start().unwrap();
        println!("TRAE acceptance pool listening on 127.0.0.1:{PORT}");
        let marker = data_path("trae-pool-acceptance.stop").unwrap();
        let deadline = Instant::now() + Duration::from_secs(300);
        while Instant::now() < deadline && !marker.exists() {
            std::thread::sleep(Duration::from_millis(250));
        }
        stop().unwrap();
    }
}

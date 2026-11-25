//! Loopback OpenAI-compatible pool for the verified SOLO message endpoint.
//! The official client owns conversation history and executes its own tools.
use super::trae_pool_protocol::{self as protocol, FrameAction, StreamGuard};
use crate::models::trae::TraeAccount;
use aes_gcm::{
    aead::{Aead, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::RngCore;
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

const ACCOUNT_MODEL_CATALOG_CACHE_TTL: Duration = Duration::from_secs(300);
const ACCOUNT_MODEL_CATALOG_CACHE_MAX_ENTRIES: usize = 128;

#[derive(Clone)]
struct CachedAccountModelCatalog {
    models: Vec<Model>,
    expires_at: Instant,
}

static ACCOUNT_MODEL_CATALOG_CACHE: LazyLock<Mutex<HashMap<String, CachedAccountModelCatalog>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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

// Model credentials use the llm-client AEAD envelope, not the unrelated
// storage.json byte-crypto envelope. These constants describe the public wire
// format; the user's local API key is never used as the encryption key.
fn native_model_cipher() -> Result<Aes256Gcm, String> {
    let mut material = *b"f4gh7jk9lkln0qs2tuxyjk9ln0qs2tu5";
    for i in 0..8 {
        material[i] ^= b"qs2tu5vw"[i];
    }
    Aes256Gcm::new_from_slice(&material).map_err(|_| "TRAE 模型密钥封装初始化失败".into())
}

fn seal_model_key(local_key: &str) -> Result<String, String> {
    if local_key.trim().is_empty() {
        return Err("本地模型连接密钥为空，未注册".into());
    }
    let mut nonce = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let encrypted = native_model_cipher()?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: local_key.as_bytes(),
                aad: b"1741255278",
            },
        )
        .map_err(|_| "TRAE 模型密钥封装失败，未回退到明文".to_string())?;
    let mut wire = nonce.to_vec();
    wire.extend(encrypted);
    Ok(B64.encode(wire))
}

fn open_model_key(encoded: &str) -> Result<String, String> {
    if encoded.len() > 8192 {
        return Err("TRAE 模型密钥封装过大".into());
    }
    let raw = B64
        .decode(encoded)
        .map_err(|_| "TRAE 模型密钥不是有效封装")?;
    if raw.len() < 28 {
        return Err("TRAE 模型密钥封装不完整".into());
    }
    let decoded = native_model_cipher()?
        .decrypt(
            Nonce::from_slice(&raw[..12]),
            Payload {
                msg: &raw[12..],
                aad: b"1741255278",
            },
        )
        .map_err(|_| "TRAE 模型密钥封装认证失败")?;
    String::from_utf8(decoded).map_err(|_| "TRAE 模型密钥内容无效".into())
}

fn registration_payload(model: &Model, local_key: &str) -> Result<Value, String> {
    let mut limits = serde_json::Map::new();
    if let Some(n) = model.input_limit.filter(|n| *n > 0) {
        limits.insert("prompt_max_tokens".into(), json!(n));
    }
    if let Some(n) = model.output_limit.filter(|n| *n > 0) {
        limits.insert("max_tokens".into(), json!(n));
    }
    Ok(
        json!({"provider":"custom_openai_compatible","model_name":format!("{PREFIX}{}",model.id),"is_custom":true,"auth_type":0,"ak":seal_model_key(local_key)?,
        "base_url":format!("http://127.0.0.1:{PORT}/v1/chat/completions"),"display_name":format!("TRAE 账号池 · {}",model.name),
        "config_detail":{"multimodal":model.multimodal,"model_hyper_params":limits,"max_turn":500}}),
    )
}

fn registration_fingerprint(payload: &Value, local_key: &str) -> String {
    let mut logical = payload.clone();
    logical["ak"] = json!(format!("{:x}", Sha256::digest(local_key.as_bytes())));
    logical["credential_encoding"] = json!("trae-llm-aead-v1");
    format!("{:x}", Sha256::digest(logical.to_string().as_bytes()))
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
        let mut payload = registration_payload(model, &local_key)?;
        let digest = registration_fingerprint(&payload, &local_key);
        let receipt_key = format!("{}:{id}", account.id);
        let wire_name = format!("{PREFIX}{id}");
        let path = if let Some(existing) = remote.get(&wire_name) {
            let key_confirmed = existing
                .get("ak")
                .and_then(Value::as_str)
                .and_then(|s| open_model_key(s).ok())
                .is_some_and(|s| s == local_key);
            if receipts.get(&receipt_key) == Some(&digest) && key_confirmed {
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
        let confirmed = remote
            .get(&wire_name)
            .and_then(|v| v.get("ak"))
            .and_then(Value::as_str)
            .and_then(|s| open_model_key(s).ok())
            .is_some_and(|s| s == local_key);
        if !confirmed {
            return Err("模型已保存，但原生密钥封装尚未验证，未记为注册成功".into());
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
    slots: super::pool_dispatch::AccountSlots,
    admission: super::pool_dispatch::AdmissionQueue,
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
pub(crate) fn key() -> Result<String, String> {
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

fn account_model_cache_key(account: &TraeAccount, origin: &str) -> String {
    let token_fingerprint = format!("{:x}", Sha256::digest(account.access_token.as_bytes()));
    format!("{origin}:{}:{token_fingerprint}", account.id)
}

fn cached_account_models(
    account: &TraeAccount,
    http: &reqwest::blocking::Client,
    origin: &str,
    force_refresh: bool,
) -> Result<(Vec<Model>, bool), String> {
    let key = account_model_cache_key(account, origin);
    if !force_refresh {
        if let Ok(mut cache) = ACCOUNT_MODEL_CATALOG_CACHE.lock() {
            let now = Instant::now();
            cache.retain(|_, entry| entry.expires_at > now);
            if let Some(entry) = cache.get(&key) {
                return Ok((entry.models.clone(), true));
            }
        }
    }

    // Cache successful catalogs only; transient failures must remain retryable.
    let models = account_models(account, http, origin)?;
    if let Ok(mut cache) = ACCOUNT_MODEL_CATALOG_CACHE.lock() {
        let now = Instant::now();
        cache.retain(|_, entry| entry.expires_at > now);
        if cache.len() >= ACCOUNT_MODEL_CATALOG_CACHE_MAX_ENTRIES && !cache.contains_key(&key) {
            if let Some(oldest_key) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest_key);
            }
        }
        cache.insert(
            key,
            CachedAccountModelCatalog {
                models: models.clone(),
                expires_at: now + ACCOUNT_MODEL_CATALOG_CACHE_TTL,
            },
        );
    }
    Ok((models, false))
}

fn invalidate_account_model_cache(account: &TraeAccount, origin: &str) {
    if let Ok(mut cache) = ACCOUNT_MODEL_CATALOG_CACHE.lock() {
        cache.remove(&account_model_cache_key(account, origin));
    }
}

pub fn models() -> Result<ModelsView, String> {
    let http = client()?;
    let mut found = Vec::<Model>::new();
    for account in accounts()? {
        if let Ok(items) = account_models(&account, &http, ORIGIN) {
            invalidate_account_model_cache(&account, ORIGIN);
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

fn debug_log(line: &str) {
    // 测试构建下仅在显式隔离了数据目录时才写日志，防止 cargo test 污染真实日志。
    if cfg!(test) && std::env::var_os("QIEHUAN_YINGYONG_TEST_DATA_DIR").is_none() {
        return;
    }
    use std::io::Write;
    let Ok(dir) = data_path("logs") else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("trae-pool-debug.log"))
    {
        let _ = writeln!(file, "{stamp} {line}");
    }
}

fn respond_error(request: Request, status: u16, message: &str) {
    let code = match (status, message) {
        (429, "本地账号池并发已满，请稍后重试") => "local_pool_busy",
        (429, "可用账号正在处理其他请求，请稍后重试") => "local_accounts_busy",
        (429, _) => "rate_limited",
        (503, _) => "service_unavailable",
        _ => "request_failed",
    };
    debug_log(&format!("reply status={status} code={code}"));
    let mut response = Response::from_string(
        json!({"error":{"type":"trae_pool_error","message":message,"code":code}}).to_string(),
    )
    .with_status_code(StatusCode(status))
    .with_header(Header::from_bytes("Content-Type", "application/json").expect("static header"));
    if status == 429 {
        let delay = if code == "rate_limited" { "60" } else { "15" };
        response.add_header(Header::from_bytes("Retry-After", delay).expect("static header"));
    }
    let _ = request.respond(response);
}

// Rejection kind is passed explicitly: never classify by comparing the human-readable message.
fn reply_classified(request: Request, status: u16, message: &str, result: &str) {
    super::model_service::record(PREFIX, false, status, result);
    respond_error(request, status, message);
}

fn reply(request: Request, status: u16, message: &str) {
    reply_classified(
        request,
        status,
        message,
        super::model_service::RESULT_FAILED,
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
            // Shared bounded dispatch: identical worker cap, backpressure, and
            // scoped-shutdown semantics as the workbuddy and doubao pools.
            super::pool_dispatch::dispatch_observed(
                &pool.server,
                &pool.stopping,
                |request| {
                    pool.active.fetch_add(1, Ordering::SeqCst);
                    let _active = ActiveRequest(pool.clone());
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        handle(request, &pool)
                    }));
                    if outcome.is_err() {
                        if let Ok(mut state) = pool.state.lock() {
                            state.last_error = Some("请求处理异常，未自动重放".into());
                        }
                    }
                },
                |status| {
                    debug_log(&format!("entry rejected status={status}"));
                    super::model_service::record(
                        PREFIX,
                        false,
                        status,
                        if status == 429 {
                            super::model_service::RESULT_ENTRY_REJECTED
                        } else {
                            super::model_service::RESULT_STOPPING
                        },
                    );
                },
            );
            if !pool.stopping.load(Ordering::SeqCst) {
                if let Ok(mut error) = LAST_ERROR.lock() {
                    *error = Some("账号池监听退出，请重新启动".into());
                }
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
    let admission = pool
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .admission
        .clone();
    let Some(next_request) = super::pool_dispatch::admission_route(request, &admission) else {
        return;
    };
    request = next_request;

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
    let Some((next_request, _admission_lease)) =
        super::pool_dispatch::admission_enter(request, &admission)
    else {
        return;
    };
    request = next_request;

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
    let requested = input
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    debug_log(&format!(
        "chat path={} bytes={} stream={}",
        request.url(),
        bytes.len(),
        input
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    ));
    let normalized = requested
        .rsplit_once("//")
        .map_or(requested, |(_, tail)| tail);
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
    debug_log(&format!(
        "forward model={model} candidates={}",
        available.len()
    ));
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
    let relay_started = std::time::Instant::now();
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
    let slots = pool.state.lock().unwrap().slots.clone();
    let mut busy_account = false;
    // Contended accounts queue briefly instead of failing the request: a slow
    // upstream stream must hand its slot to the next caller, not kill it. The
    // scan repeats after a bounded wait, so a freed or newly usable account is
    // picked up again in preference order.
    loop {
        busy_account = false;
        for account in &candidates {
            // Cheap eligibility first: never wait on a slot for an account that
            // cooldown or token expiry would skip anyway.
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
            let Some(_lease) = slots.try_acquire(&account.id) else {
                busy_account = true;
                continue;
            };
            if pool.stopping.load(Ordering::SeqCst) {
                reply(request, 503, "账号池正在停止，未继续轮换");
                return;
            }
            // The account may have entered cooldown while this request waited.
            if pool.state.lock().is_ok_and(|s| {
                s.cooldowns
                    .get(&account.id)
                    .is_some_and(|t| *t > Instant::now())
            }) {
                continue;
            }
            let (mut catalog, cache_hit) =
                match cached_account_models(account, &http, origin, false) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
            // If a cached catalog misses the requested model, refresh once before skipping this account.
            if cache_hit && !catalog.iter().any(|m| m.id == model) {
                catalog = match cached_account_models(account, &http, origin, true) {
                    Ok((fresh, _)) => fresh,
                    Err(_) => continue,
                };
            }
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
                .headers(match headers(account, true) {
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
                invalidate_account_model_cache(&account, origin);
                debug_log(&format!(
                    "rotate account={} status={status} reason=upstream_status",
                    account.id.chars().take(16).collect::<String>()
                ));
                cool(pool, &account.id, if status == 429 { 60 } else { 300 });
                continue;
            }
            if !response.status().is_success() {
                if matches!(status, 400 | 404 | 422) {
                    // A stale model capability is possible. Refresh on the next request, never replay this one.
                    invalidate_account_model_cache(&account, origin);
                }
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
                        debug_log(&format!(
                            "rotate account={} reason=quota_first_frame",
                            account.id.chars().take(16).collect::<String>()
                        ));
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
                let result = request.respond(Response::new(
                    StatusCode(200),
                    vec![
                        Header::from_bytes("Content-Type", "text/event-stream")
                            .expect("static header"),
                        Header::from_bytes("Cache-Control", "no-cache").expect("static header"),
                    ],
                    stream,
                    None,
                    None,
                ));
                super::model_service::record_timed(
                    &format!("{PREFIX}{model}"),
                    true,
                    if result.is_ok() { 200 } else { 502 },
                    super::model_service::RESULT_STREAM_ENDED,
                    Some(relay_started.elapsed().as_millis() as u64),
                );
            } else {
                match collect(stream) {
                    Ok(body) => {
                        let result = request.respond(
                            Response::from_string(body.to_string()).with_header(
                                Header::from_bytes("Content-Type", "application/json")
                                    .expect("static header"),
                            ),
                        );
                        super::model_service::record_timed(
                            &format!("{PREFIX}{model}"),
                            false,
                            if result.is_ok() { 200 } else { 502 },
                            if result.is_ok() {
                                super::model_service::RESULT_COMPLETE
                            } else {
                                super::model_service::RESULT_SEND_FAILED
                            },
                            Some(relay_started.elapsed().as_millis() as u64),
                        );
                    }
                    Err(e) => reply(request, 502, &e),
                }
            }
            return;
        }
        if !busy_account {
            break;
        }
        if pool.stopping.load(Ordering::SeqCst) {
            reply(request, 503, "账号池正在停止，未继续轮换");
            return;
        }
        if !slots.wait_any_release(&pool.stopping, super::pool_dispatch::SLOT_WAIT_MAX) {
            break;
        }
    }
    if busy_account {
        if pool.stopping.load(Ordering::SeqCst) {
            reply(request, 503, "账号池正在停止，未继续轮换");
            return;
        }
        reply(request, 429, "可用账号正在处理其他请求，请稍后重试");
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
    started: std::time::Instant,
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
            started: std::time::Instant::now(),
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
                debug_log(&format!(
                    "stream_abort model={} elapsed_s=0 reason=first_delta",
                    s.model
                ));
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
                    debug_log(&format!(
                        "stream_abort model={} elapsed_s={}",
                        self.model,
                        self.started.elapsed().as_secs()
                    ));
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
        let payload = registration_payload(&model, "local-key-fixture").unwrap();
        assert_ne!(payload["ak"], "local-key-fixture");
        assert_eq!(
            open_model_key(payload["ak"].as_str().unwrap()).unwrap(),
            "local-key-fixture"
        );
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
    fn native_model_credential_matches_independent_aead_fixture() {
        let fixture = "AAECAwQFBgcICQoLkwWArMNkg7wccds6a9vBriDUnmFfh35NiZWUu3DFtQ==";
        assert_eq!(open_model_key(fixture).unwrap(), "fixture-api-key");
        let mut corrupt = B64.decode(fixture).unwrap();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(open_model_key(&B64.encode(corrupt)).is_err());
        assert!(open_model_key("local-plaintext-key").is_err());
        assert!(seal_model_key("").is_err());
    }

    #[test]
    fn registration_fingerprint_is_stable_across_randomized_native_envelopes() {
        let model = Model {
            id: "a".into(),
            name: "A".into(),
            multimodal: false,
            tools: Some(true),
            input_limit: None,
            output_limit: None,
        };
        let first = registration_payload(&model, "fixture-key").unwrap();
        let second = registration_payload(&model, "fixture-key").unwrap();
        assert_ne!(first["ak"], second["ak"]);
        assert_eq!(
            registration_fingerprint(&first, "fixture-key"),
            registration_fingerprint(&second, "fixture-key")
        );
        assert_ne!(
            registration_fingerprint(&first, "fixture-key"),
            registration_fingerprint(&first, "another-key")
        );
        let old = format!("{:x}", Sha256::digest(first.to_string().as_bytes()));
        assert_ne!(old, registration_fingerprint(&first, "fixture-key"));
    }

    #[test]
    fn authenticated_requests_never_log_conversation_text_or_connection_key() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("trae-pool-log-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let old = std::env::var_os("QIEHUAN_YINGYONG_TEST_DATA_DIR");
        std::env::set_var("QIEHUAN_YINGYONG_TEST_DATA_DIR", &dir);
        let pool = fixture_pool();
        let endpoint = format!("http://{}/v1/chat/completions", pool.server.server_addr());
        let worker = pool.clone();
        let server = std::thread::spawn(move || {
            let request = worker.server.recv().unwrap();
            handle(request, &worker);
        });
        let response=reqwest::blocking::Client::builder().no_proxy().build().unwrap().post(endpoint)
            .bearer_auth("fixture-key").json(&json!({"model":"trae-pool-model","messages":[{"role":"user","content":"PRIVATE_CONVERSATION_SENTINEL"}]})).send().unwrap();
        assert!(!response.status().is_success());
        server.join().unwrap();
        let log = std::fs::read_to_string(dir.join("logs/trae-pool-debug.log")).unwrap();
        assert!(!log.contains("PRIVATE_CONVERSATION_SENTINEL"));
        assert!(!log.contains("fixture-key"));
        assert!(log.contains("chat path="));
        match old {
            Some(v) => std::env::set_var("QIEHUAN_YINGYONG_TEST_DATA_DIR", v),
            None => std::env::remove_var("QIEHUAN_YINGYONG_TEST_DATA_DIR"),
        };
        std::fs::remove_dir_all(dir).unwrap();
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
    fn account_model_catalog_cache_skips_network_until_refresh_or_token_rotation() {
        let account = fixture_account("catalog-cache-test-account");
        let upstream = Server::http("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", upstream.server_addr());
        let worker = std::thread::spawn(move || {
            for _ in 0..4 {
                let mut request = upstream
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), CATALOG_PATH);
                let mut body = String::new();
                request.as_reader().read_to_string(&mut body).unwrap();
                request
                    .respond(Response::from_string(fixture_catalog().to_string()))
                    .unwrap();
            }
        });
        let http = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();

        let (first, first_hit) = cached_account_models(&account, &http, &origin, false).unwrap();
        assert!(!first_hit);
        assert_eq!(first.first().map(|model| model.id.as_str()), Some("model"));
        let (second, second_hit) = cached_account_models(&account, &http, &origin, false).unwrap();
        assert!(second_hit);
        assert_eq!(second.first().map(|model| model.id.as_str()), Some("model"));

        let mut rotated = account.clone();
        rotated.access_token.push_str("-rotated");
        let (_, rotated_hit) = cached_account_models(&rotated, &http, &origin, false).unwrap();
        assert!(!rotated_hit);
        let (_, refresh_hit) = cached_account_models(&rotated, &http, &origin, true).unwrap();
        assert!(!refresh_hit);
        invalidate_account_model_cache(&rotated, &origin);
        let (_, invalidated_hit) = cached_account_models(&rotated, &http, &origin, false).unwrap();
        assert!(!invalidated_hit);

        worker.join().unwrap();
    }

    #[test]
    #[ignore = "One live upstream request using an already imported account; no setting changes"]
    fn live_single_model_low_latency_smoke_read_only() {
        let chosen_account = accounts()
            .expect("read TRAE account pool")
            .into_iter()
            .find(|a| {
                !a.access_token.is_empty()
                    && !crate::utils::jwt::parse_jwt_exp(&a.access_token)
                        .is_some_and(|exp| exp <= chrono::Utc::now().timestamp())
            })
            .expect("no authenticated TRAE account available");
        let client = client().expect("create live client");
        let direct_started = Instant::now();
        let models = account_models(&chosen_account, &client, ORIGIN)
            .expect("TRAE upstream did not return model catalog");
        let direct_catalog_ms = direct_started.elapsed().as_millis();
        cached_account_models(&chosen_account, &client, ORIGIN, true)
            .expect("warm TRAE catalog cache");
        let cache_started = Instant::now();
        let (cached_models, was_cached) =
            cached_account_models(&chosen_account, &client, ORIGIN, false)
                .expect("read cached TRAE model catalog");
        let cached_catalog_ms = cache_started.elapsed().as_millis();
        assert!(was_cached);
        assert_eq!(
            models.iter().map(|m| &m.id).collect::<Vec<_>>(),
            cached_models.iter().map(|m| &m.id).collect::<Vec<_>>()
        );
        println!(
            "TRAE_REAL_CATALOG_SPEED direct_request_ms={} cached_hit_ms={}",
            direct_catalog_ms, cached_catalog_ms
        );
        let model = models
            .iter()
            .find(|m| m.tools != Some(false))
            .or_else(|| models.first())
            .expect("no available TRAE chat model")
            .id
            .clone();
        let pool = fixture_pool();
        *pool.selected.lock().unwrap() = vec![model.clone()];
        let endpoint = format!("http://{}/v1/chat/completions", pool.server.server_addr());
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || {
            let req = worker_pool
                .server
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            handle(req, &worker_pool);
        });
        let started = Instant::now();
        let response = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap()
            .post(endpoint)
            .bearer_auth("fixture-key")
            .json(
                &json!({"model":format!("{PREFIX}{model}"), "stream":true, "max_tokens":16,
                "messages":[{"role":"user","content":"Reply with OK."}]}),
            )
            .send()
            .expect("local relay request");
        let first_response_ms = started.elapsed().as_millis();
        let status = response.status();
        let body = response.text().expect("read stream");
        let total_ms = started.elapsed().as_millis();
        worker.join().unwrap();
        println!(
            "TRAE_REAL_PROVIDER_SMOKE status={} first_response_ms={} full_stream_ms={} complete={}",
            status.as_u16(),
            first_response_ms,
            total_ms,
            body.contains("[DONE]")
        );
        assert!(
            status.is_success(),
            "TRAE live provider rejected request: HTTP {}",
            status
        );
        assert!(body.contains("choices") && body.contains("[DONE]"));
    }

    #[test]
    fn account_model_catalog_cache_real_http_latency_comparison() {
        const N: usize = 5;
        let account = fixture_account("catalog-latency-test");
        let upstream = Server::http("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", upstream.server_addr());
        let worker = std::thread::spawn(move || {
            // Uncached: N catalog requests. Cached: only one catalog request.
            for _ in 0..(N + 1) {
                let request = upstream
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), CATALOG_PATH);
                std::thread::sleep(Duration::from_millis(60));
                request
                    .respond(Response::from_string(fixture_catalog().to_string()))
                    .unwrap();
            }
        });
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .build()
            .unwrap();
        let before_direct = Instant::now();
        for _ in 0..N {
            assert_eq!(account_models(&account, &client, &origin).unwrap().len(), 1);
        }
        let direct_ms = before_direct.elapsed().as_millis();

        let before_cached = Instant::now();
        for _ in 0..N {
            assert_eq!(
                cached_account_models(&account, &client, &origin, false)
                    .unwrap()
                    .0
                    .len(),
                1
            );
        }
        let cached_ms = before_cached.elapsed().as_millis();
        worker.join().unwrap();
        println!("REAL_LOCAL_HTTP_BENCH uncached_5_requests_ms={direct_ms} cached_5_requests_ms={cached_ms} catalog_network_calls_before={N} catalog_network_calls_after=1");
        // Controlled upstream adds 60 ms RTT; avoid flaky ratio thresholds on busy Windows hosts.
        assert!(
            cached_ms < direct_ms,
            "cache should avoid repeated catalog network round trips"
        );
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

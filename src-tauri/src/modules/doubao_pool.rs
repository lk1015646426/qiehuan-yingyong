//! Loopback OpenAI-compatible pool for user-owned Doubao web sessions.
pub(crate) mod desktop;
mod protocol;
pub(crate) mod quota;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufReader, ErrorKind, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

pub(crate) const PORT: u16 = 18791;
const PREFIX: &str = "doubao-pool-";
const UPSTREAM_BASE: &str = "https://www.doubao.com";
const MAX_BODY: usize = 64 * 1024 * 1024;

static SERVICE: LazyLock<Mutex<Option<Arc<Pool>>>> = LazyLock::new(|| Mutex::new(None));
static CONFIG_LOCK: Mutex<()> = Mutex::new(());
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn selection_path() -> Result<std::path::PathBuf, String> {
    Ok(super::config::get_data_dir()?.join("doubao-pool-selection.json"))
}
fn keys_path() -> Result<std::path::PathBuf, String> {
    Ok(super::config::get_data_dir()?.join("doubao-pool-accounts.json"))
}
fn catalog_path() -> Result<std::path::PathBuf, String> {
    Ok(super::config::get_data_dir()?.join("doubao-web-pool-catalog.json"))
}
fn pool_key_path() -> Result<std::path::PathBuf, String> {
    Ok(super::config::get_data_dir()?.join("doubao-pool-local.key"))
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub model_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoubaoKeyEntry {
    pub id: String,
    pub name: String,
    pub key: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoubaoKeys {
    pub keys: Vec<DoubaoKeyEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoubaoKeysSnapshot {
    pub entries: Vec<DoubaoKeyEntry>,
    pub revision: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoubaoAccount {
    pub id: String,
    pub key: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogView {
    pub models: Vec<DoubaoModelView>,
    pub selected_model_ids: Vec<String>,
    pub catalog_source: String,
    pub updated_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoubaoModelView {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolStatus {
    pub running: bool,
    pub stopping: bool,
    pub last_error: Option<String>,
    pub endpoint: String,
    pub requests: u64,
    pub rotations: u64,
    pub cooling_accounts: usize,
    pub cooling_account_ids: Vec<String>,
    pub model_count: usize,
    pub selected_model_ids: Vec<String>,
    pub catalog_source: String,
    pub key_count: usize,
}

struct Pool {
    server: Server,
    stopping: AtomicBool,
    key: String,
    state: Mutex<PoolState>,
    catalog: Mutex<Catalog>,
    accounts: Mutex<Vec<DoubaoAccount>>,
}
#[derive(Clone, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    models: Vec<Value>,
    updated_at: i64,
    source: String,
}
#[derive(Default)]
struct PoolState {
    preferred: Option<String>,
    cooldowns: HashMap<String, Instant>,
    slots: super::pool_dispatch::AccountSlots,
    requests: u64,
    rotations: u64,
}

fn generate_pool_key() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn ensure_pool_key() -> Result<String, String> {
    let path = pool_key_path()?;
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim();
        if uuid::Uuid::parse_str(trimmed).is_ok() {
            return Ok(trimmed.to_string());
        }
    }
    let key = generate_pool_key();
    super::atomic_write::write_string_atomic(&path, &key)?;
    Ok(key)
}

pub(crate) fn load_pool_key() -> Result<String, String> {
    let raw = std::fs::read_to_string(pool_key_path()?)
        .map_err(|_| "尚未生成本地豆包账号池 API Key，请先启用账号池")?;
    let trimmed = raw.trim();
    uuid::Uuid::parse_str(trimmed).map_err(|_| "本地豆包 API Key 格式异常")?;
    Ok(trimmed.to_string())
}

fn load_saved_keys() -> Result<DoubaoKeysSnapshot, String> {
    let path = keys_path()?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(DoubaoKeysSnapshot {
                entries: Vec::new(),
                revision: None,
            });
        }
        Err(_) => return Err("豆包账号读取失败，请重试".into()),
    };
    let (keys, _): (DoubaoKeys, bool) =
        super::secure_account_storage::deserialize_account_file_read_only(&path, &raw)
            .map_err(|_| "豆包账号配置无法读取")?;
    Ok(DoubaoKeysSnapshot {
        entries: keys.keys,
        revision: Some(Sha256::digest(raw.as_bytes()).into()),
    })
}

fn read_keys_from_file() -> Result<Vec<DoubaoAccount>, String> {
    let snapshot = load_saved_keys()?;
    if snapshot.revision.is_none() {
        return Err("尚未导入豆包账号，请先登录并导入账号".into());
    }
    let accounts: Vec<DoubaoAccount> = snapshot
        .entries
        .into_iter()
        .filter(|k| k.enabled && !k.key.trim().is_empty())
        .map(|k| {
            Ok(DoubaoAccount {
                id: k.id,
                key: protocol::normalize_cookie(&k.key)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if accounts.is_empty() {
        return Err("没有可用的豆包登录账号，请先导入".into());
    }
    Ok(accounts)
}

fn validate_keys(mut entries: Vec<DoubaoKeyEntry>) -> Result<DoubaoKeys, String> {
    let mut sessions = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    for entry in &mut entries {
        if entry.id.trim().is_empty() || !ids.insert(entry.id.clone()) {
            return Err("账号 ID 为空或重复".into());
        }
        entry.key = protocol::normalize_cookie(&entry.key)?;
        if !sessions.insert(
            protocol::cookie_value(&entry.key, "sessionid")
                .unwrap()
                .to_owned(),
        ) {
            return Err("同一个豆包账号会话已重复添加".into());
        }
    }
    Ok(DoubaoKeys { keys: entries })
}

fn load_selection() -> Result<ModelSelection, String> {
    let path = selection_path()?;
    if !path.exists() {
        return Ok(ModelSelection {
            enabled: false,
            model_ids: vec!["doubao".into()],
        });
    }
    let sel: ModelSelection = serde_json::from_str(
        &std::fs::read_to_string(&path).map_err(|_| "豆包池模型选择读取失败")?,
    )
    .map_err(|_| "豆包池模型选择损坏")?;
    Ok(sel)
}

fn save_selection(sel: &ModelSelection) -> Result<(), String> {
    super::atomic_write::write_string_atomic(
        &selection_path()?,
        &serde_json::to_string_pretty(sel).map_err(|_| "豆包池模型选择序列化失败")?,
    )?;
    Ok(())
}

fn model_id(model: &str) -> Option<&str> {
    let id = model.strip_prefix(PREFIX)?;
    if id.is_empty() || id.len() > 200 {
        return None;
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
    {
        return None;
    }
    Some(id)
}

fn load_catalog_cache() -> Result<Catalog, String> {
    let path = catalog_path()?;
    let raw = std::fs::read_to_string(&path).map_err(|_| "豆包模型目录缓存不可用")?;
    serde_json::from_str(&raw).map_err(|_| "豆包模型目录缓存损坏".to_string())
}

fn load_catalog(_accounts: &[DoubaoAccount]) -> Result<Catalog, String> {
    // Web modes are not Ark model IDs and do not expose underlying model versions.
    let catalog = Catalog {
        models: vec![
            json!({"id":"doubao","name":"豆包 · 普通对话"}),
            json!({"id":"doubao-think","name":"豆包 · 深度思考"}),
        ],
        updated_at: chrono::Utc::now().timestamp(),
        source: "web-modes".into(),
    };
    super::atomic_write::write_string_atomic(
        &catalog_path()?,
        &serde_json::to_string(&catalog).map_err(|_| "豆包模式目录序列化失败")?,
    )?;
    Ok(catalog)
}

fn catalog_to_view(catalog: &Catalog, selected_ids: &[String]) -> CatalogView {
    CatalogView {
        models: catalog
            .models
            .iter()
            .filter_map(|m| {
                let id = m["id"].as_str()?.to_string();
                Some(DoubaoModelView {
                    id: id.clone(),
                    name: m
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string(),
                })
            })
            .collect(),
        selected_model_ids: selected_ids.to_vec(),
        catalog_source: catalog.source.clone(),
        updated_at: catalog.updated_at,
    }
}

fn selected_catalog(catalog: &Catalog, ids: &[String]) -> Result<Catalog, String> {
    let mut seen = std::collections::HashSet::new();
    let mut models = Vec::new();
    for id in ids {
        if !seen.insert(id.clone()) {
            continue;
        }
        let model = catalog
            .models
            .iter()
            .find(|m| m["id"].as_str() == Some(id.as_str()))
            .ok_or_else(|| format!("模型 {id} 不在当前可用目录，请移除后重新选择"))?;
        models.push(model.clone());
    }
    Ok(Catalog {
        models,
        updated_at: catalog.updated_at,
        source: catalog.source.clone(),
    })
}

pub async fn get_doubao_pool_keys() -> Result<DoubaoKeysSnapshot, String> {
    tauri::async_runtime::spawn_blocking(load_saved_keys)
        .await
        .map_err(|_| "豆包账号读取任务失败")?
}

pub async fn save_doubao_pool_keys(
    entries: Vec<DoubaoKeyEntry>,
    expected_revision: Option<[u8; 32]>,
) -> Result<DoubaoKeysSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        const CHANGED: &str = "豆包账号列表已变化，请重新读取后再试";
        let _config = CONFIG_LOCK.lock().map_err(|_| "豆包配置锁异常")?;
        if load_saved_keys()?.revision != expected_revision {
            return Err(CHANGED.into());
        }
        let keys = validate_keys(entries)?;
        ensure_pool_key()?;
        let accounts = keys
            .keys
            .iter()
            .filter(|a| a.enabled)
            .map(|a| DoubaoAccount {
                id: a.id.clone(),
                key: a.key.clone(),
            })
            .collect();
        let encoded =
            super::secure_account_storage::serialize_account_file("doubao-web-accounts", &keys)?;
        let revision = Sha256::digest(encoded.as_bytes()).into();
        let service = SERVICE.lock().map_err(|_| "豆包状态锁异常")?;
        let live = match service.as_ref() {
            Some(pool) => Some((
                pool.state.lock().map_err(|_| "豆包状态锁异常")?,
                pool.accounts.lock().map_err(|_| "豆包账号锁异常")?,
            )),
            None => None,
        };
        let path = keys_path()?;
        let written = match expected_revision {
            Some(hash) => {
                super::atomic_write::write_string_atomic_if_hash_matches(&path, hash, || Ok(encoded))?
            }
            None => super::atomic_write::replace_bytes_if_unchanged(
                &path,
                None,
                Some(encoded.as_bytes()),
            )?,
        };
        if !written {
            return Err(CHANGED.into());
        }
        if let Some((mut state, mut active_accounts)) = live {
            *active_accounts = accounts;
            state.cooldowns.clear();
            state.preferred = None;
        }
        Ok(DoubaoKeysSnapshot {
            entries: keys.keys,
            revision: Some(revision),
        })
    })
    .await
    .map_err(|_| "豆包账号保存任务失败")?
}

pub async fn get_doubao_pool_catalog(refresh: Option<bool>) -> Result<CatalogView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let sel = load_selection()?;
        let accounts = read_keys_from_file().unwrap_or_default();
        let catalog = if refresh.unwrap_or(false) {
            load_catalog(&accounts).or_else(|_| load_catalog_cache())?
        } else {
            load_catalog_cache().or_else(|_| load_catalog(&accounts))?
        };
        Ok(catalog_to_view(&catalog, &sel.model_ids))
    })
    .await
    .map_err(|_| "豆包目录查询任务失败")?
}

pub async fn save_doubao_pool_models(model_ids: Vec<String>) -> Result<CatalogView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _config = CONFIG_LOCK.lock().map_err(|_| "豆包配置锁异常")?;
        let full = load_catalog(&[])?;
        let selected = selected_catalog(&full, &model_ids)?;
        let enabled = !model_ids.is_empty() && load_selection().map(|s| s.enabled).unwrap_or(false);
        save_selection(&ModelSelection {
            enabled,
            model_ids: model_ids.clone(),
        })?;
        if let Some(pool) = SERVICE.lock().map_err(|_| "豆包状态锁异常")?.as_ref() {
            *pool.catalog.lock().map_err(|_| "豆包目录锁异常")? = selected;
            if model_ids.is_empty() {
                pool.stopping.store(true, Ordering::SeqCst);
                pool.server.unblock();
            }
        }
        Ok(catalog_to_view(&full, &model_ids))
    })
    .await
    .map_err(|_| "豆包模型保存任务失败")?
}

pub fn get_doubao_pool_status() -> Result<PoolStatus, String> {
    let service = SERVICE.lock().map_err(|_| "豆包池状态锁异常")?;
    let state = service
        .as_ref()
        .map(|p| p.state.lock())
        .transpose()
        .map_err(|_| "豆包池状态锁异常")?;
    let sel = load_selection()?;
    let last_err = LAST_ERROR.lock().map_err(|_| "豆包池诊断锁异常")?.clone();
    let key_count = service
        .as_ref()
        .map(|p| p.accounts.lock().map(|a| a.len()).unwrap_or(0))
        .unwrap_or(read_keys_from_file().map(|a| a.len()).unwrap_or(0));
    let model_count = match service.as_ref() {
        Some(p) => p.catalog.lock().map_err(|_| "模型目录锁异常")?.models.len(),
        None => sel.model_ids.len(),
    };
    let now = Instant::now();
    let cooling_account_ids: Vec<_> = state
        .as_ref()
        .into_iter()
        .flat_map(|state| state.cooldowns.iter())
        .filter(|(_, until)| **until > now)
        .map(|(id, _)| id.clone())
        .collect();
    Ok(PoolStatus {
        running: service.is_some(),
        stopping: service
            .as_ref()
            .is_some_and(|p| p.stopping.load(Ordering::SeqCst)),
        last_error: last_err,
        endpoint: format!("http://127.0.0.1:{PORT}/v1/chat/completions"),
        requests: state.as_ref().map_or(0, |s| s.requests),
        rotations: state.as_ref().map_or(0, |s| s.rotations),
        cooling_accounts: cooling_account_ids.len(),
        cooling_account_ids,
        model_count,
        selected_model_ids: sel.model_ids.clone(),
        catalog_source: service
            .as_ref()
            .map_or_else(String::new, |p| p.catalog.lock().unwrap().source.clone()),
        key_count,
    })
}

pub async fn start_doubao_pool() -> Result<PoolStatus, String> {
    let result = start_pool().await;
    if let Err(ref error) = result {
        if let Ok(mut last) = LAST_ERROR.lock() {
            *last = Some(error.clone());
        }
    }
    result
}

async fn start_pool() -> Result<PoolStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _config = CONFIG_LOCK.lock().map_err(|_| "豆包池配置锁异常")?;
        let selection = load_selection()?;
        if selection.model_ids.is_empty() {
            return Err("请先添加并保存至少一个豆包模型".to_string());
        }
        let accounts = read_keys_from_file()?;
        let full = load_catalog_cache().or_else(|_| load_catalog(&accounts))?;
        let catalog = selected_catalog(&full, &selection.model_ids)?;
        let mut service = SERVICE.lock().map_err(|_| "豆包池状态锁异常")?;
        if let Some(pool) = service.as_ref() {
            if pool.stopping.load(Ordering::SeqCst) {
                return Err("豆包池正在等待在途请求结束，端口释放后可重新启动".into());
            }
            *pool.catalog.lock().map_err(|_| "模型目录锁异常")? = catalog;
            return Ok(());
        }
        #[cfg(test)]
        let bind_port = std::env::var("QIEHUAN_TEST_POOL_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(PORT);
        #[cfg(not(test))]
        let bind_port = PORT;
        let server = Server::http(("127.0.0.1", bind_port))
            .map_err(|_| format!("本地端口 {PORT} 已被占用，未修改配置"))?;
        let key = ensure_pool_key()?;
        let pool = Arc::new(Pool {
            server,
            key,
            stopping: AtomicBool::new(false),
            state: Mutex::new(PoolState::default()),
            catalog: Mutex::new(catalog),
            accounts: Mutex::new(accounts.clone()),
        });
        let saved = ModelSelection {
            enabled: true,
            model_ids: selection.model_ids.clone(),
        };
        save_selection(&saved)?;
        let worker = pool.clone();
        let join = std::thread::spawn(move || {
            super::pool_dispatch::dispatch_observed(
                &worker.server,
                &worker.stopping,
                |request| handle(request, &worker),
                |status| {
                    super::model_service::record(
                        "doubao-pool-",
                        false,
                        status,
                        if status == 429 {
                            super::model_service::RESULT_ENTRY_REJECTED
                        } else {
                            super::model_service::RESULT_STOPPING
                        },
                    )
                },
            );
        });
        *service = Some(pool.clone());
        monitor_worker(pool, join);
        if let Ok(mut error) = LAST_ERROR.lock() {
            *error = None;
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "豆包池启动任务失败")??;
    get_doubao_pool_status()
}

fn monitor_worker(pool: Arc<Pool>, join: std::thread::JoinHandle<()>) {
    std::thread::spawn(move || {
        let outcome = join.join();
        let mut slot = SERVICE.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &pool)) {
            let owned = slot.take();
            drop(owned);
            drop(pool);
            if outcome.is_err() {
                if let Ok(mut error) = LAST_ERROR.lock() {
                    *error = Some("豆包池工作线程异常退出，请重新启动".into());
                }
            }
        }
    });
}

pub fn stop_doubao_pool() -> Result<PoolStatus, String> {
    let _config = CONFIG_LOCK.lock().map_err(|_| "豆包池配置锁异常")?;
    let mut selection = load_selection()?;
    selection.enabled = false;
    if let Err(e) = save_selection(&selection) {
        if let Ok(mut error) = LAST_ERROR.lock() {
            *error = Some(format!("已请求停止，但自动恢复设置保存失败：{e}"));
        }
    }
    if let Some(pool) = SERVICE.lock().map_err(|_| "豆包池状态锁异常")?.as_ref() {
        pool.stopping.store(true, Ordering::SeqCst);
        pool.server.unblock();
    }
    get_doubao_pool_status()
}

pub(crate) async fn restore_doubao_pool() {
    let enabled = tauri::async_runtime::spawn_blocking(|| {
        load_selection()
            .map(|s| s.enabled && !s.model_ids.is_empty())
            .unwrap_or(false)
    })
    .await;
    if matches!(enabled, Ok(true)) {
        if let Err(e) = start_doubao_pool().await {
            if let Ok(mut error) = LAST_ERROR.lock() {
                *error = Some(format!("豆包池自动恢复未完成：{e}"));
            }
        }
    }
}

fn reply(request: Request, status: u16, message: &str) {
    super::model_service::record(
        "doubao-pool-",
        false,
        status,
        super::model_service::RESULT_FAILED,
    );
    let body = json!({"error":{"message":message,"type":"doubao_pool_error"}}).to_string();
    let _ = request.respond(
        Response::from_string(body)
            .with_status_code(StatusCode(status))
            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap()),
    );
}

fn cooldown(status: u16, _body: &str) -> Option<Duration> {
    protocol::UpstreamError {
        code: status as i64,
    }
    .cooldown()
}

fn handle(mut request: Request, pool: &Pool) {
    let authenticated = request.headers().iter().any(|h| {
        h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {}", pool.key)
    });
    if !authenticated {
        reply(request, 401, "本地豆包池 API Key 无效");
        return;
    }
    if request.method() == &Method::Get && request.url() == "/v1/models" {
        let data: Vec<Value> = pool
            .catalog
            .lock()
            .unwrap()
            .models
            .iter()
            .map(|m| {
                json!({"id":format!("{PREFIX}{}",m["id"].as_str().unwrap()),"object":"model","owned_by":"local"})
            })
            .collect();
        let _ = request.respond(
            Response::from_string(json!({"object":"list","data":data}).to_string())
                .with_header(Header::from_bytes("Content-Type", "application/json").unwrap()),
        );
        return;
    }
    if request.method() != &Method::Post || request.url() != "/v1/chat/completions" {
        reply(request, 404, "接口不存在");
        return;
    }
    let mut bytes = Vec::new();
    if request
        .as_reader()
        .take((MAX_BODY + 1) as u64)
        .read_to_end(&mut bytes)
        .is_err()
    {
        reply(request, 400, "读取请求失败");
        return;
    }
    if bytes.len() > MAX_BODY {
        reply(request, 413, "请求超过 64 MiB");
        return;
    }
    let body: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(_) => {
            reply(request, 400, "请求 JSON 无效");
            return;
        }
    };
    let upstream_model = body
        .get("model")
        .and_then(Value::as_str)
        .and_then(model_id)
        .map(str::to_string);
    if upstream_model.is_none()
        || body.get("stream").is_some_and(|v| !v.is_boolean())
        || !body.get("messages").is_some_and(Value::is_array)
    {
        reply(
            request,
            400,
            "请使用 doubao-pool- 模型 ID、messages 数组及布尔 stream",
        );
        return;
    }
    let upstream_model = upstream_model.unwrap();
    let selected_ids: std::collections::HashSet<String> = pool
        .catalog
        .lock()
        .unwrap()
        .models
        .iter()
        .filter_map(|m| m["id"].as_str().map(str::to_string))
        .collect();
    if !selected_ids.contains(&upstream_model) {
        reply(request, 400, "该模型未添加到豆包池，请选择已保存的模型");
        return;
    }
    let upstream_body = match protocol::payload(&body, &upstream_model) {
        Ok(value) => value,
        Err(error) => {
            reply(request, 400, &error);
            return;
        }
    };
    let relay_started = std::time::Instant::now();
    let client = match crate::utils::http::create_domestic_blocking_client(180, "www.doubao.com") {
        Ok(c) => c,
        Err(_) => {
            reply(request, 503, "创建上游连接失败");
            return;
        }
    };
    let streaming = body["stream"].as_bool().unwrap_or(false);
    let requested_model = format!("{PREFIX}{upstream_model}");
    let mut accounts = pool.accounts.lock().unwrap().clone();
    let preferred = pool.state.lock().unwrap().preferred.clone();
    accounts.sort_by_key(|a| preferred.as_deref() != Some(&a.id));
    pool.state.lock().unwrap().requests += 1;
    let mut last_failure = if accounts.is_empty() {
        "没有可用豆包账号，请先登录并导入".to_string()
    } else {
        "全部豆包账号处于冷却期，请重新登录导入或稍后重试".to_string()
    };
    let slots = pool.state.lock().unwrap().slots.clone();
    let mut busy_account = false;
    // Contended accounts queue briefly instead of failing the request; the
    // scan repeats after a bounded wait for any slot release.
    loop {
        busy_account = false;
        'accounts: for account in &accounts {
            // Cheap eligibility first: never wait on a slot for a cooled account.
            if pool
                .state
                .lock()
                .unwrap()
                .cooldowns
                .get(&account.id)
                .is_some_and(|until| *until > Instant::now())
            {
                continue;
            }
            let Some(_lease) = slots.try_acquire(&account.id) else {
                busy_account = true;
                continue;
            };
            // The account may have entered cooldown while waiting for this scan.
            if pool
                .state
                .lock()
                .unwrap()
                .cooldowns
                .get(&account.id)
                .is_some_and(|until| *until > Instant::now())
            {
                continue;
            }
            let mut params = vec![
                ("aid", "497858"),
                ("real_aid", "497858"),
                ("device_platform", "web"),
                ("language", "zh"),
                ("region", "CN"),
                ("sys_region", "CN"),
                ("samantha_web", "1"),
                ("use-olympus-account", "1"),
                ("version_code", "20800"),
                ("pc_version", "2.41.0"),
                ("pkg_type", "release_version"),
            ];
            if let Some(token) = protocol::cookie_value(&account.key, "msToken") {
                params.push(("msToken", token));
            }
            let response = client
            .post(format!("{UPSTREAM_BASE}/samantha/chat/completion"))
            .query(&params)
            .header("Cookie", &account.key)
            .header("Origin", UPSTREAM_BASE)
            .header("Referer", "https://www.doubao.com/chat/")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36")
            .header("agw-js-conv", "str, str")
            .header("x-tt-passport-csrf-token", protocol::cookie_value(&account.key, "passport_csrf_token").unwrap_or(""))
            .header("Accept", "text/event-stream")
            .json(&upstream_body)
            .send();
            let response = match response {
                Ok(r) => r,
                Err(_) => {
                    reply(request, 502, "上游连接中断，请手动重试；未自动重放");
                    return;
                }
            };
            let status = response.status().as_u16();
            if !(200..300).contains(&status) {
                let mut error = String::new();
                let _ = response.take(64 * 1024).read_to_string(&mut error);
                if let Some(delay) = cooldown(status, &error) {
                    let mut state = pool.state.lock().unwrap();
                    state
                        .cooldowns
                        .insert(account.id.clone(), Instant::now() + delay);
                    state.rotations += 1;
                    last_failure = protocol::UpstreamError {
                        code: status as i64,
                    }
                    .message();
                    continue 'accounts;
                }
                reply(
                    request,
                    status,
                    &format!("豆包请求失败（HTTP {status}），请检查网页账号状态；未自动重放"),
                );
                return;
            }
            if !response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("text/event-stream"))
            {
                let mut raw = String::new();
                let _ = response.take(65536).read_to_string(&mut raw);
                let code = serde_json::from_str::<Value>(&raw)
                    .ok()
                    .and_then(|v| v["code"].as_i64())
                    .unwrap_or(-1);
                let error = protocol::UpstreamError { code };
                if let Some(delay) = error.cooldown() {
                    let mut state = pool.state.lock().unwrap();
                    state
                        .cooldowns
                        .insert(account.id.clone(), Instant::now() + delay);
                    state.rotations += 1;
                    last_failure = error.message();
                    continue 'accounts;
                }
                reply(request, 502, &error.message());
                return;
            }
            let mut stream =
                protocol::Stream::new(BufReader::new(response), requested_model.clone());
            if let Err(error) = stream.prime() {
                if let Some(delay) = error.cooldown() {
                    let mut state = pool.state.lock().unwrap();
                    state
                        .cooldowns
                        .insert(account.id.clone(), Instant::now() + delay);
                    state.rotations += 1;
                    last_failure = error.message();
                    continue 'accounts;
                }
                reply(request, 502, &error.message());
                return;
            }
            pool.state.lock().unwrap().preferred = Some(account.id.clone());
            if !streaming {
                match super::model_service::collect_completion(BufReader::new(stream)) {
                    Ok(value) => {
                        let result =
                            request.respond(Response::from_string(value.to_string()).with_header(
                                Header::from_bytes("Content-Type", "application/json").unwrap(),
                            ));
                        super::model_service::record_timed(
                            &requested_model,
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
                    Err(error) => reply(request, 502, &error),
                }
                return;
            }
            let result = request.respond(Response::new(
                StatusCode(200),
                vec![
                    Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                    Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                ],
                stream,
                None,
                None,
            ));
            super::model_service::record_timed(
                &requested_model,
                true,
                if result.is_ok() { 200 } else { 502 },
                super::model_service::RESULT_STREAM_ENDED,
                Some(relay_started.elapsed().as_millis() as u64),
            );
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
    reply(request, 503, &last_failure);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_accounts_are_encrypted_and_hot_reloaded_without_touching_ark_keys() {
        // This variable has the highest precedence in test builds, so the whole body must hold
        // the shared env lock or other isolated tests get redirected into this temp dir.
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("doubao-web-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let old = std::env::var_os("QIEHUAN_YINGYONG_TEST_DATA_DIR");
        std::env::set_var("QIEHUAN_YINGYONG_TEST_DATA_DIR", &root);
        struct Cleanup(std::path::PathBuf, Option<std::ffi::OsString>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                *SERVICE.lock().unwrap_or_else(|e| e.into_inner()) = None;
                if let Some(old) = &self.1 {
                    std::env::set_var("QIEHUAN_YINGYONG_TEST_DATA_DIR", old);
                } else {
                    std::env::remove_var("QIEHUAN_YINGYONG_TEST_DATA_DIR");
                }
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone(), old);
        let ark = root.join("doubao-pool-keys.json");
        std::fs::write(&ark, "existing-ark-data").unwrap();
        let catalog = load_catalog(&[]).unwrap();
        *SERVICE.lock().unwrap() = Some(Arc::new(Pool {
            server: Server::http("127.0.0.1:0").unwrap(),
            stopping: AtomicBool::new(false),
            key: "fixture".into(),
            state: Mutex::new(PoolState::default()),
            catalog: Mutex::new(catalog),
            accounts: Mutex::new(vec![]),
        }));
        let entry = DoubaoKeyEntry {
            id: "one".into(),
            name: "test".into(),
            key: "sessionid=fixture-secret; msToken=real".into(),
            enabled: true,
        };
        let empty = tauri::async_runtime::block_on(get_doubao_pool_keys()).unwrap();
        assert!(empty.entries.is_empty() && empty.revision.is_none());
        let saved = tauri::async_runtime::block_on(save_doubao_pool_keys(
            vec![entry.clone()],
            empty.revision,
        ))
        .unwrap();
        let raw = std::fs::read_to_string(keys_path().unwrap()).unwrap();
        assert!(!raw.contains("fixture-secret"));
        assert_eq!(read_keys_from_file().unwrap().len(), 1);
        let broken = "unreadable account fixture";
        let path = keys_path().unwrap();
        std::fs::write(&path, broken).unwrap();
        assert!(tauri::async_runtime::block_on(get_doubao_pool_keys()).is_err());
        assert!(
            tauri::async_runtime::block_on(save_doubao_pool_keys(vec![], saved.revision)).is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), broken);
        std::fs::write(&path, &raw).unwrap();
        assert!(
            tauri::async_runtime::block_on(save_doubao_pool_keys(vec![], empty.revision)).is_err()
        );
        assert_eq!(
            SERVICE
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .accounts
                .lock()
                .unwrap()
                .len(),
            1
        );
        assert!(
            tauri::async_runtime::block_on(save_doubao_pool_keys(
                vec![entry.clone(), entry],
                saved.revision,
            ))
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(keys_path().unwrap()).unwrap(), raw);
        tauri::async_runtime::block_on(save_doubao_pool_models(vec!["doubao-think".into()]))
            .unwrap();
        assert_eq!(
            SERVICE
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .catalog
                .lock()
                .unwrap()
                .models[0]["id"],
            "doubao-think"
        );
        tauri::async_runtime::block_on(save_doubao_pool_keys(vec![], saved.revision)).unwrap();
        assert!(SERVICE
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .accounts
            .lock()
            .unwrap()
            .is_empty());
        tauri::async_runtime::block_on(save_doubao_pool_models(vec![])).unwrap();
        assert!(SERVICE
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .stopping
            .load(Ordering::SeqCst));
        assert_eq!(std::fs::read_to_string(ark).unwrap(), "existing-ark-data");
    }

    #[test]
    fn model_id_parsing() {
        assert_eq!(model_id("doubao-pool-doubao-think"), Some("doubao-think"));
        assert!(model_id("doubao-pool-").is_none());
        assert!(model_id("raw-model").is_none());
        assert!(model_id("doubao-pool-https://bad").is_none());
    }

    #[test]
    fn limits_auth_and_rate_are_distinct() {
        assert_eq!(cooldown(429, "rate limit"), Some(Duration::from_secs(60)));
        assert_eq!(cooldown(401, ""), Some(Duration::from_secs(300)));
        assert_eq!(cooldown(403, ""), Some(Duration::from_secs(300)));
        for code in [200, 400, 404, 500, 502] {
            assert_eq!(cooldown(code, "network failure"), None);
        }
    }

    #[test]
    fn endpoint_requires_local_key() {
        let pool = Arc::new(Pool {
            server: Server::http("127.0.0.1:0").unwrap(),
            key: "fixture".into(),
            stopping: AtomicBool::new(false),
            state: Mutex::new(PoolState::default()),
            catalog: Mutex::new(Catalog {
                models: vec![],
                updated_at: 0,
                source: "fixture".into(),
            }),
            accounts: Mutex::new(vec![]),
        });
        let endpoint = format!("http://{}/v1/models", pool.server.server_addr());
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || {
            handle(worker_pool.server.recv().unwrap(), &worker_pool);
        });
        assert_eq!(
            reqwest::blocking::Client::new()
                .get(endpoint)
                .send()
                .unwrap()
                .status()
                .as_u16(),
            401
        );
        worker.join().unwrap();
    }
}

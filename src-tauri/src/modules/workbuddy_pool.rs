//! Loopback model pool: rotate upstream credentials, keep the desktop identity/session intact.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

const PORT: u16 = 18789;
const MODEL: &str = "workbuddy-pool-hy3";
const CATALOG_URL: &str = "https://copilot.tencent.com/console/enterprises/personal/models";
const MAX_BODY: usize = 64 * 1024 * 1024;
const UPSTREAM: &str = "https://copilot.tencent.com/v2/chat/completions";
static SERVICE: LazyLock<Mutex<Option<PoolService>>> = LazyLock::new(|| Mutex::new(None));
static MODEL_CONFIG_LOCK: Mutex<()> = Mutex::new(());
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);
struct PoolService {
    pool: Arc<Pool>,
}
impl std::ops::Deref for PoolService {
    type Target = Pool;
    fn deref(&self) -> &Pool {
        &self.pool
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelSelection {
    version: u32,
    model_ids: Vec<String>,
    #[serde(default)]
    enabled: Option<bool>,
}
impl Default for ModelSelection {
    fn default() -> Self {
        Self {
            version: 1,
            model_ids: Vec::new(),
            enabled: Some(false),
        }
    }
}

#[derive(Serialize)]
pub struct PoolModelView {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolModelsView {
    models: Vec<PoolModelView>,
    selected_model_ids: Vec<String>,
    catalog_source: String,
    updated_at: i64,
}

fn selection_path() -> Result<std::path::PathBuf, String> {
    Ok(super::config::get_data_dir()?.join("workbuddy-pool-selection.json"))
}

fn load_selection() -> Result<ModelSelection, String> {
    let path = selection_path()?;
    if !path.exists() {
        return Ok(ModelSelection::default());
    }
    let selection: ModelSelection =
        serde_json::from_str(&std::fs::read_to_string(path).map_err(|_| "无法读取账号池模型选择")?)
            .map_err(|_| "账号池模型选择文件损坏，未自动覆盖")?;
    if selection.version != 1 {
        return Err("账号池模型选择版本不支持".into());
    }
    Ok(selection)
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
            .find(|m| m["id"].as_str() == Some(id.as_str()) && is_chat_model(m))
            .ok_or_else(|| format!("模型 {id} 不在当前可用会话目录，请移除后重新选择"))?;
        models.push(model.clone());
    }
    Ok(Catalog {
        models,
        updated_at: catalog.updated_at,
        source: catalog.source.clone(),
    })
}

fn models_view(catalog: &Catalog, ids: Vec<String>) -> PoolModelsView {
    PoolModelsView {
        models: catalog
            .models
            .iter()
            .filter(|m| is_chat_model(m))
            .map(|m| PoolModelView {
                id: m["id"].as_str().unwrap().into(),
                name: m
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(m["id"].as_str().unwrap())
                    .into(),
            })
            .collect(),
        selected_model_ids: ids,
        catalog_source: catalog.source.clone(),
        updated_at: catalog.updated_at,
    }
}

#[tauri::command]
pub async fn get_workbuddy_pool_models(
    refresh_catalog: Option<bool>,
) -> Result<PoolModelsView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let catalog = if refresh_catalog.unwrap_or(false) {
            load_catalog()?
        } else {
            read_catalog_cache().or_else(|_| load_catalog())?
        };
        Ok(models_view(&catalog, load_selection()?.model_ids))
    })
    .await
    .map_err(|_| "模型目录查询任务失败")?
}

struct Pool {
    server: Server,
    stopping: AtomicBool,
    key: String,
    state: Mutex<PoolState>,
    catalog: Mutex<Catalog>,
}
#[derive(Clone, serde::Deserialize, Serialize)]
struct Catalog {
    models: Vec<Value>,
    updated_at: i64,
    source: String,
}
#[derive(Default)]
struct PoolState {
    preferred: Option<String>,
    cooldowns: HashMap<String, Instant>,
    requests: u64,
    rotations: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolStatus {
    running: bool,
    stopping: bool,
    last_error: Option<String>,
    model: &'static str,
    endpoint: String,
    requests: u64,
    rotations: u64,
    cooling_accounts: usize,
    model_count: usize,
    catalog_source: String,
}

#[tauri::command]
pub fn get_workbuddy_pool_status() -> Result<PoolStatus, String> {
    let service = SERVICE.lock().map_err(|_| "账号池状态锁异常")?;
    let state = service
        .as_ref()
        .map(|s| s.state.lock())
        .transpose()
        .map_err(|_| "账号池状态锁异常")?;
    Ok(PoolStatus {
        running: service.is_some(),
        stopping: service
            .as_ref()
            .is_some_and(|p| p.stopping.load(Ordering::SeqCst)),
        last_error: LAST_ERROR.lock().map_err(|_| "账号池诊断锁异常")?.clone(),
        model_count: match service.as_ref() {
            Some(pool) => pool
                .catalog
                .lock()
                .map_err(|_| "模型目录锁异常")?
                .models
                .len(),
            None => load_selection()?.model_ids.len(),
        },
        catalog_source: service
            .as_ref()
            .map_or_else(String::new, |s| s.catalog.lock().unwrap().source.clone()),
        model: MODEL,
        endpoint: format!("http://127.0.0.1:{PORT}/v1/chat/completions"),
        requests: state.as_ref().map_or(0, |s| s.requests),
        rotations: state.as_ref().map_or(0, |s| s.rotations),
        cooling_accounts: state.as_ref().map_or(0, |s| {
            s.cooldowns
                .values()
                .filter(|until| **until > Instant::now())
                .count()
        }),
    })
}

fn model_id(model: &str) -> Option<&str> {
    let id = model.strip_prefix("workbuddy-pool-")?;
    if id.is_empty()
        || id.len() > 200
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
    {
        return None;
    }
    Some(id)
}
fn is_chat_model(model: &Value) -> bool {
    !model
        .get("tags")
        .and_then(Value::as_array)
        .is_some_and(|tags| {
            tags.iter().any(|tag| {
                matches!(
                    tag.as_str(),
                    Some("text-to-image" | "text-to-video" | "image-to-video")
                )
            })
        })
}

fn parse_catalog(payload: &Value) -> Result<Vec<Value>, String> {
    let models = payload
        .pointer("/data/models")
        .or_else(|| payload.get("models"))
        .and_then(Value::as_array)
        .ok_or("模型目录格式异常")?;
    let mut seen = std::collections::HashSet::new();
    let result: Vec<Value> = models
        .iter()
        .filter(|model| {
            model.get("id").and_then(Value::as_str).is_some_and(|id| {
                model_id(&format!("workbuddy-pool-{id}")).is_some() && seen.insert(id.to_string())
            })
        })
        .cloned()
        .collect();
    if result.is_empty() {
        return Err("模型目录为空，保留当前配置".into());
    }
    Ok(result)
}

fn load_catalog() -> Result<Catalog, String> {
    let path = super::config::get_data_dir()?.join("workbuddy-pool-catalog.json");
    let live = (|| {
        let client =
            crate::utils::http::create_domestic_blocking_client(15, "copilot.tencent.com")?;
        for account in super::workbuddy_account::list_workbuddy_accounts()? {
            let token = match super::workbuddy_account::access_token(&account.id) {
                Ok(token) => token,
                Err(_) => continue,
            };
            let response = client
                .get(CATALOG_URL)
                .bearer_auth(token)
                .header("Accept", "application/json")
                .header("Origin", "https://copilot.tencent.com")
                .header("Referer", "https://copilot.tencent.com/")
                .send()
                .map_err(|_| "无法连接实时模型目录")?;
            if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
                continue;
            }
            if !response.status().is_success() {
                return Err("实时模型目录暂不可用".to_string());
            }
            let payload: Value = response.json().map_err(|_| "模型目录返回数据无效")?;
            if payload
                .get("code")
                .and_then(Value::as_i64)
                .is_some_and(|code| code != 0)
            {
                continue;
            }
            return Ok(Catalog {
                models: parse_catalog(&payload)?,
                updated_at: chrono::Utc::now().timestamp(),
                source: "live".into(),
            });
        }
        Err("没有可查询模型目录的有效账号".to_string())
    })();
    match live {
        Ok(catalog) => {
            super::atomic_write::write_string_atomic(
                &path,
                &serde_json::to_string(&catalog).map_err(|_| "模型目录序列化失败")?,
            )?;
            Ok(catalog)
        }
        Err(error) => read_catalog_cache().map_err(|_| error),
    }
}

fn read_catalog_cache() -> Result<Catalog, String> {
    let path = super::config::get_data_dir()?.join("workbuddy-pool-catalog.json");
    let mut catalog: Catalog =
        serde_json::from_str(&std::fs::read_to_string(path).map_err(|_| "模型目录缓存尚未建立")?)
            .map_err(|_| "模型目录缓存格式异常")?;
    catalog.models = parse_catalog(&json!({"models":catalog.models}))?;
    catalog.source = "cache".into();
    Ok(catalog)
}

fn native_model(model: &Value, key: &str, all_ids: &std::collections::HashSet<String>) -> Value {
    let mut item = model.clone();
    let id = model["id"].as_str().unwrap();
    item["id"] = json!(format!("workbuddy-pool-{id}"));
    item["name"] = json!(format!(
        "WorkBuddy 账号池 · {}",
        model.get("name").and_then(Value::as_str).unwrap_or(id)
    ));
    item["url"] = json!(format!("http://127.0.0.1:{PORT}/v1/chat/completions"));
    item["apiKey"] = json!(key);
    item["isDefault"] = json!(false);
    if let Some(related) = item.get_mut("relatedModels").and_then(Value::as_object_mut) {
        related.retain(|_, value| value.as_str().is_some_and(|id| all_ids.contains(id)));
        for value in related.values_mut() {
            if let Some(id) = value.as_str().filter(|id| all_ids.contains(*id)) {
                *value = json!(format!("workbuddy-pool-{id}"));
            }
        }
    }
    item
}

fn register_model(key: &str, catalog: &Catalog) -> Result<(), String> {
    let path = dirs::home_dir()
        .ok_or("无法定位用户目录")?
        .join(".workbuddy/models.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|_| "无法读取 WorkBuddy 模型配置，请先打开原客户端")?;
    let expected: [u8; 32] = Sha256::digest(raw.as_bytes()).into();
    let models: Vec<Value> =
        serde_json::from_str(&raw).map_err(|_| "WorkBuddy 模型配置格式异常，未修改")?;
    let models = reconciled_models(models, catalog, key);
    let updated = serde_json::to_string_pretty(&models).map_err(|_| "模型配置序列化失败")?;
    if !super::atomic_write::write_string_atomic_if_hash_matches(&path, expected, || Ok(updated))? {
        return Err("WorkBuddy 模型配置刚被更新，请重新保存；未覆盖其他模型".into());
    }
    Ok(())
}

fn reconciled_models(mut models: Vec<Value>, catalog: &Catalog, key: &str) -> Vec<Value> {
    // 只管理保留前缀，官方及用户其他自定义条目逐项原样保留。
    models.retain(|m| {
        !m.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id.starts_with("workbuddy-pool-"))
    });
    let all_ids: std::collections::HashSet<String> = catalog
        .models
        .iter()
        .filter_map(|m| m["id"].as_str().map(str::to_string))
        .collect();
    for catalog_model in &catalog.models {
        if !is_chat_model(catalog_model) {
            continue;
        }
        let model = native_model(catalog_model, key, &all_ids);
        models.push(model);
    }
    models
}

fn load_pool_key() -> Result<String, String> {
    let path = super::config::get_data_dir()?.join("workbuddy-pool-local.key");
    match std::fs::read_to_string(&path) {
        Ok(key) if uuid::Uuid::parse_str(key.trim()).is_ok() => Ok(key.trim().into()),
        Ok(_) => Err("本地账号池密钥格式异常，未覆盖".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let key = uuid::Uuid::new_v4().to_string();
            super::atomic_write::write_string_atomic(&path, &key)?;
            Ok(key)
        }
        Err(_) => Err("无法读取本地账号池密钥".into()),
    }
}

fn commit_model_selection(
    models_path: &std::path::Path,
    selection_path: &std::path::Path,
    catalog: &Catalog,
    key: &str,
) -> Result<Vec<String>, String> {
    let original = std::fs::read_to_string(models_path)
        .map_err(|_| "请先打开官方 WorkBuddy 客户端，再保存模型选择")?;
    let models: Vec<Value> =
        serde_json::from_str(&original).map_err(|_| "WorkBuddy 模型配置格式异常，未修改")?;
    let ids: Vec<String> = catalog
        .models
        .iter()
        .filter_map(|m| m["id"].as_str().map(str::to_string))
        .collect();
    let selection = serde_json::to_string_pretty(&ModelSelection {
        version: 1,
        model_ids: ids.clone(),
        enabled: Some(!ids.is_empty()),
    })
    .map_err(|_| "模型选择序列化失败")?;
    let updated = serde_json::to_string_pretty(&reconciled_models(models, catalog, key))
        .map_err(|_| "模型配置序列化失败")?;
    let expected: [u8; 32] = Sha256::digest(original.as_bytes()).into();
    let written: [u8; 32] = Sha256::digest(updated.as_bytes()).into();
    if !super::atomic_write::write_string_atomic_if_hash_matches(models_path, expected, || {
        Ok(updated)
    })? {
        return Err("客户端模型列表已变化，请重新保存，未覆盖其他模型".into());
    }
    if let Err(error) = super::atomic_write::write_string_atomic(selection_path, &selection) {
        if !super::atomic_write::write_string_atomic_if_hash_matches(models_path, written, || {
            Ok(original)
        })? {
            return Err("选择保存失败且客户端模型列表又有变化，未回滚覆盖新配置".into());
        }
        return Err(format!("模型选择保存失败，已恢复原配置：{error}"));
    }
    Ok(ids)
}

#[tauri::command]
pub async fn save_workbuddy_pool_models(model_ids: Vec<String>) -> Result<PoolModelsView, String> {
    let view = tauri::async_runtime::spawn_blocking(move || {
        let _config = MODEL_CONFIG_LOCK.lock().map_err(|_| "模型配置锁异常")?;
        let full = if model_ids.is_empty() {
            read_catalog_cache().unwrap_or(Catalog {
                models: Vec::new(),
                updated_at: chrono::Utc::now().timestamp(),
                source: "cache".into(),
            })
        } else {
            read_catalog_cache().or_else(|_| load_catalog())?
        };
        let catalog = selected_catalog(&full, &model_ids)?;
        let service = SERVICE.lock().map_err(|_| "账号池状态锁异常")?;
        let mut running_catalog = service
            .as_ref()
            .map(|pool| pool.catalog.lock())
            .transpose()
            .map_err(|_| "模型目录锁异常")?;
        let key = service
            .as_ref()
            .map(|p| Ok(p.key.clone()))
            .unwrap_or_else(load_pool_key)?;
        let path = dirs::home_dir()
            .ok_or("无法定位用户目录")?
            .join(".workbuddy/models.json");
        let selection_file = selection_path()?;
        let ids = commit_model_selection(&path, &selection_file, &catalog, &key)?;
        if let Some(running_catalog) = running_catalog.as_mut() {
            **running_catalog = catalog;
        }
        Ok::<PoolModelsView, String>(models_view(&full, ids))
    })
    .await
    .map_err(|_| "保存模型选择任务失败")??;
    if view.selected_model_ids.is_empty() {
        stop_workbuddy_pool()?;
    } else {
        start_workbuddy_pool()
            .await
            .map_err(|e| format!("模型已保存，但账号池尚未启用：{e}"))?;
    }
    Ok(view)
}

#[tauri::command]
pub async fn start_workbuddy_pool() -> Result<PoolStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let _config = MODEL_CONFIG_LOCK.lock().map_err(|_| "模型配置锁异常")?;
        let selection = load_selection()?;
        if selection.model_ids.is_empty() {
            return Err("请先添加并保存至少一个账号池模型".to_string());
        }
        let full = read_catalog_cache().or_else(|_| load_catalog())?;
        let catalog = selected_catalog(&full, &selection.model_ids)?;
        let mut service = SERVICE.lock().map_err(|_| "账号池状态锁异常")?;
        if let Some(pool) = service.as_ref() {
            if pool.stopping.load(Ordering::SeqCst) {
                return Err("账号池正在等待在途请求结束，端口释放后可重新启动".into());
            }
            register_model(&pool.key, &catalog)?;
            *pool.catalog.lock().map_err(|_| "模型目录锁异常")? = catalog;
            return Ok(());
        }
        if super::workbuddy_account::list_workbuddy_accounts()?.is_empty() {
            return Err("请先导入 WorkBuddy 账号".to_string());
        }
        let server = Server::http(("127.0.0.1", PORT))
            .map_err(|_| format!("本地端口 {PORT} 已被占用，未修改配置"))?;
        let key = load_pool_key()?;
        let pool = Arc::new(Pool {
            server,
            key,
            stopping: AtomicBool::new(false),
            state: Mutex::new(PoolState::default()),
            catalog: Mutex::new(catalog),
        });
        register_model(&pool.key, &pool.catalog.lock().unwrap())?;
        let saved = ModelSelection {
            version: 1,
            model_ids: selection.model_ids,
            enabled: Some(true),
        };
        super::atomic_write::write_string_atomic(
            &selection_path()?,
            &serde_json::to_string_pretty(&saved).map_err(|_| "账号池启动设置序列化失败")?,
        )?;
        let worker = pool.clone();
        let join = std::thread::spawn(move || {
            for request in worker.server.incoming_requests() {
                if worker.stopping.load(Ordering::SeqCst) {
                    reply(request, 503, "账号池正在停止，未发起新的上游请求");
                    break;
                }
                handle(request, &worker);
                if worker.stopping.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
        *service = Some(PoolService { pool: pool.clone() });
        monitor_worker(pool, join);
        if let Ok(mut error) = LAST_ERROR.lock() {
            *error = None;
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "账号池启动任务失败")??;
    get_workbuddy_pool_status()
}

fn monitor_worker(pool: Arc<Pool>, join: std::thread::JoinHandle<()>) {
    std::thread::spawn(move || {
        let outcome = join.join();
        let mut slot = SERVICE.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_some_and(|s| Arc::ptr_eq(&s.pool, &pool)) {
            let owned = slot.take();
            drop(owned);
            drop(pool);
            if outcome.is_err() {
                if let Ok(mut error) = LAST_ERROR.lock() {
                    *error = Some("账号池工作线程异常退出，请重新启动".into());
                }
            }
        }
    });
}

#[tauri::command]
pub fn stop_workbuddy_pool() -> Result<PoolStatus, String> {
    let _config = MODEL_CONFIG_LOCK.lock().map_err(|_| "模型配置锁异常")?;
    let persist = (|| {
        let mut selection = load_selection()?;
        selection.enabled = Some(false);
        super::atomic_write::write_string_atomic(
            &selection_path()?,
            &serde_json::to_string_pretty(&selection).map_err(|_| "停止设置序列化失败")?,
        )
    })();
    if persist.is_err() {
        if let Ok(mut error) = LAST_ERROR.lock() {
            *error = Some("已请求停止，但自动恢复设置保存失败，请检查文件权限".into());
        }
    }
    if let Some(pool) = SERVICE.lock().map_err(|_| "账号池状态锁异常")?.as_ref() {
        pool.stopping.store(true, Ordering::SeqCst);
        pool.server.unblock();
    }
    get_workbuddy_pool_status()
}

pub(crate) async fn restore_workbuddy_pool() {
    let enabled = tauri::async_runtime::spawn_blocking(|| {
        load_selection()
            .map(|s| s.enabled.unwrap_or(!s.model_ids.is_empty()) && !s.model_ids.is_empty())
    })
    .await;
    if matches!(enabled, Ok(Ok(true))) {
        if let Err(e) = start_workbuddy_pool().await {
            if let Ok(mut error) = LAST_ERROR.lock() {
                *error = Some(format!("账号池自动恢复未完成：{e}"));
            }
        }
    }
}

fn reply(request: Request, status: u16, message: &str) {
    let body = json!({"error": {"message": message, "type": "workbuddy_pool_error"}}).to_string();
    let _ = request.respond(
        Response::from_string(body)
            .with_status_code(StatusCode(status))
            .with_header(Header::from_bytes("Content-Type", "application/json").unwrap()),
    );
}

// Retry only explicit account failures before any response bytes have reached the client.
fn cooldown(status: u16, body: &str) -> Option<Duration> {
    let lower = body.to_ascii_lowercase();
    if lower.contains("insufficient credits")
        || lower.contains("insufficient_credit")
        || body.contains("积分不足")
        || body.contains("额度不足")
    {
        Some(Duration::from_secs(6 * 3600))
    } else if status == 429 {
        Some(Duration::from_secs(60))
    } else if status == 401
        || (status == 403
            && (lower.contains("token")
                || lower.contains("authentication")
                || body.contains("认证失效")))
    {
        Some(Duration::from_secs(300))
    } else {
        None
    }
}

fn first_event(reader: &mut impl BufRead) -> Result<Vec<u8>, ()> {
    let mut prefix = Vec::new();
    loop {
        let mut line = Vec::new();
        let n = (&mut *reader)
            .take((64 * 1024 - prefix.len()) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| ())?;
        if n == 0 || prefix.len() + n >= 64 * 1024 {
            return Err(());
        }
        let end = line == b"\n" || line == b"\r\n";
        prefix.extend(line);
        if end && prefix.windows(5).any(|w| w == b"data:") {
            return Ok(prefix);
        }
    }
}

fn handle(mut request: Request, pool: &Pool) {
    let authenticated = request.headers().iter().any(|h| {
        h.field.equiv("Authorization") && h.value.as_str() == format!("Bearer {}", pool.key)
    });
    if !authenticated {
        reply(request, 401, "本地账号池 API Key 无效");
        return;
    }
    if request.method() == &Method::Get && request.url() == "/v1/models" {
        let data: Vec<Value> = pool.catalog.lock().unwrap().models.iter().filter(|model| is_chat_model(model)).map(|model| json!({"id":format!("workbuddy-pool-{}",model["id"].as_str().unwrap()),"object":"model","owned_by":"local"})).collect();
        let _ = request.respond(
            Response::from_string(json!({"object":"list", "data":data}).to_string())
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
        reply(request, 413, "会话请求超过 64 MiB，未截断上下文");
        return;
    }
    let mut body: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
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
        || body.get("stream").and_then(Value::as_bool) != Some(true)
        || !body.get("messages").is_some_and(Value::is_array)
    {
        reply(request, 400, "请使用 workbuddy-pool- 模型 ID 和流式会话");
        return;
    }
    let upstream_model = upstream_model.unwrap();
    if !pool
        .catalog
        .lock()
        .unwrap()
        .models
        .iter()
        .any(|m| m["id"].as_str() == Some(upstream_model.as_str()))
    {
        reply(request, 400, "该模型未添加到账号池，请选择已保存的模型");
        return;
    }
    body["model"] = json!(upstream_model);
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if message.get("role").and_then(Value::as_str) == Some("developer") {
                message["role"] = json!("system");
            }
        }
    }
    let accounts = match super::workbuddy_account::list_workbuddy_accounts() {
        Ok(a) => a,
        Err(_) => {
            reply(request, 503, "无法读取账号池");
            return;
        }
    };
    let recover = |account: &crate::models::workbuddy::WorkBuddyAccountView| {
        tauri::async_runtime::block_on(super::workbuddy_account::refresh_account_auth(&account.id))
    };
    relay(
        request,
        pool,
        body,
        accounts,
        UPSTREAM,
        |account| super::workbuddy_account::access_token(&account.id),
        Some(&recover),
    );
}

fn relay(
    request: Request,
    pool: &Pool,
    body: Value,
    mut accounts: Vec<crate::models::workbuddy::WorkBuddyAccountView>,
    upstream: &str,
    token_for: impl Fn(&crate::models::workbuddy::WorkBuddyAccountView) -> Result<String, String>,
    recover: Option<
        &dyn Fn(&crate::models::workbuddy::WorkBuddyAccountView) -> Result<String, String>,
    >,
) {
    let preferred = pool.state.lock().unwrap().preferred.clone();
    accounts.sort_by_key(|a| preferred.as_deref() != Some(&a.id));
    pool.state.lock().unwrap().requests += 1;
    let host = url::Url::parse(upstream)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "copilot.tencent.com".into());
    let client = match crate::utils::http::create_domestic_blocking_client(180, &host) {
        Ok(client) => client,
        Err(_) => {
            reply(request, 503, "创建上游连接失败");
            return;
        }
    };
    'accounts: for account in accounts {
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
        let mut token = match token_for(&account) {
            Ok(t) => t,
            Err(_) => {
                pool.state.lock().unwrap().cooldowns.insert(
                    account.id.clone(),
                    Instant::now() + Duration::from_secs(300),
                );
                continue;
            }
        };
        for attempt in 0..2 {
            let response = client
                .post(upstream)
                .bearer_auth(&token)
                .header("Accept", "application/json, text/event-stream")
                .header("X-User-Id", &account.uid)
                .header("X-No-Enterprise-Id", "1")
                .header("X-No-Department-Info", "1")
                .header("X-Product", "SaaS")
                .header("X-CodeBuddy-Request", "1")
                .header("X-Agent-Purpose", "conversation")
                .header("Origin", "https://copilot.tencent.com")
                .header("Referer", "https://copilot.tencent.com/")
                .json(&body)
                .send();
            let response = match response {
                Ok(r) => r,
                Err(_) => {
                    reply(request, 502, "上游连接中断，请手动重试；未自动重放请求");
                    return;
                }
            };
            let status = response.status().as_u16();
            if status == 401 && attempt == 0 {
                if let Some(recover) = recover {
                    if let Ok(fresh) = recover(&account) {
                        if fresh != token {
                            token = fresh;
                            continue;
                        }
                    }
                }
            }
            if !(200..300).contains(&status) {
                let mut error = String::new();
                let _ = response.take(64 * 1024).read_to_string(&mut error);
                if let Some(delay) = cooldown(status, &error) {
                    let mut state = pool.state.lock().unwrap();
                    state
                        .cooldowns
                        .insert(account.id.clone(), Instant::now() + delay);
                    state.rotations += 1;
                    continue 'accounts;
                }
                reply(
                    request,
                    status,
                    "上游拒绝请求，未换号重放；请检查模型和上下文",
                );
                return;
            }
            let mut reader = BufReader::new(response);
            let prefix = match first_event(&mut reader) {
                Ok(p) => p,
                Err(_) => {
                    reply(request, 502, "上游未返回有效流式首帧，未自动重放");
                    return;
                }
            };
            // A successful HTTP status can still carry a quota error as its first SSE event.
            let text = String::from_utf8_lossy(&prefix);
            let error_event = text
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .filter_map(|s| serde_json::from_str::<Value>(s).ok())
                .any(|v| {
                    v.get("error").is_some()
                        || v.get("code")
                            .and_then(Value::as_i64)
                            .is_some_and(|c| c != 0)
                });
            if error_event {
                if let Some(delay) = cooldown(status, &text) {
                    let mut state = pool.state.lock().unwrap();
                    state
                        .cooldowns
                        .insert(account.id.clone(), Instant::now() + delay);
                    state.rotations += 1;
                    continue 'accounts;
                }
                reply(request, 502, "上游流式首帧返回错误，未重放");
                return;
            }
            pool.state.lock().unwrap().preferred = Some(account.id);
            let stream = std::io::Cursor::new(prefix).chain(reader);
            let _ = request.respond(Response::new(
                StatusCode(200),
                vec![
                    Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                    Header::from_bytes("Cache-Control", "no-cache").unwrap(),
                ],
                stream,
                None,
                None,
            ));
            return; // Never retry once response streaming has started, including disconnects.
        }
    }
    reply(
        request,
        503,
        "全部账号暂不可用或处于冷却期，请刷新积分或重新登录",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopping_is_visible_until_worker_and_listener_finish() {
        use std::io::{Read, Write};
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("pool-stop-check-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
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
        });
        let addr = pool.server.server_addr().to_string();
        let worker_pool = pool.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let req = worker_pool.server.recv().unwrap();
            ready_tx.send(()).unwrap();
            done_rx.recv().unwrap();
            let _ = req.respond(Response::from_string("done"));
        });
        *SERVICE.lock().unwrap() = Some(PoolService { pool: pool.clone() });
        monitor_worker(pool, worker);
        let target = addr.clone();
        let client = std::thread::spawn(move || {
            let mut tcp = std::net::TcpStream::connect(target).unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            tcp.write_all(b"GET / HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n")
                .unwrap();
            let mut result = String::new();
            let _ = tcp.read_to_string(&mut result);
        });
        ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let status = stop_workbuddy_pool().unwrap();
        assert!(status.running && status.stopping);
        assert!(std::net::TcpListener::bind(&addr).is_err());
        done_tx.send(()).unwrap();
        client.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while SERVICE.lock().unwrap().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!get_workbuddy_pool_status().unwrap().running);
        assert!(std::net::TcpListener::bind(&addr).is_ok());
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(dir);
    }
    fn check_auth_recovery_once(succeed: bool) {
        let upstream = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", upstream.server_addr());
        let upstream_worker = std::thread::spawn(move || {
            for token in ["old", "fresh"] {
                let req = upstream
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert!(req.headers().iter().any(|h| h.field.equiv("Authorization")
                    && h.value.as_str() == format!("Bearer {token}")));
                let response = if token == "fresh" && succeed {
                    Response::from_string("data: {\"choices\":[{\"delta\":{\"content\":\"OK\"}}]}\n\ndata: [DONE]\n\n")
                } else {
                    Response::from_string("unauthorized").with_status_code(StatusCode(401))
                };
                req.respond(response).unwrap();
            }
            assert!(upstream
                .recv_timeout(Duration::from_millis(100))
                .unwrap()
                .is_none());
        });
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
        });
        let endpoint = format!("http://{}", pool.server.server_addr());
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || {
            let calls = std::cell::Cell::new(0);
            let recovery = |_: &crate::models::workbuddy::WorkBuddyAccountView| {
                calls.set(calls.get() + 1);
                Ok("fresh".to_string())
            };
            let req = worker_pool.server.recv().unwrap();
            relay(
                req,
                &worker_pool,
                json!({"model":"fixture","stream":true,"messages":[]}),
                vec![account("a")],
                &url,
                |_| Ok("old".into()),
                Some(&recovery),
            );
            assert_eq!(calls.get(), 1);
        });
        let response = reqwest::blocking::Client::new()
            .post(endpoint)
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), if succeed { 200 } else { 503 });
        let _ = response.text().unwrap();
        worker.join().unwrap();
        upstream_worker.join().unwrap();
    }
    #[test]
    fn authentication_recovery_retries_same_account_once_before_streaming() {
        check_auth_recovery_once(true);
    }
    #[test]
    fn rejected_recovery_does_not_loop_or_replay_indefinitely() {
        check_auth_recovery_once(false);
    }

    fn selection_fixture() -> Catalog {
        Catalog {
            models: vec![
                json!({"id":"a","name":"模型 A","supportsToolCall":true,
            "contextWindow":{"defaultLength":100000},"relatedModels":{"reasoning":"b"}}),
                json!({"id":"b","name":"模型 B"}),
                json!({"id":"image","tags":["text-to-image"]}),
            ],
            updated_at: 123,
            source: "fixture".into(),
        }
    }
    #[test]
    fn new_model_selection_is_empty_instead_of_all_models() {
        assert!(ModelSelection::default().model_ids.is_empty());
        assert!(selected_catalog(&selection_fixture(), &[])
            .unwrap()
            .models
            .is_empty());
    }
    #[test]
    fn selection_validates_ids_and_keeps_official_capabilities() {
        let catalog = selection_fixture();
        let selected = selected_catalog(&catalog, &["a".into(), "a".into()]).unwrap();
        assert_eq!(selected.models.len(), 1);
        assert_eq!(selected.models[0], catalog.models[0]);
        assert!(selected_catalog(&catalog, &["missing".into()]).is_err());
        assert!(selected_catalog(&catalog, &["image".into()]).is_err());
    }
    #[test]
    fn removing_pool_models_preserves_official_and_custom_entries() {
        let existing = vec![
            json!({"id":"official","isDefault":true}),
            json!({"id":"my-custom-model","url":"https://example.invalid","notes":"keep"}),
            json!({"id":"workbuddy-pool-a"}),
            json!({"id":"workbuddy-pool-b"}),
        ];
        let chosen = selected_catalog(&selection_fixture(), &["a".into()]).unwrap();
        let result = reconciled_models(existing.clone(), &chosen, "fixture");
        assert_eq!(&result[..2], &existing[..2]);
        assert_eq!(result.len(), 3);
        assert_eq!(result[2]["id"], "workbuddy-pool-a");
        assert!(result[2]["relatedModels"].as_object().unwrap().is_empty());
        let cleared = reconciled_models(
            existing.clone(),
            &selected_catalog(&selection_fixture(), &[]).unwrap(),
            "fixture",
        );
        assert_eq!(cleared, existing[..2]);
    }
    #[test]
    fn model_selection_save_failure_restores_client_configuration() {
        let dir = std::env::temp_dir().join(format!("pool-selection-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let models = dir.join("models.json");
        let original = r#"[{"id":"official","notes":"keep"},{"id":"workbuddy-pool-old"}]"#;
        std::fs::write(&models, original).unwrap();
        let blocked = dir.join("blocked.json");
        std::fs::create_dir(&blocked).unwrap();
        let chosen = selected_catalog(&selection_fixture(), &["a".into()]).unwrap();
        assert!(commit_model_selection(&models, &blocked, &chosen, "fixture").is_err());
        assert_eq!(std::fs::read_to_string(&models).unwrap(), original);
        assert!(blocked.is_dir());
        let selection = dir.join("selection.json");
        assert_eq!(
            commit_model_selection(&models, &selection, &chosen, "fixture").unwrap(),
            vec!["a"]
        );
        let saved: ModelSelection =
            serde_json::from_str(&std::fs::read_to_string(&selection).unwrap()).unwrap();
        assert_eq!(saved.model_ids, vec!["a"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn all_catalog_models_keep_capabilities_and_related_variants() {
        let source = json!({"id":"new-vendor-v9","name":"New Model","supportsImages":true,"supportsReasoning":true,
            "supportsToolCall":true,"contextWindow":{"defaultLength":300000},"reasoning":{"supportedEfforts":["low","high"]},
            "relatedModels":{"reasoning":"new-vendor-v9"},"vendor":"e"});
        let ids = std::collections::HashSet::from(["new-vendor-v9".to_string()]);
        let item = native_model(&source, "fixture", &ids);
        for key in [
            "supportsImages",
            "supportsReasoning",
            "supportsToolCall",
            "contextWindow",
            "reasoning",
            "vendor",
        ] {
            assert_eq!(item[key], source[key]);
        }
        assert_eq!(
            item["relatedModels"]["reasoning"],
            "workbuddy-pool-new-vendor-v9"
        );
        assert_eq!(
            model_id("workbuddy-pool-new-vendor-v9"),
            Some("new-vendor-v9")
        );
    }
    #[test]
    fn catalog_deduplicates_but_does_not_limit_to_two_models() {
        let models = parse_catalog(&json!({"data":{"models":[{"id":"hy3"},{"id":"glm-5.3"},{"id":"kimi-k3-1"},{"id":"glm-5.3"}]}})).unwrap();
        assert_eq!(models.len(), 3);
        assert_eq!(models[2]["id"], "kimi-k3-1");
        assert!(parse_catalog(&json!({"data":{"models":[]}})).is_err());
        assert!(parse_catalog(&json!({"error":"offline"})).is_err());
        for invalid in [
            "workbuddy-pool-",
            "workbuddy-pool-https://other",
            "raw-model",
        ] {
            assert!(model_id(invalid).is_none());
        }
    }
    #[test]
    fn specialized_generation_models_are_not_mislabelled_as_chat_models() {
        assert!(!is_chat_model(
            &json!({"id":"image","tags":["text-to-image"]})
        ));
        assert!(is_chat_model(&json!({"id":"vision","supportsImages":true})));
    }
    #[test]
    fn quota_cools_for_six_hours() {
        assert_eq!(
            cooldown(200, "{\"error\":\"积分不足\"}"),
            Some(Duration::from_secs(21600))
        );
    }
    #[test]
    fn limits_and_auth_are_distinct() {
        assert_eq!(cooldown(429, "rate limit"), Some(Duration::from_secs(60)));
        assert_eq!(cooldown(401, ""), Some(Duration::from_secs(300)));
    }
    #[test]
    fn uncertain_failures_are_not_rotation_signals() {
        for code in [200, 400, 403, 404, 500, 502] {
            assert_eq!(cooldown(code, "network failure"), None);
        }
    }
    #[test]
    fn first_frame_preserves_utf8_and_tool_arguments() {
        let input = b": ping\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"arguments\":\"{}\"}}]}}]}\n\ndata: [DONE]\n\n";
        let mut reader = BufReader::new(&input[..]);
        let prefix = first_event(&mut reader).unwrap();
        let mut result = prefix;
        reader.read_to_end(&mut result).unwrap();
        assert_eq!(result, input);
    }
    #[test]
    fn incomplete_or_oversized_first_frame_fails() {
        assert!(first_event(&mut BufReader::new(&b"data: incomplete"[..])).is_err());
        assert!(first_event(&mut BufReader::new(&vec![b'x'; 65536][..])).is_err());
    }
    fn account(id: &str) -> crate::models::workbuddy::WorkBuddyAccountView {
        serde_json::from_value(json!({"id":id,"uid":id,"uin":null,"displayName":"fixture","maskedPhone":null,
            "checkinEnabled":false,"tokenExpiresAt":null,"createdAt":0,"updatedAt":0,"lastUsedAt":null,
            "lastGithubSyncAt":null,"lastGithubSyncState":"idle","lastGithubSyncError":null})).unwrap()
    }
    fn mock_rotation(first_status: u16, first_body: &str) {
        let upstream = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", upstream.server_addr());
        let first_body = first_body.to_string();
        let sent = json!({"model":"hy3","stream":true,"messages":[{"role":"tool","tool_call_id":"call-1","content":"原文"}],
            "tools":[{"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}]});
        let expected = sent.clone();
        let upstream_worker = std::thread::spawn(move || {
            for id in ["a", "b"] {
                let mut req = upstream
                    .recv_timeout(Duration::from_secs(10))
                    .unwrap()
                    .expect("upstream request");
                assert!(req.headers().iter().any(|h| h.field.equiv("Authorization")
                    && h.value.as_str() == format!("Bearer fixture-{id}")));
                let mut raw = String::new();
                req.as_reader().read_to_string(&mut raw).unwrap();
                assert_eq!(serde_json::from_str::<Value>(&raw).unwrap(), expected);
                let response = if id == "a" {
                    Response::from_string(first_body.clone())
                        .with_status_code(StatusCode(first_status))
                } else {
                    Response::from_string("data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"arguments\":\"{}\"}}]}}]}\n\ndata: [DONE]\n\n")
                };
                req.respond(response).unwrap();
            }
        });
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
        });
        let endpoint = format!("http://{}", pool.server.server_addr());
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || {
            let req = worker_pool
                .server
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            relay(
                req,
                &worker_pool,
                sent,
                vec![account("a"), account("b")],
                &url,
                |a| Ok(format!("fixture-{}", a.id)),
                None,
            );
        });
        let response = reqwest::blocking::Client::new()
            .post(endpoint)
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 200);
        let body = response.text().unwrap();
        assert!(body.contains("tool_calls"));
        assert!(body.contains("[DONE]"));
        worker.join().unwrap();
        upstream_worker.join().unwrap();
        let state = pool.state.lock().unwrap();
        assert_eq!(state.rotations, 1);
        assert_eq!(state.preferred.as_deref(), Some("b"));
    }
    #[test]
    fn http_quota_failure_rotates_without_losing_context() {
        mock_rotation(429, "insufficient credits");
    }
    #[test]
    fn sse_quota_failure_rotates_before_output() {
        mock_rotation(200, "data: {\"error\":{\"message\":\"积分不足\"}}\n\n");
    }
    #[test]
    fn successful_stream_is_never_replayed_even_with_later_error() {
        let upstream = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", upstream.server_addr());
        let upstream_worker = std::thread::spawn(move || {
            let req = upstream
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            req.respond(Response::from_string("data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\ndata: {\"error\":\"积分不足\"}\n\n")).unwrap();
            assert!(upstream
                .recv_timeout(Duration::from_millis(300))
                .unwrap()
                .is_none());
        });
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
        });
        let endpoint = format!("http://{}", pool.server.server_addr());
        let worker_pool = pool.clone();
        let worker = std::thread::spawn(move || {
            let req = worker_pool
                .server
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .unwrap();
            relay(
                req,
                &worker_pool,
                json!({"model":"hy3","stream":true,"messages":[]}),
                vec![account("a"), account("b")],
                &url,
                |a| Ok(format!("fixture-{}", a.id)),
                None,
            );
        });
        let body = reqwest::blocking::Client::new()
            .post(endpoint)
            .send()
            .unwrap()
            .text()
            .unwrap();
        assert!(body.contains("partial"));
        worker.join().unwrap();
        upstream_worker.join().unwrap();
        assert_eq!(pool.state.lock().unwrap().rotations, 0);
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
    #[test]
    #[ignore = "Clears real pool model selections without sending chat requests; run explicitly"]
    fn live_empty_selection_without_quota() {
        let path = dirs::home_dir().unwrap().join(".workbuddy/models.json");
        let original: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let retained: Vec<Value> = original
            .into_iter()
            .filter(|m| {
                !m["id"]
                    .as_str()
                    .unwrap_or("")
                    .starts_with("workbuddy-pool-")
            })
            .collect();
        let view = tauri::async_runtime::block_on(save_workbuddy_pool_models(vec![])).unwrap();
        assert!(view.selected_model_ids.is_empty());
        let after: Vec<Value> =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(after, retained);
        assert!(load_selection().unwrap().model_ids.is_empty());
        let reloaded =
            tauri::async_runtime::block_on(get_workbuddy_pool_models(Some(false))).unwrap();
        assert!(reloaded.selected_model_ids.is_empty());
        assert!(!reloaded.models.is_empty());
        assert!(tauri::async_runtime::block_on(start_workbuddy_pool()).is_err());
        assert!(!get_workbuddy_pool_status().unwrap().running);
    }
    #[test]
    #[ignore = "Uses real account quota for saved selections; run explicitly"]
    fn live_pool_round_trip() {
        let status = tauri::async_runtime::block_on(start_workbuddy_pool()).unwrap();
        assert!(status.running);
        let key = SERVICE.lock().unwrap().as_ref().unwrap().key.clone();
        let mut results = Vec::new();
        let models: Vec<String> = SERVICE
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .catalog
            .lock()
            .unwrap()
            .models
            .iter()
            .filter(|m| is_chat_model(m))
            .filter_map(|m| m["id"].as_str().map(str::to_string))
            .collect();
        assert!(!models.is_empty());
        for model in models {
            let response = reqwest::blocking::Client::new().post(&status.endpoint).bearer_auth(&key).json(&json!({
            "model":format!("workbuddy-pool-{model}"),"stream":true,"max_tokens":16,"messages":[{"role":"user","content":"Reply with OK."}]
        })).send().unwrap();
            results.push((model, response.status(), response.text().unwrap()));
        }
        stop_workbuddy_pool().unwrap();
        for (model, code, body) in results {
            assert_eq!(code.as_u16(), 200, "model={model}");
            assert!(body.contains("choices"), "model={model}");
            assert!(body.contains("[DONE]"), "model={model}");
        }
    }
}

use crate::models::workbuddy::{WorkBuddyAccountUpdate, WorkBuddyAccountView};
use base64::Engine;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

const INDEX_FILE: &str = "workbuddy_accounts.json";
const DETAILS_DIR: &str = "workbuddy_accounts";
const SNAPSHOT_KIND: &str = "workbuddy";

static ACCOUNT_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static REFRESH_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkBuddyAccountRecord {
    id: String,
    uid: String,
    uin: Option<String>,
    display_name: String,
    masked_phone: Option<String>,
    checkin_enabled: bool,
    token_expires_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    last_used_at: Option<i64>,
    last_github_sync_at: Option<i64>,
    last_github_sync_state: String,
    #[serde(default)]
    last_github_sync_error: Option<String>,
    snapshot: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WorkBuddyAccountSummary {
    id: String,
    uid: String,
    uin: Option<String>,
    display_name: String,
    masked_phone: Option<String>,
    checkin_enabled: bool,
    token_expires_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    last_used_at: Option<i64>,
    last_github_sync_at: Option<i64>,
    last_github_sync_state: String,
    #[serde(default)]
    last_github_sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkBuddyAccountIndex {
    version: u32,
    accounts: Vec<WorkBuddyAccountSummary>,
}

impl Default for WorkBuddyAccountIndex {
    fn default() -> Self {
        Self {
            version: 1,
            accounts: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct ParsedSnapshot {
    value: Value,
    uid: String,
    uin: Option<String>,
    nickname: Option<String>,
    masked_phone: Option<String>,
    token_expires_at: Option<i64>,
}

/// 可写入日志的认证摘要。只保留不可逆的短指纹，绝不保留 token 原文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkBuddyAuthDiagnostic {
    pub account_id: String,
    pub access_token_fingerprint: String,
    pub refresh_token_fingerprint: Option<String>,
    pub expires_at: Option<i64>,
    pub refresh_expires_at: Option<i64>,
    pub byte_len: usize,
}

impl WorkBuddyAuthDiagnostic {
    pub fn to_log_fields(&self) -> String {
        format!(
            "account_id={} access_fp={} refresh_fp={} expires_at={} refresh_expires_at={} bytes={}",
            self.account_id,
            self.access_token_fingerprint,
            self.refresh_token_fingerprint
                .as_deref()
                .unwrap_or("<missing>"),
            self.expires_at
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<missing>".to_string()),
            self.refresh_expires_at
                .map(|value| value.to_string())
                .unwrap_or_else(|| "<missing>".to_string()),
            self.byte_len
        )
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn sanitized_display_name(value: Option<&str>) -> Option<String> {
    let value = value?;
    let cleaned = value
        .chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect::<String>();
    non_empty(Some(&cleaned))
}

fn mask_phone(value: Option<&str>) -> Option<String> {
    let phone = non_empty(value)?;
    let chars = phone.chars().collect::<Vec<_>>();
    if chars.len() < 7 {
        return Some("****".to_string());
    }
    let prefix = chars.iter().take(3).collect::<String>();
    let suffix = chars.iter().skip(chars.len() - 4).collect::<String>();
    Some(format!("{}****{}", prefix, suffix))
}

fn parse_i64(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(value)) => value.trim().parse().ok(),
        _ => None,
    }
}

fn normalize_epoch_seconds(value: i64) -> i64 {
    if value > 10_000_000_000 {
        value / 1_000
    } else {
        value
    }
}

fn object_string(value: Option<&Value>, keys: &[&str]) -> Option<String> {
    let object = value?.as_object()?;
    keys.iter()
        .find_map(|key| non_empty(object.get(*key).and_then(Value::as_str)))
}

fn object_i64(value: Option<&Value>, keys: &[&str]) -> Option<i64> {
    let object = value?.as_object()?;
    keys.iter().find_map(|key| parse_i64(object.get(*key)))
}

fn normalize_access_token(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((prefix, suffix)) = trimmed.split_once('+') {
        if !prefix.trim().is_empty() && !suffix.trim().is_empty() {
            return Some(suffix.trim().to_string());
        }
    }
    Some(trimmed.to_string())
}

fn raw_access_token_from_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => non_empty(Some(value)),
        Value::Array(values) => values.iter().find_map(raw_access_token_from_value),
        Value::Object(object) => {
            for key in ["accessToken", "access_token", "token"] {
                if let Some(token) = object.get(key).and_then(Value::as_str) {
                    return non_empty(Some(token));
                }
            }
            for key in ["auth", "session", "data"] {
                if let Some(token) = object.get(key).and_then(raw_access_token_from_value) {
                    return Some(token);
                }
            }
            None
        }
        _ => None,
    }
}

fn access_token_from_value(value: &Value) -> Option<String> {
    raw_access_token_from_value(value).and_then(|token| normalize_access_token(&token))
}

fn merge_refreshed_auth(
    snapshot: &mut Value,
    access_token: &str,
    refresh_token: Option<&str>,
    expires_at: Option<i64>,
    refresh_expires_at: Option<i64>,
) -> Result<(), String> {
    let auth = snapshot
        .as_object_mut()
        .ok_or_else(|| "WorkBuddy 认证快照结构无效".to_string())?
        .entry("auth")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let auth = auth
        .as_object_mut()
        .ok_or_else(|| "WorkBuddy 认证快照 auth 结构无效".to_string())?;
    auth.insert(
        "accessToken".to_string(),
        Value::String(access_token.to_string()),
    );
    if let Some(refresh_token) = refresh_token.filter(|value| !value.trim().is_empty()) {
        auth.insert(
            "refreshToken".to_string(),
            Value::String(refresh_token.to_string()),
        );
    }
    if let Some(expires_at) = expires_at {
        auth.insert("expiresAt".to_string(), Value::Number(expires_at.into()));
    }
    if let Some(refresh_expires_at) = refresh_expires_at {
        auth.insert(
            "refreshExpiresAt".to_string(),
            Value::Number(refresh_expires_at.into()),
        );
    }
    Ok(())
}

fn token_from_value_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::String(_) => None,
        Value::Array(values) => values
            .iter()
            .find_map(|value| token_from_value_by_keys(value, keys)),
        Value::Object(object) => {
            for key in keys {
                if let Some(token) = object.get(*key).and_then(Value::as_str) {
                    if let Some(token) = non_empty(Some(token)) {
                        return Some(token);
                    }
                }
            }
            for key in ["auth", "session", "data"] {
                if let Some(token) = object
                    .get(key)
                    .and_then(|value| token_from_value_by_keys(value, keys))
                {
                    return Some(token);
                }
            }
            None
        }
        _ => None,
    }
}

fn secret_fingerprint(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    format!("sha256:{}", &format!("{:x}", digest)[..12])
}

fn auth_snapshot_diagnostic(raw: &str) -> Result<WorkBuddyAuthDiagnostic, String> {
    let parsed = parse_snapshot_json(raw)?;
    let access_token = access_token_from_value(&parsed.value)
        .ok_or_else(|| "WorkBuddy 认证文件缺少 access token".to_string())?;
    let auth = parsed.value.get("auth");
    let refresh_expires_at = object_i64(
        Some(&parsed.value),
        &["refreshExpiresAt", "refresh_expires_at"],
    )
    .or_else(|| object_i64(auth, &["refreshExpiresAt", "refresh_expires_at"]))
    .map(normalize_epoch_seconds);
    let refresh_token_fingerprint =
        token_from_value_by_keys(&parsed.value, &["refreshToken", "refresh_token"])
            .as_deref()
            .map(secret_fingerprint);

    Ok(WorkBuddyAuthDiagnostic {
        account_id: stable_account_id(&parsed.uid),
        access_token_fingerprint: secret_fingerprint(&access_token),
        refresh_token_fingerprint,
        expires_at: parsed.token_expires_at,
        refresh_expires_at,
        byte_len: raw.len(),
    })
}

pub(crate) fn auth_file_diagnostic(path: &Path) -> Result<String, String> {
    let raw = read_stable_auth_file(path)?;
    let marker = PathBuf::from(format!("{}.logged-out", path.to_string_lossy()));
    Ok(format!(
        "{} logout_marker={}",
        auth_snapshot_diagnostic(&raw)?.to_log_fields(),
        marker.exists()
    ))
}

fn uid_from_jwt(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(payload))
        .ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    ["sub", "uid", "userId"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .and_then(|value| non_empty(Some(value)))
}

fn uid_from_token(raw_token: &str) -> Option<String> {
    let trimmed = raw_token.trim();
    if let Some((prefix, suffix)) = trimmed.split_once('+') {
        if !prefix.trim().is_empty() && !suffix.trim().is_empty() {
            return non_empty(Some(prefix));
        }
    }
    uid_from_jwt(trimmed)
}

fn parse_snapshot_json(raw: &str) -> Result<ParsedSnapshot, String> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|error| format!("WorkBuddy 认证文件 JSON 无效: {}", error))?;
    let account = value.get("account");
    let auth = value.get("auth");
    let raw_token = raw_access_token_from_value(&value)
        .ok_or_else(|| "WorkBuddy 认证文件缺少 access token".to_string())?;
    let uid = object_string(Some(&value), &["uid", "id"])
        .or_else(|| object_string(account, &["uid", "id"]))
        .or_else(|| object_string(auth, &["uid", "userId"]))
        .or_else(|| uid_from_token(&raw_token))
        .ok_or_else(|| "WorkBuddy 认证文件缺少账号 UID".to_string())?;
    let uin = object_string(account, &["uin"]).or_else(|| object_string(Some(&value), &["uin"]));
    let nickname = sanitized_display_name(
        object_string(account, &["nickname", "name", "label"])
            .or_else(|| object_string(Some(&value), &["nickname", "name"]))
            .as_deref(),
    );
    let masked_phone = mask_phone(
        object_string(account, &["phoneNumber", "phone"])
            .or_else(|| object_string(Some(&value), &["phoneNumber", "phone"]))
            .as_deref(),
    );
    let token_expires_at = object_i64(Some(&value), &["expiresAt", "expires_at"])
        .or_else(|| object_i64(auth, &["expiresAt", "expires_at"]))
        .map(normalize_epoch_seconds);

    Ok(ParsedSnapshot {
        value,
        uid,
        uin,
        nickname,
        masked_phone,
        token_expires_at,
    })
}

pub fn stable_account_id(uid: &str) -> String {
    let digest = Sha256::digest(uid.as_bytes());
    let hex = format!("{:x}", digest);
    format!("wb-{}", &hex[..12])
}

fn valid_account_id(value: &str) -> bool {
    value.len() == 15
        && value.starts_with("wb-")
        && value[3..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

pub(crate) fn index_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(INDEX_FILE))
}

fn details_dir() -> Result<PathBuf, String> {
    let directory = crate::modules::account::get_data_dir()?.join(DETAILS_DIR);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("创建 WorkBuddy 账号目录失败: {}", error))?;
    Ok(directory)
}

pub(crate) fn detail_path(account_id: &str) -> Result<PathBuf, String> {
    if !valid_account_id(account_id) {
        return Err("WorkBuddy 账号 ID 无效".to_string());
    }
    Ok(details_dir()?.join(format!("{}.json", account_id)))
}

/// 索引对账缓存：`load_index` 历史上每次调用都会触发对账（解密全部账号
/// 明细文件），账号多且杀软实时扫描时可达数秒，是页面加载账号列表慢的
/// 主因。这里以（索引文件 mtime + 明细目录文件名/mtime 清单）为指纹，
/// 磁盘状态未变化时直接复用上次对账结果（list_workbuddy_accounts 与
/// has_account_id 均受益）。
static INDEX_RECONCILE_CACHE: LazyLock<
    Mutex<Option<(IndexFingerprint, WorkBuddyAccountIndex)>>,
> = LazyLock::new(|| Mutex::new(None));

#[derive(Clone, PartialEq, Eq)]
struct IndexFingerprint {
    index_modified: Option<std::time::SystemTime>,
    details: Vec<(String, Option<std::time::SystemTime>)>,
}

fn index_fingerprint() -> Result<IndexFingerprint, String> {
    let index_modified = index_path()?
        .metadata()
        .and_then(|meta| meta.modified())
        .ok();
    let mut details = Vec::new();
    if let Ok(entries) = fs::read_dir(details_dir()?) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let modified = entry.metadata().and_then(|meta| meta.modified()).ok();
            details.push((name, modified));
        }
    }
    details.sort();
    Ok(IndexFingerprint {
        index_modified,
        details,
    })
}

fn load_index() -> Result<WorkBuddyAccountIndex, String> {
    let fingerprint = index_fingerprint()?;
    if let Some((cached_at, cached)) = INDEX_RECONCILE_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.clone())
    {
        if cached_at == fingerprint {
            return Ok(cached);
        }
    }

    let path = index_path()?;
    let reconciled = if !path.exists() {
        repair_index_from_details("索引文件不存在")?
    } else {
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("读取 WorkBuddy 账号索引失败: {}", error))?;
        match crate::modules::atomic_write::parse_json_with_auto_restore::<WorkBuddyAccountIndex>(
            &path, &content,
        ) {
            Ok(index) => reconcile_index_with_details(index)?,
            Err(_) => repair_index_from_details("索引文件损坏")?,
        }
    };

    if let Ok(mut cache) = INDEX_RECONCILE_CACHE.lock() {
        *cache = Some((fingerprint, reconciled.clone()));
    }
    Ok(reconciled)
}

fn save_index(index: &WorkBuddyAccountIndex) -> Result<(), String> {
    let content = serde_json::to_string_pretty(index)
        .map_err(|error| format!("序列化 WorkBuddy 账号索引失败: {}", error))?;
    crate::modules::atomic_write::write_string_atomic(&index_path()?, &content)
        .map_err(|error| format!("保存 WorkBuddy 账号索引失败: {}", error))
}

fn build_index_from_details() -> Result<WorkBuddyAccountIndex, String> {
    let directory = details_dir()?;
    let records = crate::modules::account_index_repair::load_accounts_from_details(
        &directory,
        |account_id| {
            valid_account_id(account_id)
                .then(|| load_record_read_only(account_id).ok())
                .flatten()
        },
    )?;
    Ok(WorkBuddyAccountIndex {
        version: 1,
        accounts: records.iter().map(summary).collect(),
    })
}

fn reconcile_index_with_details(
    index: WorkBuddyAccountIndex,
) -> Result<WorkBuddyAccountIndex, String> {
    let rebuilt = build_index_from_details()?;
    if index.accounts == rebuilt.accounts {
        Ok(index)
    } else {
        repair_index_from_details("索引与加密账号详情不一致")
    }
}

fn repair_index_from_details(reason: &str) -> Result<WorkBuddyAccountIndex, String> {
    let repaired = build_index_from_details()?;
    let path = index_path()?;
    if let Err(error) = crate::modules::account_index_repair::backup_existing_index(&path) {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Account] 重建索引前备份失败，继续覆盖损坏索引: {}",
            error
        ));
    }
    save_index(&repaired)?;
    crate::modules::logger::log_warn(&format!(
        "[WorkBuddy Account] 已从加密账号详情重建索引: reason={}, recovered_accounts={}",
        reason,
        repaired.accounts.len()
    ));
    Ok(repaired)
}

fn save_record(record: &WorkBuddyAccountRecord) -> Result<(), String> {
    let content =
        crate::modules::secure_account_storage::serialize_account_file(SNAPSHOT_KIND, record)?;
    crate::modules::atomic_write::write_string_atomic(&detail_path(&record.id)?, &content)
}

fn load_record(account_id: &str) -> Result<WorkBuddyAccountRecord, String> {
    let path = detail_path(account_id)?;
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取 WorkBuddy 账号详情失败: {}", error))?;
    let (record, needs_rewrite) =
        crate::modules::secure_account_storage::deserialize_account_file(&path, &content)?;
    if needs_rewrite {
        save_record(&record)?;
    }
    Ok(record)
}

fn load_record_read_only(account_id: &str) -> Result<WorkBuddyAccountRecord, String> {
    let path = detail_path(account_id)?;
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取 WorkBuddy 账号详情失败: {}", error))?;
    let (record, _) =
        crate::modules::secure_account_storage::deserialize_account_file(&path, &content)?;
    Ok(record)
}

fn summary(record: &WorkBuddyAccountRecord) -> WorkBuddyAccountSummary {
    WorkBuddyAccountSummary {
        id: record.id.clone(),
        uid: record.uid.clone(),
        uin: record.uin.clone(),
        display_name: record.display_name.clone(),
        masked_phone: record.masked_phone.clone(),
        checkin_enabled: record.checkin_enabled,
        token_expires_at: record.token_expires_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_used_at: record.last_used_at,
        last_github_sync_at: record.last_github_sync_at,
        last_github_sync_state: record.last_github_sync_state.clone(),
        last_github_sync_error: record.last_github_sync_error.clone(),
    }
}

fn view(value: &WorkBuddyAccountSummary) -> WorkBuddyAccountView {
    let last_github_sync_state =
        if value.last_github_sync_state == "failed" && value.last_github_sync_error.is_none() {
            "pending".to_string()
        } else {
            value.last_github_sync_state.clone()
        };

    WorkBuddyAccountView {
        id: value.id.clone(),
        uid: value.uid.clone(),
        uin: value.uin.clone(),
        display_name: value.display_name.clone(),
        masked_phone: value.masked_phone.clone(),
        checkin_enabled: value.checkin_enabled,
        token_expires_at: value.token_expires_at,
        created_at: value.created_at,
        updated_at: value.updated_at,
        last_used_at: value.last_used_at,
        last_github_sync_at: value.last_github_sync_at,
        last_github_sync_state,
        last_github_sync_error: value.last_github_sync_error.clone(),
    }
}

pub fn import_snapshot_json(
    raw: &str,
    display_name: Option<String>,
) -> Result<WorkBuddyAccountView, String> {
    let parsed = parse_snapshot_json(raw)?;
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let account_id = stable_account_id(&parsed.uid);
    let now = chrono::Utc::now().timestamp();
    let existing = index
        .accounts
        .iter()
        .find(|entry| entry.id == account_id)
        .cloned();
    let token_changed = load_record_read_only(&account_id)
        .ok()
        .and_then(|record| snapshot_access_token(&record.snapshot))
        .zip(snapshot_access_token(&parsed.value))
        .is_some_and(|(old_token, new_token)| old_token != new_token);
    let requested_name = sanitized_display_name(display_name.as_deref());
    let record = WorkBuddyAccountRecord {
        id: account_id.clone(),
        uid: parsed.uid,
        uin: parsed.uin,
        display_name: requested_name
            .or_else(|| existing.as_ref().map(|entry| entry.display_name.clone()))
            .or(parsed.nickname)
            .unwrap_or_else(|| format!("WorkBuddy {}", &account_id[3..9])),
        masked_phone: parsed.masked_phone,
        checkin_enabled: existing
            .as_ref()
            .map(|entry| entry.checkin_enabled)
            .unwrap_or(true),
        token_expires_at: parsed.token_expires_at,
        created_at: existing
            .as_ref()
            .map(|entry| entry.created_at)
            .unwrap_or(now),
        updated_at: now,
        last_used_at: existing.as_ref().and_then(|entry| entry.last_used_at),
        last_github_sync_at: existing
            .as_ref()
            .and_then(|entry| entry.last_github_sync_at),
        last_github_sync_state: if token_changed {
            "pending".to_string()
        } else {
            existing
                .as_ref()
                .map(|entry| entry.last_github_sync_state.clone())
                .unwrap_or_else(|| "pending".to_string())
        },
        last_github_sync_error: if token_changed {
            None
        } else {
            existing
                .as_ref()
                .and_then(|entry| entry.last_github_sync_error.clone())
        },
        snapshot: parsed.value,
    };
    crate::modules::workbuddy_github::mark_dataset_changed()?;
    save_record(&record)?;
    let new_summary = summary(&record);
    if let Some(entry) = index
        .accounts
        .iter_mut()
        .find(|entry| entry.id == account_id)
    {
        *entry = new_summary.clone();
    } else {
        index.accounts.push(new_summary.clone());
    }
    index.accounts.sort_by(|left, right| left.id.cmp(&right.id));
    save_index(&index)?;
    Ok(view(&new_summary))
}

pub fn default_auth_file_path() -> Result<PathBuf, String> {
    if let Some(custom) =
        crate::modules::workbuddy_settings::load_workbuddy_settings().auth_file_path
    {
        return Ok(PathBuf::from(
            crate::modules::workbuddy_settings::expand_environment_path(&custom),
        ));
    }
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位 LOCALAPPDATA".to_string())?;
    Ok(local_app_data
        .join("CodeBuddyExtension")
        .join("Data")
        .join("Public")
        .join("auth")
        .join("workbuddy-desktop.info"))
}

pub fn read_stable_auth_file(path: &Path) -> Result<String, String> {
    let first = fs::read_to_string(path)
        .map_err(|error| format!("读取 WorkBuddy 认证文件失败: {}", error))?;
    std::thread::sleep(std::time::Duration::from_millis(40));
    let second = fs::read_to_string(path)
        .map_err(|error| format!("再次读取 WorkBuddy 认证文件失败: {}", error))?;
    if first != second {
        return Err("WorkBuddy 认证文件仍在写入，请稍后重试".to_string());
    }
    parse_snapshot_json(&second)?;
    Ok(second)
}

pub fn import_current_workbuddy_account(
    display_name: Option<String>,
) -> Result<WorkBuddyAccountView, String> {
    let path = default_auth_file_path()?;
    if !path.exists() {
        return Err(format!("WorkBuddy 认证文件不存在: {}", path.display()));
    }
    import_snapshot_json(&read_stable_auth_file(&path)?, display_name)
}

pub fn current_managed_account_id() -> Option<String> {
    let path = default_auth_file_path().ok()?;
    let raw = read_stable_auth_file(&path).ok()?;
    let account_id = stable_account_id(&snapshot_uid(&raw).ok()?);
    has_account_id(&account_id).ok()?.then_some(account_id)
}

/// 在切换前立即吸收官方客户端刚写入的最新认证快照。
///
/// 后台会话监测有固定轮询间隔，官方客户端可能在两次轮询之间轮换
/// access/refresh token。如果此时直接切换，旧快照会被重新写回客户端，
/// 表现为刚登录不久却被要求重新登录。
pub(crate) fn refresh_current_managed_snapshot() -> Result<Option<WorkBuddyAccountView>, String> {
    let path = default_auth_file_path()?;
    if !path.is_file() {
        return Ok(None);
    }
    let raw = read_stable_auth_file(&path)?;
    let uid = snapshot_uid(&raw)?;
    let account_id = stable_account_id(&uid);
    if !has_account_id(&account_id)? {
        return Ok(None);
    }
    import_snapshot_json(&raw, None).map(Some)
}

pub fn list_workbuddy_accounts() -> Result<Vec<WorkBuddyAccountView>, String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 账号锁已损坏".to_string())?;
    Ok(load_index()?.accounts.iter().map(view).collect())
}

pub fn update_workbuddy_account(
    account_id: &str,
    update: WorkBuddyAccountUpdate,
) -> Result<WorkBuddyAccountView, String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let entry = index
        .accounts
        .iter_mut()
        .find(|entry| entry.id == account_id)
        .ok_or_else(|| "WorkBuddy 账号不存在".to_string())?;
    let mut record = load_record(account_id)?;
    if let Some(display_name) = update.display_name {
        record.display_name = sanitized_display_name(Some(&display_name))
            .ok_or_else(|| "WorkBuddy 账号备注不能为空".to_string())?;
    }
    if let Some(enabled) = update.checkin_enabled {
        record.checkin_enabled = enabled;
    }
    crate::modules::workbuddy_github::mark_dataset_changed()?;
    record.last_github_sync_state = "pending".to_string();
    record.last_github_sync_error = None;
    record.updated_at = chrono::Utc::now().timestamp();
    save_record(&record)?;
    *entry = summary(&record);
    let result = view(entry);
    save_index(&index)?;
    Ok(result)
}

pub fn set_checkin_enabled(
    account_id: &str,
    enabled: bool,
) -> Result<WorkBuddyAccountView, String> {
    update_workbuddy_account(
        account_id,
        WorkBuddyAccountUpdate {
            display_name: None,
            checkin_enabled: Some(enabled),
        },
    )
}

pub(crate) fn load_snapshot(account_id: &str) -> Result<Value, String> {
    Ok(load_record(account_id)?.snapshot)
}

pub(crate) fn snapshot_uid(raw: &str) -> Result<String, String> {
    Ok(parse_snapshot_json(raw)?.uid)
}

pub(crate) fn snapshot_json(account_id: &str) -> Result<String, String> {
    serde_json::to_string_pretty(&load_snapshot(account_id)?)
        .map_err(|error| format!("序列化 WorkBuddy 认证快照失败: {}", error))
}

/// 将托管账号写入 WorkBuddy 默认客户端认证文件。
///
/// 认证文件是 WorkBuddy 启动时读取的共享状态，因此写入必须在客户端关闭后进行，
/// 并在原子替换后重新读取校验，避免窗口启动后仍使用旧账号。
pub(crate) fn write_account_to_default_client(account_id: &str) -> Result<(), String> {
    let auth_path = default_auth_file_path()?;
    let snapshot = load_snapshot(account_id)?;
    let expected = parse_snapshot_json(
        &serde_json::to_string(&snapshot)
            .map_err(|error| format!("序列化 WorkBuddy 目标账号失败: {}", error))?,
    )?;
    let expected_uid = expected.uid;
    let expected_token = access_token_from_value(&expected.value)
        .ok_or_else(|| "目标 WorkBuddy 账号缺少 access token".to_string())?;

    let marker = PathBuf::from(format!("{}.logged-out", auth_path.to_string_lossy()));
    if marker.exists() {
        fs::remove_file(&marker)
            .map_err(|error| format!("清理 WorkBuddy 登出标记失败: {}", error))?;
    }

    let content = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("序列化 WorkBuddy 登录信息失败: {}", error))?;
    crate::modules::atomic_write::write_string_atomic(&auth_path, &content)
        .map_err(|error| format!("写入 WorkBuddy 登录信息失败: {}", error))?;

    let written = read_stable_auth_file(&auth_path)
        .map_err(|error| format!("校验 WorkBuddy 登录信息失败: {}", error))?;
    let written = parse_snapshot_json(&written)
        .map_err(|error| format!("校验 WorkBuddy 登录信息内容失败: {}", error))?;
    let written_token = access_token_from_value(&written.value);
    if written.uid != expected_uid || written_token.as_deref() != Some(expected_token.as_str()) {
        return Err(format!(
            "校验 WorkBuddy 登录信息失败，未写入目标账号: {}",
            auth_path.display()
        ));
    }
    Ok(())
}

pub(crate) fn has_account_id(account_id: &str) -> Result<bool, String> {
    Ok(list_workbuddy_accounts()?
        .iter()
        .any(|account| account.id == account_id))
}

pub(crate) fn mark_last_used(account_id: &str) -> Result<(), String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let mut record = load_record(account_id)?;
    let now = chrono::Utc::now().timestamp();
    record.last_used_at = Some(now);
    record.updated_at = now;
    save_record(&record)?;
    let entry = index
        .accounts
        .iter_mut()
        .find(|entry| entry.id == account_id)
        .ok_or_else(|| "WorkBuddy 账号不存在".to_string())?;
    *entry = summary(&record);
    save_index(&index)
}

pub(crate) fn mark_github_sync(
    account_id: &str,
    state: &str,
    error: Option<&str>,
) -> Result<(), String> {
    mark_github_sync_many(&[account_id.to_string()], state, error)
}

pub(crate) fn mark_github_sync_many(
    account_ids: &[String],
    state: &str,
    error: Option<&str>,
) -> Result<(), String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let requested = account_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let known = index
        .accounts
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    for missing in requested.difference(&known) {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Account] 跳过已不存在账号的 GitHub 同步状态: {}",
            missing
        ));
    }

    let now = chrono::Utc::now().timestamp();
    let redacted_error = error.map(crate::modules::work_cn_github::redact_for_log);
    let mut records = index
        .accounts
        .iter()
        .filter(|entry| requested.contains(entry.id.as_str()))
        .map(|entry| load_record(&entry.id))
        .collect::<Result<Vec<_>, String>>()?;

    for record in &mut records {
        if state != "pending" {
            record.last_github_sync_at = Some(now);
        }
        record.last_github_sync_state = state.to_string();
        record.last_github_sync_error = redacted_error.clone();
        record.updated_at = now;
        save_record(record)?;
        if let Some(entry) = index
            .accounts
            .iter_mut()
            .find(|entry| entry.id == record.id)
        {
            *entry = summary(record);
        }
    }
    save_index(&index)
}

pub(crate) fn mark_github_sync_pending(account_id: &str) -> Result<(), String> {
    mark_github_sync_many(&[account_id.to_string()], "pending", None)
}

pub(crate) fn access_token(account_id: &str) -> Result<String, String> {
    let snapshot = load_record_read_only(account_id)?.snapshot;
    snapshot_access_token(&snapshot).ok_or_else(|| "WorkBuddy 快照缺少 access token".to_string())
}

fn snapshot_refresh_token(snapshot: &Value) -> Option<String> {
    token_from_value_by_keys(snapshot, &["refreshToken", "refresh_token"])
}

fn snapshot_domain(snapshot: &Value) -> Option<String> {
    snapshot_string(snapshot, &["domain"])
}

fn parse_refresh_response(
    payload: &Value,
) -> Result<(String, Option<String>, Option<i64>, Option<i64>), String> {
    let data = payload.get("data").unwrap_or(payload);
    let access = data
        .get("accessToken")
        .or_else(|| data.get("access_token"))
        .and_then(Value::as_str)
        .and_then(|value| normalize_access_token(value))
        .ok_or_else(|| "刷新响应缺少 access token".to_string())?;
    let refresh = data
        .get("refreshToken")
        .or_else(|| data.get("refresh_token"))
        .and_then(Value::as_str)
        .and_then(|value| non_empty(Some(value)));
    let expires_at =
        object_i64(Some(data), &["expiresAt", "expires_at"]).map(normalize_epoch_seconds);
    let refresh_expires_at = object_i64(Some(data), &["refreshExpiresAt", "refresh_expires_at"])
        .map(normalize_epoch_seconds);
    Ok((access, refresh, expires_at, refresh_expires_at))
}

async fn request_refreshed_auth(
    refresh_token: &str,
    domain: Option<&str>,
) -> Result<(String, Option<String>, Option<i64>, Option<i64>), String> {
    let client = crate::utils::http::create_domestic_client(30, "copilot.tencent.com");
    let headers = build_refresh_headers(refresh_token, domain)?;
    let request = client
        .post("https://copilot.tencent.com/v2/plugin/auth/token/refresh")
        .headers(headers)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({}));
    let response = request
        .send()
        .await
        .map_err(|error| format!("WorkBuddy 刷新请求失败: {error}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("refresh token 已失效，请重新登录 WorkBuddy".to_string());
    }
    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("WorkBuddy 刷新响应无法解析: {error}"))?;
    if !status.is_success() {
        return Err(format!("WorkBuddy 刷新失败（HTTP {}）", status.as_u16()));
    }
    if let Some(code) = payload.get("code").and_then(Value::as_i64) {
        if code != 0 && code != 200 {
            let message = payload
                .get("message")
                .or_else(|| payload.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or("refresh token 已失效");
            return Err(format!("WorkBuddy 刷新失败（code={code}）：{message}"));
        }
    }
    parse_refresh_response(&payload)
}

fn build_refresh_headers(
    refresh_token: &str,
    domain: Option<&str>,
) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        "X-Refresh-Token",
        HeaderValue::from_str(refresh_token)
            .map_err(|error| format!("刷新 token 请求头无效: {error}"))?,
    );
    headers.insert("X-Auth-Refresh-Source", HeaderValue::from_static("plugin"));
    if let Some(domain) = domain.filter(|value| !value.trim().is_empty()) {
        headers.insert(
            "X-Domain",
            HeaderValue::from_str(domain)
                .map_err(|error| format!("WorkBuddy 域名请求头无效: {error}"))?,
        );
    }
    Ok(headers)
}

pub(crate) async fn refresh_account_auth(account_id: &str) -> Result<String, String> {
    let _refresh_guard = REFRESH_LOCK.lock().await;
    let record = load_record_read_only(account_id)?;
    let old_snapshot = record.snapshot;
    let refresh_token = snapshot_refresh_token(&old_snapshot)
        .ok_or_else(|| "WorkBuddy 快照缺少 refresh token，请重新登录 WorkBuddy".to_string())?;
    let domain = snapshot_domain(&old_snapshot);
    let (new_access, new_refresh, expires_at, refresh_expires_at) =
        request_refreshed_auth(&refresh_token, domain.as_deref()).await?;

    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let mut record = load_record(account_id)?;
    merge_refreshed_auth(
        &mut record.snapshot,
        &new_access,
        new_refresh.as_deref(),
        expires_at,
        refresh_expires_at,
    )?;
    record.token_expires_at = expires_at.or(record.token_expires_at);
    record.updated_at = chrono::Utc::now().timestamp();
    save_record(&record)?;
    if let Some(entry) = index
        .accounts
        .iter_mut()
        .find(|entry| entry.id == account_id)
    {
        *entry = summary(&record);
    }
    save_index(&index)?;
    Ok(new_access)
}

pub(crate) fn refresh_error_requires_login(error: &str) -> bool {
    let value = error.to_ascii_lowercase();
    value.contains("refresh token 已失效")
        || value.contains("缺少 refresh token")
        || value.contains("refresh token 无效")
}

pub(crate) fn request_identity(
    account_id: &str,
) -> Result<(String, Option<String>, Option<String>), String> {
    let snapshot = load_record_read_only(account_id)?.snapshot;
    let uid = snapshot_uid_from_value(&snapshot)
        .ok_or_else(|| "WorkBuddy 快照缺少账号 UID".to_string())?;
    Ok((
        uid,
        snapshot_string(&snapshot, &["enterpriseId", "enterprise_id"]),
        snapshot_string(&snapshot, &["domain"]),
    ))
}

fn snapshot_access_token(snapshot: &Value) -> Option<String> {
    access_token_from_value(snapshot)
}

fn snapshot_uid_from_value(snapshot: &Value) -> Option<String> {
    let account = snapshot.get("account");
    let auth = snapshot.get("auth");
    let raw_token = raw_access_token_from_value(snapshot)?;
    object_string(Some(snapshot), &["uid", "id"])
        .or_else(|| object_string(account, &["uid", "id"]))
        .or_else(|| object_string(auth, &["uid", "userId"]))
        .or_else(|| uid_from_token(&raw_token))
}

fn snapshot_string(snapshot: &Value, keys: &[&str]) -> Option<String> {
    object_string(Some(snapshot), keys)
        .or_else(|| object_string(snapshot.get("account"), keys))
        .or_else(|| object_string(snapshot.get("auth"), keys))
}

pub fn remove_workbuddy_account(account_id: &str) -> Result<(), String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "WorkBuddy 账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let previous_len = index.accounts.len();
    index.accounts.retain(|entry| entry.id != account_id);
    if index.accounts.len() == previous_len {
        return Err("WorkBuddy 账号不存在".to_string());
    }
    save_index(&index)?;
    crate::modules::atomic_write::remove_file_locked(&detail_path(account_id)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::fs;

    #[test]
    fn refresh_client_uses_official_copilot_route() {
        let production_source = include_str!("workbuddy_account.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("生产代码应在测试模块之前");
        assert!(production_source.contains("create_domestic_client(30, \"copilot.tencent.com\")"));
        assert!(production_source.contains("https://copilot.tencent.com/v2/plugin/auth/token/refresh"));
    }

    #[test]
    fn refresh_request_headers_match_official_workbuddy_protocol() {
        let headers = build_refresh_headers("refresh-token", Some("www.workbuddy.cn"))
        .expect("刷新请求头应可构造");

        assert_eq!(
            headers
                .get("x-refresh-token")
                .and_then(|value| value.to_str().ok()),
            Some("refresh-token")
        );
        assert_eq!(
            headers
                .get("x-auth-refresh-source")
                .and_then(|value| value.to_str().ok()),
            Some("plugin")
        );
        assert_eq!(
            headers
                .get("x-domain")
                .and_then(|value| value.to_str().ok()),
            Some("www.workbuddy.cn")
        );
        assert!(headers.get("authorization").is_none());
    }

    #[test]
    fn refreshed_auth_tokens_are_merged_into_snapshot() {
        let mut snapshot = serde_json::json!({
            "account": {"uid": "uid-refresh"},
            "auth": {
                "accessToken": "old-access",
                "refreshToken": "old-refresh",
                "expiresAt": 1000_i64,
                "refreshExpiresAt": 2000_i64
            }
        });

        merge_refreshed_auth(
            &mut snapshot,
            "new-access",
            Some("new-refresh"),
            Some(3000),
            None,
        )
        .expect("刷新凭证应写回快照");

        assert_eq!(
            snapshot
                .pointer("/auth/accessToken")
                .and_then(Value::as_str),
            Some("new-access")
        );
        assert_eq!(
            snapshot
                .pointer("/auth/refreshToken")
                .and_then(Value::as_str),
            Some("new-refresh")
        );
        assert_eq!(
            snapshot.pointer("/auth/expiresAt").and_then(Value::as_i64),
            Some(3000)
        );
        assert_eq!(
            snapshot
                .pointer("/auth/refreshExpiresAt")
                .and_then(Value::as_i64),
            Some(2000)
        );
    }

    #[test]
    fn refresh_response_accepts_nested_tokens_and_millisecond_expiry() {
        let payload = serde_json::json!({
            "code": 0,
            "data": {
                "access_token": "new-access",
                "refresh_token": "new-refresh",
                "expires_at": 1_900_000_000_000_i64,
                "refresh_expires_at": 2_000_000_000_000_i64
            }
        });
        let parsed = parse_refresh_response(&payload).expect("刷新响应应可解析");
        assert_eq!(parsed.0, "new-access");
        assert_eq!(parsed.1.as_deref(), Some("new-refresh"));
        assert_eq!(parsed.2, Some(1_900_000_000));
        assert_eq!(parsed.3, Some(2_000_000_000));
    }

    #[test]
    fn refresh_error_only_requests_login_for_invalid_refresh_token() {
        assert!(refresh_error_requires_login(
            "refresh token 已失效，请重新登录 WorkBuddy"
        ));
        assert!(refresh_error_requires_login(
            "WorkBuddy 快照缺少 refresh token，请重新登录 WorkBuddy"
        ));
        assert!(!refresh_error_requires_login(
            "WorkBuddy 刷新请求失败: connection reset"
        ));
        assert!(!refresh_error_requires_login(
            "WorkBuddy 刷新失败（HTTP 503）"
        ));
    }

    fn fixture(uid: &str, token: &str) -> String {
        format!(
            r#"{{"account":{{"uid":"{uid}","uin":"uin-{uid}","nickname":"测试账号","phoneNumber":"13800138000"}},"auth":{{"accessToken":"{token}","refreshToken":"refresh-{token}","expiresAt":1800000000,"sessionState":"active"}},"accounts":[],"allAccounts":[]}}"#
        )
    }

    #[test]
    fn workbuddy_import_dedupes_uid_preserves_preferences_and_encrypts_snapshot() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-account-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let first =
            import_snapshot_json(&fixture("uid-a", "token-old"), Some("个人号".into())).unwrap();
        set_checkin_enabled(&first.id, false).unwrap();
        mark_github_sync(&first.id, "synced", None).unwrap();
        let updated = import_snapshot_json(&fixture("uid-a", "token-new"), None).unwrap();

        assert_eq!(first.id, updated.id);
        assert_eq!("个人号", updated.display_name);
        assert!(!updated.checkin_enabled);
        assert_eq!("pending", updated.last_github_sync_state);
        let index = fs::read_to_string(index_path().unwrap()).unwrap();
        let detail = fs::read_to_string(detail_path(&updated.id).unwrap()).unwrap();
        assert!(!index.contains("token-new"));
        assert!(!detail.contains("token-new"));
        assert!(detail.contains("AES-256-GCM"));

        let view = serde_json::to_string(&updated).unwrap();
        assert!(!view.contains("accessToken"));
        assert!(!view.contains("refreshToken"));
        assert!(!view.contains("13800138000"));

        mark_github_sync(
            &updated.id,
            "failed",
            Some("远端拒绝 token=abcdefghijklmnopqrstuvwxyz123456"),
        )
        .unwrap();
        let failed = list_workbuddy_accounts().unwrap().remove(0);
        assert_eq!(failed.last_github_sync_state, "failed");
        assert_eq!(
            failed.last_github_sync_error.as_deref(),
            Some("远端拒绝 token=[REDACTED]")
        );
        mark_github_sync(&updated.id, "synced", None).unwrap();
        assert!(list_workbuddy_accounts().unwrap()[0]
            .last_github_sync_error
            .is_none());
        mark_github_sync(&updated.id, "failed", None).unwrap();
        assert_eq!(
            list_workbuddy_accounts().unwrap()[0].last_github_sync_state,
            "pending",
            "旧版失败状态没有错误原因时应迁移为待同步"
        );

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_github_sync_state_updates_multiple_accounts_in_one_batch() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-batch-sync-state-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let first = import_snapshot_json(&fixture("uid-batch-a", "token-a"), None).unwrap();
        let second = import_snapshot_json(&fixture("uid-batch-b", "token-b"), None).unwrap();

        mark_github_sync_many(
            &[first.id.clone(), second.id.clone()],
            "failed",
            Some("remote rejected token=abcdefghijklmnopqrstuvwxyz123456"),
        )
        .unwrap();

        let accounts = list_workbuddy_accounts().unwrap();
        assert_eq!(accounts.len(), 2);
        assert!(accounts.iter().all(|account| {
            account.last_github_sync_state == "failed"
                && account.last_github_sync_at.is_some()
                && account.last_github_sync_error.as_deref()
                    == Some("remote rejected token=[REDACTED]")
        }));

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_account_update_marks_github_state_pending() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-update-pending-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let account = import_snapshot_json(&fixture("uid-update", "token-update"), None).unwrap();
        mark_github_sync(&account.id, "synced", None).unwrap();

        let updated = update_workbuddy_account(
            &account.id,
            WorkBuddyAccountUpdate {
                display_name: Some("新备注".to_string()),
                checkin_enabled: None,
            },
        )
        .unwrap();

        assert_eq!(updated.last_github_sync_state, "pending");
        assert!(updated.last_github_sync_error.is_none());
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_settings_persist_manual_auth_file_path() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-settings-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let custom_auth = dir.join("custom-workbuddy.info");

        crate::modules::workbuddy_settings::save_workbuddy_settings(
            crate::models::workbuddy::WorkBuddySettings {
                executable_path: None,
                auth_file_path: Some(custom_auth.to_string_lossy().to_string()),
            },
        )
        .unwrap();

        assert_eq!(default_auth_file_path().unwrap(), custom_auth);
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_millisecond_token_expiry_is_normalized_to_seconds() {
        let parsed = parse_snapshot_json(
            r#"{"account":{"uid":"uid-expiry"},"auth":{"accessToken":"token","expiresAt":1791974976115}}"#,
        )
        .unwrap();

        assert_eq!(parsed.token_expires_at, Some(1_791_974_976));
    }

    #[test]
    fn workbuddy_auth_diagnostic_fingerprints_tokens_without_exposing_them() {
        let raw = serde_json::json!({
            "account": {"uid": "uid-diagnostic"},
            "auth": {
                "accessToken": "access-secret-value",
                "refreshToken": "refresh-secret-value",
                "expiresAt": 1_900_000_000_i64,
                "refreshExpiresAt": 2_000_000_000_i64
            }
        })
        .to_string();

        let diagnostic = auth_snapshot_diagnostic(&raw).unwrap();

        assert_eq!(diagnostic.account_id, stable_account_id("uid-diagnostic"));
        assert_eq!(diagnostic.expires_at, Some(1_900_000_000));
        assert_eq!(diagnostic.refresh_expires_at, Some(2_000_000_000));
        assert_ne!(diagnostic.access_token_fingerprint, "access-secret-value");
        assert_ne!(
            diagnostic.refresh_token_fingerprint.as_deref(),
            Some("refresh-secret-value")
        );
        let rendered = diagnostic.to_log_fields();
        assert!(!rendered.contains("access-secret-value"));
        assert!(!rendered.contains("refresh-secret-value"));
        assert!(rendered.contains("access_fp="));
        assert!(rendered.contains("refresh_fp="));
    }

    #[test]
    fn workbuddy_parser_accepts_upstream_flat_auth_payload() {
        let parsed = parse_snapshot_json(
            r#"{"uid":"uid-flat","nickname":"Flat","access_token":"token-flat","expires_at":"1791974976"}"#,
        )
        .unwrap();

        assert_eq!(parsed.uid, "uid-flat");
        assert_eq!(
            snapshot_access_token(&parsed.value).as_deref(),
            Some("token-flat")
        );
        assert_eq!(parsed.nickname.as_deref(), Some("Flat"));
        assert_eq!(parsed.token_expires_at, Some(1_791_974_976));
    }

    #[test]
    fn workbuddy_parser_accepts_upstream_uid_prefixed_token() {
        let parsed =
            parse_snapshot_json(r#"{"auth":{"accessToken":"uid-prefixed+token-prefixed"}}"#)
                .unwrap();

        assert_eq!(parsed.uid, "uid-prefixed");
        assert_eq!(
            snapshot_access_token(&parsed.value).as_deref(),
            Some("token-prefixed")
        );
    }

    #[test]
    fn workbuddy_parser_extracts_uid_from_jwt_subject() {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"sub":"uid-jwt"}"#);
        let token = format!("{}.{}.signature", header, payload);
        let raw = serde_json::json!({"auth": {"accessToken": token}}).to_string();

        let parsed = parse_snapshot_json(&raw).unwrap();

        assert_eq!(parsed.uid, "uid-jwt");
        assert_eq!(
            snapshot_access_token(&parsed.value).as_deref(),
            Some(token.as_str())
        );
    }

    #[test]
    fn workbuddy_current_account_comes_from_the_live_auth_file() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-current-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let auth_path = dir.join("live-workbuddy.info");
        crate::modules::workbuddy_settings::save_workbuddy_settings(
            crate::models::workbuddy::WorkBuddySettings {
                executable_path: None,
                auth_file_path: Some(auth_path.to_string_lossy().to_string()),
            },
        )
        .unwrap();
        let imported =
            import_snapshot_json(&fixture("uid-current", "token-current"), None).unwrap();
        fs::write(&auth_path, fixture("uid-current", "token-current")).unwrap();

        assert_eq!(current_managed_account_id(), Some(imported.id));

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_write_account_clears_logout_marker_and_verifies_target_snapshot() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-write-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let auth_path = dir.join("live-workbuddy.info");
        crate::modules::workbuddy_settings::save_workbuddy_settings(
            crate::models::workbuddy::WorkBuddySettings {
                executable_path: None,
                auth_file_path: Some(auth_path.to_string_lossy().to_string()),
            },
        )
        .unwrap();
        let imported = import_snapshot_json(&fixture("uid-write", "token-write"), None).unwrap();
        fs::write(
            auth_path.with_file_name("live-workbuddy.info.logged-out"),
            b"logged out",
        )
        .unwrap();

        write_account_to_default_client(&imported.id).unwrap();

        assert!(!auth_path
            .with_file_name("live-workbuddy.info.logged-out")
            .exists());
        let written = read_stable_auth_file(&auth_path).unwrap();
        assert_eq!(snapshot_uid(&written).unwrap(), "uid-write");
        assert_eq!(
            snapshot_access_token(&serde_json::from_str(&written).unwrap()).as_deref(),
            Some("token-write")
        );

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_refreshes_live_managed_snapshot_before_switching_away() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-live-refresh-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let auth_path = dir.join("live-workbuddy.info");
        crate::modules::workbuddy_settings::save_workbuddy_settings(
            crate::models::workbuddy::WorkBuddySettings {
                executable_path: None,
                auth_file_path: Some(auth_path.to_string_lossy().to_string()),
            },
        )
        .unwrap();
        let imported = import_snapshot_json(&fixture("uid-live", "token-stale"), None).unwrap();

        // 官方客户端已轮换 token，但后台监测尚未来得及执行时立刻发起切换。
        fs::write(&auth_path, fixture("uid-live", "token-current")).unwrap();
        let refreshed = refresh_current_managed_snapshot().unwrap();

        assert_eq!(
            refreshed.as_ref().map(|account| &account.id),
            Some(&imported.id)
        );
        assert_eq!(access_token(&imported.id).unwrap(), "token-current");

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_keeps_refresh_token_and_expiry_when_live_snapshot_rotates() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-refresh-context-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let auth_path = dir.join("live-workbuddy.info");
        crate::modules::workbuddy_settings::save_workbuddy_settings(
            crate::models::workbuddy::WorkBuddySettings {
                executable_path: None,
                auth_file_path: Some(auth_path.to_string_lossy().to_string()),
            },
        )
        .unwrap();
        let imported = import_snapshot_json(&fixture("uid-context", "token-old"), None).unwrap();
        let live = serde_json::json!({
            "account": {"uid": "uid-context", "uin": "uin-context"},
            "auth": {
                "accessToken": "token-new",
                "refreshToken": "refresh-new",
                "expiresAt": 1_900_000_000_i64,
                "refreshExpiresAt": 2_000_000_000_i64,
                "sessionState": "active"
            },
            "accounts": [],
            "allAccounts": []
        });
        fs::write(&auth_path, serde_json::to_string(&live).unwrap()).unwrap();

        refresh_current_managed_snapshot().unwrap();
        let saved = load_snapshot(&imported.id).unwrap();
        assert_eq!(
            saved.pointer("/auth/accessToken").and_then(Value::as_str),
            Some("token-new")
        );
        assert_eq!(
            saved.pointer("/auth/refreshToken").and_then(Value::as_str),
            Some("refresh-new")
        );
        assert_eq!(
            saved.pointer("/auth/expiresAt").and_then(Value::as_i64),
            Some(1_900_000_000)
        );
        assert_eq!(
            saved
                .pointer("/auth/refreshExpiresAt")
                .and_then(Value::as_i64),
            Some(2_000_000_000)
        );

        std::env::remove_var("COCKPIT_TEST_LOCALAPPDATA");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_corrupted_index_is_rebuilt_from_encrypted_account_details() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-index-repair-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let imported = import_snapshot_json(&fixture("uid-repair", "token-repair"), None).unwrap();
        fs::write(index_path().unwrap(), "{not-valid-json").unwrap();

        let recovered = list_workbuddy_accounts().unwrap();

        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id, imported.id);
        assert!(!fs::read_to_string(index_path().unwrap())
            .unwrap()
            .contains("token-repair"));

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_stale_backup_index_is_reconciled_with_encrypted_details() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-stale-index-backup-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let first = import_snapshot_json(&fixture("uid-index-a", "token-index-a"), None).unwrap();
        let stale_index = fs::read_to_string(index_path().unwrap()).unwrap();
        let second = import_snapshot_json(&fixture("uid-index-b", "token-index-b"), None).unwrap();
        let path = index_path().unwrap();
        fs::write(path.with_extension("json.bak"), stale_index).unwrap();
        fs::write(&path, "{not-valid-json").unwrap();

        let recovered = list_workbuddy_accounts().unwrap();
        let recovered_ids = recovered
            .iter()
            .map(|account| account.id.as_str())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(recovered.len(), 2);
        assert!(recovered_ids.contains(first.id.as_str()));
        assert!(recovered_ids.contains(second.id.as_str()));

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }
}

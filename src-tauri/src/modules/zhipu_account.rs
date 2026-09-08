//! 智谱清言账号存储：客户端 Cookie 导入 + 手动粘贴 + AES-256-GCM 加密详情。
//!
//! 凭证体系：清言（chatglm.cn）网页登录态，`chatglm_token`（access JWT，
//! 约 24 小时有效）+ `chatglm_refresh_token`（约 180 天）。客户端每次启动
//! 会自动刷新 access token 并写回 Cookie，因此「导入当前客户端账号」
//! 每次都会拿到最新 token；工具后台同步保证 GitHub Secret 始终可用。
//!
//! token 读取：`%APPDATA%\chatglm\Network\Cookies`（SQLite，实测明文列）。
//! 签到由云端 Actions 完成，本模块绝不发起签到/领取请求。

use crate::models::zhipu::{ZhipuAccountUpdate, ZhipuAccountView};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::Mutex;

const INDEX_FILE: &str = "zhipu_accounts.json";
const DETAILS_DIR: &str = "zhipu_accounts";
const SNAPSHOT_KIND: &str = "zhipu";

static ACCOUNT_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZhipuAccountRecord {
    id: String,
    display_name: String,
    user_label: String,
    uid: String,
    access_token: String,
    refresh_token: String,
    checkin_enabled: bool,
    token_expires_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    last_github_sync_at: Option<i64>,
    last_github_sync_state: String,
    #[serde(default)]
    last_github_sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ZhipuAccountSummary {
    id: String,
    display_name: String,
    user_label: String,
    uid: String,
    checkin_enabled: bool,
    token_expires_at: Option<i64>,
    created_at: i64,
    updated_at: i64,
    last_github_sync_at: Option<i64>,
    last_github_sync_state: String,
    #[serde(default)]
    last_github_sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ZhipuAccountIndex {
    version: u32,
    accounts: Vec<ZhipuAccountSummary>,
}

impl Default for ZhipuAccountIndex {
    fn default() -> Self {
        Self {
            version: 1,
            accounts: Vec::new(),
        }
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

/// JWT payload 解析结果（不校验签名，仅读取声明）。
struct ParsedJwt {
    uid: Option<String>,
    sub: Option<String>,
    exp: Option<i64>,
}

fn parse_jwt_claims(token: &str) -> Option<ParsedJwt> {
    let payload_b64 = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    Some(ParsedJwt {
        uid: value.get("uid").and_then(Value::as_str).map(str::to_string),
        sub: value.get("sub").and_then(Value::as_str).map(str::to_string),
        exp: value.get("exp").and_then(Value::as_i64),
    })
}

pub fn stable_account_id(uid: &str) -> String {
    let digest = Sha256::digest(uid.as_bytes());
    let hex = format!("{:x}", digest);
    format!("zp-{}", &hex[..12])
}

fn valid_account_id(value: &str) -> bool {
    value.len() == 15
        && value.starts_with("zp-")
        && value[3..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
}

fn index_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(INDEX_FILE))
}

fn details_dir() -> Result<PathBuf, String> {
    let directory = crate::modules::account::get_data_dir()?.join(DETAILS_DIR);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("创建智谱账号目录失败: {}", error))?;
    Ok(directory)
}

pub(crate) fn detail_path(account_id: &str) -> Result<PathBuf, String> {
    if !valid_account_id(account_id) {
        return Err("智谱账号 ID 无效".to_string());
    }
    Ok(details_dir()?.join(format!("{}.json", account_id)))
}

fn load_index() -> Result<ZhipuAccountIndex, String> {
    let path = index_path()?;
    if !path.exists() {
        return repair_index_from_details("索引文件不存在");
    }
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取智谱账号索引失败: {}", error))?;
    match crate::modules::atomic_write::parse_json_with_auto_restore::<ZhipuAccountIndex>(
        &path, &content,
    ) {
        Ok(index) => reconcile_index_with_details(index),
        Err(_) => repair_index_from_details("索引文件损坏"),
    }
}

fn save_index(index: &ZhipuAccountIndex) -> Result<(), String> {
    let content = serde_json::to_string_pretty(index)
        .map_err(|error| format!("序列化智谱账号索引失败: {}", error))?;
    crate::modules::atomic_write::write_string_atomic(&index_path()?, &content)
        .map_err(|error| format!("保存智谱账号索引失败: {}", error))
}

fn build_index_from_details() -> Result<ZhipuAccountIndex, String> {
    let directory = details_dir()?;
    let records = crate::modules::account_index_repair::load_accounts_from_details(
        &directory,
        |account_id| {
            valid_account_id(account_id)
                .then(|| load_record_read_only(account_id).ok())
                .flatten()
        },
    )?;
    Ok(ZhipuAccountIndex {
        version: 1,
        accounts: records.iter().map(summary).collect(),
    })
}

fn reconcile_index_with_details(index: ZhipuAccountIndex) -> Result<ZhipuAccountIndex, String> {
    let rebuilt = build_index_from_details()?;
    if index.accounts == rebuilt.accounts {
        Ok(index)
    } else {
        repair_index_from_details("索引与加密账号详情不一致")
    }
}

fn repair_index_from_details(reason: &str) -> Result<ZhipuAccountIndex, String> {
    let repaired = build_index_from_details()?;
    let path = index_path()?;
    if let Err(error) = crate::modules::account_index_repair::backup_existing_index(&path) {
        crate::modules::logger::log_warn(&format!(
            "[Zhipu Account] 重建索引前备份失败，继续覆盖损坏索引: {}",
            error
        ));
    }
    save_index(&repaired)?;
    crate::modules::logger::log_warn(&format!(
        "[Zhipu Account] 已从加密账号详情重建索引: reason={}, recovered_accounts={}",
        reason,
        repaired.accounts.len()
    ));
    Ok(repaired)
}

fn save_record(record: &ZhipuAccountRecord) -> Result<(), String> {
    let content =
        crate::modules::secure_account_storage::serialize_account_file(SNAPSHOT_KIND, record)?;
    crate::modules::atomic_write::write_string_atomic(&detail_path(&record.id)?, &content)
}

fn load_record(account_id: &str) -> Result<ZhipuAccountRecord, String> {
    let path = detail_path(account_id)?;
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取智谱账号详情失败: {}", error))?;
    let (record, needs_rewrite) =
        crate::modules::secure_account_storage::deserialize_account_file(&path, &content)?;
    if needs_rewrite {
        save_record(&record)?;
    }
    Ok(record)
}

fn load_record_read_only(account_id: &str) -> Result<ZhipuAccountRecord, String> {
    let path = detail_path(account_id)?;
    let content = fs::read_to_string(&path)
        .map_err(|error| format!("读取智谱账号详情失败: {}", error))?;
    let (record, _) =
        crate::modules::secure_account_storage::deserialize_account_file(&path, &content)?;
    Ok(record)
}

fn summary(record: &ZhipuAccountRecord) -> ZhipuAccountSummary {
    ZhipuAccountSummary {
        id: record.id.clone(),
        display_name: record.display_name.clone(),
        user_label: record.user_label.clone(),
        uid: record.uid.clone(),
        checkin_enabled: record.checkin_enabled,
        token_expires_at: record.token_expires_at,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_github_sync_at: record.last_github_sync_at,
        last_github_sync_state: record.last_github_sync_state.clone(),
        last_github_sync_error: record.last_github_sync_error.clone(),
    }
}

fn view(value: &ZhipuAccountSummary) -> ZhipuAccountView {
    let last_github_sync_state =
        if value.last_github_sync_state == "failed" && value.last_github_sync_error.is_none() {
            "pending".to_string()
        } else {
            value.last_github_sync_state.clone()
        };
    ZhipuAccountView {
        id: value.id.clone(),
        display_name: value.display_name.clone(),
        user_label: value.user_label.clone(),
        checkin_enabled: value.checkin_enabled,
        token_expires_at: value.token_expires_at,
        created_at: value.created_at,
        updated_at: value.updated_at,
        last_github_sync_at: value.last_github_sync_at,
        last_github_sync_state,
        last_github_sync_error: value.last_github_sync_error.clone(),
    }
}

/// 校验 token 基本格式（JWT 三段式）并解析声明。
fn validate_access_token(raw: &str) -> Result<(String, Option<String>, Option<i64>), String> {
    let trimmed = raw.trim();
    if trimmed.split('.').count() != 3 {
        return Err("access token 格式无效：应为 JWT（三段式，来自 chatglm.cn 登录后的 chatglm_token）".to_string());
    }
    let claims =
        parse_jwt_claims(trimmed).ok_or_else(|| "access token 无法解析：JWT payload 无效".to_string())?;
    let uid = claims.uid.ok_or_else(|| {
        "access token 缺少 uid 声明（请确认粘贴的是 chatglm_token 而非其他凭证）".to_string()
    })?;
    Ok((uid, claims.sub, claims.exp))
}

fn validate_refresh_token(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.split('.').count() != 3 {
        return Err("refresh token 格式无效：应为 JWT（三段式，来自 chatglm_refresh_token）".to_string());
    }
    match parse_jwt_claims(trimmed) {
        Some(claims) if claims.sub.is_some() => Ok(trimmed.to_string()),
        _ => Err("refresh token 无法解析：JWT payload 无效".to_string()),
    }
}

/// 保存/更新一个账号。同一 uid 重复导入视为更新（保留备注与签到开关）。
pub fn import_account(
    access_token: &str,
    refresh_token: &str,
    display_name: Option<String>,
) -> Result<ZhipuAccountView, String> {
    let access_token = access_token.trim();
    let refresh_token = refresh_token.trim();
    let (uid, user_label, token_expires_at) = validate_access_token(access_token)?;
    let refresh_token = if refresh_token.is_empty() {
        String::new()
    } else {
        validate_refresh_token(refresh_token)?
    };

    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "智谱账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let account_id = stable_account_id(&uid);
    let now = chrono::Utc::now().timestamp();
    let existing = index
        .accounts
        .iter()
        .find(|entry| entry.id == account_id)
        .cloned();
    let token_changed = existing.as_ref().is_some_and(|_| {
        load_record_read_only(&account_id)
            .map(|record| record.access_token != access_token)
            .unwrap_or(true)
    });
    let fallback_name = user_label
        .clone()
        .unwrap_or_else(|| format!("清言用户 {}", &account_id[3..9]));
    let record = ZhipuAccountRecord {
        id: account_id.clone(),
        display_name: sanitized_display_name(display_name.as_deref())
            .or_else(|| existing.as_ref().map(|entry| entry.display_name.clone()))
            .unwrap_or(fallback_name),
        user_label: user_label.unwrap_or_default(),
        uid,
        access_token: access_token.to_string(),
        refresh_token,
        checkin_enabled: existing
            .as_ref()
            .map(|entry| entry.checkin_enabled)
            .unwrap_or(true),
        token_expires_at,
        created_at: existing
            .as_ref()
            .map(|entry| entry.created_at)
            .unwrap_or(now),
        updated_at: now,
        last_github_sync_at: existing.as_ref().and_then(|entry| entry.last_github_sync_at),
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
            existing.as_ref().and_then(|entry| entry.last_github_sync_error.clone())
        },
    };
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

// ---------- 客户端 Cookie 导入 ----------

/// 清言桌面客户端的 Cookies 数据库路径。
pub fn default_cookies_path() -> Result<PathBuf, String> {
    let app_data = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "无法定位 APPDATA".to_string())?;
    Ok(app_data.join("chatglm").join("Network").join("Cookies"))
}

/// 从 Cookies SQLite 读取指定域下的明文 cookie。
/// 先尝试只读打开（客户端运行中通常允许并发读），失败则复制到临时文件。
fn read_cookie_values(cookies_path: &Path, names: &[&str]) -> Result<Vec<(String, String)>, String> {
    if !cookies_path.exists() {
        return Err(format!("清言客户端 Cookies 文件不存在: {}", cookies_path.display()));
    }
    let placeholders = names.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let query = format!(
        "select name, value from cookies where host_key in ('chatglm.cn', '.chatglm.cn') \
         and name in ({placeholders}) and value != ''"
    );
    let open_and_query = |path: &Path| -> Result<Vec<(String, String)>, String> {
        let connection = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|error| format!("打开清言 Cookies 失败: {error}"))?;
        let mut statement = connection
            .prepare(&query)
            .map_err(|error| format!("读取清言 Cookies 失败: {error}"))?;
        let mut rows = statement
            .query(rusqlite::params_from_iter(names.iter()))
            .map_err(|error| format!("读取清言 Cookies 失败: {error}"))?;
        let mut collected = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|error| format!("读取清言 Cookies 失败: {error}"))?
        {
            let name: String = row
                .get(0)
                .map_err(|error| format!("读取清言 Cookies 失败: {error}"))?;
            let value: String = row
                .get(1)
                .map_err(|error| format!("读取清言 Cookies 失败: {error}"))?;
            collected.push((name, value));
        }
        Ok(collected)
    };
    match open_and_query(cookies_path) {
        Ok(rows) => Ok(rows),
        Err(direct_error) => {
            // 数据库被锁（客户端写入中）时复制快照重试。
            let temp = std::env::temp_dir().join(format!(
                "chatglm-cookies-{}-{}.db",
                std::process::id(),
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ));
            let result = fs::copy(cookies_path, &temp)
                .map_err(|error| {
                    format!("复制清言 Cookies 失败: {error}（原因: {direct_error}）")
                })
                .and_then(|_| open_and_query(&temp));
            let _ = fs::remove_file(&temp);
            result
        }
    }
}

/// 从清言桌面客户端读取当前登录 token（不落盘），供命令层统一走
/// 在线验证 + 导入流程。
pub fn read_current_client_tokens() -> Result<(String, String), String> {
    let cookies_path = default_cookies_path()?;
    let values = read_cookie_values(&cookies_path, &["chatglm_token", "chatglm_refresh_token"])?;
    let access = values
        .iter()
        .find(|(name, _)| name == "chatglm_token")
        .map(|(_, value)| value.clone())
        .ok_or_else(|| {
            "清言客户端未登录：请在智谱清言客户端完成登录后再导入（未找到 chatglm_token）"
                .to_string()
        })?;
    let refresh = values
        .iter()
        .find(|(name, _)| name == "chatglm_refresh_token")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    Ok((access, refresh))
}

/// 从清言桌面客户端读取当前登录账号并导入（不经在线验证的同步入口，
/// 主要供测试与内部刷新使用）。
pub fn import_current_client_account(
    display_name: Option<String>,
) -> Result<ZhipuAccountView, String> {
    let (access, refresh) = read_current_client_tokens()?;
    import_account(&access, &refresh, display_name)
}

pub fn list_zhipu_accounts() -> Result<Vec<ZhipuAccountView>, String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "智谱账号锁已损坏".to_string())?;
    Ok(load_index()?.accounts.iter().map(view).collect())
}

pub fn update_zhipu_account(
    account_id: &str,
    update: ZhipuAccountUpdate,
) -> Result<ZhipuAccountView, String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "智谱账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let entry = index
        .accounts
        .iter_mut()
        .find(|entry| entry.id == account_id)
        .ok_or_else(|| "智谱账号不存在".to_string())?;
    let mut record = load_record(account_id)?;
    if let Some(display_name) = update.display_name {
        record.display_name = sanitized_display_name(Some(&display_name))
            .ok_or_else(|| "智谱账号备注不能为空".to_string())?;
    }
    if let Some(enabled) = update.checkin_enabled {
        record.checkin_enabled = enabled;
    }
    record.last_github_sync_state = "pending".to_string();
    record.last_github_sync_error = None;
    record.updated_at = chrono::Utc::now().timestamp();
    save_record(&record)?;
    *entry = summary(&record);
    let result = view(entry);
    save_index(&index)?;
    Ok(result)
}

pub fn set_checkin_enabled(account_id: &str, enabled: bool) -> Result<ZhipuAccountView, String> {
    update_zhipu_account(
        account_id,
        ZhipuAccountUpdate {
            display_name: None,
            checkin_enabled: Some(enabled),
        },
    )
}

pub fn remove_zhipu_account(account_id: &str) -> Result<(), String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "智谱账号锁已损坏".to_string())?;
    let mut index = load_index()?;
    let previous_len = index.accounts.len();
    index.accounts.retain(|entry| entry.id != account_id);
    if index.accounts.len() == previous_len {
        return Err("智谱账号不存在".to_string());
    }
    save_index(&index)?;
    crate::modules::atomic_write::remove_file_locked(&detail_path(account_id)?)?;
    Ok(())
}

pub(crate) fn has_account_id(account_id: &str) -> Result<bool, String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "智谱账号锁已损坏".to_string())?;
    Ok(load_index()?
        .accounts
        .iter()
        .any(|entry| entry.id == account_id))
}

pub(crate) fn checkin_enabled(account_id: &str) -> Result<bool, String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "智谱账号锁已损坏".to_string())?;
    load_index()?
        .accounts
        .iter()
        .find(|entry| entry.id == account_id)
        .map(|entry| entry.checkin_enabled)
        .ok_or_else(|| "智谱账号不存在".to_string())
}

/// 读取账号 access token 原文（仅用于 GitHub secrets 同步与只读积分查询）。
pub(crate) fn access_token(account_id: &str) -> Result<String, String> {
    Ok(load_record_read_only(account_id)?.access_token)
}

pub(crate) fn refresh_token(account_id: &str) -> Result<String, String> {
    let record = load_record_read_only(account_id)?;
    if record.refresh_token.is_empty() {
        return Err("该账号未保存 refresh token".to_string());
    }
    Ok(record.refresh_token)
}

pub(crate) fn mark_github_sync_many(
    account_ids: &[String],
    state: &str,
    error: Option<&str>,
) -> Result<(), String> {
    let _guard = ACCOUNT_LOCK
        .lock()
        .map_err(|_| "智谱账号锁已损坏".to_string())?;
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
            "[Zhipu Account] 跳过已不存在账号的 GitHub 同步状态: {}",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造最小 JWT（header.payload.signature），payload 自定义。
    fn fake_jwt(payload: &str) -> String {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256"}"#);
        let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.as_bytes());
        format!("{header}.{body}.signature")
    }

    #[test]
    fn zhipu_access_token_claims_are_parsed() {
        let token = fake_jwt(r#"{"sub":"用户_T9PBW7","uid":"uid-abc","exp":1788936915}"#);
        let (uid, label, exp) = validate_access_token(&token).unwrap();
        assert_eq!(uid, "uid-abc");
        assert_eq!(label.as_deref(), Some("用户_T9PBW7"));
        assert_eq!(exp, Some(1788936915));
    }

    #[test]
    fn zhipu_access_token_requires_jwt_shape_and_uid() {
        assert!(validate_access_token("not-a-jwt").is_err());
        // 缺 uid 声明（例如误粘贴了别的凭证）应拒绝。
        let no_uid = fake_jwt(r#"{"sub":"x","exp":123}"#);
        assert!(validate_access_token(&no_uid).is_err());
    }

    #[test]
    fn zhipu_refresh_token_optional_but_validated_when_present() {
        // 单独调用时空串报错；组合入口（import_account）里空串代表"未提供"，跳过校验。
        assert!(validate_refresh_token("").is_err());
        assert!(validate_refresh_token(&fake_jwt(r#"{"sub":"x"}"#)).is_ok());
        assert!(validate_refresh_token("garbage").is_err());
    }

    #[test]
    fn zhipu_stable_account_id_uses_uid_hash() {
        let first = stable_account_id("uid-a");
        assert_eq!(first, stable_account_id("uid-a"));
        assert!(first.starts_with("zp-"));
        assert_eq!(first.len(), 15);
        assert_ne!(first, stable_account_id("uid-b"));
    }

    #[test]
    fn zhipu_import_encrypts_tokens_and_dedupes_by_uid() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "zhipu-account-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let old_access = fake_jwt(r#"{"sub":"用户A_T9","uid":"uid-one","exp":1000}"#);
        let new_access = fake_jwt(r#"{"sub":"用户A_T9","uid":"uid-one","exp":2000}"#);
        let refresh = fake_jwt(r#"{"sub":"用户A_T9","type":"refresh"}"#);

        let first = import_account(&old_access, &refresh, Some("主力号".into())).unwrap();
        set_checkin_enabled(&first.id, false).unwrap();
        let second = import_account(&new_access, &refresh, None).unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.display_name, "主力号");
        assert!(!second.checkin_enabled);
        assert_eq!(second.token_expires_at, Some(2000));
        // 视图与落盘文件都不包含 token 原文。
        let view_json = serde_json::to_string(&second).unwrap();
        assert!(!view_json.contains("signature"));
        let index = fs::read_to_string(index_path().unwrap()).unwrap();
        assert!(!index.contains("signature"));
        let detail = fs::read_to_string(detail_path(&second.id).unwrap()).unwrap();
        assert!(!detail.contains("signature"));
        assert!(detail.contains("AES-256-GCM"));
        // uid 本身不是凭证，允许出现在明文索引（与 WorkBuddy 惯例一致）。

        // token 未变化时重复导入不重置同步状态。
        mark_github_sync_many(&[first.id.clone()], "synced", None).unwrap();
        let third = import_account(&new_access, &refresh, None).unwrap();
        assert_eq!(third.last_github_sync_state, "synced");
        // 新 token 后回到待同步。
        let rotated = fake_jwt(r#"{"sub":"用户A_T9","uid":"uid-one","exp":3000}"#);
        let fourth = import_account(&rotated, &refresh, None).unwrap();
        assert_eq!(fourth.last_github_sync_state, "pending");

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn zhipu_corrupted_index_is_rebuilt_from_encrypted_details() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "zhipu-index-repair-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let imported = import_account(
            &fake_jwt(r#"{"sub":"u","uid":"uid-repair","exp":1}"#),
            "",
            None,
        )
        .unwrap();
        fs::write(index_path().unwrap(), "{not-valid-json").unwrap();

        let recovered = list_zhipu_accounts().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id, imported.id);
        assert!(!fs::read_to_string(index_path().unwrap())
            .unwrap()
            .contains("signature"));

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn zhipu_delete_removes_index_entry_and_detail_file() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "zhipu-delete-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let imported = import_account(
            &fake_jwt(r#"{"sub":"u","uid":"uid-delete","exp":1}"#),
            "",
            None,
        )
        .unwrap();
        remove_zhipu_account(&imported.id).unwrap();
        assert!(list_zhipu_accounts().unwrap().is_empty());
        assert!(!detail_path(&imported.id).unwrap().exists());
        assert!(remove_zhipu_account(&imported.id).is_err());

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn zhipu_sync_state_redacts_token_in_error() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "zhipu-sync-redact-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let token = fake_jwt(r#"{"sub":"u","uid":"uid-redact","exp":1}"#);
        let imported = import_account(&token, "", None).unwrap();
        // JWT 的 base64 段超过 20 字符，redact_for_log 会整段替换。
        let secret_fragment = token.split('.').next().unwrap();
        mark_github_sync_many(
            &[imported.id.clone()],
            "failed",
            Some(&format!("远端拒绝 token={secret_fragment}.AAAA")),
        )
        .unwrap();
        let account = list_zhipu_accounts().unwrap().remove(0);
        assert_eq!(account.last_github_sync_state, "failed");
        assert!(!account
            .last_github_sync_error
            .as_deref()
            .unwrap_or_default()
            .contains(secret_fragment));

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn zhipu_client_import_reads_plaintext_cookies_database() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "zhipu-cookies-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        // 构造模拟 Cookies 库（明文 value 列，与真实客户端一致）。
        let cookies_db = dir.join("Cookies");
        {
            let connection = rusqlite::Connection::open(&cookies_db).unwrap();
            connection
                .execute_batch(
                    "create table cookies (host_key text, name text, value text, encrypted_value blob);",
                )
                .unwrap();
            let access = fake_jwt(r#"{"sub":"用户C_T9","uid":"uid-client","exp":4000}"#);
            let refresh = fake_jwt(r#"{"sub":"用户C_T9","type":"refresh"}"#);
            for (host, name, value) in [
                ("chatglm.cn", "chatglm_token", access.as_str()),
                ("chatglm.cn", "chatglm_refresh_token", refresh.as_str()),
                ("chatglm.cn", "chatglm_user_id", "uid-client"),
                ("other.example.com", "chatglm_token", "should-not-match"),
            ] {
                connection
                    .execute(
                        "insert into cookies (host_key, name, value, encrypted_value) values (?1, ?2, ?3, x'')",
                        rusqlite::params![host, name, value],
                    )
                    .unwrap();
            }
        }

        let values = read_cookie_values(&cookies_db, &["chatglm_token", "chatglm_refresh_token"]).unwrap();
        assert_eq!(values.len(), 2, "只匹配 chatglm.cn 域: {values:?}");
        let imported = import_current_client_account_inner(&cookies_db, Some("客户端号".into())).unwrap();
        assert_eq!(imported.display_name, "客户端号");
        assert_eq!(imported.token_expires_at, Some(4000));
        assert_eq!(access_token(&imported.id).unwrap().split('.').count(), 3);
        assert_eq!(refresh_token(&imported.id).unwrap().split('.').count(), 3);

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    /// 测试用：允许指定 Cookies 路径的导入（生产入口是 import_current_client_account）。
    fn import_current_client_account_inner(
        cookies_path: &Path,
        display_name: Option<String>,
    ) -> Result<ZhipuAccountView, String> {
        let values = read_cookie_values(cookies_path, &["chatglm_token", "chatglm_refresh_token"])?;
        let access = values
            .iter()
            .find(|(name, _)| name == "chatglm_token")
            .map(|(_, value)| value.as_str())
            .ok_or_else(|| "未找到 chatglm_token".to_string())?;
        let refresh = values
            .iter()
            .find(|(name, _)| name == "chatglm_refresh_token")
            .map(|(_, value)| value.as_str())
            .unwrap_or("");
        import_account(access, refresh, display_name)
    }
}

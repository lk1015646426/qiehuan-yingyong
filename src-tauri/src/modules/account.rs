use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::models::{Account, AccountIndex, AccountSummary, QuotaData, QuotaErrorInfo, TokenData};
use crate::modules;

static ACCOUNT_INDEX_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));
static AUTO_SWITCH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static QUOTA_ALERT_LAST_SENT: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static LIST_ACCOUNTS_CACHE: std::sync::LazyLock<Mutex<Option<ListAccountsCacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));
static LIST_ACCOUNTS_LOAD_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

const QUOTA_ALERT_COOLDOWN_SECONDS: i64 = 300;
const LIST_ACCOUNTS_CACHE_TTL_MS: u64 = 800;

// Machine-readable identity for the renamed app. User-facing strings use 切换应用.
const DATA_DIR: &str = ".qiehuan_yingyong";
const DEV_DATA_DIR: &str = ".qiehuan_yingyong_dev";
const DATA_DIR_ENV: &str = "QIEHUAN_YINGYONG_DATA_DIR";
const PROFILE_ENV: &str = "QIEHUAN_YINGYONG_PROFILE";
const LEGACY_DATA_DIR_ENVS: [&str; 3] = [
    "TRAE_WORK_CN_SWITCHER_DATA_DIR",
    "COCKPIT_TOOLS_DATA_DIR",
    "COCKPIT_DATA_DIR",
];
const LEGACY_PROFILE_ENVS: [&str; 2] = ["TRAE_WORK_CN_SWITCHER_PROFILE", "COCKPIT_TOOLS_PROFILE"];

const ACCOUNTS_INDEX: &str = "accounts.json";
const ACCOUNTS_DIR: &str = "accounts";
const ACCOUNT_TOKEN_KEY_FILE: &str = "account-token.key";
const ACCOUNT_TOKEN_ENCRYPTION_VERSION: u32 = 1;
const ACCOUNT_TOKEN_ROTATION_SECONDS: i64 = 30 * 24 * 60 * 60;

fn legacy_data_dir_names() -> &'static [&'static str] {
    &[
        ".trae_work_cn_switcher",
        ".trae_work_cn_switcher_dev",
        ".antigravity_cockpit",
        ".antigravity_cockpit_dev",
        ".cockpit_tools",
        ".cockpit-tools",
    ]
}

fn copy_dir_contents(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::create_dir_all(target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_contents(&source_path, &target_path)?;
        } else if file_type.is_file() && !target_path.exists() {
            fs::copy(&source_path, &target_path)?;
        }
    }
    Ok(())
}

fn migrate_legacy_data_dirs(home: &Path, target: &Path) {
    for name in legacy_data_dir_names() {
        let source = home.join(name);
        if !source.exists() || source == target {
            continue;
        }
        if let Err(error) = copy_dir_contents(&source, target) {
            eprintln!(
                "切换应用旧数据迁移失败，保留旧目录继续运行: source={}, target={}, error={}",
                source.display(),
                target.display(),
                error
            );
        } else {
            eprintln!(
                "切换应用已兼容旧数据目录: source={}, target={}",
                source.display(),
                target.display()
            );
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedTokenEnvelope {
    version: u32,
    algorithm: String,
    key_id: String,
    nonce: String,
    ciphertext: String,
    encrypted_at: i64,
}

#[derive(Clone)]
struct ListAccountsCacheEntry {
    cached_at: Instant,
    accounts: Vec<Account>,
}

fn invalidate_list_accounts_cache() {
    if let Ok(mut cache) = LIST_ACCOUNTS_CACHE.lock() {
        *cache = None;
    }
}

fn read_list_accounts_cache() -> Option<Vec<Account>> {
    let Ok(cache) = LIST_ACCOUNTS_CACHE.lock() else {
        return None;
    };

    let Some(entry) = cache.as_ref() else {
        return None;
    };

    if entry.cached_at.elapsed() > Duration::from_millis(LIST_ACCOUNTS_CACHE_TTL_MS) {
        return None;
    }

    Some(entry.accounts.clone())
}

fn write_list_accounts_cache(accounts: &[Account]) {
    if let Ok(mut cache) = LIST_ACCOUNTS_CACHE.lock() {
        *cache = Some(ListAccountsCacheEntry {
            cached_at: Instant::now(),
            accounts: accounts.to_vec(),
        });
    }
}

fn account_token_key_path() -> Result<PathBuf, String> {
    Ok(get_data_dir()?.join(ACCOUNT_TOKEN_KEY_FILE))
}

fn read_or_create_account_token_master_key() -> Result<[u8; 32], String> {
    let key_path = account_token_key_path()?;
    if key_path.exists() {
        let raw = fs::read_to_string(&key_path)
            .map_err(|e| format!("读取账号 token 加密密钥失败: {}", e))?;
        let bytes = general_purpose::STANDARD
            .decode(raw.trim())
            .map_err(|e| format!("解析账号 token 加密密钥失败: {}", e))?;
        if bytes.len() != 32 {
            return Err("账号 token 加密密钥长度无效".to_string());
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        return Ok(key);
    }

    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    let encoded = general_purpose::STANDARD.encode(key);
    crate::modules::atomic_write::write_string_atomic(&key_path, &encoded)
        .map_err(|e| format!("写入账号 token 加密密钥失败: {}", e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

fn account_token_cipher() -> Result<Aes256Gcm, String> {
    let master_key = read_or_create_account_token_master_key()?;
    Aes256Gcm::new_from_slice(&master_key).map_err(|e| format!("初始化账号 token 加密失败: {}", e))
}

fn encrypt_token_value(token: &TokenData) -> Result<EncryptedTokenEnvelope, String> {
    let cipher = account_token_cipher()?;
    let plaintext =
        serde_json::to_vec(token).map_err(|e| format!("序列化账号 token 失败: {}", e))?;
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|e| format!("加密账号 token 失败: {:?}", e))?;
    Ok(EncryptedTokenEnvelope {
        version: ACCOUNT_TOKEN_ENCRYPTION_VERSION,
        algorithm: "AES-256-GCM".to_string(),
        key_id: "local-account-token-key-v1".to_string(),
        nonce: general_purpose::STANDARD.encode(nonce_bytes),
        ciphertext: general_purpose::STANDARD.encode(ciphertext),
        encrypted_at: chrono::Utc::now().timestamp(),
    })
}

fn decrypt_token_envelope(envelope: &EncryptedTokenEnvelope) -> Result<TokenData, String> {
    if envelope.version != ACCOUNT_TOKEN_ENCRYPTION_VERSION {
        return Err("账号 token 加密版本不受支持".to_string());
    }
    let cipher = account_token_cipher()?;
    let nonce = general_purpose::STANDARD
        .decode(envelope.nonce.trim())
        .map_err(|e| format!("解析账号 token nonce 失败: {}", e))?;
    if nonce.len() != 12 {
        return Err("账号 token nonce 长度无效".to_string());
    }
    let ciphertext = general_purpose::STANDARD
        .decode(envelope.ciphertext.trim())
        .map_err(|e| format!("解析账号 token 密文失败: {}", e))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|e| format!("解密账号 token 失败: {:?}", e))?;
    serde_json::from_slice::<TokenData>(&plaintext)
        .map_err(|e| format!("解析账号 token 明文失败: {}", e))
}

fn should_rotate_token_envelope(envelope: &EncryptedTokenEnvelope) -> bool {
    chrono::Utc::now().timestamp() - envelope.encrypted_at > ACCOUNT_TOKEN_ROTATION_SECONDS
}

fn serialize_account_for_storage(account: &Account) -> Result<String, String> {
    let mut value =
        serde_json::to_value(account).map_err(|e| format!("序列化账号数据失败: {}", e))?;
    let Some(object) = value.as_object_mut() else {
        return Err("账号数据结构无效".to_string());
    };
    object.remove("token");
    object.insert(
        "token_encrypted".to_string(),
        serde_json::to_value(encrypt_token_value(&account.token)?)
            .map_err(|e| format!("序列化账号 token 密文失败: {}", e))?,
    );
    serde_json::to_string_pretty(&value).map_err(|e| format!("序列化账号数据失败: {}", e))
}

fn deserialize_account_from_storage(
    account_path: &PathBuf,
    content: &str,
) -> Result<Account, String> {
    let mut value = serde_json::from_str::<serde_json::Value>(content)
        .map_err(|e| format!("解析账号数据失败: {}", e))?;
    let mut needs_migration = false;
    let mut needs_rotation = false;

    if value.get("token").is_none() {
        let encrypted = value
            .get("token_encrypted")
            .cloned()
            .ok_or_else(|| "账号数据缺少 token".to_string())?;
        let envelope = serde_json::from_value::<EncryptedTokenEnvelope>(encrypted)
            .map_err(|e| format!("解析账号 token 密文失败: {}", e))?;
        let token = decrypt_token_envelope(&envelope)?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "token".to_string(),
                serde_json::to_value(token).map_err(|e| format!("还原账号 token 失败: {}", e))?,
            );
        }
        needs_rotation = should_rotate_token_envelope(&envelope);
    } else {
        needs_migration = true;
    }

    if let Some(object) = value.as_object_mut() {
        object.remove("token_encrypted");
    }

    let account =
        serde_json::from_value::<Account>(value).map_err(|e| format!("解析账号数据失败: {}", e))?;

    if needs_migration || needs_rotation {
        let account_for_rewrite = account.clone();
        modules::deferred_account_rewrite::schedule_account_rewrite_if_unchanged(
            "antigravity",
            account_for_rewrite.id.clone(),
            account_path.clone(),
            content.as_bytes(),
            move || serialize_account_for_storage(&account_for_rewrite),
        );
    }

    Ok(account)
}

/// 获取数据目录路径
pub fn is_dev_profile() -> bool {
    if cfg!(debug_assertions) {
        return true;
    }

    std::iter::once(PROFILE_ENV)
        .chain(LEGACY_PROFILE_ENVS.iter().copied())
        .filter_map(|key| std::env::var(key).ok())
        .any(|value| value.trim().eq_ignore_ascii_case("dev"))
}

pub fn resolve_data_dir() -> Result<PathBuf, String> {
    for key in std::iter::once(DATA_DIR_ENV).chain(LEGACY_DATA_DIR_ENVS.iter().copied()) {
        if let Ok(raw) = std::env::var(key) {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                return Ok(PathBuf::from(trimmed));
            }
        }
    }

    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let data_dir = home.join(DATA_DIR);
    migrate_legacy_data_dirs(&home, &data_dir);
    Ok(data_dir)
}

pub fn get_data_dir() -> Result<PathBuf, String> {
    // #816: tests can isolate storage via env without touching real user data.
    // Production builds only accept this application's overrides. Test builds retain
    // the upstream aliases so the inherited suite stays isolated from real user data.
    #[cfg(test)]
    let override_keys = [
        "QIEHUAN_YINGYONG_TEST_DATA_DIR",
        "QIEHUAN_YINGYONG_DATA_DIR",
        "TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR",
        "TRAE_WORK_CN_SWITCHER_DATA_DIR",
        "COCKPIT_TOOLS_TEST_DATA_DIR",
        "COCKPIT_TEST_DATA_DIR",
    ];
    #[cfg(not(test))]
    let override_keys = [
        "QIEHUAN_YINGYONG_DATA_DIR",
        "TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR",
        "TRAE_WORK_CN_SWITCHER_DATA_DIR",
        "COCKPIT_TOOLS_DATA_DIR",
        "COCKPIT_DATA_DIR",
    ];

    for key in override_keys {
        if let Ok(override_dir) = std::env::var(key) {
            let override_dir = override_dir.trim();
            if !override_dir.is_empty() {
                let data_dir = PathBuf::from(override_dir);
                if !data_dir.exists() {
                    fs::create_dir_all(&data_dir)
                        .map_err(|e| format!("创建测试数据目录失败: {}", e))?;
                }
                return Ok(data_dir);
            }
        }
    }

    let data_dir = resolve_data_dir()?;

    if !data_dir.exists() {
        fs::create_dir_all(&data_dir).map_err(|e| format!("创建数据目录失败: {}", e))?;
    }

    Ok(data_dir)
}

/// 获取账号目录路径
pub fn get_accounts_dir() -> Result<PathBuf, String> {
    let data_dir = get_data_dir()?;
    let accounts_dir = data_dir.join(ACCOUNTS_DIR);

    if !accounts_dir.exists() {
        fs::create_dir_all(&accounts_dir).map_err(|e| format!("创建账号目录失败: {}", e))?;
    }

    Ok(accounts_dir)
}

fn repair_account_index_from_details(reason: &str) -> Result<Option<AccountIndex>, String> {
    let index_path = get_data_dir()?.join(ACCOUNTS_INDEX);
    let accounts_dir = get_accounts_dir()?;
    let mut accounts = crate::modules::account_index_repair::load_accounts_from_details(
        &accounts_dir,
        |account_id| load_account(account_id).ok(),
    )?;

    if accounts.is_empty() {
        return Ok(None);
    }

    crate::modules::account_index_repair::sort_accounts_by_recency(
        &mut accounts,
        |account| account.last_used,
        |account| account.created_at,
        |account| account.id.as_str(),
    );

    let mut index = AccountIndex::new();
    index.accounts = accounts
        .iter()
        .map(|account| AccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            name: account.name.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        })
        .collect();
    index.current_account_id = None;

    let backup_path = crate::modules::account_index_repair::backup_existing_index(&index_path)
        .unwrap_or_else(|err| {
            modules::logger::log_warn(&format!(
                "自动修复账号索引前备份失败，继续尝试重建: path={}, error={}",
                index_path.display(),
                err
            ));
            None
        });

    if let Err(err) = save_account_index(&index) {
        modules::logger::log_warn(&format!(
            "自动修复账号索引保存失败，将以内存结果继续运行: reason={}, recovered_accounts={}, error={}",
            reason,
            index.accounts.len(),
            err
        ));
    }

    modules::logger::log_warn(&format!(
        "检测到账号索引异常，已根据详情文件自动重建: reason={}, recovered_accounts={}, backup_path={}",
        reason,
        index.accounts.len(),
        backup_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "-".to_string())
    ));

    Ok(Some(index))
}

/// 加载账号索引
pub fn load_account_index() -> Result<AccountIndex, String> {
    let data_dir = get_data_dir()?;
    let index_path = data_dir.join(ACCOUNTS_INDEX);

    if !index_path.exists() {
        if let Some(index) = repair_account_index_from_details("索引文件不存在")? {
            return Ok(index);
        }
        return Ok(AccountIndex::new());
    }

    let content =
        fs::read_to_string(&index_path).map_err(|e| format!("读取账号索引失败: {}", e))?;

    if content.trim().is_empty() {
        if let Some(index) = repair_account_index_from_details("索引文件为空")? {
            return Ok(index);
        }
        return Ok(AccountIndex::new());
    }

    match crate::modules::atomic_write::parse_json_with_auto_restore::<AccountIndex>(
        &index_path,
        &content,
    ) {
        Ok(index) => {
            if index.accounts.is_empty() {
                if let Some(repaired) = repair_account_index_from_details("索引账号列表为空")?
                {
                    return Ok(repaired);
                }
            }
            Ok(index)
        }
        Err(e) => {
            if let Some(index) = repair_account_index_from_details("索引文件损坏")? {
                return Ok(index);
            }
            Err(crate::error::file_corrupted_error(
                ACCOUNTS_INDEX,
                &index_path.to_string_lossy(),
                &e.to_string(),
            ))
        }
    }
}

/// 保存账号索引
pub fn save_account_index(index: &AccountIndex) -> Result<(), String> {
    let data_dir = get_data_dir()?;
    let index_path = data_dir.join(ACCOUNTS_INDEX);

    let content =
        serde_json::to_string_pretty(index).map_err(|e| format!("序列化账号索引失败: {}", e))?;

    crate::modules::atomic_write::write_string_atomic(&index_path, &content)
        .map_err(|e| format!("写入账号索引失败: {}", e))?;
    invalidate_list_accounts_cache();
    Ok(())
}

/// 加载账号数据
pub fn load_account(account_id: &str) -> Result<Account, String> {
    let accounts_dir = get_accounts_dir()?;
    let account_path = accounts_dir.join(format!("{}.json", account_id));

    if !account_path.exists() {
        return Err(format!("账号不存在: {}", account_id));
    }

    let content =
        fs::read_to_string(&account_path).map_err(|e| format!("读取账号数据失败: {}", e))?;

    deserialize_account_from_storage(&account_path, &content)
}

/// 保存账号数据
pub fn save_account(account: &Account) -> Result<(), String> {
    let accounts_dir = get_accounts_dir()?;
    let account_path = accounts_dir.join(format!("{}.json", account.id));

    let content = serialize_account_for_storage(account)?;

    crate::modules::atomic_write::write_string_atomic(&account_path, &content)
        .map_err(|e| format!("保存账号数据失败: {}", e))?;
    invalidate_list_accounts_cache();
    Ok(())
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, String> {
    let mut result: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for raw in tags {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("标签不能为空".to_string());
        }
        if trimmed.chars().count() > 20 {
            return Err("标签长度不能超过 20 个字符".to_string());
        }
        let normalized = trimmed.to_lowercase();
        if seen.insert(normalized.clone()) {
            result.push(normalized);
        }
    }

    if result.len() > 10 {
        return Err("标签数量不能超过 10 个".to_string());
    }

    Ok(result)
}

/// 更新账号标签
pub fn update_account_tags(account_id: &str, tags: Vec<String>) -> Result<Account, String> {
    let mut account = load_account(account_id)?;
    let normalized = normalize_tags(tags)?;
    account.tags = normalized;
    save_account(&account)?;
    Ok(account)
}

/// 更新账号备注
pub fn update_account_notes(account_id: &str, notes: String) -> Result<Account, String> {
    let mut account = load_account(account_id)?;
    let trimmed = notes.trim().to_string();
    account.notes = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    save_account(&account)?;
    Ok(account)
}

/// 列出所有账号
pub fn list_accounts() -> Result<Vec<Account>, String> {
    if let Some(accounts) = read_list_accounts_cache() {
        return Ok(accounts);
    }

    let _load_guard = LIST_ACCOUNTS_LOAD_LOCK
        .lock()
        .map_err(|e| format!("获取账号列表锁失败: {}", e))?;

    if let Some(accounts) = read_list_accounts_cache() {
        return Ok(accounts);
    }

    modules::logger::log_info("开始列出账号...");
    let index = load_account_index()?;
    let mut accounts = Vec::new();

    for summary in &index.accounts {
        match load_account(&summary.id) {
            Ok(mut account) => {
                let _ = modules::quota_cache::apply_cached_quota(&mut account, "authorized");
                accounts.push(account);
            }
            Err(e) => {
                modules::logger::log_error(&format!("加载账号失败: {}", e));
            }
        }
    }

    if !index.accounts.is_empty() && accounts.is_empty() {
        return Err(format!(
            "账号索引中有 {} 个账号，但详情文件均无法读取；已保留前端缓存，请从账号备份或本地账号文件恢复。",
            index.accounts.len()
        ));
    }

    write_list_accounts_cache(&accounts);
    Ok(accounts)
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn is_strict_account_identity_match(existing: &Account, email: &str, token: &TokenData) -> bool {
    if let Some(session_id) = non_empty(token.session_id.as_deref()) {
        if non_empty(existing.token.session_id.as_deref()) == Some(session_id) {
            return true;
        }
    }

    if let Some(refresh_token) = non_empty(Some(token.refresh_token.as_str())) {
        if non_empty(Some(existing.token.refresh_token.as_str())) == Some(refresh_token) {
            return true;
        }
    }

    if existing.email == email {
        if let Some(project_id) = non_empty(token.project_id.as_deref()) {
            if non_empty(existing.token.project_id.as_deref()) == Some(project_id) {
                return true;
            }
        }
    }

    false
}

fn find_matching_account_id(
    index: &AccountIndex,
    email: &str,
    token: &TokenData,
) -> Result<Option<String>, String> {
    for summary in &index.accounts {
        let existing = match load_account(&summary.id) {
            Ok(account) => account,
            Err(err) => {
                modules::logger::log_warn(&format!(
                    "账号匹配时跳过损坏账号文件: id={}, error={}",
                    summary.id, err
                ));
                continue;
            }
        };

        if is_strict_account_identity_match(&existing, email, token) {
            return Ok(Some(existing.id));
        }
    }

    Ok(None)
}

/// 添加账号
pub fn add_account(
    email: String,
    name: Option<String>,
    token: TokenData,
) -> Result<Account, String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    if find_matching_account_id(&index, &email, &token)?.is_some() {
        return Err(format!("账号已存在: {}", email));
    }

    let account_id = Uuid::new_v4().to_string();
    let mut account = Account::new(account_id.clone(), email.clone(), token);
    account.name = name.clone();

    save_account(&account)?;

    index.accounts.push(AccountSummary {
        id: account_id.clone(),
        email: email.clone(),
        name: name.clone(),
        created_at: account.created_at,
        last_used: account.last_used,
    });

    save_account_index(&index)?;

    Ok(account)
}

/// 添加或更新账号
pub fn upsert_account(
    email: String,
    name: Option<String>,
    token: TokenData,
) -> Result<Account, String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    let existing_account_id = find_matching_account_id(&index, &email, &token)?;

    if let Some(account_id) = existing_account_id {
        match load_account(&account_id) {
            Ok(mut account) => {
                account.token = token;
                account.name = name.clone();
                if account.disabled {
                    account.disabled = false;
                    account.disabled_reason = None;
                    account.disabled_at = None;
                }
                account.update_last_used();
                save_account(&account)?;

                if let Some(idx_summary) = index.accounts.iter_mut().find(|s| s.id == account_id) {
                    idx_summary.name = name;
                    save_account_index(&index)?;
                }

                return Ok(account);
            }
            Err(e) => {
                modules::logger::log_warn(&format!("账号文件缺失，正在重建: {}", e));
                let mut account = Account::new(account_id.clone(), email.clone(), token);
                account.name = name.clone();
                save_account(&account)?;

                if let Some(idx_summary) = index.accounts.iter_mut().find(|s| s.id == account_id) {
                    idx_summary.name = name;
                    save_account_index(&index)?;
                }

                return Ok(account);
            }
        }
    }

    drop(_lock);
    add_account(email, name, token)
}

/// 删除账号
pub fn delete_account(account_id: &str) -> Result<(), String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    let original_len = index.accounts.len();
    index.accounts.retain(|s| s.id != account_id);

    if index.accounts.len() == original_len {
        return Err(format!("找不到账号 ID: {}", account_id));
    }

    if index.current_account_id.as_deref() == Some(account_id) {
        index.current_account_id = None;
    }

    save_account_index(&index)?;

    let accounts_dir = get_accounts_dir()?;
    let account_path = accounts_dir.join(format!("{}.json", account_id));

    if account_path.exists() {
        crate::modules::atomic_write::remove_file_locked(&account_path)
            .map_err(|e| format!("删除账号文件失败: {}", e))?;
    }

    Ok(())
}

/// 批量删除账号
pub fn delete_accounts(account_ids: &[String]) -> Result<(), String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    let accounts_dir = get_accounts_dir()?;

    for account_id in account_ids {
        index.accounts.retain(|s| &s.id != account_id);

        if index.current_account_id.as_deref() == Some(account_id) {
            index.current_account_id = None;
        }

        let account_path = accounts_dir.join(format!("{}.json", account_id));
        if account_path.exists() {
            let _ = crate::modules::atomic_write::remove_file_locked(&account_path);
        }
    }

    save_account_index(&index)
}

/// 重新排序账号列表
pub fn reorder_accounts(account_ids: &[String]) -> Result<(), String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;

    let id_to_summary: std::collections::HashMap<_, _> = index
        .accounts
        .iter()
        .map(|s| (s.id.clone(), s.clone()))
        .collect();

    let mut new_accounts = Vec::new();
    for id in account_ids {
        if let Some(summary) = id_to_summary.get(id) {
            new_accounts.push(summary.clone());
        }
    }

    for summary in &index.accounts {
        if !account_ids.contains(&summary.id) {
            new_accounts.push(summary.clone());
        }
    }

    index.accounts = new_accounts;

    save_account_index(&index)
}

/// 获取当前账号 ID
pub fn get_current_account_id() -> Result<Option<String>, String> {
    let index = load_account_index()?;
    Ok(index.current_account_id)
}

/// 获取当前激活账号
pub fn get_current_account() -> Result<Option<Account>, String> {
    if let Some(id) = get_current_account_id()? {
        let mut account = load_account(&id)?;
        let _ = modules::quota_cache::apply_cached_quota(&mut account, "authorized");
        Ok(Some(account))
    } else {
        Ok(None)
    }
}

/// 设置当前激活账号 ID
pub fn set_current_account_id(account_id: &str) -> Result<(), String> {
    let _lock = ACCOUNT_INDEX_LOCK
        .lock()
        .map_err(|e| format!("获取锁失败: {}", e))?;
    let mut index = load_account_index()?;
    index.current_account_id = Some(account_id.to_string());
    save_account_index(&index)?;

    // 同时写入 current_account.json 供扩展读取
    if let Ok(account) = load_account(account_id) {
        let _ = save_current_account_file(&account.email);
    }

    Ok(())
}

/// 保存当前账号信息到共享文件（供扩展启动时读取）
fn save_current_account_file(email: &str) -> Result<(), String> {
    use std::fs;
    use std::io::Write;

    let data_dir = get_data_dir()?;
    let file_path = data_dir.join("current_account.json");

    let content = serde_json::json!({
        "email": email,
        "updated_at": chrono::Utc::now().timestamp()
    });

    let json = serde_json::to_string_pretty(&content).map_err(|e| format!("序列化失败: {}", e))?;

    let mut file = fs::File::create(&file_path).map_err(|e| format!("创建文件失败: {}", e))?;
    file.write_all(json.as_bytes())
        .map_err(|e| format!("写入文件失败: {}", e))?;

    modules::logger::log_info("已保存当前账号");
    Ok(())
}

/// 更新账号配额
pub fn update_account_quota(account_id: &str, quota: QuotaData) -> Result<(), String> {
    let mut account = load_account(account_id)?;

    // 容错：如果新获取的 models 为空，但之前有数据，保留原来的 models
    if quota.models.is_empty() {
        if let Some(ref existing_quota) = account.quota {
            if !existing_quota.models.is_empty() {
                modules::logger::log_warn(&format!(
                    "⚠️ 新配额 models 为空，保留原有 {} 个模型数据",
                    existing_quota.models.len()
                ));
                // 只更新非 models 字段（subscription_tier, is_forbidden 等）
                let mut merged_quota = existing_quota.clone();
                merged_quota.subscription_tier = quota.subscription_tier.clone();
                merged_quota.is_forbidden = quota.is_forbidden;
                merged_quota.last_updated = quota.last_updated;
                account.update_quota(merged_quota);
                account.usage_updated_at = Some(chrono::Utc::now().timestamp());
                save_account(&account)?;
                return Ok(());
            }
        }
    }

    account.update_quota(quota);
    account.usage_updated_at = Some(chrono::Utc::now().timestamp());
    save_account(&account)?;
    if let Some(ref quota) = account.quota {
        let _ = modules::quota_cache::write_quota_cache("authorized", &account.email, quota);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{legacy_data_dir_names, DATA_DIR, DEV_DATA_DIR};

    #[test]
    fn uses_qiehuan_yingyong_storage_names() {
        assert_eq!(DATA_DIR, ".qiehuan_yingyong");
        assert_eq!(DEV_DATA_DIR, ".qiehuan_yingyong_dev");
    }

    #[test]
    fn keeps_all_known_legacy_storage_names_for_migration() {
        let names = legacy_data_dir_names();
        assert!(names.contains(&".trae_work_cn_switcher"));
        assert!(names.contains(&".trae_work_cn_switcher_dev"));
        assert!(names.contains(&".antigravity_cockpit"));
        assert!(names.contains(&".antigravity_cockpit_dev"));
        assert!(names.contains(&".cockpit_tools"));
        assert!(names.contains(&".cockpit-tools"));
    }
}

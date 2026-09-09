//! 阶段 7 — TRAE Work CN 后台会话监测器。
//!
//! 每 60s（首轮延迟 10s）读取官方客户端的 `storage.json`，在身份匹配账号库时
//! 把客户端轮换后的 access/refresh token 回写账号库，并（仅当账号绑定了 GitHub
//! 槽位且启用了同步时）把最新凭证同步到 GitHub Secrets。
//!
//! 硬约束（开发指南 §8.5 / 阶段 7）：
//! - **只查询、只写 GitHub Secrets**：全链路只允许 `gh auth status` 与
//!   `gh secret set`，绝不触发 GitHub 签到/领取/计费（`gh workflow run` /
//!   `gh run` / `claim` / `check-in` 全部禁区）。
//! - 只复用 `read_local_trae_auth_from_storage_path`（读+解析）与
//!   `sync_account_tokens_from_storage_path`（回写），不触碰 quota/claim 路径。
//!
//! 并发与去重策略（复用 `provider_token_keeper` 的模式）：
//! - 全局切号锁 `WORK_CN_SWITCH_LOCK` 用 `try_lock` 非阻塞探测，占用即跳过；
//! - `LAST_SEEN_MTIME` 记录上次 mtime，未变化直接跳过解密；
//! - `NEXT_ALLOWED_ATTEMPT_AT` 退避表：会话失败退避 15min、GitHub 失败退避 10min。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use tauri::{AppHandle, Emitter};

use crate::models::trae::TraeAccount;
use crate::models::work_cn::{WorkCnSessionWatchOutcome, WorkCnSessionWatchStatus};
use crate::modules::work_cn_github::{GitHubRunner, RealGitHubRunner};
use crate::modules::{logger, trae_account};

const WATCH_INTERVAL_SECONDS: u64 = 60;
const WATCH_STARTUP_DELAY_SECONDS: u64 = 10;
/// 会话读取/解密/回写失败退避时长。
const SESSION_FAILURE_BACKOFF_SECONDS: i64 = 15 * 60;
/// GitHub 同步失败退避时长。
const GITHUB_FAILURE_BACKOFF_SECONDS: i64 = 10 * 60;

static WATCHER_STARTED: AtomicBool = AtomicBool::new(false);
static LAST_SEEN_MTIME: LazyLock<Mutex<Option<SystemTime>>> = LazyLock::new(|| Mutex::new(None));
static LAST_STATUS: LazyLock<Mutex<WorkCnSessionWatchStatus>> =
    LazyLock::new(|| Mutex::new(WorkCnSessionWatchStatus::default()));
static NEXT_ALLOWED_ATTEMPT_AT: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 退避键。
const SESSION_BACKOFF_KEY: &str = "work_cn_session";

fn github_backoff_key(account_id: &str) -> String {
    format!("work_cn_github:{account_id}")
}

/// 后台 → 前端状态推送事件名（与前端 `services/workCnService.ts` 保持一致）。
pub const SESSION_WATCH_EVENT: &str = "work-cn:session-watch";

/// 启动后台监测循环（幂等，`AtomicBool` 防重入）。
pub fn ensure_started(app_handle: AppHandle) {
    if WATCHER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    logger::log_info("[WorkCnSessionWatcher] 后台会话监测已启动");
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(WATCH_STARTUP_DELAY_SECONDS)).await;

        loop {
            let status = tauri::async_runtime::spawn_blocking(|| watch_once(&RealGitHubRunner))
                .await
                .unwrap_or_else(|err| {
                    let mut failed = WorkCnSessionWatchStatus {
                        running: true,
                        last_check_at: now_ts(),
                        outcome: WorkCnSessionWatchOutcome::Failed,
                        ..WorkCnSessionWatchStatus::default()
                    };
                    failed.message = format!("后台监测任务异常: {err}");
                    failed
                });

            update_last_status(status.clone());
            if should_emit(&status) {
                let _ = app_handle.emit(SESSION_WATCH_EVENT, &status);
            }

            tokio::time::sleep(Duration::from_secs(WATCH_INTERVAL_SECONDS)).await;
        }
    });
}

/// 当前监测状态（供 `get_work_cn_session_watch_status` 命令读取）。
pub(crate) fn get_work_cn_session_watch_status() -> WorkCnSessionWatchStatus {
    let mut status = LAST_STATUS
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();
    status.running = WATCHER_STARTED.load(Ordering::SeqCst);
    status
}

/// 一次监测的核心状态机（同步、可单测）。`ensure_started` 在 `spawn_blocking`
/// 里调用；测试直接注入 `FakeGitHubRunner`。
///
/// 顺序：解析路径 → mtime 门禁 → 会话退避门禁 → 切号锁 → 读 → 匹配 → 回写 →
/// （释放锁）→ GitHub 同步（带退避门禁）。
pub(crate) fn watch_once(runner: &dyn GitHubRunner) -> WorkCnSessionWatchStatus {
    let now = now_ts();
    let mut status = WorkCnSessionWatchStatus {
        running: WATCHER_STARTED.load(Ordering::SeqCst),
        last_check_at: now,
        ..WorkCnSessionWatchStatus::default()
    };

    // 1) 解析 storage 路径。
    let path = match trae_account::resolve_current_work_cn_storage_path() {
        Ok(path) => path,
        Err(err) => {
            status.outcome = WorkCnSessionWatchOutcome::Failed;
            status.message = format!("解析 storage 路径失败: {err}");
            return status;
        }
    };

    // 2) mtime 获取；文件不存在 → NoStorage。
    let mtime = match std::fs::metadata(&path).and_then(|meta| meta.modified()) {
        Ok(mtime) => mtime,
        Err(_) => {
            set_last_seen_mtime(None);
            status.outcome = WorkCnSessionWatchOutcome::NoStorage;
            status.message = "未检测到已登录的 TRAE Work CN 会话".to_string();
            return status;
        }
    };

    // 3) 会话失败退避门禁（先于 mtime 比较，退避期内不再重复解密）。
    if !allow_attempt(SESSION_BACKOFF_KEY) {
        status.outcome = WorkCnSessionWatchOutcome::Failed;
        status.message = "后台监测处于退避期，跳过本轮".to_string();
        return status;
    }

    // 4) mtime 未变化 → 跳过解密。
    if let Some(last) = get_last_seen_mtime() {
        if last == mtime {
            status.outcome = WorkCnSessionWatchOutcome::Unchanged;
            status.message = "storage 未变化，跳过解密".to_string();
            return status;
        }
    }

    // 5) 尝试获取切号锁（非阻塞；占用即跳过）。
    let lock_guard = match trae_account::try_lock_work_cn_switch() {
        Some(guard) => guard,
        None => {
            status.outcome = WorkCnSessionWatchOutcome::SwitchBusy;
            status.message = "切号进行中，跳过本轮监测".to_string();
            return status;
        }
    };

    // 6) 读 + 解密。
    let payload = match trae_account::read_local_trae_auth_from_storage_path(&path) {
        Ok(Some(payload)) => payload,
        Ok(None) => {
            set_last_seen_mtime(None);
            status.outcome = WorkCnSessionWatchOutcome::NoStorage;
            status.message = "未检测到已登录的 TRAE Work CN 会话".to_string();
            return status;
        }
        Err(err) => {
            set_last_seen_mtime(Some(mtime));
            mark_attempt_failure_with_backoff(SESSION_BACKOFF_KEY, SESSION_FAILURE_BACKOFF_SECONDS);
            logger::log_warn(&format!(
                "[WorkCnSessionWatcher] 读取 storage 失败，进入退避: error={err}"
            ));
            status.outcome = WorkCnSessionWatchOutcome::Failed;
            status.message = format!("读取 storage 失败: {err}");
            return status;
        }
    };

    // 7) UID/email 匹配账号库。
    let account_id = match trae_account::find_work_cn_account_id_for_payload(&payload) {
        Some(id) => id,
        None => {
            set_last_seen_mtime(Some(mtime));
            status.outcome = WorkCnSessionWatchOutcome::NoMatch;
            status.message = "当前客户端账号不在账号库中，可先导入".to_string();
            return status;
        }
    };

    // 8) 加载账号 → 应用 token → 保留设备快照 → 回写。
    let mut account = match trae_account::load_account(&account_id) {
        Some(account) => account,
        None => {
            set_last_seen_mtime(Some(mtime));
            status.outcome = WorkCnSessionWatchOutcome::NoMatch;
            status.message = "账号库中未找到匹配账号".to_string();
            return status;
        }
    };

    let account_before = account.clone();
    let session_changed =
        trae_account::sync_account_tokens_from_storage_path(&mut account, &path, "后台监测");
    let token_changed = account_before.access_token != account.access_token
        || account_before.refresh_token != account.refresh_token;
    preserve_account_metadata(&account_before, &mut account);

    if let Err(err) = trae_account::persist_session_refresh_result(&account, &account_before) {
        set_last_seen_mtime(Some(mtime));
        mark_attempt_failure_with_backoff(SESSION_BACKOFF_KEY, SESSION_FAILURE_BACKOFF_SECONDS);
        logger::log_warn(&format!(
            "[WorkCnSessionWatcher] 回写账号库失败，进入退避: account_id={account_id}, error={err}"
        ));
        status.account_id = Some(account_id);
        status.outcome = WorkCnSessionWatchOutcome::Failed;
        status.message = format!("回写账号库失败: {err}");
        return status;
    }

    clear_attempt_backoff(SESSION_BACKOFF_KEY);
    set_last_seen_mtime(Some(mtime));
    // 释放切号锁：GitHub 同步在锁外执行，避免 `gh auth status` 网络慢阻塞用户切号。
    drop(lock_guard);

    status.account_id = Some(account_id.clone());

    if !token_changed {
        status.outcome = WorkCnSessionWatchOutcome::NoChange;
        status.token_changed = false;
        status.message = if session_changed {
            "已更新官方设备快照，暂无 token 变化".to_string()
        } else {
            "账号库已是最新，暂无 token 变化".to_string()
        };
        return status;
    }

    // 9) token 变化 → GitHub 同步（带退避门禁）。
    status.token_changed = true;
    let gh_key = github_backoff_key(&account_id);
    if !allow_attempt(&gh_key) {
        status.outcome = WorkCnSessionWatchOutcome::TokenUpdated;
        status.github_skipped = true;
        status.message =
            "检测到 Token 更新，账号库已更新；GitHub 同步处于退避期，稍后自动重试".to_string();
        return status;
    }

    match crate::modules::work_cn_github::sync_account_secrets_if_bound_with(runner, &account) {
        Ok(result) => {
            clear_attempt_backoff(&gh_key);
            status.github_synced = result.synced;
            status.github_skipped = result.skipped;
            if result.skipped {
                status.message = format!(
                    "检测到 Token 更新，账号库已更新；GitHub 待同步：{}",
                    result
                        .skip_reason
                        .unwrap_or_else(|| "未绑定槽位".to_string())
                );
            } else {
                status.message = "检测到 Token 更新，账号库与 GitHub 已同步".to_string();
                logger::log_info(&format!(
                    "[WorkCnSessionWatcher] 检测到 Token 更新并已同步 GitHub: account_id={account_id}"
                ));
            }
        }
        Err(err) => {
            mark_attempt_failure_with_backoff(&gh_key, GITHUB_FAILURE_BACKOFF_SECONDS);
            logger::log_warn(&format!(
                "[WorkCnSessionWatcher] GitHub 同步失败，进入退避: account_id={account_id}, error={err}"
            ));
            status.github_error = Some(err.clone());
            status.message = format!("检测到 Token 更新，账号库已更新；GitHub 同步失败：{err}");
        }
    }
    status.outcome = WorkCnSessionWatchOutcome::TokenUpdated;
    status
}

/// 事件仅在显著 outcome 时发射，避免每 60s 无谓重渲染（Unchanged/NoChange/
/// SwitchBusy 不发射）。
fn should_emit(status: &WorkCnSessionWatchStatus) -> bool {
    matches!(
        status.outcome,
        WorkCnSessionWatchOutcome::TokenUpdated
            | WorkCnSessionWatchOutcome::NoMatch
            | WorkCnSessionWatchOutcome::Failed
            | WorkCnSessionWatchOutcome::NoStorage
    )
}

fn update_last_status(status: WorkCnSessionWatchStatus) {
    if let Ok(mut guard) = LAST_STATUS.lock() {
        *guard = status;
    }
}

fn get_last_seen_mtime() -> Option<SystemTime> {
    LAST_SEEN_MTIME.lock().ok().and_then(|guard| *guard)
}

fn set_last_seen_mtime(mtime: Option<SystemTime>) {
    if let Ok(mut guard) = LAST_SEEN_MTIME.lock() {
        *guard = mtime;
    }
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

/// 设备/平台/画像/额度等元数据不会出现在 `storage.json` 的认证键里，因此
/// `sync_account_tokens_from_storage_path`（底层 `apply_payload`）会把账号库里的
/// 这些字段一并覆盖为空。监测器必须在回写前恢复它们，否则账号将丢失设备密钥与
/// 平台标识而无法再次切换。
///
/// 只恢复「非 token 类」元数据；`access_token`/`refresh_token`/`token_type`/
/// `expires_at`/`status`/`status_reason`/`email`/`user_id` 保持同步后的新值。
fn preserve_account_metadata(before: &TraeAccount, after: &mut TraeAccount) {
    if after
        .checkin_device_id
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        after.checkin_device_id = before.checkin_device_id.clone();
    }
    if after
        .machine_id
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        after.machine_id = before.machine_id.clone();
    }
    if after
        .auth_device_id
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        after.auth_device_id = before.auth_device_id.clone();
    }

    if let (Some(before_auth), Some(after_auth)) =
        (before.trae_auth_raw.as_ref(), after.trae_auth_raw.as_mut())
    {
        merge_preserved_auth_keys(
            before_auth,
            after_auth,
            &["platformId", "platform", "platform_id"],
        );
    }

    after.trae_server_raw = before.trae_server_raw.clone();
    after.trae_entitlement_raw = before.trae_entitlement_raw.clone();
    after.trae_usage_raw = before.trae_usage_raw.clone();
    after.trae_profile_raw = before.trae_profile_raw.clone();
    after.trae_usertag_raw = before.trae_usertag_raw.clone();
    after.plan_type = before.plan_type.clone();
    after.plan_reset_at = before.plan_reset_at;
    after.nickname = before.nickname.clone();
}

fn merge_preserved_auth_keys(
    before_auth: &serde_json::Value,
    after_auth: &mut serde_json::Value,
    keys: &[&str],
) {
    let Some(before_obj) = before_auth.as_object() else {
        return;
    };
    let Some(after_obj) = after_auth.as_object_mut() else {
        return;
    };
    for key in keys {
        if let Some(value) = before_obj.get(*key) {
            if !after_obj.contains_key(*key) {
                after_obj.insert((*key).to_string(), value.clone());
            }
        }
    }
}

// ===== 退避表 helper（与 provider_token_keeper 签名保持一致） =====

fn allow_attempt(key: &str) -> bool {
    let now = now_ts();
    let Ok(state) = NEXT_ALLOWED_ATTEMPT_AT.lock() else {
        return true;
    };
    state.get(key).map(|next| *next <= now).unwrap_or(true)
}

fn clear_attempt_backoff(key: &str) {
    if let Ok(mut state) = NEXT_ALLOWED_ATTEMPT_AT.lock() {
        state.remove(key);
    }
}

fn mark_attempt_failure_with_backoff(key: &str, backoff_seconds: i64) {
    if let Ok(mut state) = NEXT_ALLOWED_ATTEMPT_AT.lock() {
        state.insert(key.to_string(), now_ts() + backoff_seconds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::trae::TraeImportPayload;
    use crate::models::work_cn::{WorkCnGitHubConfig, WorkCnGitHubSlot};
    use crate::modules::trae_account::import_work_cn_account_from_payload;
    use crate::modules::work_cn_github::{save_github_config, FakeGitHubRunner};
    use base64::Engine as _;
    use std::path::{Path, PathBuf};

    fn make_payload(uid: &str, token: &str) -> TraeImportPayload {
        TraeImportPayload {
            email: format!("{uid}@example.com"),
            user_id: Some(uid.to_string()),
            nickname: None,
            access_token: token.to_string(),
            refresh_token: Some(format!("refresh-{token}")),
            token_type: Some("Bearer".to_string()),
            expires_at: None,
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: Some(serde_json::json!({
                "platformId": "trae_solo_cn",
                "deviceInfo": {"DeviceID": "1132918838145530"},
                "deviceKeyPair": {
                    "privateKeyPEM": format!("priv-{token}"),
                    "publicKeyPEM": format!("pub-{token}")
                }
            })),
            trae_profile_raw: None,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: None,
            checkin_device_id: Some("d6b8ac2e-f4d1-496d-a9a6-c9c7b4bd23e3".to_string()),
            machine_id: Some("machine-hash".to_string()),
            auth_device_id: Some("1132918838145530".to_string()),
            status: None,
            status_reason: None,
        }
    }

    fn jwt_with_exp(exp: i64) -> String {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!("{{\"exp\":{exp}}}").as_bytes());
        format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig")
    }

    fn write_storage(path: &Path, uid: &str, token: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create storage parent");
        }
        let content = serde_json::json!({
            "iCubeAuthInfo://icube.cloudide": {
                "userId": uid,
                "accessToken": token,
                "email": format!("{uid}@example.com"),
                "refreshToken": format!("refresh-{token}")
            }
        });
        std::fs::write(
            path,
            serde_json::to_string(&content).expect("serialize storage"),
        )
        .expect("write storage");
    }

    fn write_storage_with_device(path: &Path, uid: &str, token: &str, device_id: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create storage parent");
        }
        let mut content = serde_json::json!({
            "iCubeAuthInfo://icube.cloudide": {
                "userId": uid,
                "accessToken": token,
                "email": format!("{uid}@example.com"),
                "refreshToken": format!("refresh-{token}")
            },
            "telemetry.devDeviceId": "new-checkin-device",
            "telemetry.machineId": "new-machine-id"
        });
        content.as_object_mut().expect("storage object").insert(
            format!("iCubeAuthInfo://icube-dc:{device_id}"),
            serde_json::json!({
                "deviceKeyPair": {
                    "privateKeyPEM": "new-private-key",
                    "publicKeyPEM": "new-public-key"
                }
            }),
        );
        std::fs::write(
            path,
            serde_json::to_string(&content).expect("serialize storage"),
        )
        .expect("write storage");
    }

    fn write_invalid_storage(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create storage parent");
        }
        std::fs::write(path, "not-valid-json{{{").expect("write invalid storage");
    }

    fn unique_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{}-{}",
            prefix,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn reset_runtime() {
        *super::LAST_SEEN_MTIME.lock().unwrap() = None;
        *super::LAST_STATUS.lock().unwrap() = WorkCnSessionWatchStatus::default();
        super::NEXT_ALLOWED_ATTEMPT_AT.lock().unwrap().clear();
    }

    fn bind_github_for(account_id: &str) {
        save_github_config(&WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![WorkCnGitHubSlot {
                checkin_enabled: true,
                slot: 1,
                account_id: account_id.to_string(),
                token_secret: String::new(),
                device_secret: String::new(),
            }],
            workflow_file: crate::models::work_cn::default_workflow_file(),
        })
        .expect("save github config");
    }

    #[test]
    fn work_cn_watch_once_no_storage_returns_no_storage() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-nostorage");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let storage_dir = unique_dir("work-cn-watch-nostorage-storage");
        let missing = storage_dir.join("storage.json");
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &missing);

        let status = watch_once(&FakeGitHubRunner::new());
        assert_eq!(status.outcome, WorkCnSessionWatchOutcome::NoStorage);

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_unchanged_mtime_skips_decrypt() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-unchanged");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let storage_dir = unique_dir("work-cn-watch-unchanged-storage");
        let storage_path = storage_dir.join("storage.json");
        write_storage(&storage_path, "uid-x", "tok-x");
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        // 首次建立基线（无账号 → NoMatch），第二次 mtime 未变 → Unchanged。
        let first = watch_once(&FakeGitHubRunner::new());
        assert_eq!(first.outcome, WorkCnSessionWatchOutcome::NoMatch);

        let second = watch_once(&FakeGitHubRunner::new());
        assert_eq!(second.outcome, WorkCnSessionWatchOutcome::Unchanged);

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_switch_busy_when_lock_held() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-busy");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let storage_dir = unique_dir("work-cn-watch-busy-storage");
        let storage_path = storage_dir.join("storage.json");
        write_storage(&storage_path, "uid-x", "tok-x");
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        let _held = trae_account::try_lock_work_cn_switch().expect("acquire switch lock");
        let status = watch_once(&FakeGitHubRunner::new());
        assert_eq!(status.outcome, WorkCnSessionWatchOutcome::SwitchBusy);

        drop(_held);
        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_no_match_when_uid_unknown() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-nomatch");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let storage_dir = unique_dir("work-cn-watch-nomatch-storage");
        let storage_path = storage_dir.join("storage.json");
        write_storage(&storage_path, "uid-not-imported", "tok-x");
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        let status = watch_once(&FakeGitHubRunner::new());
        assert_eq!(status.outcome, WorkCnSessionWatchOutcome::NoMatch);

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_token_updated_syncs_github() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-updated");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let account = import_work_cn_account_from_payload(make_payload("uid-t", "old-token"), None)
            .expect("import account");
        let account_id = account.id.clone();
        bind_github_for(&account_id);

        let storage_dir = unique_dir("work-cn-watch-updated-storage");
        let storage_path = storage_dir.join("storage.json");
        let new_token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let new_device_id = "2232918838145530";
        write_storage_with_device(&storage_path, "uid-t", &new_token, new_device_id);
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        let runner = FakeGitHubRunner::new();
        let status = watch_once(&runner);
        assert_eq!(status.outcome, WorkCnSessionWatchOutcome::TokenUpdated);
        assert!(status.token_changed);
        assert!(status.github_synced, "应同步 GitHub 成功");
        assert!(!status.github_skipped);

        let reloaded = trae_account::load_account(&account_id).expect("reload account");
        assert_eq!(
            reloaded.access_token, new_token,
            "账号库 access_token 应更新"
        );
        // 官方客户端重新注册设备后，新快照必须优先，不能恢复成旧设备身份。
        assert_eq!(reloaded.auth_device_id.as_deref(), Some(new_device_id));
        assert_eq!(
            reloaded.checkin_device_id.as_deref(),
            Some("new-checkin-device")
        );
        assert_eq!(reloaded.machine_id.as_deref(), Some("new-machine-id"));
        let device_info_id = reloaded
            .trae_auth_raw
            .as_ref()
            .and_then(|raw| raw.pointer("/deviceInfo/DeviceID"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(device_info_id, Some(new_device_id));
        assert!(
            runner
                .recorded_calls()
                .iter()
                .any(|call| call.stdin.as_deref() == Some(new_device_id)),
            "GitHub 应收到新数字设备 ID"
        );
        assert!(
            trae_account::validate_work_cn_account_for_switch(&reloaded).is_ok(),
            "token 同步后账号仍需具备再次切换的完整快照"
        );

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_token_unchanged_no_github() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-nochange");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let account = import_work_cn_account_from_payload(make_payload("uid-t", "old-token"), None)
            .expect("import account");
        let account_id = account.id.clone();
        bind_github_for(&account_id);

        let storage_dir = unique_dir("work-cn-watch-nochange-storage");
        let storage_path = storage_dir.join("storage.json");
        write_storage(&storage_path, "uid-t", "old-token");
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        let runner = FakeGitHubRunner::new();
        let status = watch_once(&runner);
        assert_eq!(status.outcome, WorkCnSessionWatchOutcome::NoChange);
        assert!(!status.github_synced);
        assert!(runner.recorded_calls().is_empty(), "token 未变不应调用 gh");

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_device_snapshot_change_does_not_report_token_rotation() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-device-only");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let account = import_work_cn_account_from_payload(make_payload("uid-t", "old-token"), None)
            .expect("import account");
        let account_id = account.id.clone();
        bind_github_for(&account_id);

        let storage_dir = unique_dir("work-cn-watch-device-only-storage");
        let storage_path = storage_dir.join("storage.json");
        let new_device_id = "2232918838145530";
        write_storage_with_device(&storage_path, "uid-t", "old-token", new_device_id);
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        let runner = FakeGitHubRunner::new();
        let status = watch_once(&runner);
        assert_eq!(status.outcome, WorkCnSessionWatchOutcome::NoChange);
        assert!(!status.token_changed, "设备快照变化不能伪装成 Token 轮换");
        assert!(!status.github_synced);
        assert!(
            runner.recorded_calls().is_empty(),
            "Token 未轮换时不应调用 gh"
        );

        let reloaded = trae_account::load_account(&account_id).expect("reload account");
        assert_eq!(reloaded.auth_device_id.as_deref(), Some(new_device_id));

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_never_claims_checkin() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-noclaim");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let account = import_work_cn_account_from_payload(make_payload("uid-t", "old-token"), None)
            .expect("import account");
        bind_github_for(&account.id);

        let storage_dir = unique_dir("work-cn-watch-noclaim-storage");
        let storage_path = storage_dir.join("storage.json");
        let new_token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        write_storage(&storage_path, "uid-t", &new_token);
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        let runner = FakeGitHubRunner::new();
        let _ = watch_once(&runner);

        for call in runner.recorded_calls() {
            let first = call.args.first().map(|s| s.as_str()).unwrap_or("");
            assert!(
                !matches!(first, "run" | "workflow" | "claim" | "check-in"),
                "绝不允许触发签到/计费相关命令，实际首参: {first}"
            );
        }

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_failure_marks_backoff() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-fail");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let storage_dir = unique_dir("work-cn-watch-fail-storage");
        let storage_path = storage_dir.join("storage.json");
        write_invalid_storage(&storage_path);
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        let first = watch_once(&FakeGitHubRunner::new());
        assert_eq!(first.outcome, WorkCnSessionWatchOutcome::Failed);
        assert!(
            !super::allow_attempt(SESSION_BACKOFF_KEY),
            "失败后应进入会话退避"
        );

        // 退避期内再次 watch_once 直接返回 Failed，不重复解密。
        let second = watch_once(&FakeGitHubRunner::new());
        assert_eq!(second.outcome, WorkCnSessionWatchOutcome::Failed);

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }

    #[test]
    fn work_cn_watch_once_github_failure_marks_backoff() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        reset_runtime();

        let data_dir = unique_dir("work-cn-watch-ghfail");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &data_dir);

        let account = import_work_cn_account_from_payload(make_payload("uid-t", "old-token"), None)
            .expect("import account");
        let account_id = account.id.clone();
        bind_github_for(&account_id);

        let storage_dir = unique_dir("work-cn-watch-ghfail-storage");
        let storage_path = storage_dir.join("storage.json");
        let new_token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        write_storage(&storage_path, "uid-t", &new_token);
        std::env::set_var("WORK_CN_SWITCH_STORAGE_OVERRIDE", &storage_path);

        let runner = FakeGitHubRunner {
            auth_ok: false,
            ..FakeGitHubRunner::new()
        };
        let status = watch_once(&runner);
        assert_eq!(status.outcome, WorkCnSessionWatchOutcome::TokenUpdated);
        assert!(status.token_changed);
        assert!(status.github_error.is_some(), "GitHub 失败应带 error");
        assert!(
            !super::allow_attempt(&super::github_backoff_key(&account_id)),
            "GitHub 失败后应进入退避"
        );

        std::env::remove_var("WORK_CN_SWITCH_STORAGE_OVERRIDE");
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&data_dir);
        let _ = std::fs::remove_dir_all(&storage_dir);
    }
}

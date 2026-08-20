use crate::models::workbuddy::{WorkBuddySessionWatchOutcome, WorkBuddySessionWatchStatus};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

const WATCH_INTERVAL: Duration = Duration::from_secs(60);
const STARTUP_DELAY: Duration = Duration::from_secs(10);
const GITHUB_SYNC_DEBOUNCE: Duration = Duration::from_secs(20);

static STARTED: AtomicBool = AtomicBool::new(false);
static LAST_MTIME: LazyLock<Mutex<Option<SystemTime>>> = LazyLock::new(|| Mutex::new(None));
static LAST_DIGEST: LazyLock<Mutex<Option<String>>> = LazyLock::new(|| Mutex::new(None));
static LAST_STATUS: LazyLock<Mutex<WorkBuddySessionWatchStatus>> =
    LazyLock::new(|| Mutex::new(WorkBuddySessionWatchStatus::default()));

pub fn ensure_started() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::modules::logger::log_info("[WorkBuddySessionWatcher] 后台认证监测已启动");
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            let status = tauri::async_runtime::spawn_blocking(watch_once)
                .await
                .unwrap_or_else(|error| failed_status(format!("后台监测任务异常: {error}")));
            if let Ok(mut current) = LAST_STATUS.lock() {
                *current = status;
            }
            if let Some(app) = crate::get_app_handle() {
                use tauri::Emitter;
                let _ = app.emit(
                    "workbuddy:session-watch",
                    get_workbuddy_session_watch_status(),
                );
            }
            tokio::time::sleep(WATCH_INTERVAL).await;
        }
    });
}

pub fn get_workbuddy_session_watch_status() -> WorkBuddySessionWatchStatus {
    let mut status = LAST_STATUS
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default();
    status.running = STARTED.load(Ordering::SeqCst);
    status
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn failed_status(message: String) -> WorkBuddySessionWatchStatus {
    WorkBuddySessionWatchStatus {
        running: STARTED.load(Ordering::SeqCst),
        last_check_at: now(),
        outcome: WorkBuddySessionWatchOutcome::Failed,
        account_id: None,
        token_changed: false,
        github_synced: false,
        github_error: None,
        message,
    }
}

fn snapshot_digest(raw: &str) -> String {
    format!("{:x}", Sha256::digest(raw.as_bytes()))
}

fn token_update_message(checkin_enabled: bool) -> &'static str {
    if checkin_enabled {
        "检测到令牌更新，已更新本地快照，GitHub 同步已排队"
    } else {
        "检测到令牌更新，已更新本地快照；该账号未参与自动签到，未同步 GitHub"
    }
}

fn apply_github_sync_result(status: &mut WorkBuddySessionWatchStatus, result: &Result<(), String>) {
    match result {
        Ok(()) => {
            status.github_synced = true;
            status.github_error = None;
            status.message = "令牌更新已同步到 GitHub".to_string();
        }
        Err(error) => {
            let redacted = crate::modules::work_cn_github::redact_for_log(error);
            status.github_synced = false;
            status.github_error = Some(redacted);
            status.message = "令牌已更新，但 GitHub 同步失败".to_string();
        }
    }
}

pub(crate) fn record_github_sync_result(result: &Result<(), String>, update_status: bool) {
    if update_status {
        if let Ok(mut status) = LAST_STATUS.lock() {
            apply_github_sync_result(&mut status, result);
        }
    }
    if let Some(app) = crate::get_app_handle() {
        use tauri::Emitter;
        let _ = app.emit(
            "workbuddy:session-watch",
            get_workbuddy_session_watch_status(),
        );
    }
}

pub fn watch_once() -> WorkBuddySessionWatchStatus {
    let mut status = WorkBuddySessionWatchStatus {
        running: STARTED.load(Ordering::SeqCst),
        last_check_at: now(),
        ..WorkBuddySessionWatchStatus::default()
    };
    let path = match crate::modules::workbuddy_account::default_auth_file_path() {
        Ok(path) => path,
        Err(_) => return failed_status("无法定位 WorkBuddy 认证文件路径".to_string()),
    };
    let modified = match std::fs::metadata(&path).and_then(|metadata| metadata.modified()) {
        Ok(value) => value,
        Err(_) => {
            if let Ok(mut value) = LAST_MTIME.lock() {
                *value = None;
            }
            status.outcome = WorkBuddySessionWatchOutcome::NoAuthFile;
            status.message = "未检测到已登录的 WorkBuddy 会话".to_string();
            return status;
        }
    };
    let prior_mtime = LAST_MTIME.lock().ok().and_then(|value| *value);
    if prior_mtime == Some(modified) {
        status.outcome = WorkBuddySessionWatchOutcome::Unchanged;
        status.message = "认证文件未变化".to_string();
        return status;
    }
    let raw = match crate::modules::workbuddy_account::read_stable_auth_file(&path) {
        Ok(raw) => raw,
        Err(error) => return failed_status(format!("认证文件尚未稳定或无效: {error}")),
    };
    let digest = snapshot_digest(&raw);
    if LAST_DIGEST
        .lock()
        .ok()
        .and_then(|value| value.clone())
        .as_deref()
        == Some(digest.as_str())
    {
        if let Ok(mut value) = LAST_MTIME.lock() {
            *value = Some(modified);
        }
        status.outcome = WorkBuddySessionWatchOutcome::Unchanged;
        status.message = "认证内容未变化".to_string();
        return status;
    }
    let uid = match crate::modules::workbuddy_account::snapshot_uid(&raw) {
        Ok(uid) => uid,
        Err(error) => return failed_status(format!("认证账号无效: {error}")),
    };
    let account_id = crate::modules::workbuddy_account::stable_account_id(&uid);
    if !crate::modules::workbuddy_account::has_account_id(&account_id).unwrap_or(false) {
        if let Ok(mut value) = LAST_MTIME.lock() {
            *value = Some(modified);
        }
        if let Ok(mut value) = LAST_DIGEST.lock() {
            *value = Some(digest);
        }
        status.outcome = WorkBuddySessionWatchOutcome::NoMatch;
        status.message = "当前 WorkBuddy 账号未导入".to_string();
        return status;
    }
    let previous_token = crate::modules::workbuddy_account::access_token(&account_id).ok();
    if let Err(error) = crate::modules::workbuddy_account::import_snapshot_json(&raw, None) {
        return failed_status(format!("更新本地加密快照失败: {error}"));
    }
    if let Ok(mut value) = LAST_MTIME.lock() {
        *value = Some(modified);
    }
    if let Ok(mut value) = LAST_DIGEST.lock() {
        *value = Some(digest);
    }
    status.account_id = Some(account_id.clone());
    let token_changed = previous_token.as_deref()
        != crate::modules::workbuddy_account::access_token(&account_id)
            .ok()
            .as_deref();
    status.token_changed = token_changed;
    if !token_changed {
        status.outcome = WorkBuddySessionWatchOutcome::SnapshotUpdated;
        status.message = "已更新 WorkBuddy 加密快照".to_string();
        return status;
    }
    let enabled = crate::modules::workbuddy_account::list_workbuddy_accounts()
        .ok()
        .and_then(|accounts| {
            accounts
                .into_iter()
                .find(|account| account.id == account_id)
        })
        .map(|account| account.checkin_enabled)
        .unwrap_or(false);
    status.outcome = WorkBuddySessionWatchOutcome::TokenUpdated;
    if !enabled {
        status.message = token_update_message(false).to_string();
        return status;
    }
    crate::modules::workbuddy_github::queue_background_sync(
        "session watcher",
        GITHUB_SYNC_DEBOUNCE,
        true,
    );
    status.message = token_update_message(true).to_string();
    status
}

#[cfg(test)]
mod tests {
    use super::{apply_github_sync_result, snapshot_digest, token_update_message};
    use crate::models::workbuddy::WorkBuddySessionWatchStatus;

    #[test]
    fn snapshot_digest_changes_without_exposing_snapshot() {
        assert_ne!(snapshot_digest("first"), snapshot_digest("second"));
        assert_eq!(snapshot_digest("first").len(), 64);
    }

    #[test]
    fn disabled_checkin_token_update_does_not_claim_github_will_sync() {
        let message = token_update_message(false);
        assert!(message.contains("未参与自动签到"));
        assert!(message.contains("未同步 GitHub"));
        assert!(!message.contains("稍后执行"));
    }

    #[test]
    fn github_sync_result_updates_background_status_without_exposing_token() {
        let mut status = WorkBuddySessionWatchStatus::default();
        apply_github_sync_result(&mut status, &Ok(()));
        assert!(status.github_synced);
        assert!(status.github_error.is_none());
        assert!(status.message.contains("已同步"));

        apply_github_sync_result(
            &mut status,
            &Err("request failed with token secret-workbuddy-token".to_string()),
        );
        assert!(!status.github_synced);
        assert!(!status.message.contains("secret-workbuddy-token"));
        assert!(!status
            .github_error
            .as_deref()
            .unwrap_or_default()
            .contains("secret-workbuddy-token"));
    }
}

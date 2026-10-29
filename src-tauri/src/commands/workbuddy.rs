use crate::models::workbuddy::{
    command_error_to_string, WorkBuddyAccountStatus, WorkBuddyAccountUpdate, WorkBuddyAccountView,
    WorkBuddyCommandError, WorkBuddyErrorCode, WorkBuddyInstallation, WorkBuddySwitchResult,
};
use crate::modules::{process, workbuddy_account, workbuddy_github, workbuddy_status};
use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard};

static WORKBUDDY_SWITCH_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn try_lock_workbuddy_switch() -> Result<MutexGuard<'static, ()>, WorkBuddyCommandError> {
    WORKBUDDY_SWITCH_LOCK.try_lock().map_err(|_| {
        WorkBuddyCommandError::new(WorkBuddyErrorCode::Busy, "WorkBuddy 正在进行另一个账号切换")
    })
}

fn queue_workbuddy_github_sync(reason: &'static str) {
    workbuddy_github::queue_background_sync(reason, std::time::Duration::ZERO, false);
}

/// 运行状态后台探测完成事件（payload: bool，是否检测到运行中的客户端）。
/// 进程探测（PowerShell，冷启动可达 5 秒）不阻塞安装检测结果，探测完成后
/// 通过该事件补发，前端合并进 installation.running。
pub const WORKBUDDY_INSTALLATION_RUNNING_EVENT: &str = "workbuddy:installation-running";

#[tauri::command]
pub async fn get_workbuddy_installation(
    app: tauri::AppHandle,
) -> Result<WorkBuddyInstallation, String> {
    // 关键路径与 TRAE detect_installation 对齐：只做文件系统/注册表检测，
    // 毫秒级返回；running 先给 false 占位，由后台探测补齐。
    let installation = tauri::async_runtime::spawn_blocking(|| {
        let executable_path = process::resolve_workbuddy_launch_path().ok();
        let auth_file_path = workbuddy_account::default_auth_file_path()?;
        Ok(WorkBuddyInstallation {
            installed: executable_path.is_some(),
            executable_path: executable_path.map(|path| path.to_string_lossy().to_string()),
            auth_file_path: auth_file_path.to_string_lossy().to_string(),
            running: false,
            current_account_id: workbuddy_account::current_managed_account_id(),
            github_cleanup_pending: workbuddy_github::github_cleanup_pending(),
        })
    })
    .await
    .map_err(|error| format!("检测 WorkBuddy 安装状态任务失败: {error}"))?;

    tauri::async_runtime::spawn(async move {
        let running = tauri::async_runtime::spawn_blocking(|| {
            !process::collect_workbuddy_process_entries().is_empty()
        })
        .await
        .unwrap_or(false);
        use tauri::Emitter;
        let _ = app.emit(WORKBUDDY_INSTALLATION_RUNNING_EVENT, running);
    });

    installation
}

#[tauri::command]
pub fn get_workbuddy_settings() -> crate::models::workbuddy::WorkBuddySettings {
    crate::modules::workbuddy_settings::load_workbuddy_settings()
}

#[tauri::command]
pub fn save_workbuddy_settings(
    settings: crate::models::workbuddy::WorkBuddySettings,
) -> Result<crate::models::workbuddy::WorkBuddySettings, String> {
    crate::modules::workbuddy_settings::save_workbuddy_settings(settings)
}

#[tauri::command]
pub fn get_workbuddy_session_watch_status() -> crate::models::workbuddy::WorkBuddySessionWatchStatus
{
    crate::modules::workbuddy_session_watcher::get_workbuddy_session_watch_status()
}

#[tauri::command]
pub fn start_workbuddy_session_watcher() -> crate::models::workbuddy::WorkBuddySessionWatchStatus {
    crate::modules::workbuddy_session_watcher::ensure_started();
    crate::modules::workbuddy_session_watcher::get_workbuddy_session_watch_status()
}

#[tauri::command]
pub async fn import_current_workbuddy_account(
    display_name: Option<String>,
) -> Result<WorkBuddyAccountView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let account = workbuddy_account::import_current_workbuddy_account(display_name)?;
        // GitHub CLI 可能联网阻塞，不能让导入按钮等待远端同步完成。
        queue_workbuddy_github_sync("import");
        Ok(account)
    })
    .await
    .map_err(|error| format!("导入 WorkBuddy 账号任务失败: {error}"))?
}

/// 账号列表涉及逐账号解密落盘文件，账号多时较重；在阻塞线程池执行，
/// 避免同步命令占用主线程冻结窗口（与 work_cn 命令约定一致）。
#[tauri::command]
pub async fn list_workbuddy_accounts() -> Result<Vec<WorkBuddyAccountView>, String> {
    tauri::async_runtime::spawn_blocking(workbuddy_account::list_workbuddy_accounts)
        .await
        .map_err(|error| format!("加载 WorkBuddy 账号列表任务失败: {error}"))?
}

#[tauri::command]
pub async fn get_workbuddy_account_status(
    account_id: String,
) -> Result<WorkBuddyAccountStatus, String> {
    workbuddy_status::query_account_status(&account_id).await
}

/// Explicit credential recovery; balance reads never invoke this command.
#[tauri::command]
pub async fn recover_workbuddy_auth(account_id: String) -> Result<WorkBuddyAccountView, String> {
    workbuddy_account::refresh_account_auth(&account_id).await?;
    tauri::async_runtime::spawn_blocking(move || {
        workbuddy_account::list_workbuddy_accounts()?
            .into_iter()
            .find(|a| a.id == account_id)
            .ok_or_else(|| "账号已不存在".to_string())
    })
    .await
    .map_err(|_| "认证恢复结果读取失败".to_string())?
}

#[tauri::command]
pub async fn update_workbuddy_account(
    account_id: String,
    update: WorkBuddyAccountUpdate,
) -> Result<WorkBuddyAccountView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        workbuddy_account::update_workbuddy_account(&account_id, update)?;
        let account = workbuddy_account::list_workbuddy_accounts()?
            .into_iter()
            .find(|account| account.id == account_id)
            .ok_or_else(|| "更新后的 WorkBuddy 账号不存在".to_string())?;
        queue_workbuddy_github_sync("update");
        Ok(account)
    })
    .await
    .map_err(|error| format!("更新 WorkBuddy 账号任务失败: {error}"))?
}

#[tauri::command]
pub async fn delete_workbuddy_account(account_id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let previous_cleanup_pending = workbuddy_github::github_cleanup_pending();
        workbuddy_github::mark_dataset_changed()?;
        if let Err(error) = workbuddy_account::remove_workbuddy_account(&account_id) {
            let _ = workbuddy_github::set_github_cleanup_pending(previous_cleanup_pending);
            return Err(error);
        }
        // 本地删除不因远端故障回滚；后台同步负责清理远端集合。
        queue_workbuddy_github_sync("delete");
        Ok(true)
    })
    .await
    .map_err(|error| format!("删除 WorkBuddy 账号任务失败: {error}"))?
}

#[tauri::command]
pub async fn switch_workbuddy_account(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<WorkBuddySwitchResult, String> {
    use tauri::Emitter;

    let progress_account_id = account_id.clone();
    let on_progress = move |stage: &str| {
        let _ = app.emit(
            WORKBUDDY_SWITCH_PROGRESS_EVENT,
            WorkBuddySwitchProgress {
                account_id: progress_account_id.clone(),
                stage: stage.to_string(),
            },
        );
    };
    tauri::async_runtime::spawn_blocking(move || {
        switch_workbuddy_account_blocking(&account_id, on_progress)
    })
    .await
    .map_err(|error| format!("切换 WorkBuddy 账号任务失败: {error}"))?
    .map_err(|error| {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Switch] failed code={:?} error={}",
            error.code,
            crate::modules::work_cn_github::redact_for_log(&error.message)
        ));
        // 认证刷新失败沿用纯文本错误（前端直接展示），
        // 其余错误保持 JSON 结构由前端解析 code/message/detail。
        if error.code == WorkBuddyErrorCode::AuthRefreshFailed {
            return error.message;
        }
        command_error_to_string(&error)
    })
}

/// 前端订阅的切号进度事件名（与 workBuddyService.ts 常量保持一致）。
pub const WORKBUDDY_SWITCH_PROGRESS_EVENT: &str = "workbuddy-switch-progress";

/// 切号进度事件载荷。stage 取值：validating / closing / refreshing /
/// injecting / launching / syncing。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkBuddySwitchProgress {
    account_id: String,
    stage: String,
}

struct ClientAuthRecovery {
    path: std::path::PathBuf,
    auth: Option<Vec<u8>>,
    marker: Option<Vec<u8>>,
    written: Option<[u8; 32]>,
}
impl ClientAuthRecovery {
    fn capture(path: &Path) -> Result<Self, String> {
        fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
            match std::fs::read(path) {
                Ok(v) => Ok(Some(v)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        }
        Ok(Self {
            path: path.to_owned(),
            auth: read(path)?,
            marker: read(&std::path::PathBuf::from(format!(
                "{}.logged-out",
                path.to_string_lossy()
            )))?,
            written: None,
        })
    }
    fn hash(&self) -> Option<[u8; 32]> {
        use sha2::{Digest, Sha256};
        self.auth.as_ref().map(|v| Sha256::digest(v).into())
    }
    fn restore(&self) -> Result<(), String> {
        if let Some(written) = self.written {
            if !crate::modules::atomic_write::replace_bytes_if_unchanged(
                &self.path,
                Some(written),
                self.auth.as_deref(),
            )? {
                return Err("回滚期间登录状态又已变化，已保留新状态".into());
            }
            let marker =
                std::path::PathBuf::from(format!("{}.logged-out", self.path.to_string_lossy()));
            if !crate::modules::atomic_write::replace_bytes_if_unchanged(
                &marker,
                None,
                self.marker.as_deref(),
            )? {
                return Err("登出标记已有新变化，未覆盖".into());
            }
        }
        Ok(())
    }
}

fn reopen_previous_if_needed(was_running: bool) -> Result<(), String> {
    if !was_running {
        return Ok(());
    }
    #[cfg(windows)]
    if !process::collect_workbuddy_process_entries_checked()?.is_empty() {
        return Ok(());
    }
    #[cfg(not(windows))]
    if !process::collect_workbuddy_process_entries().is_empty() {
        return Ok(());
    }
    process::start_workbuddy_default_with_args_with_new_window(&[], true).map(|_| ())
}

fn switch_workbuddy_account_blocking(
    account_id: &str,
    on_progress: impl Fn(&str),
) -> Result<WorkBuddySwitchResult, WorkBuddyCommandError> {
    let _switch = try_lock_workbuddy_switch()?;
    let _auth = workbuddy_account::try_begin_auth_transaction()
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::Busy, e))?;
    let tx = uuid::Uuid::new_v4().to_string();
    let on_progress = |stage: &str| {
        crate::modules::logger::log_info(&format!("[WorkBuddy Switch] tx={tx} stage={stage}"));
        on_progress(stage);
    };
    on_progress("validating");
    let initial = workbuddy_account::snapshot_json(account_id)
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::AccountNotFound, e))?;
    let expected_uid = workbuddy_account::snapshot_uid(&initial)
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::SnapshotIncomplete, e))?;
    process::resolve_workbuddy_launch_path()
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::ClientNotInstalled, e))?;
    crate::modules::workbuddy_auth_crypto::prepare()
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::InjectFailed, e))?;
    let auth_path = workbuddy_account::default_auth_file_path()
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::InjectFailed, e))?;
    let data_dir = process::get_default_workbuddy_user_data_dir()
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::ClientCloseFailed, e))?;
    #[cfg(windows)]
    let was_running = !process::collect_workbuddy_process_entries_checked()
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::ClientCloseFailed, e))?
        .is_empty();
    #[cfg(not(windows))]
    let was_running = !process::collect_workbuddy_process_entries().is_empty();
    on_progress("closing");
    process::close_workbuddy_instances(&[data_dir.to_string_lossy().into_owned()], 20)
        .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::ClientCloseFailed, e))?;
    // Last stable official state, not the pre-close state, is the recovery point.
    let current = workbuddy_account::refresh_current_managed_snapshot_in_transaction();
    let mut recovery = match ClientAuthRecovery::capture(&auth_path) {
        Ok(v) => v,
        Err(e) => {
            let _ = reopen_previous_if_needed(was_running);
            return Err(WorkBuddyCommandError::new(
                WorkBuddyErrorCode::InjectFailed,
                e,
            ));
        }
    };
    let expected_auth_file = recovery.hash();
    let mut current_refreshed = false;
    let mut current_recovery_ready = false;
    let current_id = current
        .as_ref()
        .ok()
        .and_then(|v| v.as_ref())
        .map(|v| v.id.clone());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
        || -> Result<WorkBuddySwitchResult, WorkBuddyCommandError> {
            current.map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::InjectFailed, e))?;
            on_progress("refreshing");
            tauri::async_runtime::block_on(workbuddy_account::refresh_account_auth_in_transaction(
                account_id,
            ))
            .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::AuthRefreshFailed, e))?;
            if current_id.as_deref() == Some(account_id) {
                current_refreshed = true;
                let raw = workbuddy_account::snapshot_json(account_id)
                    .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::InjectFailed, e))?;
                let snapshot = serde_json::from_str(&raw).map_err(|_| {
                    WorkBuddyCommandError::new(
                        WorkBuddyErrorCode::SnapshotIncomplete,
                        "恢复快照格式无效",
                    )
                })?;
                // The prior token may have been rotated by this transaction; recovery must use the new one.
                recovery.auth = Some(
                    workbuddy_account::encoded_snapshot_for_client(&snapshot).map_err(|e| {
                        WorkBuddyCommandError::new(WorkBuddyErrorCode::InjectFailed, e)
                    })?,
                );
                current_recovery_ready = true;
            }
            on_progress("injecting");
            // The official process must still be closed before a credential replacement.
            #[cfg(windows)]
            if !process::collect_workbuddy_process_entries_checked()
                .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::ClientCloseFailed, e))?
                .is_empty()
            {
                return Err(WorkBuddyCommandError::new(
                    WorkBuddyErrorCode::ClientCloseFailed,
                    "WorkBuddy 已被重新打开，已取消覆盖登录文件",
                ));
            }
            recovery.written = Some(
                workbuddy_account::write_account_to_default_client_checked(
                    account_id,
                    expected_auth_file,
                    recovery.auth.as_deref(),
                )
                .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::InjectFailed, e))?,
            );
            on_progress("launching");
            let pid = process::start_workbuddy_default_with_args_with_new_window(&[], true)
                .map_err(|e| WorkBuddyCommandError::new(WorkBuddyErrorCode::LaunchFailed, e))?;
            log_workbuddy_auth_checkpoint(&tx, "after_target_write", &auth_path);
            spawn_workbuddy_post_launch_auth_checks(tx.clone(), auth_path.clone());
            let _ = workbuddy_account::mark_last_used(account_id);
            queue_workbuddy_github_sync("switch");
            on_progress("syncing");
            let _ = process::activate_workbuddy_window_for_pid(pid);
            Ok(WorkBuddySwitchResult {
                transaction_id: tx.clone(),
                account_id: account_id.into(),
                verified_uid: expected_uid,
                forced_close: false,
                github_sync_pending: true,
            })
        },
    ))
    .unwrap_or_else(|_| {
        Err(WorkBuddyCommandError::new(
            WorkBuddyErrorCode::InjectFailed,
            "切换任务异常中断，正在恢复",
        ))
    });
    match result {
        Ok(value) => {
            crate::modules::logger::log_info(&format!("[WorkBuddy Switch] tx={tx} completed"));
            Ok(value)
        }
        Err(mut error) => {
            error.transaction_id = Some(tx.clone());
            if current_refreshed && !current_recovery_ready {
                error.message=format!("{}；新凭证已保存在托管账号中，但无法准备客户端恢复文件，未自动重启旧会话。请重试写回",error.message);
                return Err(error);
            }
            match recovery.restore() {
                Ok(()) => {
                    if let Err(e) = reopen_previous_if_needed(was_running) {
                        error.message =
                            format!("{}；原登录文件已保留，但重新打开失败：{}", error.message, e);
                    }
                }
                Err(e) => error.message = format!("{}；恢复未完成：{}", error.message, e),
            }
            Err(error)
        }
    }
}

fn log_workbuddy_auth_checkpoint(transaction_id: &str, stage: &str, auth_path: &Path) {
    match workbuddy_account::auth_file_diagnostic(auth_path) {
        Ok(diagnostic) => crate::modules::logger::log_info(&format!(
            "[WorkBuddy AuthTrace] tx={} stage={} {}",
            transaction_id, stage, diagnostic
        )),
        Err(error) => crate::modules::logger::log_warn(&format!(
            "[WorkBuddy AuthTrace] tx={} stage={} read_failed={}",
            transaction_id, stage, error
        )),
    }
}

fn spawn_workbuddy_post_launch_auth_checks(transaction_id: String, auth_path: std::path::PathBuf) {
    let _ = std::thread::Builder::new()
        .name("workbuddy-auth-trace".to_string())
        .spawn(move || {
            for (delay, stage) in [
                (std::time::Duration::from_secs(1), "post_launch_1s"),
                (std::time::Duration::from_secs(2), "post_launch_3s"),
                (std::time::Duration::from_secs(7), "post_launch_10s"),
            ] {
                std::thread::sleep(delay);
                log_workbuddy_auth_checkpoint(&transaction_id, stage, &auth_path);
            }
        });
}

#[tauri::command]
pub async fn sync_workbuddy_github() -> Result<(), String> {
    workbuddy_github::sync_background_now("manual sync").await
}

#[tauri::command]
pub async fn trigger_workbuddy_checkin(account_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        workbuddy_github::trigger_workbuddy_checkin(&account_id)
    })
    .await
    .map_err(|error| format!("触发 WorkBuddy 签到任务失败: {error}"))?
}

#[cfg(test)]
mod tests {
    #[test]
    fn concurrent_workbuddy_switch_is_rejected_without_waiting() {
        let _guard = super::try_lock_workbuddy_switch().expect("first switch lock");
        let error = super::try_lock_workbuddy_switch().expect_err("second switch must be busy");
        assert_eq!(
            error.code,
            crate::models::workbuddy::WorkBuddyErrorCode::Busy
        );
    }

    #[test]
    fn recovery_restores_marker_and_latest_auth_without_overwriting_external_change() {
        use sha2::{Digest, Sha256};
        let dir = std::env::temp_dir().join(format!("workbuddy-recovery-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("auth.json");
        let marker = dir.join("auth.json.logged-out");
        std::fs::write(&path, b"after-close-final").unwrap();
        std::fs::write(&marker, b"logout-marker").unwrap();
        let mut recovery = super::ClientAuthRecovery::capture(&path).unwrap();
        std::fs::write(&path, b"target").unwrap();
        std::fs::remove_file(&marker).unwrap();
        recovery.written = Some(Sha256::digest(b"target").into());
        recovery.restore().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"after-close-final");
        assert_eq!(std::fs::read(&marker).unwrap(), b"logout-marker");
        std::fs::write(&path, b"external-new-session").unwrap();
        assert!(recovery.restore().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"external-new-session");
        let _ = std::fs::remove_dir_all(dir);
    }
}

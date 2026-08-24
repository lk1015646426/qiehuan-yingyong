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
pub async fn switch_workbuddy_account(account_id: String) -> Result<WorkBuddySwitchResult, String> {
    // 目标账号可能已在 WorkBuddy 后台轮换过 access token；先用 refresh token
    // 获取最新快照，避免把旧凭证重新写回官方客户端。
    crate::modules::workbuddy_account::refresh_account_auth(&account_id)
        .await
        .map_err(|error| {
            if crate::modules::workbuddy_account::refresh_error_requires_login(&error) {
                format!("目标 WorkBuddy 账号的 refresh token 已失效，请重新登录后再导入：{error}")
            } else {
                format!("目标 WorkBuddy 账号认证刷新失败，请稍后重试：{error}")
            }
        })?;
    tauri::async_runtime::spawn_blocking(move || switch_workbuddy_account_blocking(&account_id))
        .await
        .map_err(|error| format!("切换 WorkBuddy 账号任务失败: {error}"))?
        .map_err(|error| command_error_to_string(&error))
}

fn switch_workbuddy_account_blocking(
    account_id: &str,
) -> Result<WorkBuddySwitchResult, WorkBuddyCommandError> {
    let _switch_guard = try_lock_workbuddy_switch()?;
    let transaction_id = uuid::Uuid::new_v4().to_string();

    let snapshot = workbuddy_account::snapshot_json(account_id).map_err(|_| {
        WorkBuddyCommandError::new(
            WorkBuddyErrorCode::AccountNotFound,
            "找不到目标 WorkBuddy 账号",
        )
    })?;
    let expected_uid = workbuddy_account::snapshot_uid(&snapshot).map_err(|_| {
        WorkBuddyCommandError::new(WorkBuddyErrorCode::SnapshotIncomplete, "目标账号快照不完整")
    })?;

    // 启动路径先行校验，避免关闭旧实例后才发现无法启动新实例。
    process::resolve_workbuddy_launch_path().map_err(|error| {
        WorkBuddyCommandError::new(
            WorkBuddyErrorCode::ClientNotInstalled,
            "未找到 WorkBuddy 安装路径",
        )
        .with_detail(error)
    })?;

    let user_data_dir = process::get_default_workbuddy_user_data_dir().map_err(|error| {
        WorkBuddyCommandError::new(WorkBuddyErrorCode::ClientCloseFailed, error)
    })?;
    let user_data_dir = user_data_dir.to_string_lossy().to_string();
    let auth_path = workbuddy_account::default_auth_file_path().map_err(|error| {
        WorkBuddyCommandError::new(
            WorkBuddyErrorCode::InjectFailed,
            "无法定位 WorkBuddy 认证文件",
        )
        .with_detail(error)
    })?;
    let previous_auth_exists = auth_path.exists();
    let previous_auth = match std::fs::read(&auth_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(WorkBuddyCommandError::new(
                WorkBuddyErrorCode::InjectFailed,
                "读取 WorkBuddy 原认证文件失败，未执行切换",
            )
            .with_detail(error.to_string()));
        }
    };
    log_workbuddy_auth_checkpoint(&transaction_id, "before_close", &auth_path);
    process::close_workbuddy_instances(std::slice::from_ref(&user_data_dir), 20).map_err(
        |error| {
            WorkBuddyCommandError::new(
                WorkBuddyErrorCode::ClientCloseFailed,
                "WorkBuddy 旧实例未能关闭，未修改认证文件",
            )
            .with_detail(error)
        },
    )?;

    // 客户端退出期间可能完成最后一次 token 刷新并写回认证文件。此处必须在
    // 关闭成功后马上保存当前托管账号的新快照，避免下次切回时回写旧 token。
    if let Err(error) = workbuddy_account::refresh_current_managed_snapshot() {
        restore_previous_workbuddy_auth(&auth_path, previous_auth.as_deref(), previous_auth_exists);
        return Err(WorkBuddyCommandError::new(
            WorkBuddyErrorCode::InjectFailed,
            "读取当前 WorkBuddy 登录状态失败，未执行切换",
        )
        .with_detail(error));
    }
    log_workbuddy_auth_checkpoint(&transaction_id, "after_close_refresh", &auth_path);

    if let Err(error) = workbuddy_account::write_account_to_default_client(account_id) {
        restore_previous_workbuddy_auth(&auth_path, previous_auth.as_deref(), previous_auth_exists);
        return Err(WorkBuddyCommandError::new(
            WorkBuddyErrorCode::InjectFailed,
            "写入 WorkBuddy 目标账号失败",
        )
        .with_detail(error));
    }
    log_workbuddy_auth_checkpoint(&transaction_id, "after_target_write", &auth_path);

    let pid = match process::start_workbuddy_default_with_args_with_new_window(&[], true) {
        Ok(pid) => pid,
        Err(error) => {
            restore_previous_workbuddy_auth(
                &auth_path,
                previous_auth.as_deref(),
                previous_auth_exists,
            );
            return Err(WorkBuddyCommandError::new(
                WorkBuddyErrorCode::LaunchFailed,
                "启动 WorkBuddy 失败",
            )
            .with_detail(error));
        }
    };
    spawn_workbuddy_post_launch_auth_checks(transaction_id.clone(), auth_path.clone());
    if let Err(error) = workbuddy_account::mark_last_used(account_id) {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy] 客户端已启动，但记录当前账号失败: account_id={}, error={}",
            account_id, error
        ));
    }
    queue_workbuddy_github_sync("switch");
    if let Err(error) = process::activate_workbuddy_window_for_pid(pid) {
        // 进程和认证状态已经成功切换；窗口激活失败不应再向前端报告“切换失败”，
        // 否则用户会误以为旧账号仍在使用。记录警告后交由用户手动切到已启动窗口。
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy] 账号已切换并启动，但窗口激活失败: pid={}, error={}",
            pid, error
        ));
    }
    let github_sync_pending = true;

    Ok(WorkBuddySwitchResult {
        transaction_id,
        account_id: account_id.to_string(),
        verified_uid: expected_uid,
        forced_close: false,
        github_sync_pending,
    })
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

fn restore_previous_workbuddy_auth(path: &Path, previous: Option<&[u8]>, existed: bool) {
    let result = if let Some(previous) = previous {
        crate::modules::atomic_write::write_bytes_atomic(path, previous)
    } else if !existed {
        crate::modules::atomic_write::remove_file_locked(path).map(|_| ())
    } else {
        Ok(())
    };
    if let Err(error) = result {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy] 切换失败后恢复认证文件失败: {error}"
        ));
    }
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
    fn target_auth_write_failure_branch_restores_previous_auth() {
        let source = include_str!("workbuddy.rs");
        let write_branch = source
            .split("workbuddy_account::write_account_to_default_client(account_id)")
            .nth(1)
            .expect("目标认证写入分支应存在");
        let before_launch = write_branch
            .split("start_workbuddy_default_with_args_with_new_window")
            .next()
            .expect("写入失败分支应位于启动前");

        assert!(before_launch.contains("restore_previous_workbuddy_auth"));
    }

    #[test]
    fn window_activation_failure_does_not_report_switch_failure_after_launch() {
        let source = include_str!("workbuddy.rs");
        let activation_branch = source
            .split("process::activate_workbuddy_window_for_pid(pid)")
            .nth(1)
            .expect("窗口激活分支应存在");
        let branch_body = activation_branch
            .split("let github_sync_pending")
            .next()
            .expect("窗口激活分支应在结果构造前结束");

        assert!(!branch_body.contains("return Err"));
        assert!(branch_body.contains("log_warn"));
    }
}

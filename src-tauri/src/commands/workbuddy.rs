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

#[tauri::command]
pub async fn get_workbuddy_installation() -> Result<WorkBuddyInstallation, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let executable_path = process::resolve_workbuddy_launch_path().ok();
        let auth_file_path = workbuddy_account::default_auth_file_path()?;
        Ok(WorkBuddyInstallation {
            installed: executable_path.is_some(),
            executable_path: executable_path.map(|path| path.to_string_lossy().to_string()),
            auth_file_path: auth_file_path.to_string_lossy().to_string(),
            running: !process::collect_workbuddy_process_entries().is_empty(),
            current_account_id: workbuddy_account::current_managed_account_id(),
            github_cleanup_pending: workbuddy_github::github_cleanup_pending(),
        })
    })
    .await
    .map_err(|error| format!("检测 WorkBuddy 安装状态任务失败: {error}"))?
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

#[tauri::command]
pub fn list_workbuddy_accounts() -> Result<Vec<WorkBuddyAccountView>, String> {
    workbuddy_account::list_workbuddy_accounts()
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
    tauri::async_runtime::spawn_blocking(move || switch_workbuddy_account_blocking(&account_id))
        .await
        .map_err(|error| format!("切换 WorkBuddy 账号任务失败: {error}"))?
        .map_err(|error| command_error_to_string(&error))
}

fn switch_workbuddy_account_blocking(
    account_id: &str,
) -> Result<WorkBuddySwitchResult, WorkBuddyCommandError> {
    let _switch_guard = try_lock_workbuddy_switch()?;

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
    workbuddy_account::refresh_current_managed_snapshot().map_err(|error| {
        WorkBuddyCommandError::new(
            WorkBuddyErrorCode::InjectFailed,
            "读取当前 WorkBuddy 登录状态失败，未执行切换",
        )
        .with_detail(error)
    })?;

    workbuddy_account::write_account_to_default_client(account_id).map_err(|error| {
        WorkBuddyCommandError::new(
            WorkBuddyErrorCode::InjectFailed,
            "写入 WorkBuddy 目标账号失败",
        )
        .with_detail(error)
    })?;

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
    workbuddy_account::mark_last_used(account_id).map_err(|error| {
        WorkBuddyCommandError::new(
            WorkBuddyErrorCode::InjectFailed,
            "WorkBuddy 已启动，但更新当前账号状态失败",
        )
        .with_detail(error)
    })?;
    queue_workbuddy_github_sync("switch");
    if let Err(error) = process::activate_workbuddy_window_for_pid(pid) {
        return Err(WorkBuddyCommandError::new(
            WorkBuddyErrorCode::WindowActivationFailed,
            "账号已切换并启动，但 WorkBuddy 窗口激活失败",
        )
        .with_detail(format!("pid={pid}; {error}")));
    }
    let github_sync_pending = true;

    Ok(WorkBuddySwitchResult {
        transaction_id: uuid::Uuid::new_v4().to_string(),
        account_id: account_id.to_string(),
        verified_uid: expected_uid,
        forced_close: false,
        github_sync_pending,
    })
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
}

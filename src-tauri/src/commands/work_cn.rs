use std::path::Path;

use crate::models::work_cn::{
    command_error_to_string, WorkCnAccountView, WorkCnCreditsSummary, WorkCnInstallation,
    WorkCnSessionWatchStatus, WorkCnSnapshotValidation, WorkCnSwitchResult,
};
use crate::modules::{logger, process, trae_account};

/// User-visible product name. The internal platform identifier stays
/// `trae_solo_cn` / `TraePlatformKind::TraeSoloCn`; only the display layer is
/// renamed (development guide §0 rule 5 / §9.4).
const WORK_CN_DISPLAY_NAME: &str = "TRAE Work CN";

/// Detect the local TRAE Work CN installation without touching any login
/// secrets. Safe to call on every page load.
#[tauri::command]
pub fn get_work_cn_installation() -> Result<WorkCnInstallation, String> {
    let platform = trae_account::TraePlatformKind::TraeSoloCn;

    let executable_path = process::resolve_trae_launch_path_for_platform(platform).ok();
    let version = executable_path
        .as_ref()
        .and_then(|path| trae_account::detect_trae_product_version_for_exe(path));

    let user_data_dir = trae_account::resolve_trae_data_dir_for_platform(platform).ok();
    let storage_path = user_data_dir
        .as_ref()
        .map(|dir| build_storage_path(dir).to_string_lossy().to_string());

    let legacy_path = user_data_dir
        .as_ref()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .map(|name| name == "TRAE SOLO CN")
        .unwrap_or(false);

    let installed = executable_path.is_some();
    let display_name = if installed {
        Some(WORK_CN_DISPLAY_NAME.to_string())
    } else {
        None
    };

    logger::log_info(&format!(
        "[Work CN] installation detected: installed={}, exe={}, version={}, data_dir={}, legacy_path={}",
        installed,
        executable_path
            .as_deref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default(),
        version.as_deref().unwrap_or_default(),
        user_data_dir
            .as_deref()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_default(),
        legacy_path,
    ));

    Ok(WorkCnInstallation {
        installed,
        executable_path: executable_path
            .map(|path| path.to_string_lossy().to_string()),
        user_data_dir: user_data_dir.map(|path| path.to_string_lossy().to_string()),
        storage_path,
        display_name,
        version,
        legacy_path,
    })
}

fn build_storage_path(user_data_dir: &Path) -> std::path::PathBuf {
    user_data_dir
        .join("User")
        .join("globalStorage")
        .join("storage.json")
}

/// Import the currently logged-in TRAE Work CN account as a full snapshot
/// (tokens + device keys + ids). `label` is stored as a tag; email is never
/// overwritten. Returns a desensitized view.
#[tauri::command]
pub fn import_current_work_cn_account(
    _app: tauri::AppHandle,
    label: Option<String>,
) -> Result<WorkCnAccountView, String> {
    trae_account::import_current_work_cn_account(label)
}

/// List previously imported Work CN accounts as desensitized views.
#[tauri::command]
pub fn list_work_cn_accounts() -> Result<Vec<WorkCnAccountView>, String> {
    trae_account::list_work_cn_accounts()
}

/// Validate the completeness of a saved Work CN account snapshot.
#[tauri::command]
pub fn validate_work_cn_account(account_id: String) -> Result<WorkCnSnapshotValidation, String> {
    let Some(account) = trae_account::load_account(&account_id) else {
        return Err("账号不存在".to_string());
    };
    Ok(trae_account::validate_work_cn_account_snapshot(&account))
}

/// One-click switch to a saved Work CN account and open the official client.
/// Orchestrates the full transactional state machine (close → inject → bind →
/// launch → verify → rollback on failure). Errors are returned as a serialized
/// `WorkCnCommandError` JSON string so the frontend can branch on `code`.
#[tauri::command]
pub async fn switch_work_cn_account(
    _app: tauri::AppHandle,
    account_id: String,
) -> Result<WorkCnSwitchResult, String> {
    trae_account::switch_work_cn_account(account_id)
        .await
        .map_err(|error| command_error_to_string(&error))
}

/// Query a saved Work CN account's credit balance. Query only — this never
/// performs a local check-in / claim (开发指南 §8.4). On error the backend
/// returns a serialized `WorkCnCommandError` JSON string.
#[tauri::command]
pub async fn get_work_cn_credits(
    account_id: String,
    force_refresh: bool,
) -> Result<WorkCnCreditsSummary, String> {
    trae_account::get_work_cn_credits(&account_id, force_refresh)
        .await
        .map_err(|error| command_error_to_string(&error))
}

/// Read the current background session-watch status (阶段 7). Desensitized only;
/// never contains tokens.
#[tauri::command]
pub fn get_work_cn_session_watch_status() -> WorkCnSessionWatchStatus {
    crate::modules::work_cn_session_watcher::get_work_cn_session_watch_status()
}

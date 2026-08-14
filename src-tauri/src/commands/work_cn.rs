use std::path::Path;

use crate::models::work_cn::WorkCnInstallation;
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

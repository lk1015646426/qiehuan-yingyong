//! TRAE Work CN 安装检测 / 账号快照管理 Tauri 命令。
//!
//! 所有涉及磁盘 IO、逐账号 AES-GCM 解密或多路径探测的命令均为 `async` 并经
//! `spawn_blocking` 执行：同步命令默认占用主线程，会冻结整个窗口（与
//! work_cn_github.rs 约定一致）。`get_work_cn_session_watch_status` 只读内存
//! 静态状态，保持同步。

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
/// secrets. Safe to call on every page load. 多路径文件探测 + 版本 JSON 解析
/// 在阻塞线程池执行，不占主线程。
#[tauri::command]
pub async fn get_work_cn_installation() -> Result<WorkCnInstallation, String> {
    tauri::async_runtime::spawn_blocking(detect_installation)
        .await
        .map_err(|e| format!("客户端检测任务失败：{e}"))?
}

fn detect_installation() -> Result<WorkCnInstallation, String> {
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
        executable_path: executable_path.map(|path| path.to_string_lossy().to_string()),
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
pub async fn import_current_work_cn_account(
    _app: tauri::AppHandle,
    label: Option<String>,
) -> Result<WorkCnAccountView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        trae_account::import_current_work_cn_account(label)
    })
    .await
    .map_err(|e| format!("导入账号任务失败：{e}"))?
}

/// List previously imported Work CN accounts as desensitized views. 账号较多时
/// 逐账号 AES-GCM 解密较重，在阻塞线程池执行。
#[tauri::command]
pub async fn list_work_cn_accounts() -> Result<Vec<WorkCnAccountView>, String> {
    tauri::async_runtime::spawn_blocking(trae_account::list_work_cn_accounts)
        .await
        .map_err(|e| format!("加载账号列表任务失败：{e}"))?
}

/// Validate the completeness of a saved Work CN account snapshot.
#[tauri::command]
pub async fn validate_work_cn_account(
    account_id: String,
) -> Result<WorkCnSnapshotValidation, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(account) = trae_account::load_account(&account_id) else {
            return Err("账号不存在".to_string());
        };
        Ok(trae_account::validate_work_cn_account_snapshot(&account))
    })
    .await
    .map_err(|e| format!("校验账号快照任务失败：{e}"))?
}

/// 前端订阅的切号进度事件名（与 workCnService.ts 常量保持一致）。
const WORK_CN_SWITCH_PROGRESS_EVENT: &str = "work-cn-switch-progress";

/// 切号进度事件载荷。`stage` 取值见
/// `trae_account::WORK_CN_SWITCH_STAGE_*`（validating / closing / injecting /
/// binding / launching / verifying / syncing）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkCnSwitchProgress {
    account_id: String,
    stage: String,
}

/// One-click switch to a saved Work CN account and open the official client.
/// Orchestrates the full transactional state machine (close → inject → bind →
/// launch → verify → rollback on failure). Errors are returned as a serialized
/// `WorkCnCommandError` JSON string so the frontend can branch on `code`.
/// 各阶段进度经 `work-cn-switch-progress` 事件推送，前端据此显示阶段文案。
#[tauri::command]
pub async fn switch_work_cn_account(
    app: tauri::AppHandle,
    account_id: String,
) -> Result<WorkCnSwitchResult, String> {
    use tauri::Emitter;

    let progress_account_id = account_id.clone();
    let on_progress = move |stage: &str| {
        let _ = app.emit(
            WORK_CN_SWITCH_PROGRESS_EVENT,
            WorkCnSwitchProgress {
                account_id: progress_account_id.clone(),
                stage: stage.to_string(),
            },
        );
    };
    trae_account::switch_work_cn_account_with_progress(account_id, on_progress)
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

/// 清除本地保存的全部 TRAE Work CN 凭证（账号库 + GitHub 同步配置）。
///
/// 删除 `trae_accounts/` 目录、`trae_accounts.json` 索引与 `github.json`。
/// 危险操作：前端必须在二次确认后才可调用。
#[tauri::command]
pub async fn clear_work_cn_credentials() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let data_dir = crate::modules::account::get_data_dir()?;
        let accounts_dir = data_dir.join("trae_accounts");
        let index_path = data_dir.join("trae_accounts.json");
        let github_config_path = data_dir.join("github.json");

        if accounts_dir.exists() {
            std::fs::remove_dir_all(&accounts_dir)
                .map_err(|e| format!("删除账号库失败: {}", e))?;
        }
        if index_path.exists() {
            std::fs::remove_file(&index_path).map_err(|e| format!("删除账号索引失败: {}", e))?;
        }
        if github_config_path.exists() {
            std::fs::remove_file(&github_config_path)
                .map_err(|e| format!("删除 GitHub 同步配置失败: {}", e))?;
        }

        logger::log_info("[Work CN] 已清除本地凭证");
        Ok(())
    })
    .await
    .map_err(|e| format!("清除凭证任务失败：{e}"))?
}

/// 删除一个已导入的 Work CN 账号槽位：移除加密账号文件与索引项，并解绑
/// 其占用的 GitHub Secrets 槽位。不触碰官方客户端的 storage.json；已同步
/// 到远端 GitHub 的 Secrets 也不会被删除。
#[tauri::command]
pub async fn delete_work_cn_account(account_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(account) = trae_account::load_account(&account_id) else {
            return Err("账号不存在".to_string());
        };
        if !trae_account::is_work_cn_account_kind(trae_account::resolve_account_platform_kind(
            &account,
        )) {
            return Err("该账号不是 TRAE Work CN 账号".to_string());
        }
        trae_account::remove_account(&account_id)?;

        // 解绑该账号占用的 GitHub 槽位（若有），避免设置界面残留失效绑定。
        let mut github_config = crate::modules::work_cn_github::load_github_config();
        let before = github_config.slots.len();
        github_config
            .slots
            .retain(|slot| slot.account_id != account_id);
        if github_config.slots.len() != before {
            crate::modules::work_cn_github::save_github_config(&github_config)?;
        }

        logger::log_info(&format!(
            "[Work CN] 账号已删除并解绑 GitHub 槽位: id={}",
            account_id
        ));
        Ok(())
    })
    .await
    .map_err(|e| format!("删除账号任务失败：{e}"))?
}

/// 更新 Work CN 账号的备注（显示名）。备注存为账号 tags（与导入时的
/// label 同一存储位），卡片标题优先取 tags[0]；传空/null 清除备注，
/// 标题回退 昵称 → 邮箱 → UID。有意不更新 `last_used`，避免改名影响
/// 「最近使用」兜底的当前账号判定。
#[tauri::command]
pub async fn update_work_cn_account_label(
    account_id: String,
    label: Option<String>,
) -> Result<WorkCnAccountView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let account = trae_account::update_work_cn_account_label(&account_id, label)?;
        let validation = trae_account::validate_work_cn_account_snapshot(&account);
        Ok(trae_account::build_work_cn_account_view(&account, validation))
    })
    .await
    .map_err(|e| format!("更新备注任务失败：{e}"))?
}

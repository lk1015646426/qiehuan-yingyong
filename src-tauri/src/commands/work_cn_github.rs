//! Stage 6 — GitHub Secrets 同步 Tauri 命令（开发指南 §8.5 / 阶段 6）。

use crate::models::work_cn::{
    WorkCnGitHubCliStatus, WorkCnGitHubConfig, WorkCnGitHubSyncResult,
};
use crate::modules::trae_account::load_account;
use crate::modules::work_cn_github::{
    cli_status, find_slot_for_account, load_github_config, save_github_config,
    sync_account_secrets, RealGitHubRunner,
};

/// Read the persisted GitHub Secrets sync configuration for the settings dialog.
#[tauri::command]
pub fn get_work_cn_github_config() -> Result<WorkCnGitHubConfig, String> {
    Ok(load_github_config())
}

/// Validate and persist the GitHub Secrets sync configuration.
#[tauri::command]
pub fn save_work_cn_github_config(config: WorkCnGitHubConfig) -> Result<(), String> {
    save_github_config(&config)
}

/// Report GitHub CLI availability / auth status (no network secrets touched).
#[tauri::command]
pub fn github_cli_status() -> Result<WorkCnGitHubCliStatus, String> {
    Ok(cli_status(&RealGitHubRunner))
}

/// Sync one account's credentials to its bound GitHub slot. Never claims a
/// check-in. Intentional skips (disabled / unbound / expired token) return
/// `Ok(skipped)`; hard failures (gh not authed, secret-set error) return `Err`.
#[tauri::command]
pub fn sync_work_cn_github_account(account_id: String) -> Result<WorkCnGitHubSyncResult, String> {
    let config = load_github_config();
    if !config.enabled {
        return Ok(WorkCnGitHubSyncResult {
            account_id,
            synced: false,
            skipped: true,
            skip_reason: Some("GitHub 同步未启用".to_string()),
            error: None,
            synced_at: chrono::Utc::now().timestamp(),
        });
    }
    let slot = match find_slot_for_account(&config, &account_id) {
        Some(s) => s,
        None => {
            return Ok(WorkCnGitHubSyncResult {
                account_id,
                synced: false,
                skipped: true,
                skip_reason: Some("该账号未绑定 GitHub 槽位".to_string()),
                error: None,
                synced_at: chrono::Utc::now().timestamp(),
            });
        }
    };
    let account = load_account(&account_id).ok_or_else(|| "账号不存在".to_string())?;
    sync_account_secrets(&RealGitHubRunner, &account, slot, &config.repository)
}

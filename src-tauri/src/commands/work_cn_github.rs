//! Stage 6 — GitHub Secrets 同步 Tauri 命令（开发指南 §8.5 / 阶段 6）。
//!
//! 所有命令均为 `async` 并经 `spawn_blocking` 执行：gh 子进程与网络请求
//! 绝不在主线程跑（同步命令默认占用主线程，会冻结整个窗口）。

use crate::models::work_cn::{WorkCnGitHubCliStatus, WorkCnGitHubConfig, WorkCnGitHubSyncResult};
use crate::modules::trae_account::load_account;
use crate::modules::work_cn_github::{
    cli_status, load_github_config, save_github_config, sync_account_secrets_if_bound,
    RealGitHubRunner,
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
/// gh 子进程 + 可能的联网验证在阻塞线程池执行，不占主线程。
#[tauri::command]
pub async fn github_cli_status() -> Result<WorkCnGitHubCliStatus, String> {
    tauri::async_runtime::spawn_blocking(|| cli_status(&RealGitHubRunner))
        .await
        .map_err(|e| format!("gh 状态检测任务失败：{e}"))
}

/// 自动下载官方 gh MSI 并静默安装（进度经 `gh-setup:progress` 事件推送前端）。
/// 下载与 msiexec 都在阻塞线程池执行，UI 保持可交互。
#[tauri::command]
pub async fn gh_cli_setup_download(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::modules::gh_setup::download_and_install_gh(&app)
    })
    .await
    .map_err(|e| format!("gh 安装任务失败：{e}"))?
}

/// 用 PAT 完成 gh 登录（`gh auth login --with-token`）。Token 只走 stdin、
/// 绝不落盘、绝不出现在命令行参数中；联网登录在阻塞线程池执行。
#[tauri::command]
pub async fn gh_cli_login_with_token(token: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::modules::gh_setup::gh_login_with_token(&token)
    })
    .await
    .map_err(|e| format!("gh 登录任务失败：{e}"))?
}

/// Sync one account's credentials to its bound GitHub slot. Never claims a
/// check-in. Intentional skips (disabled / unbound / expired token) return
/// `Ok(skipped)`; hard failures (gh not authed, secret-set error) return `Err`.
#[tauri::command]
pub async fn sync_work_cn_github_account(
    account_id: String,
) -> Result<WorkCnGitHubSyncResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let account = load_account(&account_id).ok_or_else(|| "账号不存在".to_string())?;
        sync_account_secrets_if_bound(&account)
    })
    .await
    .map_err(|e| format!("GitHub 同步任务失败：{e}"))?
}

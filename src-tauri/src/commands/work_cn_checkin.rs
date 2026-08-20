//! 云端签到面板 Tauri 命令（阶段 8）：手动触发签到 workflow / 查询运行状态。
//!
//! 签到本身永远在云端 Actions 执行；这里只做触发（验证/补签）与状态查询。
//! 配置（repository + workflow_file）来自 `github.json`。

use crate::models::work_cn::CheckinWorkflowRun;
use crate::modules::work_cn_checkin::{list_workflow_runs, trigger_workflow_run};
use crate::modules::work_cn_github::{load_github_config, RealGitHubRunner};

/// 读取 github.json 并校验触发条件：已启用 + repository 非空。
fn resolve_repo_and_workflow() -> Result<(String, String), String> {
    let config = load_github_config();
    if !config.enabled {
        return Err("GitHub 同步未启用，请先在设置中启用并填写仓库".to_string());
    }
    let repository = config.repository.trim().to_string();
    if repository.is_empty() {
        return Err("GitHub 仓库未配置，请先在设置中填写 owner/repo".to_string());
    }
    let workflow_file = if config.workflow_file.trim().is_empty() {
        crate::models::work_cn::default_workflow_file()
    } else {
        config.workflow_file.trim().to_string()
    };
    Ok((repository, workflow_file))
}

/// 手动触发云端签到 workflow（用于凭证修复后的即时验证/补签）。
/// gh 子进程在阻塞线程池执行，不占主线程。
#[tauri::command]
pub async fn trigger_checkin_workflow() -> Result<(), String> {
    let (repository, workflow_file) = resolve_repo_and_workflow()?;
    tauri::async_runtime::spawn_blocking(move || {
        trigger_workflow_run(&RealGitHubRunner, &repository, &workflow_file)
    })
    .await
    .map_err(|e| format!("触发签到任务失败：{e}"))?
}

/// 查询签到 workflow 最近运行状态（默认 5 条）。
/// gh 子进程在阻塞线程池执行，不占主线程。
#[tauri::command]
pub async fn list_checkin_workflow_runs(
    limit: Option<u32>,
) -> Result<Vec<CheckinWorkflowRun>, String> {
    let (repository, workflow_file) = resolve_repo_and_workflow()?;
    let limit = limit.unwrap_or(5).clamp(1, 20);
    tauri::async_runtime::spawn_blocking(move || {
        list_workflow_runs(&RealGitHubRunner, &repository, &workflow_file, limit)
    })
    .await
    .map_err(|e| format!("查询运行历史任务失败：{e}"))?
}

/// 本地单账号签到（诊断/补签）：从本机直接调用 TRAE 签到 API。
/// 与云端 Actions 的运行环境（IP/请求形状）形成对照，用于定位风控问题。
/// 注意：这是有意打破 docs/DEVELOPMENT.md「本地绝不签到/claim」原则的入口
/// （用户 2026-08-16 决策），仅用于诊断与补签。
#[tauri::command]
pub async fn local_checkin_work_cn(
    account_id: String,
) -> Result<crate::models::work_cn::WorkCnLocalCheckinResult, String> {
    crate::modules::trae_account::local_checkin_work_cn_account(&account_id).await
}

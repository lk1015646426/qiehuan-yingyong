//! 云端签到面板后端（阶段 8）：触发 GitHub Actions 签到 workflow 与查询运行状态。
//!
//! 职责边界：本模块**绝不**在本地执行签到——签到一律由云端 Actions 仓库的
//! workflow 完成（日常定时 + 手动验证触发）。这里只通过 `gh` CLI：
//! - `gh workflow run <file> --repo o/r`：手动触发（凭证修复后的即时验证/补签）
//! - `gh run list --workflow <file> --json ...`：查询最近运行状态
//!
//! 复用 `work_cn_github::GitHubRunner` 抽象（stdin 传值、stderr 脱敏、测试可注入）。

use crate::models::work_cn::CheckinWorkflowRun;
use crate::modules::work_cn_github::{github_auth_status, GitHubRunner};

/// 触发签到 workflow（`workflow_dispatch`）。
///
/// 先校验 gh 登录态，再触发；失败错误面向用户可读。
pub fn trigger_workflow_run(
    runner: &dyn GitHubRunner,
    repository: &str,
    workflow_file: &str,
) -> Result<(), String> {
    github_auth_status(runner).map_err(|e| format!("GitHub 同步中止：{e}"))?;

    let out = runner
        .run(
            &["workflow", "run", workflow_file, "--repo", repository],
            None,
        )
        .map_err(|e| format!("运行 gh workflow run 失败：{e}"))?;
    if !out.status_success {
        return Err(format!(
            "触发签到 workflow 失败：{}",
            crate::modules::work_cn_github::redact_for_log(&out.stderr)
        ));
    }
    Ok(())
}

/// 查询签到 workflow 的最近运行列表（按创建时间倒序，由 gh 保证）。
pub fn list_workflow_runs(
    runner: &dyn GitHubRunner,
    repository: &str,
    workflow_file: &str,
    limit: u32,
) -> Result<Vec<CheckinWorkflowRun>, String> {
    github_auth_status(runner).map_err(|e| format!("GitHub 同步中止：{e}"))?;

    let limit_text = limit.to_string();
    let out = runner
        .run(
            &[
                "run",
                "list",
                "--workflow",
                workflow_file,
                "--limit",
                limit_text.as_str(),
                "--json",
                "databaseId,status,conclusion,createdAt,displayTitle,event,url",
                "--repo",
                repository,
            ],
            None,
        )
        .map_err(|e| format!("运行 gh run list 失败：{e}"))?;
    if !out.status_success {
        return Err(format!(
            "查询签到运行状态失败：{}",
            crate::modules::work_cn_github::redact_for_log(&out.stderr)
        ));
    }

    let trimmed = out.stdout.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(trimmed).map_err(|e| {
        format!(
            "解析签到运行状态失败：{e}（输出前 200 字符：{}）",
            &trimmed[..trimmed.len().min(200)]
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::work_cn_github::FakeGitHubRunner;
    use std::sync::Mutex;

    /// 可编程 Fake：按首个参数返回预设输出，记录全部调用。
    struct ScriptedRunner {
        calls: Mutex<Vec<Vec<String>>>,
        auth_ok: bool,
        run_list_stdout: String,
        run_list_ok: bool,
    }

    impl GitHubRunner for ScriptedRunner {
        fn run(
            &self,
            args: &[&str],
            _stdin: Option<&str>,
        ) -> Result<crate::modules::work_cn_github::GitHubRunOutput, String> {
            self.calls
                .lock()
                .unwrap()
                .push(args.iter().map(|s| s.to_string()).collect());
            let first = args.first().copied().unwrap_or("");
            match first {
                "auth" => Ok(crate::modules::work_cn_github::GitHubRunOutput {
                    status_success: self.auth_ok,
                    stdout: String::new(),
                    stderr: if self.auth_ok {
                        String::new()
                    } else {
                        "gh: not logged in".to_string()
                    },
                }),
                "workflow" => Ok(crate::modules::work_cn_github::GitHubRunOutput {
                    status_success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                }),
                "run" => Ok(crate::modules::work_cn_github::GitHubRunOutput {
                    status_success: self.run_list_ok,
                    stdout: self.run_list_stdout.clone(),
                    stderr: if self.run_list_ok {
                        String::new()
                    } else {
                        "gh: workflow not found".to_string()
                    },
                }),
                _ => Ok(crate::modules::work_cn_github::GitHubRunOutput {
                    status_success: true,
                    stdout: String::new(),
                    stderr: String::new(),
                }),
            }
        }
    }

    #[test]
    fn trigger_passes_workflow_file_and_repo() {
        let runner = FakeGitHubRunner::new();
        trigger_workflow_run(&runner, "lk1015646426/daily-checkin", "daily-checkin.yml").unwrap();
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 2, "auth status + workflow run");
        assert_eq!(
            calls[1].args,
            vec![
                "workflow",
                "run",
                "daily-checkin.yml",
                "--repo",
                "lk1015646426/daily-checkin"
            ]
        );
        // 触发命令不携带任何 stdin 凭证
        assert!(calls[1].stdin.is_none());
    }

    #[test]
    fn trigger_reports_not_logged_in() {
        let mut runner = FakeGitHubRunner::new();
        runner.auth_ok = false;
        let err = trigger_workflow_run(&runner, "o/r", "daily-checkin.yml").unwrap_err();
        assert!(err.contains("未登录"), "unexpected: {err}");
    }

    #[test]
    fn list_parses_gh_json_output() {
        let runner = ScriptedRunner {
            calls: Mutex::new(Vec::new()),
            auth_ok: true,
            run_list_stdout: r#"[{"databaseId":123,"status":"completed","conclusion":"success","createdAt":"2026-08-15T04:00:00Z","displayTitle":"Daily Checkin","event":"schedule","url":"https://github.com/o/r/actions/runs/123"}]"#.to_string(),
            run_list_ok: true,
        };
        let runs = list_workflow_runs(&runner, "o/r", "daily-checkin.yml", 5).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].database_id, 123);
        assert_eq!(runs[0].conclusion.as_deref(), Some("success"));
        assert_eq!(runs[0].event, "schedule");
        assert!(runs[0].url.contains("/actions/runs/123"));
    }

    #[test]
    fn list_rejects_garbage_output_without_panic() {
        let runner = ScriptedRunner {
            calls: Mutex::new(Vec::new()),
            auth_ok: true,
            run_list_stdout: "not json at all".to_string(),
            run_list_ok: true,
        };
        let err = list_workflow_run_err(&runner);
        assert!(err.contains("解析签到运行状态失败"));
    }

    #[test]
    fn list_reports_gh_failure() {
        let runner = ScriptedRunner {
            calls: Mutex::new(Vec::new()),
            auth_ok: true,
            run_list_stdout: String::new(),
            run_list_ok: false,
        };
        let err = list_workflow_run_err(&runner);
        assert!(err.contains("查询签到运行状态失败"));
    }

    #[test]
    fn legacy_github_config_defaults_workflow_file() {
        // 旧版 github.json 没有 workflowFile 字段 → serde 取默认值
        let legacy = r#"{"enabled":true,"repository":"o/r","slots":[]}"#;
        let config: crate::models::work_cn::WorkCnGitHubConfig =
            serde_json::from_str(legacy).unwrap();
        assert_eq!(config.workflow_file, "daily-checkin.yml");
        assert_eq!(config.repository, "o/r");
    }

    fn list_workflow_run_err(runner: &ScriptedRunner) -> String {
        list_workflow_runs(runner, "o/r", "daily-checkin.yml", 5).unwrap_err()
    }
}

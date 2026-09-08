//! 智谱账号 GitHub Secrets 同步与云端签到触发。
//!
//! 与 WorkBuddy 相同的链路（复用 `work_cn_github::GitHubRunner`）：
//! - `ZHIPU_ACCOUNTS_JSON` 聚合 Secret 全量覆盖（开启签到的账号：
//!   `{version, accounts:[{key, name, api_key}]}`），删除账号后下一次同步
//!   自动从远端集合移除。
//! - 触发签到：`gh workflow run <file> -f account_filter=zhipu:<account_id>`。
//!   本地绝不执行签到；签到由 daily-checkin 仓库的 GitHub Actions 完成。
//!
//! 后台同步采用「脏标记 + 单工作循环」：变更只置脏并唤醒循环，循环反复
//! 同步直到期间无新变更，避免 workbuddy 复杂调度器的同时保证不丢变更。

use crate::modules::work_cn_github::{
    github_auth_status, redact_for_log, GitHubRunner, RealGitHubRunner,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

const ZHIPU_SECRET_NAME: &str = "ZHIPU_ACCOUNTS_JSON";

static GITHUB_SYNC_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static SYNC_DIRTY: AtomicBool = AtomicBool::new(false);
static SYNC_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
struct AggregateSecret {
    version: u32,
    accounts: Vec<AggregateAccount>,
}

#[derive(Serialize)]
struct AggregateAccount {
    key: String,
    name: String,
    access_token: String,
    /// refresh token 仅同步留作云端后续增强，当前云端签到只用 access token。
    refresh_token: String,
}

fn classify_sync_precondition(enabled: bool, repository: &str) -> Result<(), String> {
    if !enabled || repository.trim().is_empty() {
        return Err("GitHub 签到仓库尚未配置（TRAE 页 GitHub 设置）".to_string());
    }
    Ok(())
}

pub fn build_aggregate_secret(
    accounts: Vec<(crate::models::zhipu::ZhipuAccountView, String, String)>,
) -> Result<String, String> {
    let mut accounts = accounts
        .into_iter()
        .map(|(account, access_token, refresh_token)| {
            if access_token.trim().is_empty() {
                return Err(format!("账号 {} 缺少 access token", account.id));
            }
            let name = account
                .display_name
                .chars()
                .filter(|character| !character.is_control())
                .take(80)
                .collect::<String>();
            if name.trim().is_empty() {
                return Err(format!("账号 {} 名称无效", account.id));
            }
            Ok(AggregateAccount {
                key: account.id,
                name,
                access_token,
                refresh_token,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    accounts.sort_by(|left, right| left.key.cmp(&right.key));
    serde_json::to_string(&AggregateSecret {
        version: 1,
        accounts,
    })
    .map_err(|error| format!("序列化智谱聚合 Secret 失败: {}", error))
}

fn set_aggregate_secret(
    runner: &dyn GitHubRunner,
    repository: &str,
    payload: &str,
) -> Result<(), String> {
    github_auth_status(runner)
        .map_err(|error| format!("GitHub 未就绪: {}", redact_for_log(&error)))?;
    let output = runner
        .run(
            &["secret", "set", ZHIPU_SECRET_NAME, "--repo", repository],
            Some(payload),
        )
        .map_err(|error| {
            format!(
                "设置智谱聚合 Secret 失败: {}",
                redact_for_log(&error)
            )
        })?;
    if !output.status_success {
        return Err(format!(
            "设置智谱聚合 Secret 失败: {}",
            redact_for_log(&output.stderr)
        ));
    }
    Ok(())
}

/// 全量同步开启签到的智谱账号到 GitHub 聚合 Secret。
/// 索引文件 mtime 变化期间的新变更由调用方的脏标记循环兜底重同步。
pub fn sync_zhipu_accounts_with(runner: &dyn GitHubRunner) -> Result<(), String> {
    let _sync_guard = GITHUB_SYNC_LOCK
        .lock()
        .map_err(|_| "智谱 GitHub 同步锁已损坏".to_string())?;
    let accounts = crate::modules::zhipu_account::list_zhipu_accounts()?;
    let account_ids = accounts
        .iter()
        .map(|account| account.id.clone())
        .collect::<Vec<_>>();
    let config = crate::modules::work_cn_github::load_github_config();
    if let Err(message) = classify_sync_precondition(config.enabled, &config.repository) {
        mark_sync_state(&account_ids, "pending", None);
        return Err(message);
    }
    let enabled_accounts = match accounts
        .into_iter()
        .filter(|account| account.checkin_enabled)
        .map(|account| {
            let access = crate::modules::zhipu_account::access_token(&account.id)?;
            let refresh = crate::modules::zhipu_account::refresh_token(&account.id)
                .unwrap_or_default();
            Ok((account, access, refresh))
        })
        .collect::<Result<Vec<_>, String>>()
    {
        Ok(accounts) => accounts,
        Err(error) => {
            mark_sync_state(&account_ids, "failed", Some(&error));
            return Err(error);
        }
    };
    let payload = match build_aggregate_secret(enabled_accounts) {
        Ok(payload) => payload,
        Err(error) => {
            mark_sync_state(&account_ids, "failed", Some(&error));
            return Err(error);
        }
    };
    match set_aggregate_secret(runner, config.repository.trim(), &payload) {
        Ok(()) => {
            mark_sync_state(&account_ids, "synced", None);
            Ok(())
        }
        Err(error) => {
            mark_sync_state(&account_ids, "failed", Some(&error));
            Err(error)
        }
    }
}

fn mark_sync_state(account_ids: &[String], state: &str, error: Option<&str>) {
    if account_ids.is_empty() {
        return;
    }
    if let Err(error) =
        crate::modules::zhipu_account::mark_github_sync_many(account_ids, state, error)
    {
        crate::modules::logger::log_warn(&format!(
            "[Zhipu GitHub] 批量更新账号同步状态失败: {}",
            error
        ));
    }
}

pub fn sync_zhipu_accounts() -> Result<(), String> {
    sync_zhipu_accounts_with(&RealGitHubRunner)
}

/// 排队一次后台同步：置脏并确保工作循环在跑。循环会读取最新账号数据，
/// 执行期间到达的新变更会再次置脏，触发下一轮。
pub fn queue_background_sync() {
    SYNC_DIRTY.store(true, Ordering::SeqCst);
    if !SYNC_RUNNING.swap(true, Ordering::SeqCst) {
        tauri::async_runtime::spawn(async {
            loop {
                // 小延迟合并快速连续变更（导入+开关切换）。
                tokio::time::sleep(Duration::from_millis(300)).await;
                while SYNC_DIRTY.swap(false, Ordering::SeqCst) {
                    let result = tauri::async_runtime::spawn_blocking(sync_zhipu_accounts)
                        .await
                        .unwrap_or_else(|error| {
                            Err(format!("后台智谱 GitHub 同步任务异常: {error}"))
                        });
                    if let Err(error) = &result {
                        crate::modules::logger::log_warn(&format!(
                            "[Zhipu GitHub] 后台同步未完成: {}",
                            redact_for_log(error)
                        ));
                    }
                }
                if SYNC_DIRTY.load(Ordering::SeqCst) {
                    continue;
                }
                SYNC_RUNNING.store(false, Ordering::SeqCst);
                // 退出竞态兜底：标记退出后立刻又有新排队则重新接管循环。
                if SYNC_DIRTY.load(Ordering::SeqCst)
                    && !SYNC_RUNNING.swap(true, Ordering::SeqCst)
                {
                    continue;
                }
                return;
            }
        });
    }
}

/// 手动同步：立即执行并等待完成（前端按钮需要结果反馈）。
pub async fn sync_background_now() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(sync_zhipu_accounts)
        .await
        .map_err(|error| format!("智谱 GitHub 同步任务失败: {error}"))?
}

fn trigger_zhipu_checkin_with(
    runner: &dyn GitHubRunner,
    repository: &str,
    workflow_file: &str,
    account_id: &str,
) -> Result<(), String> {
    if !account_id.starts_with("zp-") {
        return Err("智谱账号 ID 无效".to_string());
    }
    github_auth_status(runner)
        .map_err(|error| format!("GitHub 未就绪: {}", redact_for_log(&error)))?;
    let filter = format!("account_filter=zhipu:{}", account_id);
    let output = runner
        .run(
            &[
                "workflow",
                "run",
                workflow_file,
                "--repo",
                repository,
                "-f",
                filter.as_str(),
            ],
            None,
        )
        .map_err(|error| format!("触发智谱签到任务失败: {}", redact_for_log(&error)))?;
    if !output.status_success {
        return Err(format!(
            "触发智谱签到任务失败: {}",
            redact_for_log(&output.stderr)
        ));
    }
    Ok(())
}

pub fn trigger_zhipu_checkin(account_id: &str) -> Result<(), String> {
    if !crate::modules::zhipu_account::has_account_id(account_id)? {
        return Err("智谱账号不存在".to_string());
    }
    if !crate::modules::zhipu_account::checkin_enabled(account_id)? {
        return Err("该智谱账号未参与自动签到".to_string());
    }
    // 触发前先确保远端聚合 Secret 是最新全量（包含被删账号的清理）。
    sync_zhipu_accounts()?;
    let config = crate::modules::work_cn_github::load_github_config();
    let workflow_file = if config.workflow_file.trim().is_empty() {
        crate::models::work_cn::default_workflow_file().to_string()
    } else {
        config.workflow_file.trim().to_string()
    };
    trigger_zhipu_checkin_with(
        &RealGitHubRunner,
        config.repository.trim(),
        &workflow_file,
        account_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::zhipu::ZhipuAccountView;
    use crate::modules::work_cn_github::FakeGitHubRunner;

    /// 测试辅助：按 uid 构造 JWT 并导入账号。
    fn import_test_account(uid: &str) -> Result<ZhipuAccountView, String> {
        use base64::Engine;
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256"}"#);
        let payload = format!(r#"{{"sub":"测试_T9","uid":"{uid}","exp":9999999999}}"#);
        let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let token = format!("{header}.{body}.signature");
        crate::modules::zhipu_account::import_account(&token, "", None)
    }

    fn account(id: &str, name: &str) -> ZhipuAccountView {
        ZhipuAccountView {
            id: id.to_string(),
            display_name: name.to_string(),
            user_label: format!("用户_{name}"),
            checkin_enabled: true,
            token_expires_at: None,
            created_at: 0,
            updated_at: 0,
            last_github_sync_at: None,
            last_github_sync_state: "pending".to_string(),
            last_github_sync_error: None,
        }
    }

    #[test]
    fn zhipu_aggregate_is_sorted_and_secret_never_appears_in_gh_arguments() {
        let runner = FakeGitHubRunner::new();
        let payload = build_aggregate_secret(vec![
            (
                account("zp-ffffffffffff", "乙"),
                "access-b.JWT-BODY".to_string(),
                "refresh-b.JWT-BODY".to_string(),
            ),
            (
                account("zp-000000000000", "甲"),
                "access-a.JWT-BODY".to_string(),
                "refresh-a.JWT-BODY".to_string(),
            ),
        ])
        .unwrap();
        set_aggregate_secret(&runner, "owner/repo", &payload).unwrap();

        assert_eq!(
            payload,
            r#"{"version":1,"accounts":[{"key":"zp-000000000000","name":"甲","access_token":"access-a.JWT-BODY","refresh_token":"refresh-a.JWT-BODY"},{"key":"zp-ffffffffffff","name":"乙","access_token":"access-b.JWT-BODY","refresh_token":"refresh-b.JWT-BODY"}]}"#
        );
        let call = runner.recorded_calls().pop().unwrap();
        assert_eq!(call.stdin.as_deref(), Some(payload.as_str()));
        assert!(!call
            .args
            .iter()
            .any(|value| value.contains("access-a") || value.contains("refresh-b")));
    }

    #[test]
    fn zhipu_checkin_trigger_filters_by_stable_account_id_without_key() {
        let runner = FakeGitHubRunner::new();

        trigger_zhipu_checkin_with(
            &runner,
            "owner/repo",
            "daily-checkin.yml",
            "zp-0123456789ab",
        )
        .unwrap();

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 2, "auth status + workflow run");
        assert_eq!(
            calls[1].args,
            vec![
                "workflow".to_string(),
                "run".to_string(),
                "daily-checkin.yml".to_string(),
                "--repo".to_string(),
                "owner/repo".to_string(),
                "-f".to_string(),
                "account_filter=zhipu:zp-0123456789ab".to_string(),
            ]
        );
        assert!(calls.iter().all(|call| {
            call.args
                .iter()
                .all(|argument| !argument.contains("api_key"))
        }));
    }

    #[test]
    fn zhipu_checkin_trigger_rejects_foreign_account_prefix() {
        let runner = FakeGitHubRunner::new();
        let error =
            trigger_zhipu_checkin_with(&runner, "o/r", "daily-checkin.yml", "wb-0123456789ab")
                .unwrap_err();
        assert!(error.contains("智谱账号 ID 无效"));
    }

    #[test]
    fn zhipu_sync_without_github_config_marks_pending() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "zhipu-github-pending-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        crate::modules::work_cn_github::save_github_config(&crate::models::work_cn::WorkCnGitHubConfig {
            enabled: false,
            repository: String::new(),
            slots: Vec::new(),
            workflow_file: "daily-checkin.yml".to_string(),
        })
        .unwrap();
        let imported = import_test_account("uid-pending").unwrap();

        assert!(sync_zhipu_accounts_with(&FakeGitHubRunner::new()).is_err());
        let current = crate::modules::zhipu_account::list_zhipu_accounts()
            .unwrap()
            .into_iter()
            .find(|item| item.id == imported.id)
            .unwrap();
        assert_eq!(current.last_github_sync_state, "pending");
        assert!(current.last_github_sync_error.is_none());

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn zhipu_sync_full_overwrite_clears_deleted_accounts_remotely() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "zhipu-github-cleanup-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        crate::modules::work_cn_github::save_github_config(&crate::models::work_cn::WorkCnGitHubConfig {
            enabled: true,
            repository: "owner/repo".to_string(),
            slots: Vec::new(),
            workflow_file: "daily-checkin.yml".to_string(),
        })
        .unwrap();
        let enabled = import_test_account("uid-enabled").unwrap();
        let disabled = import_test_account("uid-disabled").unwrap();
        crate::modules::zhipu_account::set_checkin_enabled(&disabled.id, false).unwrap();

        let runner = FakeGitHubRunner::new();
        sync_zhipu_accounts_with(&runner).unwrap();

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 2, "auth status + secret set");
        let payload = calls[1].stdin.as_deref().unwrap();
        assert!(payload.contains(&enabled.id));
        assert!(!payload.contains(&disabled.id));
        // 删除开启签到的账号后，全量覆盖即为远端清理。
        crate::modules::zhipu_account::remove_zhipu_account(&enabled.id).unwrap();
        let runner_after_delete = FakeGitHubRunner::new();
        sync_zhipu_accounts_with(&runner_after_delete).unwrap();
        let calls_after_delete = runner_after_delete.recorded_calls();
        let cleanup_payload = calls_after_delete[1].stdin.as_deref().unwrap();
        assert_eq!(cleanup_payload, r#"{"version":1,"accounts":[]}"#);

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(dir);
    }
}

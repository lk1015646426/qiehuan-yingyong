use crate::models::workbuddy::WorkBuddyAccountView;
use crate::modules::work_cn_github::{
    github_auth_status, redact_for_log, GitHubRunner, RealGitHubRunner,
};
use serde::{Deserialize, Serialize};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const AGGREGATE_SECRET_NAME: &str = "WORKBUDDY_ACCOUNTS_JSON";
const GITHUB_STATE_FILE: &str = "workbuddy_github_state.json";

static GITHUB_SYNC_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static GITHUB_STATE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static BACKGROUND_SYNC_SCHEDULE: LazyLock<Mutex<BackgroundSyncSchedule>> =
    LazyLock::new(|| Mutex::new(BackgroundSyncSchedule::default()));
static BACKGROUND_SYNC_WAKE: LazyLock<tokio::sync::Notify> =
    LazyLock::new(tokio::sync::Notify::new);
type BackgroundSyncResult = Result<(), String>;
type BackgroundSyncWaiter = tokio::sync::oneshot::Sender<BackgroundSyncResult>;

#[derive(Debug, Clone, Copy)]
struct BackgroundSyncSnapshot {
    generation: u64,
    due_at: Instant,
    update_watcher_status: bool,
    reason: &'static str,
}

#[derive(Debug)]
struct BackgroundSyncSchedule {
    generation: u64,
    scheduled: bool,
    running: bool,
    due_at: Instant,
    watcher_status_generation: Option<u64>,
    reason: &'static str,
    waiters: Vec<(u64, BackgroundSyncWaiter)>,
}

impl Default for BackgroundSyncSchedule {
    fn default() -> Self {
        Self {
            generation: 0,
            scheduled: false,
            running: false,
            due_at: Instant::now(),
            watcher_status_generation: None,
            reason: "change",
            waiters: Vec::new(),
        }
    }
}

impl BackgroundSyncSchedule {
    fn request(&mut self, now: Instant, delay: Duration, update_watcher_status: bool) -> bool {
        self.request_with_reason(now, delay, update_watcher_status, "change")
    }

    fn request_with_reason(
        &mut self,
        now: Instant,
        delay: Duration,
        update_watcher_status: bool,
        reason: &'static str,
    ) -> bool {
        self.generation = self.generation.saturating_add(1);
        let candidate_due_at = now + delay;
        let should_spawn = !self.scheduled;
        if should_spawn {
            self.scheduled = true;
            self.due_at = candidate_due_at;
        } else if delay.is_zero() || self.running || self.due_at > now {
            self.due_at = candidate_due_at;
        }
        if update_watcher_status {
            self.watcher_status_generation = Some(self.generation);
        }
        self.reason = reason;
        should_spawn
    }

    fn snapshot(&self) -> BackgroundSyncSnapshot {
        BackgroundSyncSnapshot {
            generation: self.generation,
            due_at: self.due_at,
            update_watcher_status: self
                .watcher_status_generation
                .is_some_and(|generation| generation <= self.generation),
            reason: self.reason,
        }
    }

    fn start_run(&mut self) -> BackgroundSyncSnapshot {
        self.running = true;
        self.snapshot()
    }

    fn add_waiter(&mut self, waiter: BackgroundSyncWaiter) {
        self.waiters.push((self.generation, waiter));
    }

    fn take_waiters(&mut self, observed_generation: u64) -> Vec<BackgroundSyncWaiter> {
        let (ready, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|(generation, _)| *generation <= observed_generation);
        self.waiters = pending;
        ready.into_iter().map(|(_, waiter)| waiter).collect()
    }

    fn complete(&mut self, observed_generation: u64) -> bool {
        self.running = false;
        if self.generation != observed_generation {
            return false;
        }
        self.watcher_status_generation = None;
        self.scheduled = false;
        true
    }

    fn is_scheduled(&self) -> bool {
        self.scheduled
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SyncPrecondition {
    Ready,
    Pending(String),
}

fn classify_sync_precondition(enabled: bool, repository: &str) -> SyncPrecondition {
    if !enabled || repository.trim().is_empty() {
        SyncPrecondition::Pending("GitHub 签到仓库尚未配置".to_string())
    } else {
        SyncPrecondition::Ready
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct WorkBuddyGitHubState {
    #[serde(default)]
    cleanup_pending: bool,
    #[serde(default)]
    generation: u64,
}

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
}

fn github_state_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(GITHUB_STATE_FILE))
}

pub fn github_cleanup_pending() -> bool {
    load_github_state().cleanup_pending
}

fn load_github_state() -> WorkBuddyGitHubState {
    let Ok(path) = github_state_path() else {
        return WorkBuddyGitHubState::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<WorkBuddyGitHubState>(&raw).ok())
        .unwrap_or_default()
}

fn save_github_state(state: &WorkBuddyGitHubState) -> Result<(), String> {
    let raw = serde_json::to_string_pretty(state)
        .map_err(|error| format!("序列化 WorkBuddy GitHub 状态失败: {error}"))?;
    crate::modules::atomic_write::write_string_atomic(&github_state_path()?, &raw)
        .map_err(|error| format!("保存 WorkBuddy GitHub 状态失败: {error}"))
}

pub fn set_github_cleanup_pending(cleanup_pending: bool) -> Result<(), String> {
    let _guard = GITHUB_STATE_LOCK
        .lock()
        .map_err(|_| "WorkBuddy GitHub 状态锁已损坏".to_string())?;
    let mut state = load_github_state();
    state.cleanup_pending = cleanup_pending;
    save_github_state(&state)
}

pub fn mark_dataset_changed() -> Result<u64, String> {
    let _guard = GITHUB_STATE_LOCK
        .lock()
        .map_err(|_| "WorkBuddy GitHub 状态锁已损坏".to_string())?;
    let mut state = load_github_state();
    state.generation = state.generation.saturating_add(1);
    state.cleanup_pending = true;
    save_github_state(&state)?;
    Ok(state.generation)
}

fn dataset_generation() -> u64 {
    let Ok(_guard) = GITHUB_STATE_LOCK.lock() else {
        return 0;
    };
    load_github_state().generation
}

fn clear_cleanup_if_generation(observed_generation: u64) -> Result<bool, String> {
    let _guard = GITHUB_STATE_LOCK
        .lock()
        .map_err(|_| "WorkBuddy GitHub 状态锁已损坏".to_string())?;
    let mut state = load_github_state();
    if state.generation != observed_generation {
        return Ok(false);
    }
    state.cleanup_pending = false;
    save_github_state(&state)?;
    Ok(true)
}

pub fn build_aggregate_secret(
    accounts: Vec<(WorkBuddyAccountView, String)>,
) -> Result<String, String> {
    let mut accounts = accounts
        .into_iter()
        .map(|(account, token)| {
            if token.trim().is_empty() {
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
                access_token: token,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    accounts.sort_by(|left, right| left.key.cmp(&right.key));
    serde_json::to_string(&AggregateSecret {
        version: 1,
        accounts,
    })
    .map_err(|error| format!("序列化 WorkBuddy 聚合 Secret 失败: {}", error))
}

pub fn set_aggregate_secret(
    runner: &dyn GitHubRunner,
    repository: &str,
    payload: &str,
) -> Result<(), String> {
    github_auth_status(runner)
        .map_err(|error| format!("GitHub 未就绪: {}", redact_for_log(&error)))?;
    let output = runner
        .run(
            &["secret", "set", AGGREGATE_SECRET_NAME, "--repo", repository],
            Some(payload),
        )
        .map_err(|error| {
            format!(
                "设置 WorkBuddy 聚合 Secret 失败: {}",
                redact_for_log(&error)
            )
        })?;
    if !output.status_success {
        return Err(format!(
            "设置 WorkBuddy 聚合 Secret 失败: {}",
            redact_for_log(&output.stderr)
        ));
    }
    Ok(())
}

pub fn sync_workbuddy_accounts_with(runner: &dyn GitHubRunner) -> Result<(), String> {
    let _sync_guard = GITHUB_SYNC_LOCK
        .lock()
        .map_err(|_| "WorkBuddy GitHub 同步锁已损坏".to_string())?;
    let observed_generation = dataset_generation();
    let accounts = crate::modules::workbuddy_account::list_workbuddy_accounts()?;
    let account_ids = accounts
        .iter()
        .map(|account| account.id.clone())
        .collect::<Vec<_>>();
    let config = crate::modules::work_cn_github::load_github_config();
    if let SyncPrecondition::Pending(message) =
        classify_sync_precondition(config.enabled, &config.repository)
    {
        mark_sync_state(&account_ids, "pending", None);
        return Err(message);
    }
    let enabled_accounts = match accounts
        .into_iter()
        .filter(|account| account.checkin_enabled)
        .map(|account| {
            let token = crate::modules::workbuddy_account::access_token(&account.id)?;
            Ok((account, token))
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
            if dataset_generation() == observed_generation {
                mark_sync_state(&account_ids, "synced", None);
                clear_cleanup_if_generation(observed_generation)?;
            }
            Ok(())
        }
        Err(error) => {
            if dataset_generation() == observed_generation {
                mark_sync_state(&account_ids, "failed", Some(&error));
            }
            Err(error)
        }
    }
}

fn mark_sync_state(account_ids: &[String], state: &str, error: Option<&str>) {
    if let Err(error) =
        crate::modules::workbuddy_account::mark_github_sync_many(account_ids, state, error)
    {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy GitHub] 批量更新账号同步状态失败: {}",
            error
        ));
    }
}

pub fn sync_workbuddy_accounts() -> Result<(), String> {
    sync_workbuddy_accounts_with(&RealGitHubRunner)
}

fn enqueue_background_sync(
    reason: &'static str,
    delay: Duration,
    update_watcher_status: bool,
    waiter: Option<BackgroundSyncWaiter>,
) -> bool {
    let should_spawn = match BACKGROUND_SYNC_SCHEDULE.lock() {
        Ok(mut schedule) => {
            let should_spawn =
                schedule.request_with_reason(Instant::now(), delay, update_watcher_status, reason);
            if let Some(waiter) = waiter {
                schedule.add_waiter(waiter);
            }
            should_spawn
        }
        Err(_) => {
            crate::modules::logger::log_warn("[WorkBuddy GitHub] 后台同步调度锁已损坏，无法排队");
            return false;
        }
    };
    BACKGROUND_SYNC_WAKE.notify_one();
    should_spawn
}

pub fn queue_background_sync(reason: &'static str, delay: Duration, update_watcher_status: bool) {
    if enqueue_background_sync(reason, delay, update_watcher_status, None) {
        spawn_background_sync_worker();
    }
}

pub async fn sync_background_now(reason: &'static str) -> Result<(), String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if enqueue_background_sync(reason, Duration::ZERO, false, Some(sender)) {
        spawn_background_sync_worker();
    }
    receiver
        .await
        .map_err(|_| "WorkBuddy GitHub 后台同步任务意外终止".to_string())?
}

fn spawn_background_sync_worker() {
    tauri::async_runtime::spawn(async {
        'worker: loop {
            let snapshot = match BACKGROUND_SYNC_SCHEDULE.lock() {
                Ok(schedule) => schedule.snapshot(),
                Err(_) => {
                    crate::modules::logger::log_warn(
                        "[WorkBuddy GitHub] 后台同步调度锁已损坏，任务已停止",
                    );
                    return;
                }
            };

            loop {
                let now = Instant::now();
                if snapshot.due_at <= now {
                    break;
                }
                tokio::select! {
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(snapshot.due_at)) => break,
                    _ = BACKGROUND_SYNC_WAKE.notified() => continue 'worker,
                }
            }

            let snapshot = match BACKGROUND_SYNC_SCHEDULE.lock() {
                Ok(mut schedule) if schedule.snapshot().due_at <= Instant::now() => {
                    schedule.start_run()
                }
                Ok(_) => continue,
                Err(_) => {
                    crate::modules::logger::log_warn(
                        "[WorkBuddy GitHub] 后台同步调度锁已损坏，任务已停止",
                    );
                    return;
                }
            };
            let result = tauri::async_runtime::spawn_blocking(sync_workbuddy_accounts)
                .await
                .unwrap_or_else(|error| {
                    Err(format!("后台 WorkBuddy GitHub 同步任务异常: {error}"))
                });
            if let Err(error) = &result {
                crate::modules::logger::log_warn(&format!(
                    "[WorkBuddy GitHub] {}后的后台同步未完成: {}",
                    snapshot.reason,
                    redact_for_log(error)
                ));
            }
            crate::modules::workbuddy_session_watcher::record_github_sync_result(
                &result,
                snapshot.update_watcher_status,
            );

            let (completed, waiters) = match BACKGROUND_SYNC_SCHEDULE.lock() {
                Ok(mut schedule) => {
                    let waiters = schedule.take_waiters(snapshot.generation);
                    (schedule.complete(snapshot.generation), waiters)
                }
                Err(_) => {
                    crate::modules::logger::log_warn(
                        "[WorkBuddy GitHub] 后台同步调度锁已损坏，任务已停止",
                    );
                    return;
                }
            };
            for waiter in waiters {
                let _ = waiter.send(result.clone());
            }
            if completed {
                break;
            }
        }
    });
}

pub fn trigger_workbuddy_checkin_with(
    runner: &dyn GitHubRunner,
    repository: &str,
    workflow_file: &str,
    account_id: &str,
) -> Result<(), String> {
    if !account_id.starts_with("wb-") {
        return Err("WorkBuddy 账号 ID 无效".to_string());
    }
    github_auth_status(runner)
        .map_err(|error| format!("GitHub 未就绪: {}", redact_for_log(&error)))?;
    let filter = format!("account_filter=workbuddy:{}", account_id);
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
        .map_err(|error| format!("触发 WorkBuddy 签到任务失败: {}", redact_for_log(&error)))?;
    if !output.status_success {
        return Err(format!(
            "触发 WorkBuddy 签到任务失败: {}",
            redact_for_log(&output.stderr)
        ));
    }
    Ok(())
}

pub fn trigger_workbuddy_checkin(account_id: &str) -> Result<(), String> {
    let account = crate::modules::workbuddy_account::list_workbuddy_accounts()?
        .into_iter()
        .find(|account| account.id == account_id)
        .ok_or_else(|| "WorkBuddy 账号不存在".to_string())?;
    if !account.checkin_enabled {
        return Err("该 WorkBuddy 账号未参与自动签到".to_string());
    }
    sync_workbuddy_accounts()?;
    let config = crate::modules::work_cn_github::load_github_config();
    let workflow_file = if config.workflow_file.trim().is_empty() {
        crate::models::work_cn::default_workflow_file().to_string()
    } else {
        config.workflow_file.trim().to_string()
    };
    trigger_workbuddy_checkin_with(
        &RealGitHubRunner,
        config.repository.trim(),
        &workflow_file,
        account_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::work_cn::WorkCnGitHubConfig;
    use crate::models::workbuddy::WorkBuddyAccountView;
    use crate::modules::work_cn_github::{FakeGitHubRunner, GitHubRunOutput};
    use std::fs;
    use std::time::{Duration, Instant};

    #[test]
    fn background_sync_schedule_coalesces_requests_and_preserves_immediate_priority() {
        let start = Instant::now();
        let mut schedule = BackgroundSyncSchedule::default();

        assert!(schedule.request(start, Duration::from_secs(20), false));
        assert_eq!(schedule.snapshot().generation, 1);
        assert_eq!(schedule.snapshot().due_at, start + Duration::from_secs(20));

        assert!(!schedule.request(start + Duration::from_secs(2), Duration::ZERO, false));
        assert_eq!(schedule.snapshot().generation, 2);
        assert_eq!(schedule.snapshot().due_at, start + Duration::from_secs(2));
    }

    #[test]
    fn background_sync_schedule_resets_debounce_and_runs_again_for_new_generation() {
        let start = Instant::now();
        let mut schedule = BackgroundSyncSchedule::default();

        assert!(schedule.request(start, Duration::from_secs(20), true));
        assert!(!schedule.request(
            start + Duration::from_secs(5),
            Duration::from_secs(20),
            false,
        ));
        let snapshot = schedule.snapshot();
        assert_eq!(snapshot.generation, 2);
        assert_eq!(snapshot.due_at, start + Duration::from_secs(25));
        assert!(snapshot.update_watcher_status);

        assert!(!schedule.complete(1));
        assert!(schedule.is_scheduled());
        assert!(schedule.snapshot().update_watcher_status);
        assert!(schedule.complete(2));
        assert!(!schedule.is_scheduled());
    }

    #[test]
    fn background_sync_schedule_debounces_watcher_request_arriving_during_sync() {
        let start = Instant::now();
        let mut schedule = BackgroundSyncSchedule::default();
        assert!(schedule.request(start, Duration::ZERO, false));
        schedule.start_run();

        assert!(!schedule.request(
            start + Duration::from_secs(1),
            Duration::from_secs(20),
            true,
        ));
        assert_eq!(schedule.snapshot().due_at, start + Duration::from_secs(21));
    }

    struct DatasetMutatingRunner;

    impl GitHubRunner for DatasetMutatingRunner {
        fn run(&self, args: &[&str], _stdin: Option<&str>) -> Result<GitHubRunOutput, String> {
            if args.first().copied() == Some("secret") {
                mark_dataset_changed().unwrap();
            }
            Ok(GitHubRunOutput {
                status_success: true,
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn missing_github_config_is_pending_not_failed() {
        assert_eq!(
            classify_sync_precondition(false, ""),
            SyncPrecondition::Pending("GitHub 签到仓库尚未配置".to_string())
        );
    }

    #[test]
    fn background_sync_waiter_is_completed_by_covering_generation() {
        let start = Instant::now();
        let mut schedule = BackgroundSyncSchedule::default();
        assert!(schedule.request(start, Duration::ZERO, false));
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        schedule.add_waiter(sender);

        let waiters = schedule.take_waiters(1);
        assert_eq!(waiters.len(), 1);
        waiters.into_iter().for_each(|sender| {
            let _ = sender.send(Ok(()));
        });
        assert_eq!(receiver.try_recv(), Ok(Ok(())));
    }

    #[test]
    fn missing_github_config_keeps_account_pending() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-github-pending-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let account = crate::modules::workbuddy_account::import_snapshot_json(
            r#"{"account":{"uid":"pending-account"},"auth":{"accessToken":"token-pending"}}"#,
            None,
        )
        .unwrap();

        assert!(sync_workbuddy_accounts_with(&FakeGitHubRunner::new()).is_err());
        let current = crate::modules::workbuddy_account::list_workbuddy_accounts()
            .unwrap()
            .into_iter()
            .find(|item| item.id == account.id)
            .unwrap();
        assert_eq!(current.last_github_sync_state, "pending");
        assert!(current.last_github_sync_error.is_none());

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    fn account(id: &str, name: &str) -> WorkBuddyAccountView {
        WorkBuddyAccountView {
            id: id.to_string(),
            uid: id.to_string(),
            uin: None,
            display_name: name.to_string(),
            masked_phone: None,
            checkin_enabled: true,
            token_expires_at: None,
            created_at: 0,
            updated_at: 0,
            last_used_at: None,
            last_github_sync_at: None,
            last_github_sync_state: "pending".to_string(),
            last_github_sync_error: None,
        }
    }

    #[test]
    fn workbuddy_aggregate_is_sorted_and_secret_never_appears_in_gh_arguments() {
        let runner = FakeGitHubRunner::new();
        let payload = build_aggregate_secret(vec![
            (account("wb-ffffffffffff", "乙"), "token-z".to_string()),
            (account("wb-000000000000", "甲"), "token-a".to_string()),
        ])
        .unwrap();
        set_aggregate_secret(&runner, "owner/repo", &payload).unwrap();

        assert_eq!(
            payload,
            r#"{"version":1,"accounts":[{"key":"wb-000000000000","name":"甲","access_token":"token-a"},{"key":"wb-ffffffffffff","name":"乙","access_token":"token-z"}]}"#
        );
        let call = runner.recorded_calls().pop().unwrap();
        assert_eq!(call.stdin.as_deref(), Some(payload.as_str()));
        assert!(!call
            .args
            .iter()
            .any(|value| value.contains("token-a") || value.contains("token-z")));
    }

    #[test]
    fn workbuddy_checkin_trigger_filters_by_stable_account_id_without_token() {
        let runner = FakeGitHubRunner::new();

        trigger_workbuddy_checkin_with(
            &runner,
            "owner/repo",
            "daily-checkin.yml",
            "wb-0123456789ab",
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
                "account_filter=workbuddy:wb-0123456789ab".to_string(),
            ]
        );
        assert!(calls.iter().all(|call| {
            call.args
                .iter()
                .all(|argument| !argument.contains("access_token"))
        }));
    }

    #[test]
    fn workbuddy_successful_sync_marks_disabled_accounts_synced_after_remote_removal() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-github-state-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        crate::modules::work_cn_github::save_github_config(&WorkCnGitHubConfig {
            enabled: true,
            repository: "owner/repo".to_string(),
            slots: Vec::new(),
            workflow_file: "daily-checkin.yml".to_string(),
        })
        .unwrap();
        let enabled = crate::modules::workbuddy_account::import_snapshot_json(
            r#"{"account":{"uid":"sync-enabled"},"auth":{"accessToken":"token-enabled"}}"#,
            None,
        )
        .unwrap();
        let disabled = crate::modules::workbuddy_account::import_snapshot_json(
            r#"{"account":{"uid":"sync-disabled"},"auth":{"accessToken":"token-disabled"}}"#,
            None,
        )
        .unwrap();
        crate::modules::workbuddy_account::set_checkin_enabled(&disabled.id, false).unwrap();

        sync_workbuddy_accounts_with(&FakeGitHubRunner::new()).unwrap();

        let accounts = crate::modules::workbuddy_account::list_workbuddy_accounts().unwrap();
        assert_eq!(
            accounts
                .iter()
                .find(|account| account.id == enabled.id)
                .unwrap()
                .last_github_sync_state,
            "synced"
        );
        assert_eq!(
            accounts
                .iter()
                .find(|account| account.id == disabled.id)
                .unwrap()
                .last_github_sync_state,
            "synced"
        );

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workbuddy_empty_sync_clears_pending_remote_accounts() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-github-cleanup-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        crate::modules::work_cn_github::save_github_config(&WorkCnGitHubConfig {
            enabled: true,
            repository: "owner/repo".to_string(),
            slots: Vec::new(),
            workflow_file: "daily-checkin.yml".to_string(),
        })
        .unwrap();
        set_github_cleanup_pending(true).unwrap();
        let runner = FakeGitHubRunner::new();

        sync_workbuddy_accounts_with(&runner).unwrap();

        assert!(!github_cleanup_pending());
        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 2, "auth status + secret set");
        assert_eq!(
            calls[1].stdin.as_deref(),
            Some(r#"{"version":1,"accounts":[]}"#)
        );

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sync_does_not_clear_cleanup_when_dataset_changes_in_flight() {
        let _env = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "workbuddy-github-generation-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        crate::modules::work_cn_github::save_github_config(&WorkCnGitHubConfig {
            enabled: true,
            repository: "owner/repo".to_string(),
            slots: Vec::new(),
            workflow_file: "daily-checkin.yml".to_string(),
        })
        .unwrap();
        crate::modules::workbuddy_account::import_snapshot_json(
            r#"{"account":{"uid":"generation-account"},"auth":{"accessToken":"token-generation"}}"#,
            None,
        )
        .unwrap();
        mark_dataset_changed().unwrap();

        sync_workbuddy_accounts_with(&DatasetMutatingRunner).unwrap();

        assert!(github_cleanup_pending());
        assert_eq!(
            crate::modules::workbuddy_account::list_workbuddy_accounts().unwrap()[0]
                .last_github_sync_state,
            "pending",
            "旧代次同步不能覆盖新数据的待同步状态"
        );
        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = fs::remove_dir_all(dir);
    }
}

//! Stage 6 — GitHub Secrets 槽位同步（开发指南 §8.5 / 阶段 6）。
//!
//! 本模块只负责把本地最新签到凭证（access token + checkin_device_id）同步到
//! 已有 GitHub Actions 仓库的 Secrets。**绝不**在本地执行签到（claim）。
//!
//! 所有 `gh` 子进程调用都抽象为 `GitHubRunner`，测试用 `FakeGitHubRunner`
//! 验证参数不含 token、token 走 stdin、两个 secret 都被调用、且首个失败后
//! 不报告整体成功（开发指南 §8.5 自动测试要求）。

use std::path::PathBuf;
use std::sync::Mutex;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use crate::models::trae::TraeAccount;
use crate::models::work_cn::{
    WorkCnGitHubCliStatus, WorkCnGitHubConfig, WorkCnGitHubSlot, WorkCnGitHubSyncResult,
};

/// Output of a single `gh` invocation.
#[derive(Debug, Clone)]
pub struct GitHubRunOutput {
    pub status_success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Abstraction over running `gh`. Real path spawns the CLI; tests use a fake.
pub trait GitHubRunner: Send + Sync {
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<GitHubRunOutput, String>;
}

/// Real runner that shells out to `gh` (Windows: `gh.exe` is on PATH after
/// `gh auth login`). Secret values are written to stdin only and never placed
/// on the command line.
pub struct RealGitHubRunner;

impl GitHubRunner for RealGitHubRunner {
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<GitHubRunOutput, String> {
        use std::process::{Command, Stdio};

        let mut cmd = Command::new("gh");
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| format!("无法启动 gh：{e}"))?;

        if let Some(input) = stdin {
            if let Some(mut stdin_pipe) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin_pipe.write_all(input.as_bytes());
                // drop stdin_pipe to close the pipe so gh sees EOF
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("等待 gh 进程失败：{e}"))?;
        Ok(GitHubRunOutput {
            status_success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: redact_for_log(&String::from_utf8_lossy(&output.stderr)),
        })
    }
}

/// Records every invocation so tests can assert on args + stdin without touching
/// the network or GitHub.
pub struct FakeGitHubRunner {
    pub calls: Mutex<Vec<FakeCall>>,
    pub cli_available: bool,
    pub auth_ok: bool,
    pub secret_set_ok: bool,
}

#[derive(Debug, Clone)]
pub struct FakeCall {
    pub args: Vec<String>,
    pub stdin: Option<String>,
}

impl FakeGitHubRunner {
    pub fn new() -> Self {
        FakeGitHubRunner {
            calls: Mutex::new(Vec::new()),
            cli_available: true,
            auth_ok: true,
            secret_set_ok: true,
        }
    }

    pub fn recorded_calls(&self) -> Vec<FakeCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl Default for FakeGitHubRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl GitHubRunner for FakeGitHubRunner {
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<GitHubRunOutput, String> {
        self.calls.lock().unwrap().push(FakeCall {
            args: args.iter().map(|s| s.to_string()).collect(),
            stdin: stdin.map(|s| s.to_string()),
        });
        let first = args.first().copied().unwrap_or("");
        if first == "--version" {
            return Ok(GitHubRunOutput {
                status_success: self.cli_available,
                stdout: "gh version 2.0.0".to_string(),
                stderr: String::new(),
            });
        }
        if first == "auth" {
            return Ok(GitHubRunOutput {
                status_success: self.auth_ok,
                stdout: String::new(),
                stderr: if self.auth_ok {
                    String::new()
                } else {
                    "gh: not logged in".to_string()
                },
            });
        }
        if first == "secret" {
            return Ok(GitHubRunOutput {
                status_success: self.secret_set_ok,
                stdout: String::new(),
                stderr: if self.secret_set_ok {
                    String::new()
                } else {
                    "gh: secret set failed".to_string()
                },
            });
        }
        Ok(GitHubRunOutput {
            status_success: true,
            stdout: String::new(),
            stderr: String::new(),
        })
    }
}

/// Parse `exp` (seconds) out of a JWT access token's payload segment.
pub fn parse_jwt_exp(token: &str) -> Option<i64> {
    let parts: Vec<&str> = token.split('.').collect();
    let payload = parts.get(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    value.get("exp")?.as_i64()
}

/// Replace any long base64-ish / JWT-ish run with `[REDACTED]` so logs never leak
/// a token, refresh token, private key, or GitHub PAT (开发指南 §13.4 / §14.4).
pub fn redact_for_log(input: &str) -> String {
    let re = regex_literal_secret();
    re.replace_all(input, "[REDACTED]").into_owned()
}

/// Small inline matcher for secret-like runs (>= 20 chars of base64url alphabet).
fn regex_literal_secret() -> regex::Regex {
    // Lazily compiled once per call is fine for logging volume here.
    regex::Regex::new(r"[A-Za-z0-9_-]{20,}").unwrap()
}

/// `owner/repo` form, no PAT, no trailing slash.
pub fn validate_repository(repository: &str) -> Result<(), String> {
    let repo = repository.trim();
    if repo.is_empty() {
        return Err("GitHub 仓库不能为空".to_string());
    }
    let mut parts = repo.splitn(2, '/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if name.is_empty() || !repo.contains('/') || repo.matches('/').count() != 1 {
        return Err("GitHub 仓库需为 owner/repo 形式".to_string());
    }
    if !owner
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("GitHub 仓库 owner 含非法字符".to_string());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("GitHub 仓库名含非法字符".to_string());
    }
    Ok(())
}

/// Validate the persisted config: slot range, no duplicate slot/account, secret
/// names `[A-Z0-9_]+`, repository form.
pub fn validate_github_config(config: &WorkCnGitHubConfig) -> Result<(), String> {
    if config.enabled && config.repository.is_empty() {
        return Err("启用 GitHub 同步时必须填写仓库 owner/repo".to_string());
    }
    if config.enabled {
        validate_repository(&config.repository)?;
    }
    let mut seen_slots = std::collections::HashSet::new();
    let mut seen_accounts = std::collections::HashSet::new();
    for slot in &config.slots {
        if !(1..=4).contains(&slot.slot) {
            return Err(format!("槽位 {} 必须在 1~4 之间", slot.slot));
        }
        if !seen_slots.insert(slot.slot) {
            return Err(format!("槽位 {} 被重复绑定", slot.slot));
        }
        if !seen_accounts.insert(slot.account_id.clone()) {
            return Err(format!("账号 {} 绑定了多个槽位", slot.account_id));
        }
        if !slot.token_secret.is_empty() && !is_secret_name(&slot.token_secret) {
            return Err(format!("secret 名 '{}' 只能是 [A-Z0-9_]+", slot.token_secret));
        }
        if !slot.device_secret.is_empty() && !is_secret_name(&slot.device_secret) {
            return Err(format!("secret 名 '{}' 只能是 [A-Z0-9_]+", slot.device_secret));
        }
    }
    Ok(())
}

fn is_secret_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Resolve a slot bound to an account id, if any.
pub fn find_slot_for_account<'a>(
    config: &'a WorkCnGitHubConfig,
    account_id: &str,
) -> Option<&'a WorkCnGitHubSlot> {
    config.slots.iter().find(|s| s.account_id == account_id)
}

/// 供 watcher/切换链路注入 FakeRunner 的可测版本：读取配置 → 启用校验 →
/// 槽位绑定校验 → 真实同步。未启用/未绑定返回 `Ok(skipped)`，不触任何 gh 调用。
pub(crate) fn sync_account_secrets_if_bound_with(
    runner: &dyn GitHubRunner,
    account: &TraeAccount,
) -> Result<WorkCnGitHubSyncResult, String> {
    let config = load_github_config();
    if !config.enabled {
        return Ok(skip_result(&account.id, "GitHub 同步未启用"));
    }
    let Some(slot) = find_slot_for_account(&config, &account.id) else {
        return Ok(skip_result(&account.id, "该账号未绑定 GitHub 槽位"));
    };
    sync_account_secrets(runner, account, slot, &config.repository)
}

/// 生产入口（命令层、切换链路、watcher 均走这里）。
pub(crate) fn sync_account_secrets_if_bound(
    account: &TraeAccount,
) -> Result<WorkCnGitHubSyncResult, String> {
    sync_account_secrets_if_bound_with(&RealGitHubRunner, account)
}

/// 构造「跳过」结果，与命令层原逻辑保持一致。
fn skip_result(account_id: &str, reason: &str) -> WorkCnGitHubSyncResult {
    WorkCnGitHubSyncResult {
        account_id: account_id.to_string(),
        synced: false,
        skipped: true,
        skip_reason: Some(reason.to_string()),
        error: None,
        synced_at: chrono::Utc::now().timestamp(),
    }
}

/// Is the GitHub CLI installed?
pub fn github_cli_available(runner: &dyn GitHubRunner) -> bool {
    runner
        .run(&["--version"], None)
        .map(|o| o.status_success)
        .unwrap_or(false)
}

/// Verify `gh auth status` succeeds.
pub fn github_auth_status(runner: &dyn GitHubRunner) -> Result<(), String> {
    let out = runner
        .run(&["auth", "status"], None)
        .map_err(|e| format!("运行 gh auth status 失败：{e}"))?;
    if out.status_success {
        Ok(())
    } else {
        Err(format!("gh 未登录：{}", out.stderr))
    }
}

/// Sync one account's credentials to its GitHub slot. Never claims check-in.
///
/// Returns `Ok(WorkCnGitHubSyncResult)` for intentional skips (expired token /
/// missing device id); returns `Err(String)` for hard failures (gh not authed,
/// a secret-set failed). The first secret failing short-circuits before the
/// second is attempted, so the overall result is never reported as success.
pub fn sync_account_secrets(
    runner: &dyn GitHubRunner,
    account: &TraeAccount,
    slot: &WorkCnGitHubSlot,
    repository: &str,
) -> Result<WorkCnGitHubSyncResult, String> {
    let now = chrono::Utc::now().timestamp();
    let synced_at = now;

    // 1) token must carry a valid, unexpired exp
    let exp = parse_jwt_exp(&account.access_token)
        .ok_or_else(|| "无法解析 access token 的 exp，跳过同步".to_string())?;
    if exp <= now {
        return Ok(WorkCnGitHubSyncResult {
            account_id: account.id.clone(),
            synced: false,
            skipped: true,
            skip_reason: Some("access token 已过期，跳过同步".to_string()),
            error: None,
            synced_at,
        });
    }

    // 2) device id required for the check-in workflow
    let device_id = match account.checkin_device_id.as_deref() {
        Some(id) if !id.is_empty() => id,
        _ => {
            return Ok(WorkCnGitHubSyncResult {
                account_id: account.id.clone(),
                synced: false,
                skipped: true,
                skip_reason: Some("缺少 checkin_device_id，跳过同步".to_string()),
                error: None,
                synced_at,
            });
        }
    };

    // 3) gh must be authed
    github_auth_status(runner).map_err(|e| format!("GitHub 同步中止：{e}"))?;

    let token_secret = if slot.token_secret.is_empty() {
        format!("TRAE{}_TOKEN", slot.slot)
    } else {
        slot.token_secret.clone()
    };
    let device_secret = if slot.device_secret.is_empty() {
        format!("TRAE{}_DEVICE_ID", slot.slot)
    } else {
        slot.device_secret.clone()
    };

    // 4) set token secret (stdin only)
    let r1 = runner
        .run(
            &["secret", "set", &token_secret, "--repo", repository],
            Some(&account.access_token),
        )
        .map_err(|e| format!("设置 {token_secret} 失败：{e}"))?;
    if !r1.status_success {
        return Err(format!("设置 {token_secret} 失败：{}", r1.stderr));
    }

    // 5) set device id secret (stdin only)
    let r2 = runner
        .run(
            &["secret", "set", &device_secret, "--repo", repository],
            Some(device_id),
        )
        .map_err(|e| format!("设置 {device_secret} 失败：{e}"))?;
    if !r2.status_success {
        return Err(format!("设置 {device_secret} 失败：{}", r2.stderr));
    }

    Ok(WorkCnGitHubSyncResult {
        account_id: account.id.clone(),
        synced: true,
        skipped: false,
        skip_reason: None,
        error: None,
        synced_at,
    })
}

/// Path to the persisted `github.json` (next to the account store, never a token).
pub fn github_config_path() -> Result<PathBuf, String> {
    let dir = crate::modules::account::get_data_dir()?;
    Ok(dir.join("github.json"))
}

pub fn load_github_config() -> WorkCnGitHubConfig {
    let path = match github_config_path() {
        Ok(p) => p,
        Err(_) => return WorkCnGitHubConfig::default(),
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => WorkCnGitHubConfig::default(),
    }
}

pub fn save_github_config(config: &WorkCnGitHubConfig) -> Result<(), String> {
    validate_github_config(config)?;
    let path = github_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败：{e}"))?;
    }
    let text = serde_json::to_string_pretty(config)
        .map_err(|e| format!("序列化 GitHub 配置失败：{e}"))?;
    std::fs::write(&path, text).map_err(|e| format!("写入 GitHub 配置失败：{e}"))?;
    Ok(())
}

/// Convenience: build a CLI status object from a runner.
pub fn cli_status(runner: &dyn GitHubRunner) -> WorkCnGitHubCliStatus {
    let available = github_cli_available(runner);
    if !available {
        return WorkCnGitHubCliStatus {
            available: false,
            authed: false,
            detail: Some("未检测到 GitHub CLI（gh）".to_string()),
        };
    }
    match github_auth_status(runner) {
        Ok(()) => WorkCnGitHubCliStatus {
            available: true,
            authed: true,
            detail: None,
        },
        Err(e) => WorkCnGitHubCliStatus {
            available: true,
            authed: false,
            detail: Some(redact_for_log(&e)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::trae::TraeAccount;

    fn make_account(id: &str, token: &str, device: Option<&str>) -> TraeAccount {
        TraeAccount {
            id: id.to_string(),
            email: format!("{id}@example.com"),
            user_id: Some("u1".to_string()),
            nickname: None,
            tags: None,
            access_token: token.to_string(),
            refresh_token: Some("rt".to_string()),
            token_type: Some("Bearer".to_string()),
            expires_at: None,
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: None,
            trae_profile_raw: None,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: None,
            checkin_device_id: device.map(|s| s.to_string()),
            machine_id: None,
            auth_device_id: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: 0,
            last_used: 0,
        }
    }

    fn jwt_with_exp(exp: i64) -> String {
        // header.payload.signature — payload carries exp
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(format!("{{\"exp\":{exp}}}").as_bytes());
        format!("eyJhbGciOiJIUzI1NiJ9.{payload}.signature")
    }

    #[test]
    fn work_cn_github_parse_jwt_exp_roundtrip() {
        let exp = 9_999_999_999i64;
        let token = jwt_with_exp(exp);
        assert_eq!(parse_jwt_exp(&token), Some(exp));
        assert_eq!(parse_jwt_exp("not-a-jwt"), None);
    }

    #[test]
    fn work_cn_github_validate_repository_rules() {
        assert!(validate_repository("lk1015646426/daily-checkin").is_ok());
        assert!(validate_repository("owner/repo/extra").is_err());
        assert!(validate_repository("nopath").is_err());
        assert!(validate_repository("").is_err());
    }

    #[test]
    fn work_cn_github_validate_config_rejects_dup_slot_and_bad_secret() {
        let cfg = WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![
                WorkCnGitHubSlot {
                    slot: 1,
                    account_id: "a".to_string(),
                    token_secret: String::new(),
                    device_secret: String::new(),
                },
                WorkCnGitHubSlot {
                    slot: 1,
                    account_id: "b".to_string(),
                    token_secret: String::new(),
                    device_secret: String::new(),
                },
            ],
        };
        assert!(validate_github_config(&cfg).is_err(), "重复槽位必须被拒绝");

        let cfg2 = WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![WorkCnGitHubSlot {
                slot: 5,
                account_id: "a".to_string(),
                token_secret: String::new(),
                device_secret: String::new(),
            }],
        };
        assert!(validate_github_config(&cfg2).is_err(), "槽位越界必须被拒绝");

        let cfg3 = WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![WorkCnGitHubSlot {
                slot: 1,
                account_id: "a".to_string(),
                token_secret: "bad-name".to_string(),
                device_secret: String::new(),
            }],
        };
        assert!(validate_github_config(&cfg3).is_err(), "非法 secret 名必须被拒绝");
    }

    #[test]
    fn work_cn_github_sync_calls_two_secrets_with_stdin_not_args() {
        let runner = FakeGitHubRunner::new();
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("dev-uuid"));
        let slot = WorkCnGitHubSlot {
            slot: 1,
            account_id: "acc1".to_string(),
            token_secret: String::new(),
            device_secret: String::new(),
        };
        let result = sync_account_secrets(&runner, &account, &slot, "o/r").unwrap();
        assert!(result.synced, "应同步成功");
        assert!(!result.skipped);

        let calls = runner.recorded_calls();
        // auth + 2 secret sets = 3 calls
        assert_eq!(calls.len(), 3, "应调用 auth status + 两个 secret set");
        let secret_calls: Vec<&FakeCall> = calls
            .iter()
            .filter(|c| c.args.first().map(|s| s.as_str()) == Some("secret"))
            .collect();
        assert_eq!(secret_calls.len(), 2, "应恰好设置两个 secret");

        for c in &secret_calls {
            // token/device must NOT appear on the command line
            let joined = c.args.join(" ");
            assert!(!joined.contains(&account.access_token), "token 不能出现在参数里");
            assert!(!joined.contains("dev-uuid"), "device id 不能出现在参数里");
            // token goes to stdin of the first, device id to stdin of the second
            assert!(c.stdin.is_some(), "secret 值必须走 stdin");
        }
        // first secret call carries the token, second carries the device id
        assert_eq!(secret_calls[0].stdin.as_deref(), Some(account.access_token.as_str()));
        assert_eq!(secret_calls[1].stdin.as_deref(), Some("dev-uuid"));
        // default secret names auto-generated
        assert!(secret_calls[0].args.contains(&"TRAE1_TOKEN".to_string()));
        assert!(secret_calls[1].args.contains(&"TRAE1_DEVICE_ID".to_string()));
    }

    #[test]
    fn work_cn_github_first_secret_failure_is_not_overall_success() {
        let runner = FakeGitHubRunner {
            secret_set_ok: false,
            ..FakeGitHubRunner::new()
        };
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("dev-uuid"));
        let slot = WorkCnGitHubSlot {
            slot: 1,
            account_id: "acc1".to_string(),
            token_secret: String::new(),
            device_secret: String::new(),
        };
        let result = sync_account_secrets(&runner, &account, &slot, "o/r");
        assert!(result.is_err(), "第一个 secret 失败必须返回 Err（非整体成功）");
    }

    #[test]
    fn work_cn_github_expired_token_is_skipped_not_error() {
        let runner = FakeGitHubRunner::new();
        let token = jwt_with_exp(chrono::Utc::now().timestamp() - 3600); // already expired
        let account = make_account("acc1", &token, Some("dev-uuid"));
        let slot = WorkCnGitHubSlot {
            slot: 1,
            account_id: "acc1".to_string(),
            token_secret: String::new(),
            device_secret: String::new(),
        };
        let result = sync_account_secrets(&runner, &account, &slot, "o/r").unwrap();
        assert!(!result.synced);
        assert!(result.skipped);
        assert!(result.skip_reason.is_some());
        // expired token must NOT trigger any gh call
        assert!(runner.recorded_calls().is_empty());
    }

    #[test]
    fn work_cn_github_missing_device_id_is_skipped() {
        let runner = FakeGitHubRunner::new();
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, None);
        let slot = WorkCnGitHubSlot {
            slot: 1,
            account_id: "acc1".to_string(),
            token_secret: String::new(),
            device_secret: String::new(),
        };
        let result = sync_account_secrets(&runner, &account, &slot, "o/r").unwrap();
        assert!(!result.synced);
        assert!(result.skipped);
        assert!(runner.recorded_calls().is_empty(), "缺 device id 不应调用 gh");
    }

    #[test]
    fn work_cn_github_not_authed_is_hard_error() {
        let runner = FakeGitHubRunner {
            auth_ok: false,
            ..FakeGitHubRunner::new()
        };
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("dev-uuid"));
        let slot = WorkCnGitHubSlot {
            slot: 1,
            account_id: "acc1".to_string(),
            token_secret: String::new(),
            device_secret: String::new(),
        };
        let result = sync_account_secrets(&runner, &account, &slot, "o/r");
        assert!(result.is_err(), "gh 未登录必须返回 Err");
    }

    #[test]
    fn work_cn_github_redact_masks_long_secrets() {
        let log = "token=eyJabcDEF1234567890xyz token2=short ok";
        let redacted = redact_for_log(log);
        assert!(!redacted.contains("eyJabcDEF1234567890xyz"), "长 secret 必须被脱敏");
        assert!(redacted.contains("ok"));
    }

    #[test]
    fn work_cn_github_sync_if_bound_disabled_returns_skipped() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "work-cn-gh-disabled-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        let runner = FakeGitHubRunner::new();
        let account = make_account("acc1", "tok", Some("dev"));
        let result = sync_account_secrets_if_bound_with(&runner, &account).unwrap();
        assert!(!result.synced);
        assert!(result.skipped);
        assert_eq!(result.skip_reason.as_deref(), Some("GitHub 同步未启用"));
        assert!(runner.recorded_calls().is_empty());

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_cn_github_sync_if_bound_unbound_returns_skipped() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "work-cn-gh-unbound-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        save_github_config(&WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![],
        })
        .expect("save config");

        let runner = FakeGitHubRunner::new();
        let account = make_account("acc1", "tok", Some("dev"));
        let result = sync_account_secrets_if_bound_with(&runner, &account).unwrap();
        assert!(!result.synced);
        assert!(result.skipped);
        assert_eq!(result.skip_reason.as_deref(), Some("该账号未绑定 GitHub 槽位"));
        assert!(runner.recorded_calls().is_empty());

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_cn_github_sync_if_bound_bound_calls_sync() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "work-cn-gh-bound-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        save_github_config(&WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![WorkCnGitHubSlot {
                slot: 1,
                account_id: "acc1".to_string(),
                token_secret: String::new(),
                device_secret: String::new(),
            }],
        })
        .expect("save config");

        let runner = FakeGitHubRunner::new();
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("dev-uuid"));
        let result = sync_account_secrets_if_bound_with(&runner, &account).unwrap();
        assert!(result.synced, "绑定 + 启用 + 已登录应同步成功");
        assert!(!result.skipped);

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 3, "应调用 auth status + 两个 secret set");
        assert_eq!(calls[0].args.first().map(|s| s.as_str()), Some("auth"));
        assert_eq!(calls[1].args.first().map(|s| s.as_str()), Some("secret"));
        assert_eq!(calls[2].args.first().map(|s| s.as_str()), Some("secret"));

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_cn_github_sync_if_bound_not_authed_returns_err() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "work-cn-gh-noauth-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        save_github_config(&WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![WorkCnGitHubSlot {
                slot: 1,
                account_id: "acc1".to_string(),
                token_secret: String::new(),
                device_secret: String::new(),
            }],
        })
        .expect("save config");

        let runner = FakeGitHubRunner {
            auth_ok: false,
            ..FakeGitHubRunner::new()
        };
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("dev-uuid"));
        let result = sync_account_secrets_if_bound_with(&runner, &account);
        assert!(result.is_err(), "gh 未登录必须返回 Err");

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

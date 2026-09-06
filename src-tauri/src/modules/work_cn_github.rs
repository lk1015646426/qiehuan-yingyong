//! Stage 6 — GitHub Secrets 槽位同步（开发指南 §8.5 / 阶段 6）。
//!
//! 本模块只负责把本地最新签到凭证（access token + 官方数字设备上下文）同步到
//! 已有 GitHub Actions 仓库的 Secrets。**绝不**在本地执行签到（claim）。
//!
//! 所有 `gh` 子进程调用都抽象为 `GitHubRunner`，测试用 `FakeGitHubRunner`
//! 验证参数不含凭证、secret 值走 stdin、四个 secret 都被调用、且首个失败后
//! 不报告整体成功（开发指南 §8.5 自动测试要求）。

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex, MutexGuard};

use crate::models::trae::TraeAccount;
use crate::models::work_cn::{
    WorkCnGitHubCliStatus, WorkCnGitHubConfig, WorkCnGitHubSlot, WorkCnGitHubSyncResult,
};

// Serialize every Work CN GitHub write. A switch, the session watcher, and a
// manual sync can otherwise race and let an older snapshot overwrite a newer
// token/device pair.
static WORK_CN_GITHUB_SYNC_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn lock_work_cn_github_sync() -> MutexGuard<'static, ()> {
    WORK_CN_GITHUB_SYNC_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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

fn should_retry_github_with_proxy(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "connect",
        "connection",
        "network",
        "timeout",
        "timed out",
        "tls",
        "ssl",
        "proxy",
        "eof",
        "api.github.com",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[cfg(test)]
mod network_route_tests {
    use super::{normalize_proxy_url, select_proxy_server, should_retry_github_with_proxy};

    #[test]
    fn network_failures_are_eligible_for_vpn_retry() {
        assert!(should_retry_github_with_proxy(
            "Get https://api.github.com: dial tcp: connection timed out"
        ));
    }

    #[test]
    fn authentication_failures_are_not_retried_as_network_failures() {
        assert!(!should_retry_github_with_proxy("gh: not logged in"));
    }

    #[test]
    fn windows_proxy_server_prefers_https_then_http_entries() {
        assert_eq!(
            select_proxy_server("http=127.0.0.1:7890;https=127.0.0.1:7897"),
            Some("127.0.0.1:7897")
        );
        assert_eq!(
            select_proxy_server("http=127.0.0.1:7890"),
            Some("127.0.0.1:7890")
        );
    }

    #[test]
    fn windows_proxy_server_accepts_bare_address_and_explicit_scheme() {
        assert_eq!(
            select_proxy_server("127.0.0.1:7897").map(normalize_proxy_url),
            Some("http://127.0.0.1:7897".to_string())
        );
        assert_eq!(
            select_proxy_server("https://127.0.0.1:7897").map(normalize_proxy_url),
            Some("https://127.0.0.1:7897".to_string())
        );
    }
}

/// 定位 `gh` 可执行文件：优先探测 MSI 默认安装目录（安装后注册表 PATH 更新，
/// 但运行中的进程环境不会刷新），再回退扫描当前进程 PATH。
pub fn resolve_gh_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        for candidate in [
            r"C:\Program Files\GitHub CLI\gh.exe",
            r"C:\Program Files (x86)\GitHub CLI\gh.exe",
        ] {
            let p = PathBuf::from(candidate);
            if p.is_file() {
                return Some(p);
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let p = PathBuf::from(local).join(r"Programs\GitHub CLI\gh.exe");
            if p.is_file() {
                return Some(p);
            }
        }
    }
    let exe_name = if cfg!(windows) { "gh.exe" } else { "gh" };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let p = dir.join(exe_name);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

impl GitHubRunner for RealGitHubRunner {
    fn run(&self, args: &[&str], stdin: Option<&str>) -> Result<GitHubRunOutput, String> {
        let direct = run_gh_once(args, stdin, None);
        let direct_error = match direct {
            Ok(output) if output.status_success => return Ok(output),
            Ok(output) => {
                let detail = format!("{} {}", output.stdout, output.stderr);
                if !should_retry_github_with_proxy(&detail) {
                    return Ok(output);
                }
                redact_for_log(&output.stderr)
            }
            Err(error) => {
                if !should_retry_github_with_proxy(&error) {
                    return Err(error);
                }
                redact_for_log(&error)
            }
        };

        let Some(proxy_url) = github_proxy_url() else {
            return Err(format!(
                "GitHub 直连失败，未检测到可用 VPN 代理，请开启 VPN 后重试：{}",
                direct_error
            ));
        };
        match run_gh_once(args, stdin, Some(&proxy_url)) {
            Ok(output) if output.status_success => Ok(output),
            Ok(output) => Err(format!(
                "GitHub 直连失败，VPN 代理重试也失败：{}",
                redact_for_log(&output.stderr)
            )),
            Err(error) => Err(format!(
                "GitHub 直连失败，VPN 代理重试也失败：{}",
                redact_for_log(&error)
            )),
        }
    }
}

fn github_proxy_url() -> Option<String> {
    for key in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(normalize_proxy_url(value));
            }
        }
    }
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        use winreg::RegKey;
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
            .ok()?;
        if key.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 0 {
            return None;
        }
        let value = key.get_value::<String, _>("ProxyServer").ok()?;
        let value = select_proxy_server(&value)?;
        (!value.is_empty()).then(|| normalize_proxy_url(value))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn select_proxy_server(value: &str) -> Option<&str> {
    let entries: Vec<&str> = value
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    for scheme in ["https", "http"] {
        if let Some(proxy) = entries.iter().find_map(|entry| {
            let (key, proxy) = entry.split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case(scheme)
                .then_some(proxy.trim())
        }) {
            if !proxy.is_empty() {
                return Some(proxy);
            }
        }
    }
    entries.into_iter().find(|entry| !entry.contains('='))
}

fn normalize_proxy_url(value: &str) -> String {
    if value.contains("://") {
        value.to_string()
    } else {
        format!("http://{value}")
    }
}

fn run_gh_once(
    args: &[&str],
    stdin: Option<&str>,
    proxy_url: Option<&str>,
) -> Result<GitHubRunOutput, String> {
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    let gh = resolve_gh_path()
        .ok_or_else(|| "未检测到 GitHub CLI（gh）：请先安装并登录 gh".to_string())?;
    let mut cmd = Command::new(gh);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        cmd.env_remove(key);
    }
    if let Some(proxy_url) = proxy_url {
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            cmd.env(key, proxy_url);
        }
    }
    // GUI 应用 spawn 控制台程序时 Windows 会弹黑色命令行窗口，
    // 必须显式 CREATE_NO_WINDOW（gh 是控制台程序，每次调用都会闪窗）。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("无法启动 gh：{e}"))?;

    if let Some(input) = stdin {
        if let Some(mut stdin_pipe) = child.stdin.take() {
            use std::io::Write;
            let _ = stdin_pipe.write_all(input.as_bytes());
            // drop stdin_pipe to close the pipe so gh sees EOF
        }
    }

    // 超时看门狗：`gh auth status` 会联网验证 token，网络黑洞时可挂住很久。
    // 探测类调用（--version / auth）30s，写 Secrets 类调用放宽到 180s；
    // 超时后强杀进程树并返回明确错误，绝不让一次 gh 调用无限期挂起。
    let timeout_secs = match args.first() {
        Some(&"auth") | Some(&"--version") => 30u64,
        _ => 180,
    };
    let timed_out = Arc::new(AtomicBool::new(false));
    let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
    let flag = Arc::clone(&timed_out);
    let child_ref = Arc::new(Mutex::new(Some(child)));
    let watcher_child = Arc::clone(&child_ref);
    let watchdog = std::thread::spawn(move || {
        if cancel_rx
            .recv_timeout(Duration::from_secs(timeout_secs))
            .is_err()
        {
            flag.store(true, Ordering::SeqCst);
            // 从共享槽位取走 child 并强杀（取不走说明主流程刚好已结束）。
            if let Some(mut c) = watcher_child.lock().unwrap().take() {
                let _ = c.kill();
            }
        }
    });

    // 与看门狗竞争取 child：正常路径等待输出；被看门狗抢先（超时强杀）则
    // 走下方 timed_out 错误分支，绝不 panic。
    let output = match child_ref.lock().unwrap().take() {
        Some(c) => c
            .wait_with_output()
            .map_err(|e| format!("等待 gh 进程失败：{e}")),
        None => Err("gh 进程被超时终止".to_string()),
    };
    let _ = cancel_tx.send(());
    let _ = watchdog.join();

    if timed_out.load(Ordering::SeqCst) {
        return Err(format!(
            "gh 响应超时（>{timeout_secs} 秒），请检查网络后重试"
        ));
    }
    let output = output?;
    Ok(GitHubRunOutput {
        status_success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: redact_for_log(&String::from_utf8_lossy(&output.stderr)),
    })
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
/// 实现已抽取到 `utils::jwt`，此处转发以保持既有调用点稳定。
pub fn parse_jwt_exp(token: &str) -> Option<i64> {
    crate::utils::jwt::parse_jwt_exp(token)
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
        if slot.slot < 1 {
            return Err(format!("槽位 {} 必须从 1 开始", slot.slot));
        }
        if !seen_slots.insert(slot.slot) {
            return Err(format!("槽位 {} 被重复绑定", slot.slot));
        }
        if !seen_accounts.insert(slot.account_id.clone()) {
            return Err(format!("账号 {} 绑定了多个槽位", slot.account_id));
        }
        if !slot.token_secret.is_empty() && !is_secret_name(&slot.token_secret) {
            return Err(format!(
                "secret 名 '{}' 只能是 [A-Z0-9_]+",
                slot.token_secret
            ));
        }
        if !slot.device_secret.is_empty() && !is_secret_name(&slot.device_secret) {
            return Err(format!(
                "secret 名 '{}' 只能是 [A-Z0-9_]+",
                slot.device_secret
            ));
        }
        configured_secret_stem(slot)?;
    }
    Ok(())
}

fn is_secret_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// 从账号槽位名推导 GitHub secret 名主干：大写化并把非法字符折叠为 `_`；
/// 纯非 ASCII 名（如中文备注）清洗后为空，依次回退
/// tags[0] → 昵称 → 邮箱前缀 → user_id → 账号 id。
/// GitHub 规定 secret 不能以数字或 `GITHUB_` 开头，此前补 `A_` 前缀。
pub fn slot_secret_stem(account: &TraeAccount) -> String {
    let email_local = account.email.split('@').next().unwrap_or("");
    let email_candidate = if email_local.eq_ignore_ascii_case("unknown") {
        String::new()
    } else {
        email_local.to_string()
    };
    let candidates = [
        account
            .tags
            .as_ref()
            .and_then(|t| t.first().cloned())
            .unwrap_or_default(),
        account.nickname.clone().unwrap_or_default(),
        email_candidate,
        account.user_id.clone().unwrap_or_default(),
        account.id.clone(),
    ];
    for candidate in candidates {
        let stem = sanitize_secret_stem(&candidate);
        if !stem.is_empty() {
            return stem;
        }
    }
    "ACCOUNT".to_string()
}

fn configured_secret_stem(slot: &WorkCnGitHubSlot) -> Result<Option<String>, String> {
    let token_stem = if slot.token_secret.is_empty() {
        None
    } else {
        Some(
            slot.token_secret
                .strip_suffix("_TOKEN")
                .unwrap_or(slot.token_secret.as_str()),
        )
    };
    let device_stem = if slot.device_secret.is_empty() {
        None
    } else {
        Some(
            slot.device_secret
                .strip_suffix("_DEVICE_ID")
                .unwrap_or(slot.device_secret.as_str()),
        )
    };
    match (token_stem, device_stem) {
        (None, None) => Ok(None),
        (Some(stem), None) | (None, Some(stem)) if !stem.is_empty() => Ok(Some(stem.to_string())),
        (Some(token), Some(device)) if !token.is_empty() && token == device => {
            Ok(Some(token.to_string()))
        }
        _ => Err(format!(
            "槽位 {} 的 token/device secret 必须使用同一主干",
            slot.slot
        )),
    }
}

fn resolved_secret_names(
    account: &TraeAccount,
    slot: &WorkCnGitHubSlot,
) -> Result<[String; 4], String> {
    let stem = configured_secret_stem(slot)?.unwrap_or_else(|| slot_secret_stem(account));
    Ok([
        if slot.token_secret.is_empty() {
            format!("{stem}_TOKEN")
        } else {
            slot.token_secret.clone()
        },
        if slot.device_secret.is_empty() {
            format!("{stem}_DEVICE_ID")
        } else {
            slot.device_secret.clone()
        },
        format!("{stem}_DEVICE_BRAND"),
        format!("{stem}_DEVICE_TYPE"),
    ])
}

fn sanitize_secret_stem(raw: &str) -> String {
    let mut s = String::new();
    let mut last_underscore = false;
    for c in raw.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_uppercase());
            last_underscore = false;
        } else if !last_underscore {
            s.push('_');
            last_underscore = true;
        }
    }
    while s.starts_with('_') {
        s.remove(0);
    }
    while s.ends_with('_') {
        s.pop();
    }
    if s.starts_with(|c: char| c.is_ascii_digit()) || s.starts_with("GITHUB_") {
        s.insert_str(0, "A_");
    }
    s
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
    let _sync_guard = lock_work_cn_github_sync();
    sync_account_secrets_if_bound_unlocked(runner, account)
}

fn sync_account_secrets_if_bound_unlocked(
    runner: &dyn GitHubRunner,
    account: &TraeAccount,
) -> Result<WorkCnGitHubSyncResult, String> {
    let config = load_github_config();
    if !config.enabled {
        return Ok(skip_result(&account.id, "GitHub 同步未启用"));
    }
    validate_github_config(&config)?;
    let Some(slot) = find_slot_for_account(&config, &account.id) else {
        return Ok(skip_result(&account.id, "该账号未绑定 GitHub 槽位"));
    };
    if crate::modules::trae_account::validate_work_cn_account_for_switch(account).is_err() {
        return Ok(skip_result(
            &account.id,
            "官方设备快照不完整，已跳过 GitHub 同步",
        ));
    }
    sync_account_secrets_unlocked(runner, account, slot, &config.repository)
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
    let _sync_guard = lock_work_cn_github_sync();
    sync_account_secrets_unlocked(runner, account, slot, repository)
}

fn sync_account_secrets_unlocked(
    runner: &dyn GitHubRunner,
    account: &TraeAccount,
    slot: &WorkCnGitHubSlot,
    repository: &str,
) -> Result<WorkCnGitHubSyncResult, String> {
    let platform = crate::modules::trae_account::resolve_account_platform_kind(account);
    let (device_brand, device_type) =
        crate::modules::trae_oauth::official_checkin_device_headers(platform);
    sync_account_secrets_with_context(
        runner,
        account,
        slot,
        repository,
        device_brand.as_str(),
        device_type.as_str(),
    )
}

fn sync_account_secrets_with_context(
    runner: &dyn GitHubRunner,
    account: &TraeAccount,
    slot: &WorkCnGitHubSlot,
    repository: &str,
    device_brand: &str,
    device_type: &str,
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

    // 2) official numeric device id required for the check-in workflow
    let device_id = match crate::modules::trae_account::resolve_official_checkin_device_id(account)
    {
        Ok(id) => id,
        Err(_) => {
            return Ok(WorkCnGitHubSyncResult {
                account_id: account.id.clone(),
                synced: false,
                skipped: true,
                skip_reason: Some("缺少数字 auth_device_id，跳过同步".to_string()),
                error: None,
                synced_at,
            });
        }
    };

    // 3) gh must be authed
    github_auth_status(runner).map_err(|e| format!("GitHub 同步中止：{e}"))?;

    // 默认 secret 名以本地账号槽位名命名（如备注「backup-1」→ BACKUP_1_TOKEN），
    // 手动配置过的名字优先。
    let [token_secret, device_secret, device_brand_secret, device_type_secret] =
        resolved_secret_names(account, slot)?;

    // 4) set every value through stdin only. Ordering is stable for tests and
    // makes the token failure short-circuit before any device metadata write.
    for (secret_name, secret_value) in [
        (token_secret.as_str(), account.access_token.as_str()),
        (device_secret.as_str(), device_id),
        (device_brand_secret.as_str(), device_brand.trim()),
        (device_type_secret.as_str(), device_type.trim()),
    ] {
        let output = runner
            .run(
                &["secret", "set", secret_name, "--repo", repository],
                Some(secret_value),
            )
            .map_err(|e| format!("设置 {secret_name} 失败：{e}"))?;
        if !output.status_success {
            return Err(format!("设置 {secret_name} 失败：{}", output.stderr));
        }
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
    let mut config = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(config) => config,
            // 主文件损坏（如写入中断、版本升级字段不兼容）时回退备份，避免静默清空。
            Err(e) => {
                let bak = path.with_extension("json.bak");
                crate::modules::logger::log_warn(&format!(
                    "[Work CN] github.json 解析失败({e})，尝试备份恢复: {}",
                    bak.display()
                ));
                std::fs::read_to_string(&bak)
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok())
                    .unwrap_or_default()
            }
        },
        Err(_) => WorkCnGitHubConfig::default(),
    };
    for slot in &mut config.slots {
        if slot.token_secret == format!("TRAE{}_TOKEN", slot.slot) {
            slot.token_secret.clear();
        }
        if slot.device_secret == format!("TRAE{}_DEVICE_ID", slot.slot) {
            slot.device_secret.clear();
        }
    }
    config
}

pub fn save_github_config(config: &WorkCnGitHubConfig) -> Result<(), String> {
    validate_github_config(config)?;
    // secret 名（默认按账号名推导或手动指定）不能重复，否则后同步的账号会
    // 静默覆盖前一个账号的同名 Secret。本地找不到的账号跳过（同步时再报错）。
    let mut seen_names = std::collections::HashSet::new();
    for slot in &config.slots {
        let names =
            if let Some(account) = crate::modules::trae_account::load_account(&slot.account_id) {
                resolved_secret_names(&account, slot)?
                    .into_iter()
                    .collect::<Vec<_>>()
            } else {
                [slot.token_secret.as_str(), slot.device_secret.as_str()]
                    .into_iter()
                    .filter(|name| !name.is_empty())
                    .map(str::to_string)
                    .collect()
            };
        for name in names {
            if !seen_names.insert(name.clone()) {
                return Err(format!(
                    "secret 名 '{name}' 被多个账号使用，请给账号改用不重复的槽位名"
                ));
            }
        }
    }
    let path = github_config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败：{e}"))?;
    }
    let text =
        serde_json::to_string_pretty(config).map_err(|e| format!("序列化 GitHub 配置失败：{e}"))?;

    // 原子写：先备份旧文件，再写临时文件后 rename，保证任何时刻磁盘上都有完整配置，
    // 杜绝写入中断产生半截 JSON 导致“配置丢失”。
    if path.exists() {
        let bak = path.with_extension("json.bak");
        std::fs::copy(&path, &bak).map_err(|e| format!("备份 GitHub 配置失败：{e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &text).map_err(|e| format!("写入 GitHub 配置失败：{e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("落盘 GitHub 配置失败：{e}"))?;
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
    use base64::Engine;

    fn make_account(id: &str, token: &str, auth_device: Option<&str>) -> TraeAccount {
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
            trae_auth_raw: auth_device.map(|device_id| {
                serde_json::json!({
                    "platformId": "trae_solo_cn",
                    "deviceInfo": {"DeviceID": device_id},
                    "deviceKeyPair": {
                        "privateKeyPEM": "private-key",
                        "publicKeyPEM": "public-key"
                    }
                })
            }),
            trae_profile_raw: None,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: None,
            checkin_device_id: Some("d6b8ac2e-f4d1-496d-a9a6-c9c7b4bd23e3".to_string()),
            machine_id: None,
            auth_device_id: auth_device.map(|s| s.to_string()),
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
            workflow_file: crate::models::work_cn::default_workflow_file(),
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
            workflow_file: crate::models::work_cn::default_workflow_file(),
        };
        // 槽位不再有上限：第 5 个及以后的槽位同样合法（secret 名按账号槽位名推导）。
        assert!(validate_github_config(&cfg2).is_ok(), "槽位 5 应合法");

        let cfg2b = WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![WorkCnGitHubSlot {
                slot: 0,
                account_id: "a".to_string(),
                token_secret: String::new(),
                device_secret: String::new(),
            }],
            workflow_file: crate::models::work_cn::default_workflow_file(),
        };
        assert!(validate_github_config(&cfg2b).is_err(), "槽位 0 必须被拒绝");

        let cfg3 = WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![WorkCnGitHubSlot {
                slot: 1,
                account_id: "a".to_string(),
                token_secret: "bad-name".to_string(),
                device_secret: String::new(),
            }],
            workflow_file: crate::models::work_cn::default_workflow_file(),
        };
        assert!(
            validate_github_config(&cfg3).is_err(),
            "非法 secret 名必须被拒绝"
        );

        let cfg4 = WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![WorkCnGitHubSlot {
                slot: 1,
                account_id: "a".to_string(),
                token_secret: "FIRST_TOKEN".to_string(),
                device_secret: "SECOND_DEVICE_ID".to_string(),
            }],
            workflow_file: crate::models::work_cn::default_workflow_file(),
        };
        assert!(
            validate_github_config(&cfg4).is_err(),
            "自定义 token/device secret 必须使用同一主干"
        );
    }

    #[test]
    fn work_cn_github_sync_calls_official_device_secrets_with_stdin_not_args() {
        let runner = FakeGitHubRunner::new();
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("1132918838145530"));
        let slot = WorkCnGitHubSlot {
            slot: 1,
            account_id: "acc1".to_string(),
            token_secret: String::new(),
            device_secret: String::new(),
        };
        let result =
            sync_account_secrets_with_context(&runner, &account, &slot, "o/r", "INVA", "windows")
                .unwrap();
        assert!(result.synced, "应同步成功");
        assert!(!result.skipped);

        let calls = runner.recorded_calls();
        // auth + 4 secret sets = 5 calls
        assert_eq!(calls.len(), 5, "应调用 auth status + 四个 secret set");
        let secret_calls: Vec<&FakeCall> = calls
            .iter()
            .filter(|c| c.args.first().map(|s| s.as_str()) == Some("secret"))
            .collect();
        assert_eq!(secret_calls.len(), 4, "应恰好设置四个 secret");

        for c in &secret_calls {
            // token/device must NOT appear on the command line
            let joined = c.args.join(" ");
            assert!(
                !joined.contains(&account.access_token),
                "token 不能出现在参数里"
            );
            assert!(
                !joined.contains("1132918838145530"),
                "device id 不能出现在参数里"
            );
            assert!(!joined.contains("INVA"), "device brand 不能出现在参数里");
            assert!(!joined.contains("windows"), "device type 不能出现在参数里");
            assert!(c.stdin.is_some(), "secret 值必须走 stdin");
        }
        assert_eq!(
            secret_calls[0].stdin.as_deref(),
            Some(account.access_token.as_str())
        );
        assert_eq!(secret_calls[1].stdin.as_deref(), Some("1132918838145530"));
        assert_eq!(secret_calls[2].stdin.as_deref(), Some("INVA"));
        assert_eq!(secret_calls[3].stdin.as_deref(), Some("windows"));
        // default secret names derive from the account slot name (email fallback here)
        assert!(secret_calls[0].args.contains(&"ACC1_TOKEN".to_string()));
        assert!(secret_calls[1].args.contains(&"ACC1_DEVICE_ID".to_string()));
        assert!(secret_calls[2]
            .args
            .contains(&"ACC1_DEVICE_BRAND".to_string()));
        assert!(secret_calls[3]
            .args
            .contains(&"ACC1_DEVICE_TYPE".to_string()));
    }

    #[test]
    fn work_cn_github_secret_names_keep_account_stem_for_device_context() {
        let mut account = make_account("acc1", "token", Some("1132918838145530"));
        account.tags = Some(vec!["backup-1".to_string()]);
        let slot = WorkCnGitHubSlot {
            slot: 1,
            account_id: account.id.clone(),
            token_secret: "CUSTOM_TOKEN".to_string(),
            device_secret: "CUSTOM_DEVICE_ID".to_string(),
        };

        assert_eq!(
            resolved_secret_names(&account, &slot).unwrap(),
            [
                "CUSTOM_TOKEN".to_string(),
                "CUSTOM_DEVICE_ID".to_string(),
                "CUSTOM_DEVICE_BRAND".to_string(),
                "CUSTOM_DEVICE_TYPE".to_string(),
            ]
        );
    }

    #[test]
    fn work_cn_github_load_migrates_legacy_default_secret_names() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "work-cn-gh-legacy-defaults-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        let config = serde_json::json!({
            "enabled": true,
            "repository": "o/r",
            "workflowFile": "daily-checkin.yml",
            "slots": [{
                "slot": 1,
                "accountId": "acc1",
                "tokenSecret": "TRAE1_TOKEN",
                "deviceSecret": "TRAE1_DEVICE_ID"
            }]
        });
        std::fs::write(
            dir.join("github.json"),
            serde_json::to_string_pretty(&config).expect("serialize config"),
        )
        .expect("write config");

        let loaded = load_github_config();
        assert_eq!(loaded.slots.len(), 1);
        assert!(loaded.slots[0].token_secret.is_empty());
        assert!(loaded.slots[0].device_secret.is_empty());

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_cn_github_secret_stem_prefers_tag_with_sanitization() {
        let mut account = make_account("acc1", "tok", None);
        account.tags = Some(vec!["main-1 号".to_string()]);
        account.nickname = Some("昵称X".to_string());
        // 标签清洗：大写、非法字符折叠为单个 _、去首尾 _（中文字符被折叠后剔除）
        assert_eq!(slot_secret_stem(&account), "MAIN_1");

        // 纯非 ASCII 标签清洗为空 → 回退昵称；昵称也为空 → 回退邮箱前缀
        account.tags = Some(vec!["主号".to_string()]);
        account.nickname = Some("backup.acc".to_string());
        assert_eq!(slot_secret_stem(&account), "BACKUP_ACC");

        account.tags = None;
        account.nickname = Some("主号".to_string());
        assert_eq!(slot_secret_stem(&account), "ACC1");
    }

    #[test]
    fn work_cn_github_secret_stem_github_prefix_and_digit_start() {
        let mut account = make_account("acc1", "tok", None);
        account.tags = Some(vec!["github_ci".to_string()]);
        assert_eq!(slot_secret_stem(&account), "A_GITHUB_CI");

        account.tags = Some(vec!["1st".to_string()]);
        assert_eq!(slot_secret_stem(&account), "A_1ST");

        account.tags = Some(vec!["主号".to_string()]);
        account.nickname = None;
        account.email = "unknown".to_string();
        account.user_id = Some("u-99".to_string());
        assert_eq!(slot_secret_stem(&account), "U_99");
    }

    #[test]
    fn work_cn_github_save_rejects_duplicate_secret_names() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "work-cn-gh-dup-{}",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);

        // 手动指定的 secret 名重复：保存必须被拒绝
        let cfg = WorkCnGitHubConfig {
            enabled: true,
            repository: "o/r".to_string(),
            slots: vec![
                WorkCnGitHubSlot {
                    slot: 1,
                    account_id: "a1".to_string(),
                    token_secret: "SAME_NAME".to_string(),
                    device_secret: "D1".to_string(),
                },
                WorkCnGitHubSlot {
                    slot: 2,
                    account_id: "a2".to_string(),
                    token_secret: "SAME_NAME".to_string(),
                    device_secret: "D2".to_string(),
                },
            ],
            workflow_file: crate::models::work_cn::default_workflow_file(),
        };
        assert!(
            save_github_config(&cfg).is_err(),
            "重复 secret 名必须被拒绝"
        );

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_cn_github_first_secret_failure_is_not_overall_success() {
        let runner = FakeGitHubRunner {
            secret_set_ok: false,
            ..FakeGitHubRunner::new()
        };
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("1132918838145530"));
        let slot = WorkCnGitHubSlot {
            slot: 1,
            account_id: "acc1".to_string(),
            token_secret: String::new(),
            device_secret: String::new(),
        };
        let result = sync_account_secrets(&runner, &account, &slot, "o/r");
        assert!(
            result.is_err(),
            "第一个 secret 失败必须返回 Err（非整体成功）"
        );
    }

    #[test]
    fn work_cn_github_expired_token_is_skipped_not_error() {
        let runner = FakeGitHubRunner::new();
        let token = jwt_with_exp(chrono::Utc::now().timestamp() - 3600); // already expired
        let account = make_account("acc1", &token, Some("1132918838145530"));
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
        assert!(
            runner.recorded_calls().is_empty(),
            "缺 device id 不应调用 gh"
        );
    }

    #[test]
    fn work_cn_github_not_authed_is_hard_error() {
        let runner = FakeGitHubRunner {
            auth_ok: false,
            ..FakeGitHubRunner::new()
        };
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("1132918838145530"));
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
        assert!(
            !redacted.contains("eyJabcDEF1234567890xyz"),
            "长 secret 必须被脱敏"
        );
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
            workflow_file: crate::models::work_cn::default_workflow_file(),
        })
        .expect("save config");

        let runner = FakeGitHubRunner::new();
        let account = make_account("acc1", "tok", Some("dev"));
        let result = sync_account_secrets_if_bound_with(&runner, &account).unwrap();
        assert!(!result.synced);
        assert!(result.skipped);
        assert_eq!(
            result.skip_reason.as_deref(),
            Some("该账号未绑定 GitHub 槽位")
        );
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
            workflow_file: crate::models::work_cn::default_workflow_file(),
        })
        .expect("save config");

        let runner = FakeGitHubRunner::new();
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("1132918838145530"));
        let result = sync_account_secrets_if_bound_with(&runner, &account).unwrap();
        assert!(result.synced, "绑定 + 启用 + 已登录应同步成功");
        assert!(!result.skipped);

        let calls = runner.recorded_calls();
        assert_eq!(calls.len(), 5, "应调用 auth status + 四个 secret set");
        assert_eq!(calls[0].args.first().map(|s| s.as_str()), Some("auth"));
        assert_eq!(calls[1].args.first().map(|s| s.as_str()), Some("secret"));
        assert_eq!(calls[2].args.first().map(|s| s.as_str()), Some("secret"));
        assert_eq!(calls[3].args.first().map(|s| s.as_str()), Some("secret"));
        assert_eq!(calls[4].args.first().map(|s| s.as_str()), Some("secret"));

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn work_cn_github_sync_if_bound_skips_incomplete_official_device_snapshot() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let dir = std::env::temp_dir().join(format!(
            "work-cn-github-incomplete-device-{}",
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
            workflow_file: crate::models::work_cn::default_workflow_file(),
        })
        .expect("save config");

        let runner = FakeGitHubRunner::new();
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let mut account = make_account("acc1", &token, Some("1132918838145530"));
        account.trae_auth_raw = None;

        let result = sync_account_secrets_if_bound_with(&runner, &account).unwrap();
        assert!(result.skipped);
        assert!(!result.synced);
        assert_eq!(
            result.skip_reason.as_deref(),
            Some("官方设备快照不完整，已跳过 GitHub 同步")
        );
        assert!(
            runner.recorded_calls().is_empty(),
            "快照不完整时绝不调用 gh"
        );

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
            workflow_file: crate::models::work_cn::default_workflow_file(),
        })
        .expect("save config");

        let runner = FakeGitHubRunner {
            auth_ok: false,
            ..FakeGitHubRunner::new()
        };
        let token = jwt_with_exp(chrono::Utc::now().timestamp() + 3600);
        let account = make_account("acc1", &token, Some("1132918838145530"));
        let result = sync_account_secrets_if_bound_with(&runner, &account);
        assert!(result.is_err(), "gh 未登录必须返回 Err");

        std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

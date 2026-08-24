//! GitHub CLI（gh）自动安装与登录引导。
//!
//! 检测到本机缺少 gh 时，引导弹窗触发「自动下载并安装」：
//! 1. 优先调 GitHub Releases API 取最新版本号，失败回退固定版本（资产永久有效）；
//! 2. 下载官方 MSI 到临时目录，进度通过 Tauri 事件推送给前端；
//! 3. `msiexec /passive` 静默安装（系统会弹 UAC 确认）；
//! 4. 安装后通过已知安装目录定位 gh.exe（进程 PATH 不会自动刷新）。
//!
//! 登录采用 PAT + `gh auth login --with-token`：Token 只走 stdin，绝不落盘、
//! 绝不出现在命令行参数或日志中（stderr 已由 `redact_for_log` 兜底）。

use std::io::{Read, Write};
use std::time::Duration;

use serde::Serialize;
use tauri::Emitter;

/// 前端订阅的安装进度事件名。
pub const GH_SETUP_EVENT: &str = "gh-setup:progress";

/// 兜底版本：Releases API 不可达时使用（历史 release 下载链接永久有效）。
const FALLBACK_VERSION: &str = "2.63.2";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhSetupProgress {
    /// resolving / downloading / installing / done / failed
    pub phase: String,
    pub received: u64,
    pub total: u64,
}

fn emit(app: &tauri::AppHandle, phase: &str, received: u64, total: u64) {
    let _ = app.emit(
        GH_SETUP_EVENT,
        GhSetupProgress {
            phase: phase.to_string(),
            received,
            total,
        },
    );
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("qiehuan-yingyong-gh-setup")
        // 版本探测超时收紧：网络不佳时快速回退固定版本，别让用户干等。
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("构建 HTTP 客户端失败：{e}"))
}

/// 解析最新 gh 版本号（如 "2.74.1"）。API 失败时静默回退固定版本。
fn latest_version() -> String {
    let fetched = http_client().and_then(|client| {
        let resp = client
            .get("https://api.github.com/repos/cli/cli/releases/latest")
            .send()
            .map_err(|e| format!("{e}"))?;
        let value: serde_json::Value = resp.json().map_err(|e| format!("{e}"))?;
        let tag = value
            .get("tag_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();
        if let Some(stripped) = tag.strip_prefix('v') {
            if !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return Ok(stripped.to_string());
            }
        }
        Err("tag_name 格式异常".to_string())
    });
    fetched.unwrap_or_else(|_| FALLBACK_VERSION.to_string())
}

/// 下载 gh 官方 MSI 并静默安装。进度经 `GH_SETUP_EVENT` 推送前端。
pub fn download_and_install_gh(app: &tauri::AppHandle) -> Result<(), String> {
    if crate::modules::work_cn_github::resolve_gh_path().is_some() {
        emit(app, "done", 0, 0);
        return Ok(()); // 已安装，无需重复下载。
    }
    if !cfg!(windows) {
        return Err("自动安装仅支持 Windows，请到 cli.github.com 手动安装".to_string());
    }

    let version = latest_version();
    let url = format!(
        "https://github.com/cli/cli/releases/download/v{v}/gh_{v}_windows_amd64.msi",
        v = version
    );

    // ---- 下载 ----
    emit(app, "downloading", 0, 0);
    let client = http_client()?;
    let mut resp = client
        .get(&url)
        .timeout(Duration::from_secs(600))
        .send()
        .map_err(|e| format!("下载 gh v{version} 失败：{e}"))?;
    if !resp.status().is_success() {
        return Err(format!(
            "下载 gh v{version} 失败：HTTP {}。若网络受限，可到 cli.github.com 手动下载",
            resp.status()
        ));
    }
    let total = resp.content_length().unwrap_or(0);
    let msi_path = std::env::temp_dir().join(format!("gh_{version}_windows_amd64.msi"));
    let mut file =
        std::fs::File::create(&msi_path).map_err(|e| format!("创建临时文件失败：{e}"))?;
    let mut received: u64 = 0;
    let mut last_emit: u64 = 0;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = resp.read(&mut buf).map_err(|e| format!("下载中断：{e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("写入临时文件失败：{e}"))?;
        received += n as u64;
        // 每 512KB 推送一次进度，避免事件刷屏。
        if received.saturating_sub(last_emit) >= 512 * 1024 || (total > 0 && received >= total) {
            emit(app, "downloading", received, total);
            last_emit = received;
        }
    }
    drop(file);

    // ---- 安装：/passive 显示最小进度条，由系统弹出 UAC 提权确认 ----
    emit(app, "installing", received, total);
    let mut cmd = std::process::Command::new("msiexec");
    cmd.args([
        "/i",
        msi_path.to_str().unwrap_or_default(),
        "/passive",
        "/norestart",
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let status = cmd.status().map_err(|e| format!("启动安装程序失败：{e}"))?;
    let _ = std::fs::remove_file(&msi_path); // 安装包用完即删。
    if !status.success() {
        return Err(format!(
            "gh 安装失败（退出码 {}）。可到 cli.github.com 手动下载安装",
            status.code().unwrap_or(-1)
        ));
    }
    if crate::modules::work_cn_github::resolve_gh_path().is_none() {
        return Err("安装已完成但未能定位 gh.exe，请重启本软件后再试".to_string());
    }
    emit(app, "done", total, total);
    Ok(())
}

/// 用 PAT 完成 `gh auth login --with-token`，随后 `gh auth status` 校验。
/// Token 只经 stdin 传给 gh，绝不落盘、绝不出现在命令行参数中。
pub fn gh_login_with_token(token: &str) -> Result<(), String> {
    let token = token.trim();
    if token.is_empty() {
        return Err("请填写 GitHub Token".to_string());
    }
    if token.chars().any(|c| c.is_whitespace()) {
        return Err("Token 中不能包含空格或换行".to_string());
    }
    let runner = crate::modules::work_cn_github::RealGitHubRunner;
    use crate::modules::work_cn_github::GitHubRunner as _;
    let out = runner
        .run(&["auth", "login", "--with-token"], Some(token))
        .map_err(|e| format!("运行 gh auth login 失败：{e}"))?;
    if !out.status_success {
        return Err(format!(
            "登录失败：Token 无效或权限不足{}",
            if out.stderr.is_empty() {
                String::new()
            } else {
                format!("（{}）", out.stderr)
            }
        ));
    }
    crate::modules::work_cn_github::github_auth_status(&runner)
        .map_err(|e| format!("登录后校验失败：{e}"))
}

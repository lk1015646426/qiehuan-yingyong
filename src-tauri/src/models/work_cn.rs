use serde::{Deserialize, Serialize};

/// Detection result for the local TRAE Work CN install.
///
/// `legacy_path` is `true` when the resolved user-data directory still uses the
/// pre-rebrand "TRAE SOLO CN" path, so the UI can surface the compatibility
/// note from the development guide §9.3.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnInstallation {
    pub installed: bool,
    pub executable_path: Option<String>,
    pub user_data_dir: Option<String>,
    pub storage_path: Option<String>,
    pub display_name: Option<String>,
    pub version: Option<String>,
    pub legacy_path: bool,
}

/// Raw device-snapshot extracted from the local TRAE Work CN `storage.json`
/// (阶段 3). Never serializes secrets directly; consumed by the import flow.
#[derive(Debug, Clone, Default)]
pub struct LocalWorkCnDeviceSnapshot {
    pub checkin_device_id: Option<String>,
    pub machine_id: Option<String>,
    pub auth_device_id: Option<String>,
    pub device_private_key: Option<String>,
    pub device_public_key: Option<String>,
}

/// Validation result for a saved Work CN account snapshot. Deliberately not a
/// bare bool so the UI can explain *why* a snapshot is (or isn't) switch-ready.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnSnapshotValidation {
    pub valid_for_switch: bool,
    pub has_access_token: bool,
    pub has_refresh_token: bool,
    pub has_user_id: bool,
    pub has_auth_device_id: bool,
    pub has_device_private_key: bool,
    pub has_device_public_key: bool,
    pub has_checkin_device_id: bool,
    pub warnings: Vec<String>,
}

/// Desensitized view of a saved Work CN account. Tokens and private keys are
/// intentionally absent — the UI only ever receives boolean flags + masked ids.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnAccountView {
    pub id: String,
    pub email: Option<String>,
    pub user_id: Option<String>,
    pub nickname: Option<String>,
    pub tags: Option<Vec<String>>,
    pub plan_type: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
    pub has_access_token: bool,
    pub has_refresh_token: bool,
    pub has_user_id: bool,
    pub has_auth_device_id: bool,
    pub has_checkin_device_id: bool,
    pub has_machine_id: bool,
    pub has_device_private_key: bool,
    pub has_device_public_key: bool,
    pub valid_for_switch: bool,
    pub warnings: Vec<String>,
}

/// Result of a one-click Work CN account switch (阶段 4).
///
/// `github_synced` is best-effort: a GitHub sync failure never blocks the local
/// switch, it is only reported here so the UI can surface a "待同步" hint.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnSwitchResult {
    pub account_id: String,
    pub user_id: Option<String>,
    pub launched: bool,
    pub verified: bool,
    pub github_synced: bool,
    pub warning: Option<String>,
}

/// Work CN credit balance summary (阶段 5).
///
/// `total`/`remaining` are `None` when the account has no parsed entitlement
/// data — the UI shows "暂无积分数据" and must NOT show 0. `unlimited` is true
/// when any pack carries an infinite `-1` quota — the UI shows "无限". `remaining`
/// is always floored at 0 (never negative).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnCreditsSummary {
    pub total: Option<i64>,
    pub used: i64,
    pub remaining: Option<i64>,
    pub unlimited: bool,
    pub updated_at: i64,
}

/// One GitHub Secrets slot binding: which account goes to which `TRAE{N}` secret pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnGitHubSlot {
    pub slot: u8,
    pub account_id: String,
    /// Secret name for the access token. Empty → auto `TRAE{N}_TOKEN`.
    pub token_secret: String,
    /// Secret name for the device id. Empty → auto `TRAE{N}_DEVICE_ID`.
    pub device_secret: String,
}

/// Persisted GitHub Secrets sync configuration (saved to `github.json`, never holds a PAT).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnGitHubConfig {
    pub enabled: bool,
    /// `owner/repo` for the GitHub Actions repository that performs the daily check-in.
    pub repository: String,
    pub slots: Vec<WorkCnGitHubSlot>,
}

impl Default for WorkCnGitHubConfig {
    fn default() -> Self {
        WorkCnGitHubConfig {
            enabled: false,
            repository: String::new(),
            slots: Vec::new(),
        }
    }
}

/// Result of a single account's GitHub secret sync. `skipped=true` means the
/// account was intentionally not synced (e.g. expired token) — not a failure.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnGitHubSyncResult {
    pub account_id: String,
    pub synced: bool,
    pub skipped: bool,
    pub skip_reason: Option<String>,
    pub error: Option<String>,
    pub synced_at: i64,
}

/// GitHub CLI availability / auth status for the settings dialog.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnGitHubCliStatus {
    pub available: bool,
    pub authed: bool,
    pub detail: Option<String>,
}

/// Structured error codes for Work CN commands so the frontend never has to
/// branch on Chinese error strings (开发指南 §8.4).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkCnErrorCode {
    AccountNotFound,
    SnapshotIncomplete,
    ClientNotInstalled,
    ClientCloseFailed,
    StorageBackupFailed,
    InjectFailed,
    LaunchFailed,
    VerifyTimeout,
    VerifyAccountMismatch,
    RollbackFailed,
    Busy,
}

/// Unified command error: a machine-readable `code` plus a human message and
/// optional detail. Serialized to a JSON string for Tauri command `Err`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnCommandError {
    pub code: WorkCnErrorCode,
    pub message: String,
    pub detail: Option<String>,
}

impl WorkCnCommandError {
    pub fn new(code: WorkCnErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

/// 后台会话监测器的对外状态（阶段 7）。仅含脱敏信息，绝不含 token。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnSessionWatchStatus {
    /// 监测器是否已启动。
    pub running: bool,
    /// 上次检查时间戳（秒）。
    pub last_check_at: i64,
    pub outcome: WorkCnSessionWatchOutcome,
    /// 命中的账号 id（脱敏后仍安全）。
    pub account_id: Option<String>,
    /// 本次是否检测到 token 轮换。
    pub token_changed: bool,
    pub github_synced: bool,
    pub github_skipped: bool,
    pub github_error: Option<String>,
    /// 给人看的摘要（不含 token）。
    pub message: String,
}

/// 一次监测的结果枚举。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkCnSessionWatchOutcome {
    /// 尚未执行过检查 / 命令层默认值。
    Idle,
    /// storage 不存在 / 未登录。
    NoStorage,
    /// mtime 未变化，跳过解密。
    Unchanged,
    /// 切号锁被占用，跳过。
    SwitchBusy,
    /// UID 不在账号库。
    NoMatch,
    /// 匹配但 token 未变化。
    NoChange,
    /// token 变化，已回写账号库。
    TokenUpdated,
    /// 读取/解密/回写失败。
    Failed,
}

impl Default for WorkCnSessionWatchStatus {
    fn default() -> Self {
        Self {
            running: false,
            last_check_at: 0,
            outcome: WorkCnSessionWatchOutcome::Idle,
            account_id: None,
            token_changed: false,
            github_synced: false,
            github_skipped: false,
            github_error: None,
            message: String::new(),
        }
    }
}

/// Serialize a `WorkCnCommandError` into the JSON-string form Tauri commands use
/// for `Err`, so the frontend can `JSON.parse` it back into a structured error.
pub fn command_error_to_string(err: &WorkCnCommandError) -> String {
    serde_json::to_string(err).unwrap_or_else(|_| {
        format!(
            "{{\"code\":{},\"message\":{}}}",
            serde_json::to_string(&err.code).unwrap_or_else(|_| "\"UNKNOWN\"".to_string()),
            serde_json::to_string(&err.message).unwrap_or_else(|_| "\"error\"".to_string())
        )
    })
}

#[cfg(test)]
mod tests {
    use super::WorkCnSessionWatchOutcome;

    #[test]
    fn work_cn_watch_outcome_serializes_screaming_snake_case() {
        assert_eq!(
            serde_json::to_value(WorkCnSessionWatchOutcome::TokenUpdated).unwrap(),
            serde_json::json!("TOKEN_UPDATED")
        );
        assert_eq!(
            serde_json::to_value(WorkCnSessionWatchOutcome::NoStorage).unwrap(),
            serde_json::json!("NO_STORAGE")
        );
        assert_eq!(
            serde_json::to_value(WorkCnSessionWatchOutcome::SwitchBusy).unwrap(),
            serde_json::json!("SWITCH_BUSY")
        );
        assert_eq!(
            serde_json::to_value(WorkCnSessionWatchOutcome::Idle).unwrap(),
            serde_json::json!("IDLE")
        );
    }
}


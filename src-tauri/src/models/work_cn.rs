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


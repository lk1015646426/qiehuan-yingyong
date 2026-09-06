use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyAccountView {
    pub id: String,
    pub uid: String,
    pub uin: Option<String>,
    pub display_name: String,
    pub masked_phone: Option<String>,
    pub checkin_enabled: bool,
    pub token_expires_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_used_at: Option<i64>,
    pub last_github_sync_at: Option<i64>,
    pub last_github_sync_state: String,
    pub last_github_sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyAccountUpdate {
    pub display_name: Option<String>,
    pub checkin_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyAccountStatus {
    pub credits: Option<f64>,
    pub today_reward: Option<f64>,
    pub streak_days: Option<i64>,
    pub updated_at: i64,
    pub credits_error: Option<String>,
    pub activity_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyInstallation {
    pub installed: bool,
    pub executable_path: Option<String>,
    pub auth_file_path: String,
    pub running: bool,
    pub current_account_id: Option<String>,
    pub github_cleanup_pending: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddySettings {
    pub executable_path: Option<String>,
    pub auth_file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddySwitchResult {
    pub transaction_id: String,
    pub account_id: String,
    pub verified_uid: String,
    pub forced_close: bool,
    pub github_sync_pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkBuddySessionWatchOutcome {
    Idle,
    NoAuthFile,
    Unchanged,
    NoMatch,
    SnapshotUpdated,
    TokenUpdated,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddySessionWatchStatus {
    pub running: bool,
    pub last_check_at: i64,
    pub outcome: WorkBuddySessionWatchOutcome,
    pub account_id: Option<String>,
    pub token_changed: bool,
    pub github_synced: bool,
    pub github_error: Option<String>,
    pub message: String,
}

impl Default for WorkBuddySessionWatchStatus {
    fn default() -> Self {
        Self {
            running: false,
            last_check_at: 0,
            outcome: WorkBuddySessionWatchOutcome::Idle,
            account_id: None,
            token_changed: false,
            github_synced: false,
            github_error: None,
            message: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkBuddyErrorCode {
    AccountNotFound,
    AuthFileNotFound,
    AuthFileInvalid,
    AuthRefreshFailed,
    SnapshotIncomplete,
    ClientNotInstalled,
    ClientCloseFailed,
    ForceCloseRequired,
    SwitchCancelled,
    BackupFailed,
    InjectFailed,
    LaunchFailed,
    WindowActivationFailed,
    VerifyTimeout,
    VerifyAccountMismatch,
    RollbackFailed,
    GithubNotReady,
    GithubSyncFailed,
    WorkflowTriggerFailed,
    Busy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkBuddyCommandError {
    pub code: WorkBuddyErrorCode,
    pub message: String,
    pub detail: Option<String>,
    pub transaction_id: Option<String>,
}

impl WorkBuddyCommandError {
    pub fn new(code: WorkBuddyErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
            transaction_id: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

pub fn command_error_to_string(error: &WorkBuddyCommandError) -> String {
    serde_json::to_string(error).unwrap_or_else(|_| error.message.clone())
}

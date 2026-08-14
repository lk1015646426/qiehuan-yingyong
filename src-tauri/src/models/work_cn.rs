use serde::Serialize;

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

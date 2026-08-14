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

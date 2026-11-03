//! Tauri commands for the TRAE account pool.

use serde::Serialize;

use crate::models::instance::InstanceProfileView;
use crate::modules::trae_account::TraePlatformKind;
use crate::modules::trae_model_pool;
use crate::modules::trae_pool;

#[tauri::command]
pub async fn get_trae_model_pool_status() -> Result<trae_model_pool::Status, String> {
    trae_model_pool::status()
}

#[tauri::command]
pub async fn get_trae_model_pool_models() -> Result<trae_model_pool::ModelsView, String> {
    tauri::async_runtime::spawn_blocking(trae_model_pool::models)
        .await
        .map_err(|_| "模型目录查询任务失败")?
}

#[tauri::command]
pub async fn save_trae_model_pool_models(
    model_ids: Vec<String>,
) -> Result<trae_model_pool::ModelsView, String> {
    tauri::async_runtime::spawn_blocking(move || trae_model_pool::save_models(model_ids))
        .await
        .map_err(|_| "保存模型任务失败")?
}

#[tauri::command]
pub async fn start_trae_model_pool() -> Result<trae_model_pool::Status, String> {
    tauri::async_runtime::spawn_blocking(trae_model_pool::start)
        .await
        .map_err(|_| "账号池启动任务失败")?
}

#[tauri::command]
pub async fn stop_trae_model_pool() -> Result<trae_model_pool::Status, String> {
    tauri::async_runtime::spawn_blocking(trae_model_pool::stop)
        .await
        .map_err(|_| "账号池停止任务失败")?
}

#[tauri::command]
pub async fn register_trae_model_pool_models(
    account_id: String,
) -> Result<trae_model_pool::RegistrationResult, String> {
    tauri::async_runtime::spawn_blocking(move || trae_model_pool::register_models(&account_id))
        .await
        .map_err(|_| "TRAE 模型注册任务失败")?
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraePoolLaunchResult {
    pub account_id: String,
    pub instance_id: String,
    pub instance_name: String,
    pub launched: bool,
}

fn parse_platform(raw: Option<String>) -> Result<TraePlatformKind, String> {
    match raw
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // This build only drives TRAE Work CN; `TraePlatformKind::parse(None)`
        // would fall back to the international client and read the wrong
        // instance store.
        None => Ok(TraePlatformKind::TraeSoloCn),
        Some(value) => TraePlatformKind::parse(Some(value)),
    }
}

fn launch_platform(platform: TraePlatformKind) -> Option<String> {
    Some(platform.provider_key().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_platform_is_preserved_for_launch() {
        let platform = parse_platform(None).unwrap();
        assert_eq!(launch_platform(platform).as_deref(), Some("trae_solo_cn"));
        let platform = parse_platform(Some("trae_cn".into())).unwrap();
        assert_eq!(launch_platform(platform).as_deref(), Some("trae_cn"));
    }
}

#[tauri::command]
pub fn get_trae_pool_status(
    platform_id: Option<String>,
) -> Result<trae_pool::TraePoolStatus, String> {
    let platform = parse_platform(platform_id)?;
    trae_pool::pool_status(platform)
}

/// Create the isolated instance for an account without launching it, so the
/// user can review the profile directory before first start.
#[tauri::command]
pub fn ensure_trae_pool_instance(
    platform_id: Option<String>,
    account_id: String,
) -> Result<InstanceProfileView, String> {
    let platform = parse_platform(platform_id)?;
    let instance = trae_pool::ensure_instance(platform, &account_id)?;
    let initialized = std::path::Path::new(&instance.user_data_dir).exists();
    Ok(InstanceProfileView::from_profile(
        instance,
        false,
        initialized,
    ))
}

/// Launch the account in its own instance. Reuses the normal start transaction,
/// which closes the old process, injects the bound snapshot, then verifies login.
#[tauri::command]
pub async fn start_trae_pool_account(
    platform_id: Option<String>,
    account_id: String,
) -> Result<TraePoolLaunchResult, String> {
    let platform = parse_platform(platform_id)?;
    let instance = trae_pool::ensure_instance(platform, &account_id)?;
    let view = crate::commands::trae_instance::trae_start_instance(
        launch_platform(platform),
        instance.id.clone(),
    )
    .await?;
    Ok(TraePoolLaunchResult {
        account_id,
        instance_id: view.id,
        instance_name: view.name,
        launched: view.running,
    })
}

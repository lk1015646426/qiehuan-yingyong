//! Tauri commands exposed by the Doubao account pool.
use crate::modules::doubao_pool::{quota, DoubaoKeyEntry, DoubaoKeysSnapshot, PoolStatus};
use tauri::Manager;

const LOGIN_LABEL: &str = "doubao-account-login";

#[tauri::command]
pub async fn import_doubao_desktop(
    window: tauri::WebviewWindow,
    name: Option<String>,
) -> Result<DoubaoKeyEntry, String> {
    if window.label() != "main" {
        return Err("请从主窗口导入豆包账号".into());
    }
    let key =
        tauri::async_runtime::spawn_blocking(crate::modules::doubao_pool::desktop::read_cookie)
            .await
            .map_err(|_| "读取豆包客户端账号失败")??;
    Ok(DoubaoKeyEntry {
        id: uuid::Uuid::new_v4().to_string(),
        name: name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "豆包账号".into()),
        key,
        enabled: true,
    })
}

#[tauri::command]
pub async fn open_doubao_login(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("请从主窗口添加豆包账号".into());
    }
    if let Some(login) = app.get_webview_window(LOGIN_LABEL) {
        login.show().map_err(|_| "显示登录窗口失败")?;
        return login.set_focus().map_err(|_| "激活登录窗口失败".into());
    }
    // A fresh profile for every account prevents accidentally importing the previous login.
    let profile = crate::modules::config::get_data_dir()?
        .join("doubao-login")
        .join(uuid::Uuid::new_v4().to_string());
    tauri::WebviewWindowBuilder::new(
        &app,
        LOGIN_LABEL,
        tauri::WebviewUrl::External(
            "https://www.doubao.com/chat/"
                .parse()
                .map_err(|_| "登录地址无效")?,
        ),
    )
    .title("豆包账号登录 · 登录后返回主窗口导入")
    .inner_size(1080.0, 800.0)
    .data_directory(profile)
    .build()
    .map_err(|_| "无法打开豆包登录窗口")?;
    Ok(())
}

#[tauri::command]
pub async fn import_doubao_login(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    name: Option<String>,
) -> Result<DoubaoKeyEntry, String> {
    if window.label() != "main" {
        return Err("请从主窗口导入豆包账号".into());
    }
    let login = app
        .get_webview_window(LOGIN_LABEL)
        .ok_or("请先打开豆包登录窗口")?;
    let cookies = login
        .cookies_for_url(
            "https://www.doubao.com/"
                .parse()
                .map_err(|_| "登录地址无效")?,
        )
        .map_err(|_| "无法读取豆包登录状态，请使用 Cookie 手动导入")?;
    if !cookies
        .iter()
        .any(|c| c.name() == "sessionid" && !c.value().is_empty())
    {
        return Err("尚未检测到登录会话，请在豆包窗口完成登录和验证后再导入".into());
    }
    let key = cookies
        .iter()
        .map(|c| format!("{}={}", c.name(), c.value()))
        .collect::<Vec<_>>()
        .join("; ");
    // Keep the window open until persistence succeeds; the caller closes it explicitly.
    Ok(DoubaoKeyEntry {
        id: uuid::Uuid::new_v4().to_string(),
        name: name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "豆包账号".into()),
        key,
        enabled: true,
    })
}

#[tauri::command]
pub async fn close_doubao_login(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    if window.label() != "main" {
        return Err("请从主窗口操作".into());
    }
    if let Some(login) = app.get_webview_window(LOGIN_LABEL) {
        login.close().map_err(|_| "关闭登录窗口失败")?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_doubao_pool_status() -> Result<PoolStatus, String> {
    crate::modules::doubao_pool::get_doubao_pool_status()
}

#[tauri::command]
pub async fn get_doubao_pool_keys() -> Result<DoubaoKeysSnapshot, String> {
    crate::modules::doubao_pool::get_doubao_pool_keys().await
}

#[tauri::command]
pub async fn get_doubao_account_quota(
    window: tauri::WebviewWindow,
    account_id: String,
) -> Result<quota::DoubaoQuota, String> {
    if window.label() != "main" {
        return Err("请从主窗口查询豆包额度".into());
    }
    quota::query_account(account_id).await
}

#[tauri::command]
pub async fn save_doubao_pool_keys(
    entries: Vec<DoubaoKeyEntry>,
    expected_revision: Option<[u8; 32]>,
) -> Result<DoubaoKeysSnapshot, String> {
    crate::modules::doubao_pool::save_doubao_pool_keys(entries, expected_revision).await
}

#[tauri::command]
pub async fn get_doubao_pool_catalog(
    refresh: Option<bool>,
) -> Result<crate::modules::doubao_pool::CatalogView, String> {
    crate::modules::doubao_pool::get_doubao_pool_catalog(refresh).await
}

#[tauri::command]
pub async fn save_doubao_pool_models(
    model_ids: Vec<String>,
) -> Result<crate::modules::doubao_pool::CatalogView, String> {
    crate::modules::doubao_pool::save_doubao_pool_models(model_ids).await
}

#[tauri::command]
pub async fn start_doubao_pool() -> Result<PoolStatus, String> {
    crate::modules::doubao_pool::start_doubao_pool().await
}

#[tauri::command]
pub async fn stop_doubao_pool() -> Result<PoolStatus, String> {
    crate::modules::doubao_pool::stop_doubao_pool()
}

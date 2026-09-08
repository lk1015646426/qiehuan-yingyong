//! 智谱清言账号命令层：导入（客户端 Cookie / 手动粘贴）/ 列表 / 更新 / 删除 /
//! 只读积分查询 / GitHub 同步 / 云端签到触发。
//!
//! 红线：本地绝不签到（daily_login_score 只能由云端 Actions 调用）；
//! 这里只通过 `gh` CLI 同步凭证与触发 `account_filter=zhipu:<id>`。

use crate::models::zhipu::{
    ZhipuAccountStatus, ZhipuAccountUpdate, ZhipuAccountView, ZhipuImportNotice,
};
use crate::modules::{zhipu_account, zhipu_github, zhipu_status};

#[tauri::command]
pub async fn list_zhipu_accounts() -> Result<Vec<ZhipuAccountView>, String> {
    tauri::async_runtime::spawn_blocking(zhipu_account::list_zhipu_accounts)
        .await
        .map_err(|error| format!("加载智谱账号列表任务失败: {error}"))?
}

/// 从清言桌面客户端导入当前登录账号（读取 Cookie 中的最新 token，
/// 客户端每次启动会自动刷新，因此重复导入总能拿到有效 token）。
#[tauri::command]
pub async fn import_current_zhipu_account(
    display_name: Option<String>,
) -> Result<(ZhipuAccountView, ZhipuImportNotice), String> {
    let (access, refresh) = tauri::async_runtime::spawn_blocking(|| {
        zhipu_account::read_current_client_tokens()
    })
    .await
    .map_err(|error| format!("读取清言客户端任务失败: {error}"))??;
    import_with_verification(access, refresh, display_name).await
}

/// 手动粘贴 token 导入（客户端不可用时的备选）。
#[tauri::command]
pub async fn import_zhipu_account(
    access_token: String,
    refresh_token: Option<String>,
    display_name: Option<String>,
) -> Result<(ZhipuAccountView, ZhipuImportNotice), String> {
    import_with_verification(access_token, refresh_token.unwrap_or_default(), display_name).await
}

async fn import_with_verification(
    access_token: String,
    refresh_token: String,
    display_name: Option<String>,
) -> Result<(ZhipuAccountView, ZhipuImportNotice), String> {
    // 联网验证：认证失败则不保存；网络失败仍允许保存。
    let verification = zhipu_status::verify_token_online(access_token.trim()).await;
    if let Err(auth_error) = &verification {
        return Err(format!("token 验证失败，未保存：{auth_error}"));
    }
    let notice = match verification {
        Ok(true) => ZhipuImportNotice {
            verification_skipped: false,
            message: "登录态已验证有效".to_string(),
        },
        Ok(false) => ZhipuImportNotice {
            verification_skipped: true,
            message: "网络原因未能在线验证，账号已保存，可稍后点击「刷新积分」确认".to_string(),
        },
        Err(_) => unreachable!("认证失败分支已提前返回"),
    };
    let account = tauri::async_runtime::spawn_blocking(move || -> Result<ZhipuAccountView, String> {
        let account = zhipu_account::import_account(&access_token, &refresh_token, display_name)?;
        zhipu_github::queue_background_sync();
        Ok(account)
    })
    .await
    .map_err(|error| format!("导入智谱账号任务失败: {error}"))??;
    Ok((account, notice))
}

#[tauri::command]
pub async fn update_zhipu_account(
    account_id: String,
    update: ZhipuAccountUpdate,
) -> Result<ZhipuAccountView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        zhipu_account::update_zhipu_account(&account_id, update)?;
        let account = zhipu_account::list_zhipu_accounts()?
            .into_iter()
            .find(|account| account.id == account_id)
            .ok_or_else(|| "更新后的智谱账号不存在".to_string())?;
        zhipu_github::queue_background_sync();
        Ok(account)
    })
    .await
    .map_err(|error| format!("更新智谱账号任务失败: {error}"))?
}

#[tauri::command]
pub async fn delete_zhipu_account(account_id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        zhipu_account::remove_zhipu_account(&account_id)?;
        // 本地删除不因远端故障回滚；聚合 Secret 全量覆盖即远端清理。
        zhipu_github::queue_background_sync();
        Ok(true)
    })
    .await
    .map_err(|error| format!("删除智谱账号任务失败: {error}"))?
}

#[tauri::command]
pub async fn get_zhipu_account_status(account_id: String) -> Result<ZhipuAccountStatus, String> {
    zhipu_status::query_account_status(&account_id).await
}

#[tauri::command]
pub async fn sync_zhipu_github() -> Result<(), String> {
    zhipu_github::sync_background_now().await
}

#[tauri::command]
pub async fn trigger_zhipu_checkin(account_id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        zhipu_github::trigger_zhipu_checkin(&account_id)
    })
    .await
    .map_err(|error| format!("触发智谱签到任务失败: {error}"))?
}

#[cfg(test)]
mod tests {
    #[test]
    fn zhipu_import_success_returns_masked_view_and_notice_only() {
        let source = include_str!("zhipu.rs");
        // 成功返回只含脱敏视图与提示，不透传 token 原文。
        let signature = "Result<(ZhipuAccountView, ZhipuImportNotice), String>";
        assert!(source.contains(signature));
        // 本地绝不签到：命令层不得出现签到端点 URL（拼接写法避免自引用）。
        let endpoint = ["member", "-", "api"].concat();
        assert!(!source.contains(&endpoint));
    }
}

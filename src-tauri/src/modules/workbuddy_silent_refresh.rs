//! 只维护临近到期的非当前账号；当前客户端身份不明时停止本轮维护。
//! 共享认证提交负责版本保护、失败冷却及去重同步，不批量轮换健康账号。

use crate::models::workbuddy::WorkBuddyAccountView;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const STARTUP_DELAY: Duration = Duration::from_secs(120);
const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

static STARTED: AtomicBool = AtomicBool::new(false);

pub fn ensure_started() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::modules::logger::log_info("[WorkBuddySilentRefresh] 后台静默刷新已启动");
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(STARTUP_DELAY).await;
        loop {
            refresh_all().await;
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    });
}

async fn refresh_all() {
    let accounts: Vec<WorkBuddyAccountView> = match tauri::async_runtime::spawn_blocking(
        crate::modules::workbuddy_account::list_workbuddy_accounts,
    )
    .await
    {
        Ok(Ok(accounts)) => accounts,
        Ok(Err(error)) => {
            crate::modules::logger::log_warn(&format!(
                "[WorkBuddySilentRefresh] 读取账号列表失败: {error}"
            ));
            return;
        }
        Err(error) => {
            crate::modules::logger::log_warn(&format!(
                "[WorkBuddySilentRefresh] 读取账号列表任务失败: {error}"
            ));
            return;
        }
    };
    // 当前在官方客户端里登录的账号由会话监视器持续跟进快照；
    // 这里跳过它，避免与客户端自身的 token 刷新竞争同一条轮换链。
    let active = match tauri::async_runtime::spawn_blocking(
        crate::modules::workbuddy_account::current_managed_account_id_checked,
    )
    .await
    {
        Ok(Ok(id)) => id,
        _ => {
            crate::modules::logger::log_warn(
                "[WorkBuddySilentRefresh] 无法确认当前客户端身份，本轮不轮换任何凭证",
            );
            return;
        }
    };
    for account in accounts {
        if active.as_deref() == Some(account.id.as_str()) {
            continue;
        }
        if !crate::modules::workbuddy_account::auth_refresh_due(&account.id).unwrap_or(false) {
            continue;
        }
        if let Err(error) =
            crate::modules::workbuddy_account::refresh_account_auth(&account.id).await
        {
            crate::modules::logger::log_warn(&format!(
                "[WorkBuddySilentRefresh] 账号 {} 认证维护未完成：{}",
                account.display_name, error
            ));
        }
    }
}

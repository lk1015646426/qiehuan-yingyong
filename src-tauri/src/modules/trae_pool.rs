//! TRAE account pool.
//!
//! The pool is an index over stored account snapshots plus one isolated client
//! instance per account. It never mutates credentials itself: launching goes
//! through the existing switch/inject transaction, so the encrypted snapshots
//! stay the single source of truth.
//!
//! This module manages isolated instances. Message-level account rotation needs
//! a separate, validated native-protocol adapter; it is not implemented here.

use serde::Serialize;
use std::path::PathBuf;

use crate::models::instance::{InstanceProfile, InstanceStore};
use crate::modules::instance_store::CreateInstanceParams;
use crate::modules::trae_account::{self, TraePlatformKind};
use crate::modules::trae_instance;

/// Instance names created by the pool carry this prefix so they stay
/// distinguishable from instances the user created by hand.
const POOL_INSTANCE_PREFIX: &str = "账号池";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraePoolAccountView {
    pub account_id: String,
    pub label: String,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub credits_remaining: Option<f64>,
    pub credits_unlimited: bool,
    pub credits_updated_at: Option<i64>,
    pub token_expires_at: Option<i64>,
    pub token_days_left: Option<i64>,
    pub switchable: bool,
    pub warnings: Vec<String>,
    pub instance_id: Option<String>,
    pub instance_name: Option<String>,
    pub instance_running: bool,
    /// 1-based position in the pool ranking (usable accounts first, then credits).
    pub rank: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraePoolStatus {
    pub accounts: Vec<TraePoolAccountView>,
    pub recommended_account_id: Option<String>,
    pub total_accounts: usize,
    pub ready_accounts: usize,
    pub running_instances: usize,
    pub instance_root: Option<String>,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn account_label(account: &crate::models::trae::TraeAccount) -> String {
    if let Some(nickname) = account
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return nickname.to_string();
    }
    if let Some(tag) = account
        .tags
        .as_ref()
        .and_then(|tags| tags.iter().find(|tag| !tag.trim().is_empty()))
    {
        return tag.trim().to_string();
    }
    if !account.email.trim().is_empty() {
        return account.email.clone();
    }
    account.id.clone()
}

/// A snapshot is switchable only when every piece the inject path needs exists.
/// This mirrors `validate_work_cn_account_snapshot` without duplicating its
/// full rule set, and reports the missing pieces as warnings for the UI.
fn switchability(account: &crate::models::trae::TraeAccount) -> (bool, Vec<String>) {
    let validation = trae_account::validate_work_cn_account_snapshot(account);
    let mut warnings = validation.warnings;
    if !validation.has_access_token {
        warnings.push("缺少访问令牌，请重新导入账号".into());
    }
    if !validation.has_refresh_token {
        warnings.push("缺少刷新令牌，令牌过期后需重新登录".into());
    }
    (validation.valid_for_switch, warnings)
}

fn instance_for_account(store: &InstanceStore, account_id: &str) -> Option<InstanceProfile> {
    store
        .instances
        .iter()
        .filter(|item| item.bind_account_id.as_deref() == Some(account_id))
        .max_by_key(|item| item.created_at)
        .cloned()
}

fn instance_running(instance: &InstanceProfile) -> bool {
    instance
        .last_pid
        .map(crate::modules::process::is_pid_running)
        .unwrap_or(false)
}

/// Build the pool view from local state only: no network, no credential writes.
pub fn pool_status(platform: TraePlatformKind) -> Result<TraePoolStatus, String> {
    let accounts = trae_account::list_accounts_checked()?;
    let store = trae_instance::load_instance_store_for_platform(platform)?;
    let now = now_secs();

    let mut views: Vec<TraePoolAccountView> = accounts
        .iter()
        .filter(|account| {
            // Removed accounts keep a tombstone id in some snapshots; skip blanks.
            !account.id.trim().is_empty()
                && trae_account::resolve_account_platform_kind(account) == platform
        })
        .map(|account| {
            let (switchable, warnings) = switchability(account);
            let credits = trae_account::parse_work_cn_credits_from_usage(&account.trae_usage_raw);
            let token_expires_at = account.expires_at;
            let token_days_left = token_expires_at.map(|exp| (exp - now).div_euclid(86_400));
            let instance = instance_for_account(&store, &account.id);
            let instance_id = instance.as_ref().map(|item| item.id.clone());
            let instance_name = instance.as_ref().map(|item| item.name.clone());
            let running = instance.as_ref().map(instance_running).unwrap_or(false);
            TraePoolAccountView {
                account_id: account.id.clone(),
                label: account_label(account),
                email: Some(account.email.clone()).filter(|v| !v.trim().is_empty()),
                plan_type: account.plan_type.clone(),
                credits_remaining: credits.remaining.or(credits.total),
                credits_unlimited: credits.unlimited,
                credits_updated_at: account.usage_updated_at.filter(|v| *v > 0),
                token_expires_at,
                token_days_left,
                switchable,
                warnings,
                instance_id,
                instance_name,
                instance_running: running,
                rank: 0,
            }
        })
        .collect();

    // Ranking: usable accounts first, then by remaining credits (unknown last),
    // then by most recent use. This is what "启动下一个可用账号" walks.
    views.sort_by(|a, b| {
        b.switchable
            .cmp(&a.switchable)
            .then_with(|| {
                let left = a.credits_remaining.unwrap_or(-1.0);
                let right = b.credits_remaining.unwrap_or(-1.0);
                right
                    .partial_cmp(&left)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.label.cmp(&b.label))
    });
    for (index, view) in views.iter_mut().enumerate() {
        view.rank = index + 1;
    }

    let recommended_account_id = views
        .iter()
        .find(|view| view.switchable)
        .map(|view| view.account_id.clone());
    let ready_accounts = views.iter().filter(|view| view.switchable).count();
    let running_instances = views.iter().filter(|view| view.instance_running).count();
    let instance_root = trae_instance::get_default_instances_root_dir_for_platform(platform)
        .ok()
        .map(|path| path.to_string_lossy().to_string());

    Ok(TraePoolStatus {
        total_accounts: views.len(),
        ready_accounts,
        running_instances,
        recommended_account_id,
        instance_root,
        accounts: views,
    })
}

fn pool_dir_name(account_id: &str) -> String {
    let suffix: String = account_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("pool-{suffix}")
}

/// Return the instance bound to this account, creating one (copied from the
/// default client directory) when the account has never been pooled.
pub fn ensure_instance(
    platform: TraePlatformKind,
    account_id: &str,
) -> Result<InstanceProfile, String> {
    let store = trae_instance::load_instance_store_for_platform(platform)?;
    if let Some(existing) = instance_for_account(&store, account_id) {
        return Ok(existing);
    }

    let account = trae_account::load_account(account_id)
        .ok_or_else(|| "账号不存在，请先刷新列表".to_string())?;
    let (switchable, _) = switchability(&account);
    if !switchable {
        return Err("该账号快照不完整，无法用于切换；请重新导入".to_string());
    }

    let root = trae_instance::get_default_instances_root_dir_for_platform(platform)?;
    let dir: PathBuf = root.join(pool_dir_name(account_id));
    if dir.exists() {
        // Leftover from a deleted instance: reuse only when empty, otherwise it
        // is safer to fail loudly than to copy into a dirty profile directory.
        let has_entries = std::fs::read_dir(&dir)
            .map(|mut iter| iter.next().is_some())
            .unwrap_or(true);
        if has_entries {
            return Err(format!(
                "实例目录已存在且非空，请先清理: {}",
                dir.to_string_lossy()
            ));
        }
    }

    let name = format!("{POOL_INSTANCE_PREFIX}-{}", account_label(&account));
    trae_instance::create_instance_for_platform(
        platform,
        CreateInstanceParams {
            name,
            user_data_dir: dir.to_string_lossy().to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: Some(account_id.to_string()),
            copy_source_instance_id: Some("__default__".to_string()),
            init_mode: Some("copy".to_string()),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::trae::TraeAccount;

    fn account(id: &str) -> TraeAccount {
        TraeAccount {
            id: id.to_string(),
            email: String::new(),
            user_id: None,
            nickname: None,
            tags: None,
            access_token: String::new(),
            refresh_token: None,
            token_type: None,
            expires_at: None,
            plan_type: None,
            plan_reset_at: None,
            trae_auth_raw: None,
            trae_profile_raw: None,
            trae_entitlement_raw: None,
            trae_usage_raw: None,
            trae_server_raw: None,
            trae_usertag_raw: None,
            checkin_device_id: None,
            machine_id: None,
            auth_device_id: None,
            status: None,
            status_reason: None,
            quota_query_last_error: None,
            quota_query_last_error_at: None,
            usage_updated_at: None,
            created_at: 0,
            last_used: 0,
        }
    }

    #[test]
    fn pool_dir_name_is_stable_and_path_safe() {
        let first = pool_dir_name("trae_64787fb5f684a3226422ad03c3bb2314");
        assert_eq!(first, "pool-c3bb2314");
        assert!(first.starts_with("pool-"));
        assert!(first
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_ne!(first, pool_dir_name("trae_9bf38a69c2ed98437167b198974692e1"));
        // 少于 8 位时不应 panic。
        assert_eq!(pool_dir_name("ab"), "pool-ab");
    }

    #[test]
    fn incomplete_snapshot_is_not_switchable() {
        let mut bare = account("trae_1");
        let (ok, warnings) = switchability(&bare);
        assert!(!ok);
        assert!(!warnings.is_empty());

        bare.access_token = "token".into();
        bare.auth_device_id = Some("123456".into());
        let (ok, _) = switchability(&bare);
        assert!(!ok, "缺少设备密钥时仍应拒绝");

        bare.trae_auth_raw = Some(serde_json::json!({"deviceKeyPair": {"publicKey": "x"}}));
        let (ok, warnings) = switchability(&bare);
        assert!(!ok, "只有公钥且缺少用户和设备快照时必须拒绝");
        assert!(!warnings.is_empty());
    }

    #[test]
    fn empty_device_key_object_is_not_a_complete_snapshot() {
        let mut snapshot = account("trae_1");
        snapshot.access_token = "token".into();
        snapshot.auth_device_id = Some("123456".into());
        snapshot.trae_auth_raw = Some(serde_json::json!({"deviceKeyPair": {}}));
        assert!(!switchability(&snapshot).0);
    }

    #[test]
    fn complete_official_snapshot_uses_existing_switch_validation() {
        let mut snapshot = account("trae_1");
        snapshot.access_token = "token".into();
        snapshot.user_id = Some("user".into());
        snapshot.auth_device_id = Some("123456".into());
        snapshot.checkin_device_id = Some("telemetry-device".into());
        snapshot.refresh_token = Some("refresh".into());
        snapshot.trae_auth_raw = Some(serde_json::json!({"deviceKeyPair": {
            "publicKeyPEM": "public", "privateKeyPEM": "private"
        }}));
        assert!(switchability(&snapshot).0);
        snapshot.trae_auth_raw.as_mut().unwrap()["deviceKeyPair"]["privateKeyPEM"] = serde_json::json!("");
        assert!(!switchability(&snapshot).0);
    }
}

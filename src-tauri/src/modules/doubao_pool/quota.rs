use reqwest::header::{HeaderValue, ACCEPT, COOKIE};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::Duration;

// Consumer subscription usage, separate from chat and Ark billing.
const QUOTA_URL: &str = "https://www.doubao.com/alice/commerce/sale/subscription/quota/summary/";
static CLIENT: LazyLock<Result<Client, reqwest::Error>> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
});

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum QuotaKind {
    Package,
    Bonus,
    FiveHour,
    Weekly,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct QuotaWindow {
    kind: QuotaKind,
    remaining: Option<f64>,
    total: Option<f64>,
    remaining_percent: Option<f64>,
    ends_at: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoubaoQuota {
    plan: Option<String>,
    available_points: Option<f64>,
    windows: Vec<QuotaWindow>,
    checked_at: i64,
}

fn amount(value: &Value) -> Option<f64> {
    let value = value.as_f64().or_else(|| {
        let raw = value.as_str()?.trim();
        if !raw.contains(',') {
            return raw.parse().ok();
        }
        let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
        if fraction.contains(',') {
            return None;
        }
        let mut groups = whole.split(',');
        let first = groups.next()?;
        if first.is_empty()
            || first.len() > 3
            || !first.bytes().all(|byte| byte.is_ascii_digit())
            || groups.any(|group| group.len() != 3 || !group.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
        raw.replace(',', "").parse().ok()
    })?;
    (0.0..=9_007_199_254_740_991.0).contains(&value).then_some(value)
}

fn quota_window(limit: &Value, bonus_section: bool) -> Option<QuotaWindow> {
    let kind = match (bonus_section, limit["window_type"].as_u64()?) {
        (false, 1) => QuotaKind::FiveHour,
        (false, 2) => QuotaKind::Weekly,
        (true, 3) => QuotaKind::Bonus,
        (false, 4) => QuotaKind::Package,
        _ => return None,
    };
    let total = amount(&limit["total_amount"]);
    let remaining = total
        .zip(amount(&limit["used_amount"]))
        .map(|(total, used)| (total - used).max(0.0));
    let remaining_percent = limit["used_percent"]
        .as_f64()
        .filter(|percent| (0.0..=100.0).contains(percent))
        .map(|used| 100.0 - used);
    let ends_at = limit["end_time"]
        .as_i64()
        .filter(|time| (100_000_000_000..=253_402_300_799_999).contains(time));
    Some(QuotaWindow {
        kind,
        remaining,
        total,
        remaining_percent,
        ends_at,
    })
}

fn available_points(windows: &[QuotaWindow], checked_at: i64) -> Option<f64> {
    let mut available = None;
    let mut has_package = false;
    for window in windows {
        if window.ends_at.is_some_and(|end| end <= checked_at) {
            continue;
        }
        match window.kind {
            QuotaKind::Package if has_package => return None,
            QuotaKind::Package => has_package = true,
            QuotaKind::Bonus => {}
            // Older windows have their own usage limits and units.
            _ => return None,
        }
        available = Some(available.unwrap_or(0.0) + window.remaining?);
    }
    available.filter(|value| (0.0..=9_007_199_254_740_991.0).contains(value))
}

fn parse_summary(payload: &Value, checked_at: i64) -> Result<DoubaoQuota, String> {
    if let Some(code) = payload.get("code") {
        match code.as_i64().or_else(|| code.as_str()?.parse().ok()) {
            Some(0) => {}
            Some(710012001) => return Err("豆包登录已失效，请重新登录后导入".into()),
            Some(710022004) => return Err("豆包要求验证，请在官方页面完成验证后重试".into()),
            Some(code) => return Err(format!("豆包未返回额度（服务状态 {code}）")),
            None => return Err("无法识别豆包额度响应".into()),
        }
    }
    let data = payload
        .get("data")
        .filter(|data| data.is_object())
        .ok_or("豆包未返回额度数据")?;
    let mut windows = Vec::new();
    for (section, bonus) in [("window_limit_section", false), ("incentive_quota_section", true)] {
        if let Some(groups) = data[section]["window_limit_groups"].as_array() {
            for group in groups {
                if let Some(limits) = group["window_limits"].as_array() {
                    windows.extend(limits.iter().filter_map(|limit| quota_window(limit, bonus)));
                }
            }
        }
    }
    let plan = ["short_name", "product_name"].into_iter().find_map(|field| {
        data["current_subscription"]["display"][field]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.chars().count() <= 64)
            .map(str::to_owned)
    });
    Ok(DoubaoQuota {
        plan,
        available_points: available_points(&windows, checked_at),
        windows,
        checked_at,
    })
}

pub(crate) async fn query_account(account_id: String) -> Result<DoubaoQuota, String> {
    let account = tauri::async_runtime::spawn_blocking(move || {
        super::load_saved_keys()?
            .entries
            .into_iter()
            .find(|entry| entry.id == account_id)
            .ok_or_else(|| "找不到该豆包账号".to_string())
    })
    .await
    .map_err(|_| "读取豆包账号失败")??;
    let cookie = super::protocol::normalize_cookie(&account.key)
        .map_err(|_| "豆包登录信息无法使用，请重新导入")?;
    let mut header = HeaderValue::from_str(&cookie).map_err(|_| "豆包登录信息格式无效")?;
    header.set_sensitive(true);
    let client = CLIENT.as_ref().map_err(|_| "无法初始化额度查询")?;
    let response = client
        .post(QUOTA_URL)
        .query(&[("aid", "497858")])
        .header(COOKIE, header)
        .header(ACCEPT, "application/json")
        .json(&json!({ "product_line": "membership" }))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "查询豆包额度超时，请重试"
            } else {
                "无法连接豆包额度服务，请检查网络"
            }
        })?;
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err("豆包拒绝额度查询，请在官方订阅页核对登录状态".into());
        }
        StatusCode::TOO_MANY_REQUESTS => return Err("额度查询过于频繁，请稍后重试".into()),
        status if status.is_redirection() => {
            return Err("豆包返回页面跳转，请在官方订阅页核对登录状态".into());
        }
        status if !status.is_success() => return Err(format!("额度查询失败（HTTP {status}）")),
        _ => {}
    }
    let payload = response
        .json::<Value>()
        .await
        .map_err(|_| "无法识别豆包额度响应")?;
    parse_summary(&payload, chrono::Utc::now().timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000_000;

    fn summary(package: Value, bonus: Value) -> Value {
        json!({"code": 0, "data": {
            "current_subscription": {"display": {"short_name": "示例套餐"}},
            "window_limit_section": {"window_limit_groups": [{"window_limits": package}]},
            "incentive_quota_section": {"window_limit_groups": [{"window_limits": bonus}]},
            "enterprise_window_limit_section": {"window_limit_groups": [{"window_limits": [
                {"window_type": 4, "total_amount": 999, "used_amount": 0}
            ]}]}
        }})
    }

    #[test]
    fn combines_personal_package_and_active_bonus_amounts() {
        let quota = parse_summary(&summary(
            json!([{"window_type": 4, "used_amount": 700, "total_amount": "9,000"}]),
            json!([{"window_type": 3, "used_amount": "200", "total_amount": "1,100", "end_time": NOW + 1},
                   {"window_type": 3, "used_amount": 0, "total_amount": 500, "end_time": NOW - 1}]),
        ), NOW).unwrap();
        assert_eq!(quota.plan.as_deref(), Some("示例套餐"));
        assert_eq!(quota.available_points, Some(9200.0));
        assert_eq!(quota.windows.len(), 3);
        assert_eq!(quota.windows[0].ends_at, None);
    }

    #[test]
    fn distinguishes_unknown_amounts_percentages_and_a_real_zero() {
        assert_eq!(amount(&json!("1,200.50")), Some(1200.5));
        assert_eq!(amount(&json!("12.0,0")), None);
        let quota = parse_summary(&summary(
            json!([{"window_type": 4, "used_percent": 30, "total_amount": "10,00"}]),
            json!([]),
        ), NOW).unwrap();
        assert_eq!(quota.available_points, None);
        assert_eq!(quota.windows[0].remaining_percent, Some(70.0));
        let full_use = json!([{"window_type": 4, "used_amount": 50, "total_amount": 50}]);
        assert_eq!(
            parse_summary(&summary(full_use, json!([])), NOW).unwrap().available_points,
            Some(0.0)
        );
        assert_eq!(
            parse_summary(&summary(json!([]), json!([])), NOW).unwrap().available_points,
            None
        );
    }

    #[test]
    fn keeps_legacy_or_ambiguous_balances_separate() {
        let quota = parse_summary(&summary(
            json!([{"window_type": 2, "used_amount": 10, "total_amount": 100},
                   {"window_type": 4, "used_amount": 10, "total_amount": 100}]),
            json!([]),
        ), NOW).unwrap();
        assert_eq!(quota.windows[0].kind, QuotaKind::Weekly);
        assert_eq!(quota.available_points, None);
        let package = json!({"window_type": 4, "used_amount": 5, "total_amount": 20});
        let quota = parse_summary(&summary(json!([package.clone(), package]), json!([])), NOW).unwrap();
        assert_eq!(quota.available_points, None);
        assert_eq!(quota.windows.len(), 2);
    }

    #[test]
    fn handles_auth_rejection_and_bad_response_without_a_balance() {
        assert!(parse_summary(&json!({"code": "710012001"}), NOW).unwrap_err().contains("重新登录"));
        assert!(parse_summary(&json!({"code": 710022004}), NOW).unwrap_err().contains("验证"));
        assert!(parse_summary(&json!({"code": 0, "data": null}), NOW).is_err());
    }
}

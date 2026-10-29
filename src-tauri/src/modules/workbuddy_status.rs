use crate::models::workbuddy::WorkBuddyAccountStatus;
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, ORIGIN, REFERER, USER_AGENT};
use serde_json::{json, Value};
use std::time::Instant;

const RESOURCE_URL: &str = "https://copilot.tencent.com/billing/meter/get-user-resource-summary";
const ACTIVITY_URL: &str = "https://copilot.tencent.com/v2/billing/meter/checkin-activity-status";

fn number(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn integer(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn parse_available_credits(payload: &Value) -> Option<f64> {
    // 2026-10-18 与真实账号汇总核对：采用官方 Packages 的当前周期剩余量，
    // 不按旧资源包类型排除 500 基础积分，也不把 CreditGroups 再累计一次。
    // 契约参考 L0NE-6/WorkBuddy-Daily queryCredits；缺失/错误与真实零余额分开。
    if payload.get("code").and_then(Value::as_i64) != Some(0) {
        return None;
    }
    let packages = payload.pointer("/data/Packages")?.as_array()?;
    let mut total = 0.0;
    for package in packages {
        let remain = number(package.get("CycleRemainCapacity"))?;
        if !remain.is_finite() {
            return None;
        }
        total += remain.max(0.0);
    }
    let rounded = (total * 100.0).round() / 100.0;
    rounded.is_finite().then_some(rounded)
}

fn parse_activity(payload: &Value) -> (Option<f64>, Option<i64>) {
    let data = payload.get("data").unwrap_or(payload);
    let reward = number(data.get("today_credit"))
        .or_else(|| number(data.get("daily_credit")))
        .or_else(|| number(data.get("todayCredit")))
        .or_else(|| number(data.get("dailyCredit")));
    let streak = integer(data.get("streak_days")).or_else(|| integer(data.get("streakDays")));
    (reward, streak)
}

fn business_error(payload: &Value) -> Option<String> {
    let code = integer(payload.get("code"))?;
    if code == 0 {
        return None;
    }
    let message = payload
        .get("msg")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .map(|value| {
            value
                .chars()
                .filter(|character| !character.is_control())
                .take(120)
                .collect::<String>()
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "上游返回业务错误".to_string());
    if code == 401 || code == 403 {
        return Some(format!("认证已失效：{message} (code={code})"));
    }
    Some(format!("{message} (code={code})"))
}

fn network_error_message(label: &str, detail: impl std::fmt::Display) -> String {
    format!("{label}国内网络请求失败: {detail}")
}

async fn query_json(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    label: &str,
) -> Result<Value, String> {
    let started = Instant::now();
    crate::modules::logger::log_info(&format!(
        "[WorkBuddy Network] request_start label={} url={}",
        label, url
    ));
    let response = client
        .post(url)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "application/json")
        .header(ORIGIN, "https://www.codebuddy.cn")
        .header(REFERER, "https://www.codebuddy.cn/")
        .header(USER_AGENT, "Mozilla/5.0 WorkBuddy Desktop Switcher")
        .json(&json!({}))
        .send()
        .await
        .map_err(|error| {
            let detail = crate::utils::http::format_request_error(label, &error);
            crate::modules::logger::log_warn(&format!(
                "[WorkBuddy Network] request_failed label={} elapsed_ms={} detail={}",
                label,
                started.elapsed().as_millis(),
                detail
            ));
            detail
        })?;
    let status = response.status();
    crate::modules::logger::log_info(&format!(
        "[WorkBuddy Network] response_received label={} status={} elapsed_ms={}",
        label,
        status.as_u16(),
        started.elapsed().as_millis()
    ));
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(format!("{label}保存的登录凭证被拒绝，请核对或手动恢复认证"));
    }
    if !status.is_success() {
        return Err(format!("{label}返回 HTTP {}", status.as_u16()));
    }
    let payload = response
        .json::<Value>()
        .await
        .map_err(|_| format!("{label}返回数据无法解析"))?;
    if let Some(error) = business_error(&payload) {
        return Err(format!("{label}失败: {error}"));
    }
    Ok(payload)
}

fn is_auth_error(error: &str) -> bool {
    error.contains("认证已失效")
}

async fn query_both(
    client: &reqwest::Client,
    token: &str,
) -> (Result<Value, String>, Result<Value, String>) {
    tokio::join!(
        query_json(client, RESOURCE_URL, token, "积分查询"),
        query_json(client, ACTIVITY_URL, token, "签到状态查询"),
    )
}

fn assemble_status(
    credits: Result<Option<f64>, String>,
    activity: Result<(Option<f64>, Option<i64>), String>,
    updated_at: i64,
) -> WorkBuddyAccountStatus {
    let (credits, credits_error) = match credits {
        Ok(value) => (value, None),
        Err(error) => (None, Some(error)),
    };
    let (today_reward, streak_days, activity_error) = match activity {
        Ok((reward, streak)) => (reward, streak, None),
        Err(error) => (None, None, Some(error)),
    };
    WorkBuddyAccountStatus {
        credits,
        today_reward,
        streak_days,
        updated_at,
        credits_error,
        activity_error,
    }
}

pub async fn query_account_status(account_id: &str) -> Result<WorkBuddyAccountStatus, String> {
    let client = tauri::async_runtime::spawn_blocking(|| {
        crate::utils::http::create_domestic_client(30, "copilot.tencent.com")
    })
    .await
    .map_err(|_| "余额查询初始化失败".to_string())?;
    let token = crate::modules::workbuddy_account::access_token(account_id)?;
    let (credits_payload, activity_payload) = query_both(&client, &token).await;
    let credits = credits_payload.and_then(|payload| {
        parse_available_credits(&payload).ok_or_else(|| "积分汇总未返回有效余额".to_string())
    });
    let activity = activity_payload.and_then(|payload| {
        let parsed = parse_activity(&payload);
        if parsed.0.is_none() && parsed.1.is_none() {
            Err("签到状态查询返回结构不完整".to_string())
        } else {
            Ok(parsed)
        }
    });
    Ok(assemble_status(
        credits.map(Some),
        activity,
        chrono::Utc::now().timestamp(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        assemble_status, network_error_message, parse_activity, parse_available_credits,
        ACTIVITY_URL, RESOURCE_URL,
    };
    use serde_json::json;

    #[test]
    fn workbuddy_status_uses_only_read_only_endpoints() {
        assert!(RESOURCE_URL.ends_with("get-user-resource-summary"));
        assert!(ACTIVITY_URL.ends_with("checkin-activity-status"));
        let write_endpoint = concat!("daily", "-checkin");
        assert!(!RESOURCE_URL.contains(write_endpoint));
        assert!(!ACTIVITY_URL.contains(write_endpoint));
    }

    #[test]
    fn workbuddy_summary_includes_base_rewards_and_addons_without_double_counting() {
        let payload = json!({"code":0,"data":{"Packages":[
            {"CycleRemainCapacity":"2932.12000031","CycleTotalCapacity":"3400"},
            {"CycleRemainCapacity":"500"},{"CycleRemainCapacity":1000}],
            "TotalCredits":"4432.12000031","CreditGroups":[{"CycleRemainCapacity":"4432.12000031"}]}});
        assert_eq!(parse_available_credits(&payload), Some(4432.12));
    }

    #[test]
    fn workbuddy_summary_zero_is_not_a_missing_or_failed_query() {
        assert_eq!(
            parse_available_credits(&json!({"code":0,"data":{"Packages":[]}})),
            Some(0.0)
        );
        assert_eq!(
            parse_available_credits(&json!({"code":0,"data":{"Packages":[
            {"CycleRemainCapacity":"0","CycleTotalCapacity":500}]}})),
            Some(0.0)
        );
        assert_eq!(parse_available_credits(&json!({})), None);
        assert_eq!(parse_available_credits(&json!({"code":0,"data":{}})), None);
        assert_eq!(
            parse_available_credits(&json!({"code":401,"data":{"Packages":[]}})),
            None
        );
    }

    #[test]
    fn workbuddy_summary_rejects_invalid_values_instead_of_silently_omitting_packages() {
        for value in [json!(true), json!(null), json!("NaN"), json!("unknown")] {
            let payload = json!({"code":0,"data":{"Packages":[{"CycleRemainCapacity":100},
                {"CycleRemainCapacity":value}]}});
            assert_eq!(parse_available_credits(&payload), None);
        }
    }

    #[test]
    fn workbuddy_activity_accepts_current_and_legacy_reward_fields() {
        assert_eq!(
            parse_activity(&json!({"data": {"today_credit": 100, "streak_days": 7}})),
            (Some(100.0), Some(7)),
        );
        assert_eq!(
            parse_activity(&json!({"data": {"daily_credit": "88.5", "streak_days": "3"}})),
            (Some(88.5), Some(3)),
        );
    }

    #[test]
    fn workbuddy_status_keeps_partial_success_and_endpoint_error() {
        let status = assemble_status(
            Ok(Some(564.51)),
            Err("签到状态查询返回 HTTP 503".to_string()),
            123,
        );

        assert_eq!(status.credits, Some(564.51));
        assert_eq!(status.today_reward, None);
        assert_eq!(status.streak_days, None);
        assert_eq!(status.updated_at, 123);
        assert_eq!(status.credits_error, None);
        assert_eq!(
            status.activity_error.as_deref(),
            Some("签到状态查询返回 HTTP 503")
        );
    }

    #[test]
    fn workbuddy_network_error_is_not_reported_as_token_expiry() {
        let message = network_error_message("积分查询", "connection refused");
        assert!(message.contains("国内网络请求失败"));
        assert!(!message.contains("认证已失效"));
    }

    #[test]
    fn workbuddy_business_auth_codes_are_refreshable() {
        assert!(
            super::business_error(&json!({"code": 401, "message": "unauthorized"}))
                .unwrap()
                .contains("认证已失效")
        );
        assert!(
            super::business_error(&json!({"code": 403, "message": "forbidden"}))
                .unwrap()
                .contains("认证已失效")
        );
    }
}

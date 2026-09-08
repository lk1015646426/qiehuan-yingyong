//! 智谱清言只读积分查询。
//!
//! 红线：本模块只调用只读接口，绝不执行签到/领取。已验证的端点：
//! - 积分/活动状态：GET https://chatglm.cn/chatglm/member-api/member/score_activity_status
//!   响应 {"status":0,"result":{"current_score":948,"status":-2,...}}
//! 鉴权：`Authorization: Bearer <chatglm_token>`（无需签名头）。
//! 签到接口（daily_login_score）只能由云端 GitHub Actions 链路调用。

use crate::models::zhipu::ZhipuAccountStatus;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde_json::Value;
use std::time::Instant;

const SCORE_URL: &str = "https://chatglm.cn/chatglm/member-api/member/score_activity_status";

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

/// 解析积分响应：current_score（当前积分）+ status（活动状态原始值）。
fn parse_score(payload: &Value) -> Result<(Option<f64>, Option<i64>), String> {
    let result = payload
        .get("result")
        .filter(|value| !value.is_null())
        .ok_or_else(|| "积分响应缺少 result".to_string())?;
    let score = number(result.get("current_score"));
    let status = integer(result.get("status"));
    if score.is_none() && status.is_none() {
        return Err("积分响应结构不完整".to_string());
    }
    Ok((score, status))
}

fn business_error(payload: &Value) -> Option<String> {
    let status = integer(payload.get("status"))?;
    if status == 0 {
        return None;
    }
    let message = payload
        .get("message")
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
    if status == 40001 || status == 40100 || status == 401 {
        return Some(format!("登录态已失效：{message} (status={status})"));
    }
    Some(format!("{message} (status={status})"))
}

async fn query_score(
    client: &reqwest::Client,
    token: &str,
    label: &str,
) -> Result<Value, String> {
    let started = Instant::now();
    crate::modules::logger::log_info(&format!(
        "[Zhipu Network] request_start label={} url={}",
        label, SCORE_URL
    ));
    let response = client
        .get(SCORE_URL)
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(ACCEPT, "application/json")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|error| {
            let detail = crate::utils::http::format_request_error(label, &error);
            crate::modules::logger::log_warn(&format!(
                "[Zhipu Network] request_failed label={} elapsed_ms={} detail={}",
                label,
                started.elapsed().as_millis(),
                detail
            ));
            detail
        })?;
    let status = response.status();
    crate::modules::logger::log_info(&format!(
        "[Zhipu Network] response_received label={} status={} elapsed_ms={}",
        label,
        status.as_u16(),
        started.elapsed().as_millis()
    ));
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(format!("{label}登录态已失效：请重新打开智谱清言客户端并点击「导入当前账号」更新"));
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
    error.contains("登录态已失效")
}

/// 导入时的在线验证：认证失败返回 Err（拒绝导入），网络失败返回 Ok(false)。
pub async fn verify_token_online(token: &str) -> Result<bool, String> {
    let client = crate::utils::http::create_domestic_client(15, "chatglm.cn");
    match query_score(&client, token, "登录验证").await {
        Ok(_) => Ok(true),
        Err(error) if is_auth_error(&error) => Err(error),
        Err(_) => Ok(false),
    }
}

pub async fn query_account_status(account_id: &str) -> Result<ZhipuAccountStatus, String> {
    let client = crate::utils::http::create_domestic_client(30, "chatglm.cn");
    let token = crate::modules::zhipu_account::access_token(account_id)?;
    let payload = query_score(&client, &token, "积分查询").await?;
    let (current_score, activity_status) = match parse_score(&payload) {
        Ok(value) => value,
        Err(error) => {
            return Ok(ZhipuAccountStatus {
                current_score: None,
                activity_status: None,
                updated_at: chrono::Utc::now().timestamp(),
                score_error: Some(error),
            })
        }
    };
    Ok(ZhipuAccountStatus {
        current_score,
        activity_status,
        updated_at: chrono::Utc::now().timestamp(),
        score_error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{business_error, parse_score, SCORE_URL};
    use serde_json::json;

    #[test]
    fn zhipu_status_uses_only_read_only_endpoint() {
        assert!(SCORE_URL.contains("score_activity_status"));
        let write_endpoint = "daily_login_score";
        assert!(!SCORE_URL.contains(write_endpoint));
    }

    #[test]
    fn zhipu_score_parses_current_score_and_activity_status() {
        // 真实样本（2026-09-08 抓取）。
        let payload = json!({
            "status": 0, "message": "success",
            "result": {"status": -2, "remain_days": 0, "current_score": 948, "target_score": 0}
        });
        assert_eq!(parse_score(&payload).unwrap(), (Some(948.0), Some(-2)));
    }

    #[test]
    fn zhipu_score_rejects_incomplete_payload() {
        assert!(parse_score(&json!({"status": 0, "result": {}})).is_err());
        assert!(parse_score(&json!({"status": 0})).is_err());
    }

    #[test]
    fn zhipu_business_error_flags_expired_login() {
        assert!(business_error(&json!({"status": 40001, "message": "bad request"}))
            .unwrap()
            .contains("登录态已失效"));
        assert!(business_error(&json!({"status": 0, "message": "ok"})).is_none());
        assert!(business_error(&json!({"status": 500, "message": "boom"})).is_some());
    }
}

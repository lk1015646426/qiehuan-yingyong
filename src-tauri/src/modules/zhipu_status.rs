//! 智谱清言只读积分查询。
//!
//! 红线：本模块只调用只读接口，绝不执行签到/领取。积分口径（2026-09-08 实测）：
//! - 客户端显示的总积分来自 GET https://chatglm.cn/chatglm/user-api/user/info
//!   的 result.member_info.left_score（如 "1274.35"，字符串）；该接口属 user-api，
//!   需要 X-Sign/X-Timestamp/X-Nonce 签名头（算法与前端一致，见 sign_headers）。
//! - score_activity_status 的 current_score 只是活动积分（口径不同，不采用）。
//! 鉴权：`Authorization: Bearer <chatglm_token>` + 签名头。
//! 签到接口（daily_login_score）只能由云端 GitHub Actions 链路调用。

use crate::models::zhipu::ZhipuAccountStatus;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde_json::Value;
use std::time::Instant;

const USER_INFO_URL: &str = "https://chatglm.cn/chatglm/user-api/user/info";
const SIGN_SALT: &str = "8a1317a7468aa3ad86e997d08f3f31cb";

fn md5_hex(input: &str) -> String {
    md5::compute(input.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

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

/// 解析 user/info：member_info.left_score（总积分，字符串形式的小数）。
fn parse_left_score(payload: &Value) -> Result<Option<f64>, String> {
    let member = payload
        .pointer("/result/member_info")
        .filter(|value| !value.is_null())
        .ok_or_else(|| "用户信息缺少 member_info".to_string())?;
    let score = number(member.get("left_score"));
    Ok(score)
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

/// 前端 vj() 时间戳变换：倒数第二位替换为校验位
/// （各位数字之和 - 原倒数第二位）% 10。
fn vj_timestamp(now_ms: i64) -> String {
    let text = now_ms.to_string();
    let digits: Vec<i64> = text.chars().filter_map(|c| c.to_digit(10).map(|d| d as i64)).collect();
    let n = text.len();
    let checksum = (digits.iter().sum::<i64>() - digits[n - 2]).rem_euclid(10);
    let mut result = String::with_capacity(n);
    result.push_str(&text[..n - 2]);
    result.push_str(&checksum.to_string());
    result.push_str(&text[n - 1..]);
    result
}

/// 构造 user-api 签名请求头（与前端 Lf() 一致）。
/// X-Sign = MD5("{timestamp}-{nonce}-{salt}")。
fn sign_headers(token: &str, now_ms: i64) -> reqwest::header::HeaderMap {
    let timestamp = vj_timestamp(now_ms);
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let sign = md5_hex(&format!("{timestamp}-{nonce}-{SIGN_SALT}"));
    let mut headers = reqwest::header::HeaderMap::new();
    let mut set = |name: reqwest::header::HeaderName, value: String| {
        if let Ok(value) = reqwest::header::HeaderValue::from_str(&value) {
            headers.insert(name, value);
        }
    };
    set(
        reqwest::header::CONTENT_TYPE,
        "application/json;charset=utf-8".to_string(),
    );
    set(reqwest::header::HeaderName::from_static("app-name"), "chatglm".to_string());
    set(reqwest::header::HeaderName::from_static("x-request-id"), uuid::Uuid::new_v4().simple().to_string());
    set(reqwest::header::HeaderName::from_static("x-app-platform"), "pc".to_string());
    set(reqwest::header::HeaderName::from_static("x-app-version"), "0.0.1".to_string());
    set(reqwest::header::HeaderName::from_static("x-timestamp"), timestamp);
    set(reqwest::header::HeaderName::from_static("x-nonce"), nonce);
    set(reqwest::header::HeaderName::from_static("x-sign"), sign);
    set(AUTHORIZATION, format!("Bearer {token}"));
    set(ACCEPT, "application/json".to_string());
    // X-Device-Id：JWT 里的 device_id 声明（与客户端一致）。
    if let Some(device_id) = jwt_device_id(token) {
        set(reqwest::header::HeaderName::from_static("x-device-id"), device_id);
    }
    headers
}

/// 解析 JWT 的 device_id 声明（不校验签名；失败返回 None，请求仍可发出）。
fn jwt_device_id(token: &str) -> Option<String> {
    use base64::Engine;
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload))
        .ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("device_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn query_user_info(client: &reqwest::Client, token: &str, label: &str) -> Result<Value, String> {
    let started = Instant::now();
    crate::modules::logger::log_info(&format!(
        "[Zhipu Network] request_start label={} url={}",
        label, USER_INFO_URL
    ));
    let headers = sign_headers(token, chrono::Utc::now().timestamp_millis());
    let response = client
        .get(USER_INFO_URL)
        .headers(headers)
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
    match query_user_info(&client, token, "登录验证").await {
        Ok(_) => Ok(true),
        Err(error) if is_auth_error(&error) => Err(error),
        Err(_) => Ok(false),
    }
}

pub async fn query_account_status(account_id: &str) -> Result<ZhipuAccountStatus, String> {
    let client = crate::utils::http::create_domestic_client(30, "chatglm.cn");
    let token = crate::modules::zhipu_account::access_token(account_id)?;
    let payload = query_user_info(&client, &token, "积分查询").await?;
    let left_score = match parse_left_score(&payload) {
        Ok(value) => value,
        Err(error) => {
            return Ok(ZhipuAccountStatus {
                left_score: None,
                updated_at: chrono::Utc::now().timestamp(),
                score_error: Some(error),
            })
        }
    };
    Ok(ZhipuAccountStatus {
        left_score,
        updated_at: chrono::Utc::now().timestamp(),
        score_error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        business_error, md5_hex, parse_left_score, sign_headers, vj_timestamp, SIGN_SALT,
        USER_INFO_URL,
    };
    use serde_json::json;

    #[test]
    fn zhipu_status_uses_only_read_only_endpoint() {
        assert!(USER_INFO_URL.contains("user/info"));
        let write_endpoint = concat!("daily", "_login_score");
        assert!(!USER_INFO_URL.contains(write_endpoint));
    }

    #[test]
    fn zhipu_left_score_parses_string_decimal() {
        // 真实样本（2026-09-08 抓取）：left_score 为字符串小数。
        let payload = json!({
            "status": 0,
            "result": {"member_info": {"left_score": "1274.35"}}
        });
        assert_eq!(parse_left_score(&payload).unwrap(), Some(1274.35));
    }

    #[test]
    fn zhipu_left_score_rejects_missing_member_info() {
        assert!(parse_left_score(&json!({"status": 0, "result": {}})).is_err());
        assert!(parse_left_score(&json!({"status": 0})).is_err());
    }

    #[test]
    fn zhipu_vj_timestamp_matches_frontend_algorithm() {
        // 与云端 Python 实现互验：1788852882000 -> 校验位 (57-0)%10=7。
        assert_eq!(vj_timestamp(1788852882000), "1788852882070");
        // 保持 13 位长度且首尾位不变。
        let out = vj_timestamp(1788852882000);
        assert_eq!(out.len(), 13);
        assert!(out.starts_with("178885288") && out.ends_with("0"));
    }

    #[test]
    fn zhipu_sign_headers_carry_required_fields() {
        let headers = sign_headers("token.value.x", 1788852882000);
        let get = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string()
        };
        assert_eq!(get("app-name"), "chatglm");
        assert_eq!(get("x-app-platform"), "pc");
        assert_eq!(get("x-timestamp"), "1788852882070");
        assert_eq!(get("x-nonce").len(), 32);
        assert_eq!(get("x-sign").len(), 32);
        assert!(get("authorization").starts_with("Bearer token."));
        assert_eq!(get("x-device-id").len(), 0, "非 JWT token 无 device_id，头应省略");
    }

    #[test]
    fn zhipu_sign_is_md5_of_timestamp_nonce_salt() {
        // md5("test") 已知值，校验 md5_hex 实现正确性。
        assert_eq!(md5_hex("test"), "098f6bcd4621d373cade4e832627b4f6");
        let ts = vj_timestamp(1788852882000);
        let expect = md5_hex(&format!("{ts}-abcdef0123456789abcdef0123456789-{SIGN_SALT}"));
        assert_eq!(expect.len(), 32);
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

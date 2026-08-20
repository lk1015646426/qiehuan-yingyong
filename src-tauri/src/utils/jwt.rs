//! JWT 工具：从 token 的 payload 段离线解析非敏感时间戳。
//!
//! 从 `work_cn_github.rs` 抽取到此处，供 GitHub 同步、账号视图、签到面板
//! 共用，避免 `trae_account → work_cn_github` 的反向依赖。

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

fn parse_jwt_timestamp_claim(token: &str, claim: &str) -> Option<i64> {
    let parts: Vec<&str> = token.split('.').collect();
    let payload = parts.get(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    value.get(claim)?.as_i64()
}

/// Parse `exp` (seconds) out of a JWT access token's payload segment.
pub fn parse_jwt_exp(token: &str) -> Option<i64> {
    parse_jwt_timestamp_claim(token, "exp")
}

/// Parse `iat` (seconds) out of a JWT access token's payload segment.
pub fn parse_jwt_iat(token: &str) -> Option<i64> {
    parse_jwt_timestamp_claim(token, "iat")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iat_without_exposing_token_payload() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"iat":123,"exp":456}"#);
        let token = format!("header.{payload}.signature");

        assert_eq!(parse_jwt_iat(&token), Some(123));
        assert_eq!(parse_jwt_iat("not-a-jwt"), None);
    }
}

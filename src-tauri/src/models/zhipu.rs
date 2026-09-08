use serde::{Deserialize, Serialize};

/// 智谱清言账号对前端展示的脱敏视图。token 原文只存在加密详情文件中，
/// 任何视图/日志都只出现 uid 哈希后的账号 ID 或用户名。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZhipuAccountView {
    pub id: String,
    pub display_name: String,
    /// JWT sub 声明里的用户标识（如「用户名_T9PBW7」），脱敏展示用。
    pub user_label: String,
    pub checkin_enabled: bool,
    /// access token 过期时间（秒级时间戳）。
    pub token_expires_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_github_sync_at: Option<i64>,
    pub last_github_sync_state: String,
    #[serde(default)]
    pub last_github_sync_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZhipuAccountUpdate {
    pub display_name: Option<String>,
    pub checkin_enabled: Option<bool>,
}

/// 只读积分状态：清言 member-api 的 score_activity_status。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ZhipuAccountStatus {
    /// 当前积分余额（current_score）。
    pub current_score: Option<f64>,
    /// 活动状态原始值（-2 = 无进行中活动；正数为活动进行中）。
    pub activity_status: Option<i64>,
    pub updated_at: i64,
    pub score_error: Option<String>,
}

/// 导入时在线验证的结果提示：认证失败直接拒绝导入，网络失败仅提示。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ZhipuImportNotice {
    /// 网络原因未能完成在线验证（账号已保存，可稍后刷新积分确认）。
    pub verification_skipped: bool,
    pub message: String,
}

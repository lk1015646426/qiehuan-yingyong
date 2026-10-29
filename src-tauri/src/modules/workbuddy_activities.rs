//! 活动配置与只读总览。写入活动由原云签到仓库执行；凭证仍由桌面端统一续期。
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const SETTINGS: &str = "workbuddy_activities.json";
const BASE: &str = "https://copilot.tencent.com/v2/activity/growth";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ActivitySettings {
    pub enabled: bool,
    pub compensation: bool,
    pub tasks: bool,
    pub travel: bool,
    pub expiry_days: u32,
}

impl Default for ActivitySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            compensation: true,
            tasks: true,
            travel: true,
            expiry_days: 7,
        }
    }
}

impl ActivitySettings {
    fn validate(&self) -> Result<(), String> {
        if !(1..=30).contains(&self.expiry_days) {
            return Err("到期提醒天数应在 1 到 30 之间".into());
        }
        Ok(())
    }
}

pub fn load_settings() -> Result<ActivitySettings, String> {
    let path = super::account::get_data_dir()?.join(SETTINGS);
    if !path.exists() {
        return Ok(ActivitySettings::default());
    }
    let raw = std::fs::read_to_string(path).map_err(|_| "读取活动配置失败")?;
    let settings: ActivitySettings =
        serde_json::from_str(&raw).map_err(|_| "活动配置损坏，请重新保存")?;
    settings.validate()?;
    Ok(settings)
}

#[tauri::command]
pub fn get_workbuddy_activity_settings() -> Result<ActivitySettings, String> {
    load_settings()
}

#[tauri::command]
pub async fn save_workbuddy_activity_settings(
    settings: ActivitySettings,
) -> Result<ActivitySettings, String> {
    settings.validate()?;
    let saved = settings.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let path = super::account::get_data_dir()?.join(SETTINGS);
        let raw = serde_json::to_string_pretty(&saved).map_err(|_| "活动配置序列化失败")?;
        super::atomic_write::write_string_atomic(&path, &raw).map_err(|_| "保存活动配置失败")?;
        super::workbuddy_github::mark_dataset_changed()?;
        Ok(())
    })
    .await
    .map_err(|_| "活动配置保存任务未完成")??;
    super::workbuddy_github::sync_background_now("activity settings")
        .await
        .map_err(|_| {
            "活动配置已保存在本机，但云端同步失败，请检查 GitHub 连接后重试".to_string()
        })?;
    Ok(settings)
}

async fn query(
    client: &reqwest::Client,
    token: &str,
    url: &str,
    post: bool,
) -> Result<Value, String> {
    let builder = if post {
        client.post(url).json(&json!({}))
    } else {
        client.get(url)
    };
    let response = builder.bearer_auth(token)
        .header("Origin", "https://www.codebuddy.cn")
        .header("Referer", "https://www.codebuddy.cn/")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/124.0.0.0 Safari/537.36")
        .send().await.map_err(|_| "网络请求未完成，请稍后重试".to_string())?;
    if response.status().as_u16() == 401 {
        return Err("登录凭证被拒绝，请检查账号同步状态".into());
    }
    if !response.status().is_success() {
        return Err(format!("接口返回 HTTP {}", response.status().as_u16()));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| "返回数据无法解析".to_string())?;
    if body.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("接口尚未返回有效活动数据".into());
    }
    Ok(body.get("data").cloned().unwrap_or(Value::Null))
}

fn end_timestamp(value: &Value) -> Option<i64> {
    if let Some(raw) = value.as_i64().or_else(|| value.as_str()?.parse().ok()) {
        return Some(if raw > 10_000_000_000 {
            raw / 1000
        } else {
            raw
        });
    }
    let text = value.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|v| v.timestamp())
        .or_else(|| {
            let date = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S").ok()?;
            FixedOffset::east_opt(8 * 3600)?
                .from_local_datetime(&date)
                .single()
                .map(|v| v.timestamp())
        })
}

fn number_value(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    let number = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())?;
    number.is_finite().then_some(number)
}

fn tasks_view(data: &Value, now: i64) -> Result<Vec<Value>, String> {
    let tasks = data
        .get("tasks")
        .and_then(Value::as_array)
        .ok_or("任务列表结构无法识别")?;
    tasks.iter().map(|task| {
        let code = task.get("task_code").and_then(Value::as_str).ok_or("任务编号缺失")?;
        let state = task.get("accept_status").and_then(Value::as_str).unwrap_or("unknown");
        let end = task.get("valid_end").and_then(end_timestamp);
        let start = task.get("valid_start").and_then(end_timestamp);
        let locked = task.get("locked").and_then(Value::as_bool).unwrap_or(false)
            || end.is_some_and(|v| v < now) || start.is_some_and(|v| v > now);
        Ok(json!({"code": code, "title": task.get("title").and_then(Value::as_str).unwrap_or(code),
            "state": state, "current": number_value(task.pointer("/progress/current")), "target": number_value(task.pointer("/progress/target")),
            "rewardCredit": number_value(task.get("reward_credit")), "locked": locked, "endsAt": end}))
    }).collect()
}

fn credit_expiries(data: &Value, now: i64, days: u32) -> Result<Vec<Value>, String> {
    let packages = data
        .pointer("/Response/Data/Accounts")
        .and_then(Value::as_array)
        .ok_or("积分包结构无法识别")?;
    Ok(packages
        .iter()
        .filter_map(|p| {
            if p.get("Status").and_then(Value::as_i64) != Some(0)
                || p.get("CapacityType").and_then(Value::as_i64) == Some(4)
            {
                return None;
            }
            let remain = p
                .get("CapacityRemainPrecise")
                .and_then(Value::as_str)
                .and_then(|v| v.parse::<f64>().ok())
                .or_else(|| p.get("CapacityRemain").and_then(Value::as_f64))?;
            let end = [
                "DeductionEndTime",
                "ExpiredTime",
                "ExpireTime",
                "CycleEndTime",
            ]
            .iter()
            .find_map(|key| p.get(*key).and_then(end_timestamp))?;
            if !remain.is_finite()
                || remain <= 0.0
                || end < now
                || end > now + i64::from(days) * 86400
            {
                return None;
            }
            Some(json!({"credits": remain, "endsAt": end}))
        })
        .collect())
}

fn travel_view(status: &Value, buddy_info: Option<&Value>) -> Result<Value, String> {
    let state = status
        .get("state")
        .and_then(Value::as_str)
        .ok_or("旅行状态缺失")?;
    if !["idle", "traveling", "arrived"].contains(&state) {
        return Err("旅行状态无法识别".into());
    }
    let state = if status.get("buddy_id").and_then(Value::as_i64) == Some(0) {
        let buddy = buddy_info
            .and_then(|v| v.get("buddy"))
            .ok_or("Buddy 拥有状态未查询到")?;
        if buddy.is_null() {
            "unowned"
        } else if buddy.as_object().is_some_and(|v| !v.is_empty()) {
            state
        } else {
            return Err("Buddy 拥有状态无法识别".into());
        }
    } else {
        state
    };
    Ok(
        json!({"state":state, "arriveAt":status.get("arrive_at"), "dailyLimitReached":status.get("daily_limit_reached")}),
    )
}

#[tauri::command]
pub async fn get_workbuddy_activity_overview(account_id: String) -> Result<Value, String> {
    let token = super::workbuddy_account::access_token(&account_id)?;
    let settings = load_settings()?;
    let client = tauri::async_runtime::spawn_blocking(|| {
        crate::utils::http::create_domestic_client(20, "copilot.tencent.com")
    })
    .await
    .map_err(|_| "活动查询初始化失败")?;
    let tasks_url = format!("{BASE}/tasks");
    let travel_url = format!("{BASE}/buddy/travel/status");
    let buddy_url = format!("{BASE}/buddy/info");
    let energy_url = format!("{BASE}/energy");
    let (tasks, travel, buddy, energy, credits, compensation) = tokio::join!(
        query(&client, &token, &tasks_url, false),
        query(&client, &token, &travel_url, false),
        query(&client, &token, &buddy_url, false),
        query(&client, &token, &energy_url, false),
        query(
            &client,
            &token,
            "https://copilot.tencent.com/v2/billing/meter/get-user-resource",
            true
        ),
        query(
            &client,
            &token,
            "https://www.codebuddy.cn/billing/meter/compensation-status",
            true
        ),
    );
    let now = Utc::now().timestamp();
    let mut errors = Vec::new();
    let tasks = match tasks.and_then(|v| tasks_view(&v, now)) {
        Ok(v) => v,
        Err(e) => {
            errors.push(format!("任务：{e}"));
            Vec::new()
        }
    };
    let travel = match travel.and_then(|v| travel_view(&v, buddy.as_ref().ok())) {
        Ok(v) => v,
        Err(e) => {
            errors.push(format!("旅行：{e}"));
            Value::Null
        }
    };
    let energy = match energy {
        Ok(v) => v.get("balance").cloned().unwrap_or(Value::Null),
        Err(e) => {
            errors.push(format!("能量：{e}"));
            Value::Null
        }
    };
    let expiries = match credits.and_then(|v| credit_expiries(&v, now, settings.expiry_days)) {
        Ok(v) => v,
        Err(e) => {
            errors.push(format!("到期查询：{e}"));
            Vec::new()
        }
    };
    let compensation = match compensation {
        Ok(v) => {
            if v.get("claimed").and_then(Value::as_bool) == Some(true) {
                "已领取"
            } else if v.get("active").and_then(Value::as_bool) == Some(true) {
                "可领取"
            } else {
                "暂无可领补偿"
            }
        }
        Err(e) => {
            errors.push(format!("补偿：{e}"));
            "查询未完成"
        }
    };
    Ok(
        json!({"accountId": account_id, "checkedAt": now, "tasks": tasks, "travel": travel,
        "energy": energy, "expiries": expiries, "compensation": compensation, "errors": errors}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_zero_travel_id_does_not_hide_an_owned_buddy() {
        let travel = json!({"state":"idle", "buddy_id":0});
        assert_eq!(
            travel_view(&travel, Some(&json!({"buddy":{"instance_id":123}}))).unwrap()["state"],
            "idle"
        );
        assert_eq!(
            travel_view(&travel, Some(&json!({"buddy":null}))).unwrap()["state"],
            "unowned"
        );
        assert!(travel_view(&travel, None).is_err());
        assert!(travel_view(&travel, Some(&json!({}))).is_err());
    }
    #[test]
    fn activities_default_to_off_without_changing_other_features() {
        let settings: ActivitySettings = serde_json::from_str("{}").unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.expiry_days, 7);
        assert!(settings.validate().is_ok());
        assert!(ActivitySettings {
            expiry_days: 0,
            ..settings
        }
        .validate()
        .is_err());
    }
    #[test]
    fn task_progress_and_expiry_are_not_reported_as_completion() {
        let tasks = tasks_view(
            &json!({"tasks": [{"task_code":"test", "title":"任务",
            "accept_status":"accepted", "progress":{"current":2,"target":5},
            "valid_end":"2026-10-01T00:00:00Z"}]}),
            1792290000,
        )
        .unwrap();
        assert_eq!(tasks[0]["state"], "accepted");
        assert_eq!(tasks[0]["current"], 2.0);
        assert_eq!(tasks[0]["locked"], true);
        assert!(tasks_view(&json!({}), 0).is_err());
    }
    #[test]
    fn actual_package_expiry_preserves_decimal_credits_and_excludes_trial() {
        let packages = json!({"Response":{"Data":{"Accounts":[
            {"Status":0,"CapacityType":1,"CapacityRemainPrecise":"2.96","DeductionEndTime":1792376400000i64},
            {"Status":0,"CapacityType":4,"CapacityRemain":500,"DeductionEndTime":1792376400000i64}]}}});
        let rows = credit_expiries(&packages, 1792290000, 7).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["credits"], 2.96);
        assert_eq!(
            end_timestamp(&json!("2026-10-19 05:59:58")),
            Some(1792360798)
        );
    }
}

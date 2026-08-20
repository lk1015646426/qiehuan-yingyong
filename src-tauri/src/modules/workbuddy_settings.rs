use crate::models::workbuddy::WorkBuddySettings;
use std::path::PathBuf;

const SETTINGS_FILE: &str = "workbuddy_settings.json";

pub fn settings_path() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(SETTINGS_FILE))
}

fn clean_path(value: Option<String>) -> Option<String> {
    value.and_then(|path| {
        let path = path.trim().to_string();
        (!path.is_empty()).then_some(path)
    })
}

pub fn expand_environment_path(path: &str) -> String {
    let mut expanded = path.to_string();
    let mut cursor = 0;
    while let Some(start_offset) = expanded[cursor..].find('%') {
        let start = cursor + start_offset;
        let value_start = start + 1;
        let Some(end_offset) = expanded[value_start..].find('%') else {
            break;
        };
        let end = value_start + end_offset;
        let variable = &expanded[value_start..end];
        if variable.is_empty() {
            cursor = end + 1;
            continue;
        }
        if let Ok(value) = std::env::var(variable) {
            expanded.replace_range(start..=end, &value);
            cursor = start + value.len();
        } else {
            cursor = end + 1;
        }
    }
    expanded
}

pub fn load_workbuddy_settings() -> WorkBuddySettings {
    let Ok(path) = settings_path() else {
        return WorkBuddySettings::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_workbuddy_settings(
    mut settings: WorkBuddySettings,
) -> Result<WorkBuddySettings, String> {
    settings.executable_path = clean_path(settings.executable_path);
    settings.auth_file_path = clean_path(settings.auth_file_path);
    let content = serde_json::to_string_pretty(&settings)
        .map_err(|error| format!("序列化 WorkBuddy 设置失败: {error}"))?;
    crate::modules::atomic_write::write_string_atomic(&settings_path()?, &content)
        .map_err(|error| format!("保存 WorkBuddy 设置失败: {error}"))?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    #[test]
    fn expands_windows_environment_variable_in_manual_auth_path() {
        std::env::set_var("WORKBUDDY_TEST_LOCAL", r"C:\\Users\\tester\\AppData\\Local");
        assert_eq!(
            super::expand_environment_path(r"%WORKBUDDY_TEST_LOCAL%\\auth\\workbuddy-desktop.info"),
            r"C:\\Users\\tester\\AppData\\Local\\auth\\workbuddy-desktop.info"
        );
        std::env::remove_var("WORKBUDDY_TEST_LOCAL");
    }
}

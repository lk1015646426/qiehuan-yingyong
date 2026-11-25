//! Conservative edits for the verified desktop profile format; preserve unknown rows.
use serde_json::{json, Value};
use std::path::PathBuf;

fn home() -> Result<PathBuf, String> {
    Ok(std::env::var_os("DSH_HOME")
        .map(PathBuf::from)
        .unwrap_or(dirs::home_dir().ok_or("找不到用户目录")?.join(".dsh")))
}
pub(crate) fn detect() -> Result<Value, String> {
    let root = home()?;
    let mut version =
        std::fs::read_to_string(root.join("dsh-runtimes/dsh-primary-runtime/runtime.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v["desktopVersion"].as_str().map(str::to_string));
    let mut installation: Option<String> = None;
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        if let Ok(uninstall) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall")
        {
            for name in uninstall.enum_keys().filter_map(Result::ok) {
                if let Ok(key) = uninstall.open_subkey(name) {
                    if key.get_value::<String, _>("DisplayName").ok().as_deref()
                        == Some("DeepSeek Harness")
                    {
                        let path = key.get_value::<String, _>("InstallLocation").ok();
                        if path
                            .as_ref()
                            .is_some_and(|p| PathBuf::from(p).join("DeepSeek Harness.exe").exists())
                        {
                            installation = path;
                            version = key
                                .get_value::<String, _>("DisplayVersion")
                                .ok()
                                .or(version);
                            break;
                        }
                    }
                }
            }
        }
    }
    let patch = root.join("profiles/desktop/cordis.patch.yml");
    let contents = std::fs::read_to_string(&patch).unwrap_or_default();
    let managed = contents
        .split_once("# BEGIN QIEHUAN MODEL SERVICE")
        .and_then(|(_, tail)| {
            tail.split_once("# END QIEHUAN MODEL SERVICE")
                .map(|(body, _)| body)
        });
    let mut configured_pools = Vec::new();
    let mut configured_pool = None;
    if let Some(body) = managed {
        for line in body.lines() {
            if let Some(row) = line
                .strip_prefix("- ")
                .and_then(|row| serde_json::from_str::<Value>(row).ok())
            {
                if row["id"] == "llm-pi-ai" {
                    let providers = &row["config"]["providers"];
                    for (id, pool) in [
                        ("qiehuan-workbuddy", "workbuddy"),
                        ("qiehuan-trae", "trae"),
                        ("qiehuan-doubao", "doubao"),
                    ] {
                        if providers.get(id).is_some() {
                            configured_pools.push(pool);
                        }
                    }
                    // Recognize the original one-pool preview configuration.
                    if let Some(old) = providers.get("qiehuan-pool") {
                        if old["baseURL"]
                            .as_str()
                            .is_some_and(|u| u.contains(":18791/"))
                        {
                            configured_pools.push("doubao");
                        } else if old["baseURL"]
                            .as_str()
                            .is_some_and(|u| u.contains(":18790/"))
                        {
                            configured_pools.push("trae");
                        } else if old["baseURL"]
                            .as_str()
                            .is_some_and(|u| u.contains(":18789/"))
                        {
                            configured_pools.push("workbuddy");
                        }
                    }
                }
                if row["id"] == "agent-default-model" {
                    let id = row["config"]["model"].as_str().unwrap_or("");
                    configured_pool = if id.starts_with("trae-pool-") {
                        Some("trae")
                    } else if id.starts_with("workbuddy-pool-") {
                        Some("workbuddy")
                    } else if id.starts_with("doubao-pool-") {
                        Some("doubao")
                    } else {
                        None
                    };
                }
            }
        }
    }
    Ok(
        json!({"detected":patch.exists(),"installed":installation.is_some(),"installation":installation,"version":version,"profile":"desktop","configPath":patch,"configured":managed.is_some(),"configuredPool":configured_pool,"configuredPools":configured_pools}),
    )
}

/// Remove only our marker block. Refuse malformed markers rather than guessing.
fn unmanaged(text: &str) -> Result<String, String> {
    let begin = "# BEGIN QIEHUAN MODEL SERVICE";
    let end = "# END QIEHUAN MODEL SERVICE";
    match (text.find(begin), text.find(end)) {
        (None, None) => Ok(text.to_string()),
        (Some(a), Some(b))
            if b > a && text.matches(begin).count() == 1 && text.matches(end).count() == 1 =>
        {
            Ok(format!("{}{}", &text[..a], &text[b + end.len()..]))
        }
        _ => Err("受管配置标记损坏，保留原文件".into()),
    }
}

fn patch(
    text: &str,
    workbuddy: &[String],
    trae: &[String],
    doubao: &[String],
    model: &str,
) -> Result<String, String> {
    let text = unmanaged(text)?;
    // Harness replaces the entire plugin config; never discard an existing provider.
    if text.contains("llm-pi-ai")
        || text.contains("qiehuan-pool")
        || text.contains("qiehuan-trae")
        || text.contains("qiehuan-workbuddy")
        || text.contains("qiehuan-doubao")
        || text.contains("!!js")
        || text.contains("---")
    {
        return Err(
            "发现已有自定义提供商或复杂 YAML，请在 Harness 模型设置中合并；未覆盖配置".into(),
        );
    }
    let mut kept = String::new();
    let mut skip = false;
    for line in text.lines() {
        if line.starts_with("- ") {
            skip = line.trim_end() == "- id: agent-default-model";
        }
        if !skip {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if kept.trim() == "[]" {
        kept.clear();
    }
    let mut providers = serde_json::Map::new();
    for (name, models, port, key) in [
        (
            "qiehuan-workbuddy",
            workbuddy,
            18789,
            "QIEHUAN_WORKBUDDY_API_KEY",
        ),
        ("qiehuan-trae", trae, 18790, "QIEHUAN_TRAE_API_KEY"),
        ("qiehuan-doubao", doubao, 18791, "QIEHUAN_DOUBAO_API_KEY"),
    ] {
        if !models.is_empty() {
            providers.insert(name.into(), json!({"displayName":name,"api":"openai-completions","baseURL":format!("http://127.0.0.1:{port}/v1"),"apiKeyEnv":key,"compat":{"supportsDeveloperRole":false,"maxTokensField":"max_tokens"},"models":models.iter().map(|id|json!({"id":id})).collect::<Vec<_>>() }));
        }
    }
    let provider = if workbuddy.iter().any(|id| id == model) {
        "qiehuan-workbuddy"
    } else if trae.iter().any(|id| id == model) {
        "qiehuan-trae"
    } else if doubao.iter().any(|id| id == model) {
        "qiehuan-doubao"
    } else {
        return Err("默认模型不在现有账号池选择中".into());
    };
    let entries = json!([
        {"id":"credentials","disabled":false},
        {"id":"llm-pi-ai","config":{"providers":providers}},
        {"id":"agent-default-model","config":{"provider":provider,"model":model}}
    ]);
    kept.push_str("\n# BEGIN QIEHUAN MODEL SERVICE\n");
    for entry in entries.as_array().unwrap() {
        kept.push_str(&format!("- {}\n", entry));
    }
    kept.push_str("# END QIEHUAN MODEL SERVICE\n");
    Ok(kept)
}

fn credentials(text: &str, name: &str, key: &str) -> Result<String, String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if text.contains(name) {
        let positions: Vec<_> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.starts_with(&format!("  {name}:")))
            .map(|(i, _)| i)
            .collect();
        if positions.len() != 1 {
            return Err("已有凭证引用格式不支持，未覆盖".into());
        }
        lines[positions[0]] = format!("  {name}: {}", json!(key));
    } else if let Some(i) = lines.iter().position(|l| l == "refs:") {
        lines.insert(i + 1, format!("  {name}: {}", json!(key)));
    } else if text.trim().is_empty() {
        lines = vec![
            "version: 1".into(),
            "refs:".into(),
            format!("  {name}: {}", json!(key)),
        ];
    } else {
        if text.contains("refs:")
            || !text.lines().any(|l| l == "version: 1")
            || text.contains("---")
        {
            return Err("凭证文件格式不支持，未覆盖".into());
        }
        lines.push("refs:".into());
        lines.push(format!("  {name}: {}", json!(key)));
    }
    Ok(format!("{}\n", lines.join("\n")))
}

pub(crate) fn configure(model: &str) -> Result<String, String> {
    let root = home()?;
    let file = root.join("profiles/desktop/cordis.patch.yml");
    if !file.exists() {
        return Err("尚未检测到 Harness desktop 配置，请先启动 Harness 一次".into());
    }
    if std::fs::read_to_string(root.join("cordis.patch.yml"))
        .unwrap_or_default()
        .contains("llm-pi-ai")
    {
        return Err("Harness 全局层覆盖模型配置，需先在 Harness 中处理；未修改".into());
    }
    let creds = root.join(".credentials.yaml");
    // Follow Harness' exclusive sibling-lock convention. Do not race a live editor.
    let paths = [
        file.with_file_name("cordis.patch.yml.lock"),
        root.join(".credentials.yaml.lock"),
    ];
    struct Locks(Vec<PathBuf>);
    impl Drop for Locks {
        fn drop(&mut self) {
            for p in &self.0 {
                let _ = std::fs::remove_file(p);
            }
        }
    }
    let mut locks = Locks(vec![]);
    for path in paths {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| "Harness 配置正在写入，请稍后重试")?;
        locks.0.push(path);
        writeln!(f, "{}", std::process::id()).map_err(|_| "配置锁写入失败")?;
    }
    let read_models = |filename: &str, prefix: &str| -> Result<Vec<String>, String> {
        let file = super::config::get_data_dir()?.join(filename);
        let contents = match std::fs::read_to_string(file) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err("读取账号池模型选择失败".into()),
        };
        let selection: Value =
            serde_json::from_str(&contents).map_err(|_| "账号池选择无效，未覆盖 Harness 配置")?;
        let ids = selection["modelIds"]
            .as_array()
            .ok_or("账号池模型列表无效")?;
        ids.iter()
            .map(|id| {
                id.as_str()
                    .map(|s| format!("{prefix}{s}"))
                    .ok_or_else(|| "模型 ID 无效".into())
            })
            .collect()
    };
    let workbuddy = read_models("workbuddy-pool-selection.json", "workbuddy-pool-")?;
    let trae = read_models("trae-model-pool.json", "trae-pool-")?;
    let doubao = read_models("doubao-pool-selection.json", "doubao-pool-")?;
    let original = std::fs::read_to_string(&file).map_err(|_| "读取 Harness 配置失败")?;
    let old_creds = match std::fs::read_to_string(&creds) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(_) => return Err("读取 Harness 凭证失败".into()),
    };
    let next = patch(&original, &workbuddy, &trae, &doubao, model)?;
    let mut next_creds = old_creds.clone();
    if !workbuddy.is_empty() {
        next_creds = credentials(
            &next_creds,
            "QIEHUAN_WORKBUDDY_API_KEY",
            &super::workbuddy_pool::load_pool_key()?,
        )?;
    }
    if !trae.is_empty() {
        next_creds = credentials(
            &next_creds,
            "QIEHUAN_TRAE_API_KEY",
            &super::trae_model_pool::key()?,
        )?;
    }
    if !doubao.is_empty() {
        next_creds = credentials(
            &next_creds,
            "QIEHUAN_DOUBAO_API_KEY",
            &super::doubao_pool::load_pool_key()?,
        )?;
    }
    let backup = root.join(format!("qiehuan-backup-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&backup).map_err(|_| "备份目录创建失败")?;
    std::fs::write(backup.join("cordis.patch.yml"), &original).map_err(|_| "配置备份失败")?;
    super::atomic_write::write_string_atomic(&creds, &next_creds)?;
    if let Err(e) = super::atomic_write::write_string_atomic(&file, &next) {
        return Err(format!(
            "配置保存失败（本地服务凭证已写入），备份位于 {}：{e}",
            backup.display()
        ));
    }
    Ok(format!(
        "已配置 desktop 默认模型。请重启 Harness 后新建会话；备份：{}",
        backup.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_rows_and_refuses_other_providers() {
        let v = patch(
            "- id: agent-default-model\n  config:\n    model: old\n- id: other\n  config: {}\n",
            &["m".into()],
            &[],
            &[],
            "m",
        )
        .unwrap();
        assert!(v.contains("- id: other"));
        assert!(!v.contains("model: old"));
        assert_eq!(
            patch(&v, &["m".into()], &[], &[], "m")
                .unwrap()
                .matches("# BEGIN QIEHUAN MODEL SERVICE")
                .count(),
            1
        );
        assert!(patch("- id: llm-pi-ai", &[], &[], &[], "m").is_err());
        assert!(unmanaged("# BEGIN QIEHUAN MODEL SERVICE").is_err());
    }
    #[test]
    fn selected_pool_uses_corresponding_local_endpoint() {
        for (id, endpoint) in [
            ("trae-pool-example", "18790/v1"),
            ("workbuddy-pool-example", "18789/v1"),
            ("doubao-pool-example", "18791/v1"),
        ] {
            let (workbuddy, trae, doubao) = if id.starts_with("trae-") {
                (vec![], vec![id.into()], vec![])
            } else if id.starts_with("doubao-") {
                (vec![], vec![], vec![id.into()])
            } else {
                (vec![id.into()], vec![], vec![])
            };
            let output = patch("[]", &workbuddy, &trae, &doubao, id).unwrap();
            assert!(output.contains(endpoint));
            assert!(output.contains(id));
        }
    }
    #[test]
    fn all_pools_remain_registered_when_default_changes() {
        let wb = vec!["workbuddy-pool-a".into()];
        let tr = vec!["trae-pool-b".into()];
        let db = vec!["doubao-pool-c".into()];
        let first = patch("[]", &wb, &tr, &db, &wb[0]).unwrap();
        let second = patch(&first, &wb, &tr, &db, &tr[0]).unwrap();
        let third = patch(&second, &wb, &tr, &db, &db[0]).unwrap();
        for output in [&first, &second, &third] {
            assert!(output.contains("127.0.0.1:18789/v1"));
            assert!(output.contains("127.0.0.1:18790/v1"));
            assert!(output.contains("127.0.0.1:18791/v1"));
            assert!(output.contains("QIEHUAN_WORKBUDDY_API_KEY"));
            assert!(output.contains("QIEHUAN_TRAE_API_KEY"));
            assert!(output.contains("QIEHUAN_DOUBAO_API_KEY"));
        }
        let first_creds = credentials("version: 1\n", "QIEHUAN_WORKBUDDY_API_KEY", "a").unwrap();
        let creds = credentials(&first_creds, "QIEHUAN_TRAE_API_KEY", "b").unwrap();
        let creds = credentials(&creds, "QIEHUAN_DOUBAO_API_KEY", "c").unwrap();
        assert!(creds.contains("QIEHUAN_WORKBUDDY_API_KEY: \"a\""));
        assert!(creds.contains("QIEHUAN_TRAE_API_KEY: \"b\""));
        assert!(creds.contains("QIEHUAN_DOUBAO_API_KEY: \"c\""));
    }
    #[test]
    fn preserves_existing_credentials() {
        let source = "version: 1\nrecords:\n  test/id:\n    kind: grant\n    payload: {}\n";
        let v = credentials(source, "QIEHUAN_POOL_API_KEY", "local-key").unwrap();
        assert!(v.starts_with(source));
        assert!(v.contains("refs:\n  QIEHUAN_POOL_API_KEY:"));
        assert_eq!(
            credentials(&v, "QIEHUAN_POOL_API_KEY", "new-key")
                .unwrap()
                .matches("QIEHUAN_POOL_API_KEY:")
                .count(),
            1
        );
        assert!(credentials("{refs: {}}", "QIEHUAN_POOL_API_KEY", "k").is_err());
    }
}

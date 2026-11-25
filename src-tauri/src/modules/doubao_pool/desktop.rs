//! Read the signed-in Windows Doubao profile without modifying it.
#[cfg(windows)]
pub fn read_cookie() -> Result<String, String> {
    use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
    use base64::{engine::general_purpose::STANDARD, Engine};
    use sha2::{Digest, Sha256};
    use windows::Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };
    let root = dirs::data_local_dir()
        .ok_or("找不到本地数据目录")?
        .join("Doubao/User Data");
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("Local State")).map_err(|_| "未找到豆包客户端数据")?,
    )
    .map_err(|_| "豆包客户端配置格式无效")?;
    let encoded = state["os_crypt"]["encrypted_key"]
        .as_str()
        .ok_or("豆包客户端加密密钥不可用")?;
    let encrypted = STANDARD
        .decode(encoded)
        .map_err(|_| "豆包客户端密钥格式无效")?;
    let raw = encrypted
        .strip_prefix(b"DPAPI")
        .ok_or("豆包客户端使用了尚不支持的加密方式")?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: raw.len() as u32,
        pbData: raw.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&input, None, None, None, None, 0, &mut output)
            .map_err(|_| "无法解密豆包本机密钥，请使用登录豆包的 Windows 账号")?;
    }
    let key = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        let _ = LocalFree(HLOCAL(output.pbData as *mut _));
    }
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| "豆包客户端密钥长度无效")?;
    let mut profiles = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|_| "无法读取豆包资料目录")? {
        let entry = entry.map_err(|_| "无法读取豆包资料目录")?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name != "Default" && !name.starts_with("Profile ") {
            continue;
        }
        let path = entry.path().join("Network/Cookies");
        if !path.exists() {
            continue;
        }
        let db =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .map_err(|_| "豆包账号数据库被占用，请完全退出豆包后重试")?;
        let version: i64 = db
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key='version'",
                [],
                |r| r.get(0),
            )
            .map_err(|_| "豆包账号数据库版本无法读取")?;
        let mut query = db.prepare("SELECT host_key,name,encrypted_value,value FROM cookies WHERE (host_key='.doubao.com' OR host_key='www.doubao.com') AND (expires_utc=0 OR expires_utc > (strftime('%s','now')+11644473600)*1000000) ORDER BY length(host_key)").map_err(|_| "豆包账号数据库格式不受支持")?;
        let rows = query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| "无法读取豆包登录状态")?;
        let mut cookies = std::collections::BTreeMap::new();
        for row in rows {
            let (host, name, encrypted, plain) = row.map_err(|_| "豆包账号数据读取失败")?;
            let value = if encrypted.is_empty() {
                plain
            } else {
                if !encrypted.starts_with(b"v10") || encrypted.len() < 31 {
                    continue;
                }
                let decoded = cipher
                    .decrypt(Nonce::from_slice(&encrypted[3..15]), &encrypted[15..])
                    .map_err(|_| "豆包登录凭证解密失败")?;
                let decoded = if version >= 24 {
                    let digest = Sha256::digest(host.as_bytes());
                    if decoded.len() < 32 || decoded[..32] != digest[..] {
                        return Err("豆包 Cookie 域名校验失败".into());
                    }
                    &decoded[32..]
                } else {
                    &decoded[..]
                };
                match String::from_utf8(decoded.to_vec()) {
                    Ok(value) => value,
                    Err(_) if name != "sessionid" => continue,
                    Err(_) => return Err("豆包 sessionid 编码无效".into()),
                }
            };
            if !value.is_empty() {
                cookies.insert(name, value);
            }
        }
        if cookies.contains_key("sessionid") {
            profiles.push(cookies);
        }
    }
    if profiles.len() != 1 {
        return Err(if profiles.is_empty() {
            "未找到已登录的豆包账号，请先登录客户端并完全退出后重试"
        } else {
            "检测到多个已登录的豆包资料，暂无法确定当前账号"
        }
        .into());
    }
    let cookie = profiles
        .pop()
        .unwrap()
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    super::protocol::normalize_cookie(&cookie)
}
#[cfg(not(windows))]
pub fn read_cookie() -> Result<String, String> {
    Err("暂仅支持从 Windows 豆包客户端导入".into())
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn signed_in_desktop_read_only_probe() {
        if std::env::var_os("QIEHUAN_DOUBAO_LIVE_READ_PROBE").is_none() {
            return;
        }
        let cookie = super::read_cookie().expect("读取豆包桌面登录会话失败");
        assert!(super::super::protocol::cookie_value(&cookie, "sessionid").is_some());
    }
}

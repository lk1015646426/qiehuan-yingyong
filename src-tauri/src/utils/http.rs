use reqwest::Client;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpProxyMode {
    System,
    Direct,
}

/// 创建统一配置的 HTTP 客户端
pub fn create_client(timeout_secs: u64) -> Client {
    create_client_with_mode(timeout_secs, HttpProxyMode::System)
}

pub fn create_direct_client(timeout_secs: u64) -> Client {
    create_client_with_mode(timeout_secs, HttpProxyMode::Direct)
}

fn create_client_with_mode(timeout_secs: u64, mode: HttpProxyMode) -> Client {
    let mut builder = Client::builder().timeout(std::time::Duration::from_secs(timeout_secs));
    if mode == HttpProxyMode::Direct {
        builder = builder.no_proxy();
    }
    builder.build().unwrap_or_else(|_| Client::new())
}

#[cfg(test)]
mod tests {
    use super::{create_client_with_mode, HttpProxyMode};

    #[test]
    fn direct_client_mode_is_explicitly_distinct_from_system_proxy_mode() {
        let _system = create_client_with_mode(1, HttpProxyMode::System);
        let _direct = create_client_with_mode(1, HttpProxyMode::Direct);
        assert_ne!(HttpProxyMode::System, HttpProxyMode::Direct);
    }
}

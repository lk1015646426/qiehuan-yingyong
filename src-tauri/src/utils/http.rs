use reqwest::Client;
use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

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

/// 创建国内查询专用客户端。
///
/// Clash TUN/全局模式会同时接管 DNS 和默认路由，`no_proxy()` 本身无法绕过
/// 这种内核级接管。Windows 下这里从非 Mihomo 网卡读取本地地址和网关，直接向
/// 网关 DNS 查询真实地址，并把 TCP socket 绑定到本地网卡；因此查询不会落入
/// Clash 的 fake-ip（28.0.0.0/8）和 TUN 默认路由。失败时仍回退到普通直连，
/// 让无 VPN 或非 Windows 环境保持原有行为。
pub fn create_domestic_client(timeout_secs: u64, host: &str) -> Client {
    let mut builder = Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .no_proxy();

    #[cfg(windows)]
    if let Some((local_ip, addresses)) = resolve_domestic_route(host) {
        crate::modules::logger::log_info(&format!(
            "[WorkBuddy Network] route_ready host={} local_ip={} addresses={}",
            host,
            local_ip,
            addresses
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
        builder = builder.local_address(IpAddr::V4(local_ip));
        if let Some(address) = addresses.first() {
            builder = builder.resolve(host, SocketAddr::new(IpAddr::V4(*address), 443));
        }
    } else {
        crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Network] route_unavailable host={}, falling back to direct resolver",
            host
        ));
    }

    match builder.build() {
        Ok(client) => client,
        Err(error) => {
            crate::modules::logger::log_error(&format!(
                "[WorkBuddy Network] client_build_failed host={} error={}",
                host,
                format_request_error("客户端创建", &error)
            ));
            Client::new()
        }
    }
}

pub(crate) fn format_request_error(label: &str, error: &dyn Error) -> String {
    let mut parts = Vec::new();
    let mut current = Some(error);
    while let Some(value) = current {
        parts.push(redact_diagnostic_text(&value.to_string()));
        current = value.source();
    }
    format!("{label}国内网络请求失败: {}", parts.join(" <- "))
}

fn redact_diagnostic_text(value: &str) -> String {
    let mut result = value.to_string();
    for marker in ["access_token=", "refresh_token=", "token=", "Bearer "] {
        let mut search_from = 0;
        while let Some(relative) = result[search_from..].find(marker) {
            let start = search_from + relative + marker.len();
            let end = result[start..]
                .find(|character: char| character.is_whitespace() || ";,)]}".contains(character))
                .map(|offset| start + offset)
                .unwrap_or(result.len());
            result.replace_range(start..end, "<redacted>");
            search_from = start + "<redacted>".len();
        }
    }
    result
}

fn create_client_with_mode(timeout_secs: u64, mode: HttpProxyMode) -> Client {
    let mut builder = Client::builder().timeout(std::time::Duration::from_secs(timeout_secs));
    if mode == HttpProxyMode::Direct {
        builder = builder.no_proxy();
    }
    builder.build().unwrap_or_else(|_| Client::new())
}

fn select_preferred_ipv4(addresses: &[IpAddr]) -> Option<Ipv4Addr> {
    addresses.iter().find_map(|address| match address {
        IpAddr::V4(value)
            if !value.is_unspecified()
                && !value.is_loopback()
                && !value.is_link_local()
                && value.octets()[0] != 28 =>
        {
            Some(*value)
        }
        _ => None,
    })
}

#[cfg(windows)]
fn resolve_domestic_route(host: &str) -> Option<(Ipv4Addr, Vec<Ipv4Addr>)> {
    if let Ok(cache) = DOMESTIC_ROUTE_CACHE.lock() {
        if let Some(entry) = cache
            .as_ref()
            .filter(|entry| entry.host == host && entry.expires_at > Instant::now())
        {
            return Some((entry.local_ip, entry.addresses.clone()));
        }
    }

    let (local_ip, gateway) = discover_non_mihomo_route()?;
    crate::modules::logger::log_info(&format!(
        "[WorkBuddy Network] route_probe host={} local_ip={} gateway={}",
        host, local_ip, gateway
    ));
    let mut addresses = Vec::new();
    for dns in [
        gateway,
        Ipv4Addr::new(223, 5, 5, 5),
        Ipv4Addr::new(114, 114, 114, 114),
    ] {
        if let Ok(values) = query_dns_a(host, local_ip, dns) {
            crate::modules::logger::log_info(&format!(
                "[WorkBuddy Network] dns_probe host={} server={} addresses={}",
                host,
                dns,
                values
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ));
            addresses.extend(values);
            if !addresses.is_empty() {
                break;
            }
        } else {
            crate::modules::logger::log_warn(&format!(
                "[WorkBuddy Network] dns_probe_failed host={} server={}",
                host, dns
            ));
        }
    }
    let addresses = addresses
        .into_iter()
        .filter(|value| select_preferred_ipv4(&[IpAddr::V4(*value)]).is_some())
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return None;
    }
    if let Ok(mut cache) = DOMESTIC_ROUTE_CACHE.lock() {
        *cache = Some(CachedDomesticRoute {
            host: host.to_string(),
            local_ip,
            addresses: addresses.clone(),
            expires_at: Instant::now() + Duration::from_secs(30),
        });
    }
    Some((local_ip, addresses))
}

#[cfg(windows)]
#[derive(Clone)]
struct CachedDomesticRoute {
    host: String,
    local_ip: Ipv4Addr,
    addresses: Vec<Ipv4Addr>,
    expires_at: Instant,
}

#[cfg(windows)]
static DOMESTIC_ROUTE_CACHE: LazyLock<Mutex<Option<CachedDomesticRoute>>> =
    LazyLock::new(|| Mutex::new(None));

#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AdapterRoute {
    local_ip: Ipv4Addr,
    gateway: Ipv4Addr,
    interface_type: u32,
    metric: u32,
}

#[cfg(windows)]
fn select_domestic_adapter_route(routes: &[AdapterRoute]) -> Option<(Ipv4Addr, Ipv4Addr)> {
    use windows::Win32::NetworkManagement::IpHelper::{IF_TYPE_ETHERNET_CSMACD, IF_TYPE_IEEE80211};

    routes
        .iter()
        .filter(|route| {
            matches!(
                route.interface_type,
                IF_TYPE_ETHERNET_CSMACD | IF_TYPE_IEEE80211
            ) && is_usable_local(route.local_ip)
                && is_usable_gateway(route.gateway)
                && route.local_ip.octets()[0] != 28
                && route.gateway.octets()[0] != 28
        })
        .min_by_key(|route| route.metric)
        .map(|route| (route.local_ip, route.gateway))
}

#[cfg(windows)]
fn discover_non_mihomo_route() -> Option<(Ipv4Addr, Ipv4Addr)> {
    let routes = read_windows_adapter_routes();
    let selected = select_domestic_adapter_route(&routes);
    match selected {
        Some((local_ip, gateway)) => {
            crate::modules::logger::log_info(&format!(
                "[WorkBuddy Network] adapter_route_ready candidates={} local_ip={} gateway={}",
                routes.len(),
                local_ip,
                gateway
            ));
        }
        None => crate::modules::logger::log_warn(&format!(
            "[WorkBuddy Network] adapter_route_unavailable candidates={}",
            routes.len()
        )),
    }
    selected
}

#[cfg(windows)]
fn is_usable_local(value: Ipv4Addr) -> bool {
    !value.is_unspecified()
        && !value.is_loopback()
        && !value.is_link_local()
        && !value.is_multicast()
}

#[cfg(windows)]
fn is_usable_gateway(value: Ipv4Addr) -> bool {
    is_usable_local(value) && value.octets() != [255, 255, 255, 255]
}

#[cfg(windows)]
fn read_windows_adapter_routes() -> Vec<AdapterRoute> {
    use std::mem::{size_of, MaybeUninit};
    use windows::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS};
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_INCLUDE_GATEWAYS, GAA_FLAG_SKIP_ANYCAST,
        GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows::Win32::Networking::WinSock::AF_INET;

    let mut buffer_len = 15 * 1024;
    for _ in 0..3 {
        let slots = (buffer_len + size_of::<IP_ADAPTER_ADDRESSES_LH>() - 1)
            / size_of::<IP_ADAPTER_ADDRESSES_LH>();
        let mut buffer = vec![MaybeUninit::<IP_ADAPTER_ADDRESSES_LH>::uninit(); slots];
        let mut buffer_size = (slots * size_of::<IP_ADAPTER_ADDRESSES_LH>()) as u32;
        let status = unsafe {
            GetAdaptersAddresses(
                AF_INET.0.into(),
                GAA_FLAG_INCLUDE_GATEWAYS
                    | GAA_FLAG_SKIP_MULTICAST
                    | GAA_FLAG_SKIP_ANYCAST
                    | GAA_FLAG_SKIP_DNS_SERVER,
                None,
                Some(buffer.as_mut_ptr().cast()),
                &mut buffer_size,
            )
        };
        if status == ERROR_BUFFER_OVERFLOW.0 {
            buffer_len = buffer_size as usize;
            continue;
        }
        if status != ERROR_SUCCESS.0 {
            crate::modules::logger::log_warn(&format!(
                "[WorkBuddy Network] adapter_probe_failed win32_error={}",
                status
            ));
            return Vec::new();
        }

        let mut routes = Vec::new();
        let mut adapter = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        while !adapter.is_null() {
            let current = unsafe { &*adapter };
            if current.OperStatus == IfOperStatusUp {
                if let (Some(local_ip), Some(gateway)) = (
                    first_ipv4_unicast(current.FirstUnicastAddress),
                    first_ipv4_gateway(current.FirstGatewayAddress),
                ) {
                    routes.push(AdapterRoute {
                        local_ip,
                        gateway,
                        interface_type: current.IfType,
                        metric: current.Ipv4Metric,
                    });
                }
            }
            adapter = current.Next;
        }
        return routes;
    }
    crate::modules::logger::log_warn("[WorkBuddy Network] adapter_probe_buffer_exhausted");
    Vec::new()
}

#[cfg(windows)]
fn first_ipv4_unicast(
    mut address: *mut windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_UNICAST_ADDRESS_LH,
) -> Option<Ipv4Addr> {
    while !address.is_null() {
        let current = unsafe { &*address };
        if let Some(value) = ipv4_from_socket_address(current.Address) {
            if is_usable_local(value) {
                return Some(value);
            }
        }
        address = current.Next;
    }
    None
}

#[cfg(windows)]
fn first_ipv4_gateway(
    mut address: *mut windows::Win32::NetworkManagement::IpHelper::IP_ADAPTER_GATEWAY_ADDRESS_LH,
) -> Option<Ipv4Addr> {
    while !address.is_null() {
        let current = unsafe { &*address };
        if let Some(value) = ipv4_from_socket_address(current.Address) {
            if is_usable_gateway(value) {
                return Some(value);
            }
        }
        address = current.Next;
    }
    None
}

#[cfg(windows)]
fn ipv4_from_socket_address(
    address: windows::Win32::Networking::WinSock::SOCKET_ADDRESS,
) -> Option<Ipv4Addr> {
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    if address.lpSockaddr.is_null() {
        return None;
    }
    let sockaddr = unsafe { &*address.lpSockaddr };
    if sockaddr.sa_family != AF_INET {
        return None;
    }
    let value = unsafe {
        (&*(address.lpSockaddr.cast::<SOCKADDR_IN>()))
            .sin_addr
            .S_un
            .S_addr
    };
    Some(Ipv4Addr::from(value.to_ne_bytes()))
}

#[cfg(windows)]
fn query_dns_a(host: &str, local_ip: Ipv4Addr, dns: Ipv4Addr) -> Result<Vec<Ipv4Addr>, String> {
    use std::net::UdpSocket;
    let socket = UdpSocket::bind(SocketAddr::new(IpAddr::V4(local_ip), 0))
        .map_err(|error| error.to_string())?;
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    let query = dns_query(host)?;
    socket
        .send_to(&query, SocketAddr::new(IpAddr::V4(dns), 53))
        .map_err(|error| error.to_string())?;
    let mut response = [0_u8; 2048];
    let (length, _) = socket
        .recv_from(&mut response)
        .map_err(|error| error.to_string())?;
    parse_dns_a_records(&response[..length])
}

#[cfg(windows)]
fn dns_query(host: &str) -> Result<Vec<u8>, String> {
    let mut packet = vec![
        0x42, 0x42, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    for label in host.split('.') {
        let bytes = label.as_bytes();
        if bytes.is_empty() || bytes.len() > 63 {
            return Err("DNS 主机名无效".to_string());
        }
        packet.push(bytes.len() as u8);
        packet.extend_from_slice(bytes);
    }
    packet.extend_from_slice(&[0, 0, 1, 0, 1]);
    Ok(packet)
}

#[cfg(windows)]
fn parse_dns_a_records(packet: &[u8]) -> Result<Vec<Ipv4Addr>, String> {
    if packet.len() < 12 {
        return Err("DNS 响应过短".to_string());
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut offset = 12;
    for _ in 0..questions {
        offset = skip_dns_name(packet, offset)?;
        offset = offset
            .checked_add(4)
            .filter(|value| *value <= packet.len())
            .ok_or_else(|| "DNS 问题段无效".to_string())?;
    }
    let mut result = Vec::new();
    for _ in 0..answers {
        offset = skip_dns_name(packet, offset)?;
        if offset + 10 > packet.len() {
            return Err("DNS 资源记录无效".to_string());
        }
        let record_type = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let class = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
        let length = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        if offset + length > packet.len() {
            return Err("DNS 地址数据无效".to_string());
        }
        if record_type == 1 && class == 1 && length == 4 {
            result.push(Ipv4Addr::new(
                packet[offset],
                packet[offset + 1],
                packet[offset + 2],
                packet[offset + 3],
            ));
        }
        offset += length;
    }
    Ok(result)
}

#[cfg(windows)]
fn skip_dns_name(packet: &[u8], mut offset: usize) -> Result<usize, String> {
    loop {
        let length = *packet
            .get(offset)
            .ok_or_else(|| "DNS 名称无效".to_string())?;
        if length == 0 {
            return Ok(offset + 1);
        }
        if length & 0xc0 == 0xc0 {
            return Ok(offset + 2);
        }
        if length > 63 {
            return Err("DNS 名称压缩指针无效".to_string());
        }
        offset = offset
            .checked_add(1 + length as usize)
            .filter(|value| *value <= packet.len())
            .ok_or_else(|| "DNS 名称越界".to_string())?;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        create_client_with_mode, format_request_error, select_preferred_ipv4, HttpProxyMode,
    };
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::Duration;

    #[test]
    fn domestic_route_probe_does_not_spawn_ipconfig() {
        let forbidden = concat!("Command::new", "(\"ipconfig\")");
        assert!(!include_str!("http.rs").contains(forbidden));
    }

    #[test]
    fn request_diagnostics_keep_error_chain_without_credentials() {
        let error = std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "socket failed for token=secret-token",
        );
        let message = format_request_error("积分查询", &error);
        assert!(message.contains("积分查询"));
        assert!(message.contains("socket failed"));
        assert!(!message.contains("secret-token"));
    }

    #[test]
    fn direct_client_mode_is_explicitly_distinct_from_system_proxy_mode() {
        let _system = create_client_with_mode(1, HttpProxyMode::System);
        let _direct = create_client_with_mode(1, HttpProxyMode::Direct);
        assert_ne!(HttpProxyMode::System, HttpProxyMode::Direct);
    }

    #[test]
    fn domestic_resolution_ignores_clash_fake_ip() {
        let addresses = [
            "28.0.0.17".parse().unwrap(),
            "27.159.69.232".parse().unwrap(),
        ];
        assert_eq!(
            select_preferred_ipv4(&addresses).unwrap().to_string(),
            "27.159.69.232"
        );
    }

    #[cfg(windows)]
    #[test]
    fn domestic_route_uses_physical_adapter_with_lowest_metric() {
        let selected = super::select_domestic_adapter_route(&[
            super::AdapterRoute {
                local_ip: "28.0.0.2".parse().unwrap(),
                gateway: "28.0.0.1".parse().unwrap(),
                interface_type: 6,
                metric: 1,
            },
            super::AdapterRoute {
                local_ip: "192.168.2.9".parse().unwrap(),
                gateway: "192.168.2.1".parse().unwrap(),
                interface_type: 71,
                metric: 35,
            },
            super::AdapterRoute {
                local_ip: "10.0.0.8".parse().unwrap(),
                gateway: "10.0.0.1".parse().unwrap(),
                interface_type: 6,
                metric: 50,
            },
        ]);

        assert_eq!(
            selected,
            Some((
                "192.168.2.9".parse().unwrap(),
                "192.168.2.1".parse().unwrap()
            ))
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an active non-TUN network adapter"]
    fn domestic_route_can_resolve_copilot_without_clash_fake_ip() {
        let (local, addresses) = super::resolve_domestic_route("copilot.tencent.com")
            .expect("must find a direct local route");
        assert_ne!(local.octets()[0], 28);
        assert!(!addresses.is_empty());
        assert!(addresses.iter().all(|address| address.octets()[0] != 28));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires an active domestic network route"]
    fn domestic_client_reaches_copilot_without_credentials() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let response = runtime
            .block_on(async {
                super::create_domestic_client(10, "copilot.tencent.com")
                    .get("https://copilot.tencent.com/v2/billing/meter/get-user-resource")
                    .send()
                    .await
            })
            .expect("TLS request should reach the service");
        assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "diagnostic: checks each domestic edge address"]
    fn diagnostic_each_domestic_copilot_address() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let addresses = [
            "121.204.231.62".parse().unwrap(),
            "27.159.69.232".parse().unwrap(),
            "124.72.128.203".parse().unwrap(),
        ];
        let results = runtime.block_on(async {
            let mut results = Vec::new();
            for address in addresses {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .no_proxy()
                    .local_address(IpAddr::V4(Ipv4Addr::new(192, 168, 2, 9)))
                    .resolve(
                        "copilot.tencent.com",
                        SocketAddr::new(IpAddr::V4(address), 443),
                    )
                    .build()
                    .unwrap();
                let result = client
                    .get("https://copilot.tencent.com/v2/billing/meter/get-user-resource")
                    .send()
                    .await
                    .map(|response| response.status().as_u16())
                    .map_err(|error| error.to_string());
                results.push((address, result));
            }
            results
        });
        assert!(results.iter().any(|(_, result)| result == &Ok(401)));
    }
}

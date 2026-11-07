use crate::model::{NetworkInfo, NetworkInterface};
#[cfg(unix)]
use crate::util::run_stdout_timeout;
use std::time::Duration;
use sysinfo::{InterfaceOperationalState, Networks};

/// 网卡清单。
///
/// `rx_bytes_per_sec` / `tx_bytes_per_sec` 在快照里恒为 0：速率必须由两次采样相除得到，
/// 请使用 `monitor::Sampler` 产出的 `MonitorTick.per_interface`。
pub fn collect() -> NetworkInfo {
    let mut interfaces: Vec<NetworkInterface> = Vec::new();

    if let Ok(nets) = Networks::new_with_refreshed_list() {
        let hints = port_hints();
        for (name, data) in nets.iter() {
            let name = name.to_string_lossy().to_string();

            let mac = Some(data.mac_address())
                .filter(|m| !m.is_unspecified())
                .map(|m| m.to_string());
            let mut ipv4 = Vec::new();
            let mut ipv6 = Vec::new();
            for ip in data.ip_networks() {
                match ip.addr {
                    std::net::IpAddr::V4(v4) => ipv4.push(v4.to_string()),
                    std::net::IpAddr::V6(v6) => ipv6.push(v6.to_string()),
                }
            }

            let kind = classify_interface(&name, hints.get(&name).map(String::as_str));
            let is_loopback = kind == "loopback";
            // 物理层可达且至少有一个地址才算"在线"；操作系统给不出状态时按地址兜底。
            let has_ip = !ipv4.is_empty() || !ipv6.is_empty();
            let is_up = is_loopback
                || match data.operational_state() {
                    InterfaceOperationalState::Down
                    | InterfaceOperationalState::NotPresent
                    | InterfaceOperationalState::Dormant
                    | InterfaceOperationalState::Testing => false,
                    _ => has_ip,
                };

            interfaces.push(NetworkInterface {
                link_speed_mbps: data
                    .receive_link_speed()
                    .or_else(|| data.transmit_link_speed())
                    .map(|bps| bps / 1_000_000)
                    .filter(|v| *v > 0),
                name,
                mac,
                ipv4,
                ipv6,
                is_up,
                is_loopback,
                kind: kind.to_string(),
                rx_bytes_per_sec: 0,
                tx_bytes_per_sec: 0,
                rx_total_bytes: data.total_received(),
                tx_total_bytes: data.total_transmitted(),
            });
        }
    }
    interfaces.sort_by(|a, b| a.name.cmp(&b.name));

    let mut dns_servers = read_dns_servers();
    sanitize_dns(&mut dns_servers);
    NetworkInfo {
        interfaces,
        public_ip: None,
        default_gateway: read_default_gateway(),
        dns_servers,
    }
}

/// macOS 的 `networksetup` 能准确说出每个 enN 是 Wi-Fi 还是以太网；其他平台没有该来源。
#[cfg(target_os = "macos")]
fn port_hints() -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    let Some(text) = run_stdout_timeout(
        "/usr/sbin/networksetup",
        &["-listallhardwareports"],
        Duration::from_secs(3),
    ) else {
        return map;
    };
    let mut port: Option<String> = None;
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("Hardware Port:") {
            port = Some(p.trim().to_string());
        } else if let Some(d) = line.strip_prefix("Device:")
            && let Some(p) = port.take()
        {
            map.insert(d.trim().to_string(), p);
        }
    }
    map
}

#[cfg(not(target_os = "macos"))]
fn port_hints() -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

pub(crate) fn classify_interface(name: &str, hint: Option<&str>) -> &'static str {
    let lower = name.to_lowercase();
    if lower == "lo" || lower == "lo0" || lower.starts_with("loopback") {
        return "loopback";
    }
    if let Some(h) = hint {
        let h = h.to_lowercase();
        if h.contains("wi-fi") || h.contains("wifi") || h.contains("airport") {
            return "wifi";
        }
        if h.contains("bluetooth") {
            return "bluetooth";
        }
        if h.contains("thunderbolt bridge") || h.contains("iphone") || h.contains("ipad") {
            return "virtual";
        }
        if h.contains("ethernet") || h.contains("lan") {
            return "ethernet";
        }
    }
    #[cfg(target_os = "linux")]
    if let Some(k) = linux_kind(name) {
        return k;
    }
    if lower.contains("bluetooth") || lower.starts_with("pan") {
        return "bluetooth";
    }
    // Apple 私有接口（AirDrop / 低延迟 WLAN / 共享 / IPv6 隧道 / Thunderbolt）、
    // VPN / 隧道 / 容器 / 虚拟交换机。
    const VIRTUAL_PREFIXES: [&str; 23] = [
        "awdl",
        "llw",
        "ap1",
        "gif",
        "stf",
        "anpi",
        "xhc",
        "utun",
        "tun",
        "tap",
        "wg",
        "ipsec",
        "ppp",
        "docker",
        "br-",
        "veth",
        "vmnet",
        "vmenet",
        "vboxnet",
        "zt",
        "tailscale",
        "bridge",
        "vethernet",
    ];
    if VIRTUAL_PREFIXES.iter().any(|p| lower.starts_with(p))
        || lower == "br0"
        || lower.contains("vmware")
        || lower.contains("virtualbox")
        || lower.contains("hyper-v")
        || lower.contains("wsl")
        || lower.contains("local area connection*")
    {
        return "virtual";
    }
    if lower.starts_with("wl")
        || lower.contains("wi-fi")
        || lower.contains("wifi")
        || lower.contains("wlan")
    {
        return "wifi";
    }
    if lower.starts_with("en")
        || lower.starts_with("eth")
        || lower.contains("ethernet")
        || lower.contains("以太网")
    {
        return "ethernet";
    }
    "other"
}

/// Linux：sysfs 能给出权威答案——有 `wireless` 目录就是 Wi-Fi，
/// 位于 `/sys/devices/virtual/net` 下的都是软件创建的虚拟接口。
#[cfg(target_os = "linux")]
fn linux_kind(name: &str) -> Option<&'static str> {
    let base = std::path::Path::new("/sys/class/net").join(name);
    if base.join("wireless").exists() || base.join("phy80211").exists() {
        return Some("wifi");
    }
    let real = std::fs::canonicalize(&base).ok()?;
    if real.starts_with("/sys/devices/virtual/") {
        return Some("virtual");
    }
    None
}

fn sanitize_dns(servers: &mut Vec<String>) {
    let bad = ["127.0.0.1", "127.0.0.53", "0.0.0.0", "::1"];
    servers.retain(|s| !s.is_empty() && !bad.contains(&s.as_str()));
    let mut seen = std::collections::HashSet::new();
    servers.retain(|s| seen.insert(s.clone()));
}

// ---------------------------------------------------------------- 默认网关

#[cfg(target_os = "linux")]
fn read_default_gateway() -> Option<String> {
    read_default_gateway_v4().or_else(read_default_gateway_v6)
}

#[cfg(target_os = "linux")]
fn read_default_gateway_v4() -> Option<String> {
    let content = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in content.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 8 || cols[1] != "00000000" {
            continue;
        }
        // 小端十六进制，例如 0101A8C0 → 192.168.1.1
        let gw = u32::from_str_radix(cols[2], 16).ok()?;
        if gw != 0 {
            return Some(std::net::Ipv4Addr::from(gw.swap_bytes()).to_string());
        }
    }
    None
}

/// 纯 IPv6 网络（部分 VPS / 容器）：解析 /proc/net/ipv6_route。
/// 每行：dest_prefix dest_len src_prefix src_len next_hop metric refcnt usecnt flags iface
#[cfg(target_os = "linux")]
fn read_default_gateway_v6() -> Option<String> {
    const ZERO: &str = "00000000000000000000000000000000";
    let content = std::fs::read_to_string("/proc/net/ipv6_route").ok()?;
    for line in content.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 || cols[0] != ZERO || cols[1] != "00" || cols[4] == ZERO {
            continue;
        }
        let bits = u128::from_str_radix(cols[4], 16).ok()?;
        return Some(std::net::Ipv6Addr::from(bits).to_string());
    }
    None
}

#[cfg(target_os = "macos")]
fn read_default_gateway() -> Option<String> {
    let text = run_stdout_timeout(
        "/sbin/route",
        &["-n", "get", "default"],
        Duration::from_secs(3),
    )?;
    text.lines().find_map(|l| {
        let v = l.trim().strip_prefix("gateway:")?.trim();
        (!v.is_empty()).then(|| v.to_string())
    })
}

#[cfg(target_os = "windows")]
fn read_default_gateway() -> Option<String> {
    #[derive(serde::Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Cfg {
        #[serde(rename = "DefaultIPGateway")]
        default_ip_gateway: Option<Vec<String>>,
    }
    let conn = crate::platform::windows::wmi_cimv2()?;
    let rows: Vec<Cfg> = conn
        .raw_query(
            "SELECT DefaultIPGateway FROM Win32_NetworkAdapterConfiguration WHERE IPEnabled = TRUE",
        )
        .ok()?;
    rows.into_iter()
        .find_map(|r| r.default_ip_gateway.and_then(|v| v.into_iter().next()))
}

// --------------------------------------------------------------------- DNS

#[cfg(target_os = "linux")]
fn read_dns_servers() -> Vec<String> {
    let primary = read_resolv_conf();
    let stub_only = primary.iter().all(|s| s.starts_with("127.") || s == "::1");
    if !primary.is_empty() && !stub_only {
        return primary;
    }
    // systemd-resolved 的桩地址：问 resolvectl 拿真实上游。
    let resolved = read_resolvectl();
    if resolved.is_empty() {
        primary
    } else {
        resolved
    }
}

#[cfg(target_os = "linux")]
fn read_resolvectl() -> Vec<String> {
    let Some(text) = run_stdout_timeout("resolvectl", &["status"], Duration::from_secs(3)) else {
        return Vec::new();
    };
    let mut servers: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("Current DNS Server:")
            .or_else(|| line.strip_prefix("DNS Servers:"))
        else {
            continue;
        };
        for tok in rest.split_whitespace() {
            if tok.parse::<std::net::IpAddr>().is_ok() && !servers.iter().any(|s| s == tok) {
                servers.push(tok.to_string());
            }
        }
    }
    servers
}

#[cfg(target_os = "macos")]
fn read_dns_servers() -> Vec<String> {
    let primary = read_resolv_conf();
    if !primary.is_empty() {
        return primary;
    }
    let Some(text) = run_stdout_timeout("/usr/sbin/scutil", &["--dns"], Duration::from_secs(3))
    else {
        return Vec::new();
    };
    let mut servers: Vec<String> = Vec::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        if k.trim().starts_with("nameserver") {
            let v = v.trim();
            if v.parse::<std::net::IpAddr>().is_ok() && !servers.iter().any(|s| s == v) {
                servers.push(v.to_string());
            }
        }
    }
    servers
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_resolv_conf() -> Vec<String> {
    let Ok(content) = std::fs::read_to_string("/etc/resolv.conf") else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("nameserver") {
            let v = rest.trim();
            if !v.is_empty() && !out.iter().any(|s| s == v) {
                out.push(v.to_string());
            }
        }
    }
    out
}

#[cfg(target_os = "windows")]
fn read_dns_servers() -> Vec<String> {
    #[derive(serde::Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Cfg {
        #[serde(rename = "DNSServerSearchOrder")]
        dns_server_search_order: Option<Vec<String>>,
    }
    let Some(conn) = crate::platform::windows::wmi_cimv2() else {
        return Vec::new();
    };
    let rows: Vec<Cfg> = conn
        .raw_query(
            "SELECT DNSServerSearchOrder FROM Win32_NetworkAdapterConfiguration WHERE IPEnabled = TRUE",
        )
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for s in rows
        .into_iter()
        .filter_map(|r| r.dns_server_search_order)
        .flatten()
    {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

// ---------------------------------------------------------------- 公网 IP

/// 查询公网 IP。只有用户在设置里显式开启时才会被调用。
///
/// - HTTPS（rustls），不依赖系统 OpenSSL。
/// - 多个备用端点，首个返回合法 IP 字面量的即采用，避免被 captive portal 注入的 HTML 污染。
/// - 单个端点总超时 4 秒。
pub fn fetch_public_ip() -> Option<String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(4)))
        .user_agent(concat!("pc-specs/", env!("CARGO_PKG_VERSION")))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    const ENDPOINTS: [&str; 3] = [
        "https://api.ipify.org",
        "https://ifconfig.co/ip",
        "https://icanhazip.com",
    ];
    ENDPOINTS.iter().find_map(|url| {
        let mut resp = agent.get(*url).call().ok()?;
        let body = resp.body_mut().read_to_string().ok()?;
        let ip = body.trim();
        ip.parse::<std::net::IpAddr>()
            .is_ok()
            .then(|| ip.to_string())
    })
}

use crate::model::{NetworkInfo, NetworkInterface};
use crate::state::SharedSys;
use std::sync::Arc;

/// 注意：每接口的 rx_bytes_per_sec / tx_bytes_per_sec 字段并非真正的"每秒字节数"，
/// 而是 sysinfo 在两次 refresh 之间累积的 delta。其精度依赖于外部 refresh 节奏：
/// 当 Monitor 任务在 1Hz 运行时，该值 ~ 1s 内的字节差，可作为瞬时速率近似；
/// 当 Monitor 未启动时该值是「自上次 refresh 以来」的累积，不应当作 BPS 解读。
/// 真正的实时速率由 MonitorTick.net_rx_bps/net_tx_bps 提供（聚合所有接口）。
pub fn collect(shared: &Arc<SharedSys>) -> NetworkInfo {
    let nets = shared.networks.lock();
    let mut interfaces: Vec<NetworkInterface> = Vec::new();

    let up_set = read_up_interfaces();

    for (name, data) in nets.iter() {
        let mac = {
            let s = data.mac_address().to_string();
            if s == "00:00:00:00:00:00" || s.is_empty() {
                None
            } else {
                Some(s)
            }
        };
        let mut ipv4 = Vec::new();
        let mut ipv6 = Vec::new();
        for ip in data.ip_networks() {
            match ip.addr {
                std::net::IpAddr::V4(v4) => ipv4.push(v4.to_string()),
                std::net::IpAddr::V6(v6) => ipv6.push(v6.to_string()),
            }
        }
        let kind = classify_interface(name);
        let is_loopback = kind == "loopback";
        let is_up = if is_loopback {
            true
        } else if let Some(set) = up_set.as_ref() {
            set.iter().any(|s| s.eq_ignore_ascii_case(name))
        } else {
            !ipv4.is_empty() || !ipv6.is_empty()
        };

        interfaces.push(NetworkInterface {
            name: name.to_string(),
            mac,
            ipv4,
            ipv6,
            is_up,
            is_loopback,
            kind,
            link_speed_mbps: read_link_speed(name),
            rx_bytes_per_sec: data.received(),
            tx_bytes_per_sec: data.transmitted(),
            rx_total_bytes: data.total_received(),
            tx_total_bytes: data.total_transmitted(),
        });
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

fn classify_interface(name: &str) -> String {
    let lower = name.to_lowercase();
    // Loopback
    if lower == "lo" || lower == "lo0" || lower.starts_with("loopback") {
        return "loopback".to_string();
    }
    // Bluetooth (must be before generic "bt")
    if lower.contains("bluetooth") || lower.starts_with("pan") {
        return "bluetooth".to_string();
    }
    // Apple 私有：AWDL（AirDrop）/ LLW（Low-Latency WLAN）/ AP（Internet Sharing）
    // GIF/STF（IPv6 over IPv4 隧道）/ XHC（Thunderbolt）
    if lower.starts_with("awdl")
        || lower.starts_with("llw")
        || lower.starts_with("ap1")
        || lower.starts_with("gif")
        || lower.starts_with("stf")
        || lower.starts_with("anpi")
        || lower.starts_with("xhc")
    {
        return "virtual".to_string();
    }
    // VPN / 隧道 / 容器 / 桥
    if lower.starts_with("utun")
        || lower.starts_with("tun")
        || lower.starts_with("tap")
        || lower.starts_with("wg")
        || lower.starts_with("ipsec")
        || lower.starts_with("ppp")
        || lower.starts_with("docker")
        || lower.starts_with("br-")
        || lower == "br0"
        || lower.starts_with("veth")
        || lower.starts_with("vmnet")
        || lower.starts_with("vboxnet")
        || lower.starts_with("zt")
        || lower.starts_with("tailscale")
    {
        return "virtual".to_string();
    }
    // Wi-Fi（系统默认 / systemd predictable / Windows）
    if lower.starts_with("wl")
        || lower.starts_with("wlp")
        || lower.starts_with("wlx")
        || lower.contains("wi-fi")
        || lower.contains("wifi")
        || lower.contains("wlan")
        || lower.starts_with("wifi")
    {
        return "wifi".to_string();
    }
    // 以太网（包含 systemd predictable enp/enx）
    if lower.starts_with("en")
        || lower.starts_with("eth")
        || lower.starts_with("eno")
        || lower.starts_with("enp")
        || lower.starts_with("enx")
        || lower.contains("ethernet")
        || lower.contains("以太网")
    {
        return "ethernet".to_string();
    }
    "other".to_string()
}

fn sanitize_dns(servers: &mut Vec<String>) {
    let bad = ["127.0.0.1", "127.0.0.53", "0.0.0.0", "::1"];
    servers.retain(|s| !bad.contains(&s.as_str()) && !s.is_empty());
    let mut seen = std::collections::HashSet::new();
    servers.retain(|s| seen.insert(s.clone()));
}

#[cfg(target_os = "linux")]
fn read_up_interfaces() -> Option<Vec<String>> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir("/sys/class/net").ok()?;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let path = e.path().join("operstate");
        if let Ok(state) = std::fs::read_to_string(&path) {
            let st = state.trim();
            if st == "up" || st == "unknown" {
                out.push(name);
            }
        }
    }
    Some(out)
}

#[cfg(target_os = "macos")]
fn read_up_interfaces() -> Option<Vec<String>> {
    // 在 macOS 上 sysinfo 没法告诉我们物理 link 状态，仅有 IP 地址。
    // 拔了网线后 en0 可能仍保留旧 IP，所以单看 ipv4.is_empty() 会误报 up。
    // ifconfig 输出含 "status: active" / "status: inactive" 行，是最权威的信号。
    let out = std::process::Command::new("ifconfig").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut up: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    let mut current_active = false;
    let mut has_status_line = false;
    let flush = |current: &mut Option<String>,
                 active: bool,
                 has_status: bool,
                 up: &mut Vec<String>| {
        if let Some(name) = current.take() {
            if active {
                up.push(name);
            } else if !has_status {
                // 没有 status 行的接口（loopback / utun / awdl 之类），
                // 默认视为 up 让 collect() 的 ipv4 fallback 决定。
                up.push(name);
            }
        }
    };
    for line in s.lines() {
        if !line.starts_with('\t') && !line.starts_with(' ') {
            // 新接口块: "en0: flags=8863<UP,BROADCAST,SMART,...>"
            flush(&mut current, current_active, has_status_line, &mut up);
            current_active = false;
            has_status_line = false;
            if let Some((name, _)) = line.split_once(':') {
                current = Some(name.trim().to_string());
            }
            continue;
        }
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("status:") {
            has_status_line = true;
            current_active = rest.trim().eq_ignore_ascii_case("active");
        }
    }
    flush(&mut current, current_active, has_status_line, &mut up);
    Some(up)
}

#[cfg(target_os = "windows")]
fn read_up_interfaces() -> Option<Vec<String>> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Adapter {
        name: Option<String>,
        net_connection_status: Option<u16>,
    }
    let com = COMLibrary::new().ok()?;
    let conn = WMIConnection::new(com).ok()?;
    // NetConnectionStatus 文档值：
    //   2 = Connected, 7 = Media Disconnected, 9 = Authentication Succeeded, ...
    let rows: Vec<Adapter> = conn
        .raw_query("SELECT Name, NetConnectionStatus FROM Win32_NetworkAdapter")
        .ok()?;
    Some(
        rows.into_iter()
            .filter_map(|a| {
                let connected = matches!(a.net_connection_status, Some(2) | Some(9));
                if connected { a.name } else { None }
            })
            .collect(),
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn read_up_interfaces() -> Option<Vec<String>> {
    None
}

#[cfg(target_os = "linux")]
fn read_link_speed(name: &str) -> Option<u64> {
    let p = format!("/sys/class/net/{name}/speed");
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .and_then(|v| if v <= 0 { None } else { Some(v as u64) })
}

#[cfg(not(target_os = "linux"))]
fn read_link_speed(_name: &str) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn read_default_gateway() -> Option<String> {
    if let Some(v4) = read_default_gateway_v4() {
        return Some(v4);
    }
    // 纯 IPv6 网络（如某些 VPS/容器）：解析 /proc/net/ipv6_route。
    read_default_gateway_v6()
}

#[cfg(target_os = "linux")]
fn read_default_gateway_v4() -> Option<String> {
    let content = std::fs::read_to_string("/proc/net/route").ok()?;
    for line in content.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 8 {
            continue;
        }
        if cols[1] == "00000000" {
            let gw_hex = cols[2];
            if gw_hex.len() == 8 {
                let bytes = (0..4)
                    .filter_map(|i| u8::from_str_radix(&gw_hex[i * 2..i * 2 + 2], 16).ok())
                    .collect::<Vec<_>>();
                if bytes.len() == 4 {
                    return Some(format!("{}.{}.{}.{}", bytes[3], bytes[2], bytes[1], bytes[0]));
                }
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn read_default_gateway_v6() -> Option<String> {
    // /proc/net/ipv6_route 每行格式：
    // dest_prefix dest_prefix_len src_prefix src_prefix_len next_hop metric refcount usecount flags iface
    // 默认路由特征：dest_prefix == "00000000000000000000000000000000" 且 prefix_len == 0
    let content = std::fs::read_to_string("/proc/net/ipv6_route").ok()?;
    for line in content.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 10 {
            continue;
        }
        if cols[0] == "00000000000000000000000000000000" && cols[1] == "00" {
            let next_hop = cols[4];
            if next_hop == "00000000000000000000000000000000" {
                continue; // 链路本地或未指定
            }
            return parse_hex_ipv6(next_hop);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn parse_hex_ipv6(hex: &str) -> Option<String> {
    if hex.len() != 32 {
        return None;
    }
    let groups: Vec<String> = (0..8)
        .map(|i| {
            let part = &hex[i * 4..i * 4 + 4];
            // 每 4 个 hex char = 16 bit big-endian
            u16::from_str_radix(part, 16)
                .map(|v| format!("{v:x}"))
                .unwrap_or_else(|_| "0".into())
        })
        .collect();
    let raw = groups.join(":");
    // 解析后再用 std::net 规范化（处理 ::）
    raw.parse::<std::net::Ipv6Addr>()
        .map(|a| a.to_string())
        .ok()
}

#[cfg(target_os = "macos")]
fn read_default_gateway() -> Option<String> {
    let out = std::process::Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("gateway:") {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn read_default_gateway() -> Option<String> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Cfg {
        default_ip_gateway: Option<Vec<String>>,
        ip_enabled: Option<bool>,
    }
    let com = COMLibrary::new().ok()?;
    let conn = WMIConnection::new(com).ok()?;
    let rows: Vec<Cfg> = conn
        .raw_query(
            "SELECT DefaultIPGateway, IPEnabled FROM Win32_NetworkAdapterConfiguration WHERE IPEnabled = TRUE",
        )
        .ok()?;
    rows.into_iter()
        .filter(|r| r.ip_enabled.unwrap_or(false))
        .find_map(|r| r.default_ip_gateway.and_then(|v| v.into_iter().next()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn read_default_gateway() -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn read_dns_servers() -> Vec<String> {
    let primary = read_resolv_conf();
    if !primary.is_empty()
        && !primary.iter().all(|s| s.starts_with("127.0.0.") || s == "::1")
    {
        return primary;
    }
    // systemd-resolved 桩——尝试 resolvectl
    let resolved = read_resolvectl();
    if !resolved.is_empty() {
        return resolved;
    }
    primary
}

#[cfg(target_os = "linux")]
fn read_resolvectl() -> Vec<String> {
    let out = match std::process::Command::new("resolvectl").arg("status").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let s = String::from_utf8_lossy(&out.stdout);
    let mut servers = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if let Some(rest) = line
            .strip_prefix("Current DNS Server:")
            .or_else(|| line.strip_prefix("DNS Servers:"))
        {
            for tok in rest.split_whitespace() {
                if tok.parse::<std::net::IpAddr>().is_ok() && !servers.contains(&tok.to_string()) {
                    servers.push(tok.to_string());
                }
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
    let out = match std::process::Command::new("scutil").arg("--dns").output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let s = String::from_utf8_lossy(&out.stdout);
    let mut servers = Vec::new();
    for line in s.lines() {
        let line = line.trim();
        if let Some(rest) = line.split_once(':').and_then(|(k, v)| {
            if k.trim_start_matches(|c: char| !c.is_alphabetic()).starts_with("nameserver") {
                Some(v.trim())
            } else {
                None
            }
        }) {
            if rest.parse::<std::net::IpAddr>().is_ok() && !servers.contains(&rest.to_string()) {
                servers.push(rest.to_string());
            }
        }
    }
    servers
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn read_resolv_conf() -> Vec<String> {
    let content = match std::fs::read_to_string("/etc/resolv.conf") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("nameserver") {
            let v = rest.trim();
            if !v.is_empty() && !out.iter().any(|s: &String| s == v) {
                out.push(v.to_string());
            }
        }
    }
    out
}

#[cfg(target_os = "windows")]
fn read_dns_servers() -> Vec<String> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Cfg {
        dns_server_search_order: Option<Vec<String>>,
        ip_enabled: Option<bool>,
    }
    let Ok(com) = COMLibrary::new() else { return Vec::new() };
    let Ok(conn) = WMIConnection::new(com) else { return Vec::new() };
    let rows: Vec<Cfg> = conn
        .raw_query(
            "SELECT DNSServerSearchOrder, IPEnabled FROM Win32_NetworkAdapterConfiguration WHERE IPEnabled = TRUE",
        )
        .unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for r in rows.into_iter().filter(|r| r.ip_enabled.unwrap_or(false)) {
        if let Some(list) = r.dns_server_search_order {
            for s in list {
                if !s.is_empty() && !out.contains(&s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn read_dns_servers() -> Vec<String> {
    Vec::new()
}

/// 查询公网 IP。仅当用户在设置中显式启用 publicIpEnabled 时才会被调用。
///
/// - 使用 HTTPS（rustls），由 ureq 提供 TLS handshake；不再依赖系统 OpenSSL。
/// - 多个备用域名（ipify / icanhazip / ifconfig.me），首个成功即返回。
/// - 总超时 ≤ 5s（连接 + 读各 2.5s）。
pub fn fetch_public_ip() -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(3))
        .timeout_read(std::time::Duration::from_secs(3))
        .timeout_write(std::time::Duration::from_secs(3))
        .user_agent(concat!("pc-specs/", env!("CARGO_PKG_VERSION")))
        .build();
    let endpoints = [
        "https://api.ipify.org",
        "https://ifconfig.co/ip",
        "https://icanhazip.com",
    ];
    for url in endpoints {
        match agent.get(url).call() {
            Ok(resp) => {
                if let Ok(body) = resp.into_string() {
                    let trimmed = body.trim();
                    // 简单校验：必须是 IPv4 或 IPv6 字面量，避免被 captive portal 注入 HTML。
                    if trimmed.parse::<std::net::IpAddr>().is_ok() {
                        return Some(trimmed.to_string());
                    }
                }
            }
            Err(_) => continue,
        }
    }
    None
}

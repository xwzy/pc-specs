//! 网络体检：到网关、DNS 和公网的延迟与丢包，以及当前 Wi-Fi 连接的细节。
//!
//! 延迟用 TCP 握手的耗时来量：建立连接要一个来回，和 ping 的往返时间等价，
//! 却不用管理员权限、不用起外部进程，三个平台的行为一致。对方端口没开、回了「拒绝连接」
//! 也算一次有效的往返（拒绝的报文同样要走一个来回）；超时或者路由不通才算丢了。
//!
//! 开着 Clash、Surge 这类「虚拟网卡」代理时，连往任何地址的握手都会在本机被代理接住：
//! 耗时只有零点几毫秒，不是真实延迟，断了网也照样「连得上」。所以探测用的套接字会先绑到网关
//! 所在的物理网卡上（macOS `IP_BOUND_IF`、Linux `SO_BINDTOIFINDEX`、Windows `IP_UNICAST_IF`），
//! 绕开代理直接出去。

use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroU32;
use std::time::{Duration, Instant};

/// 一次连接最多等多久；超过就当这一次丢了。
const TIMEOUT: Duration = Duration::from_millis(1000);

/// 没绑网卡的连接里，比这更快的读数不可信：公网上的地址不可能这么近，那是代理在本机接住了握手。
const MIN_PLAIN_RTT_MS: f32 = 1.0;

/// 公网探测点：阿里、腾讯和 Cloudflare、Google 的公共 DNS。国内外各有两个，
/// 不管在哪一边的网络里，至少有一个能直连；同时量，取最快有回应的那个。
const PUBLIC: [([u8; 4], u16); 4] = [
    ([223, 5, 5, 5], 443),
    ([119, 29, 29, 29], 80),
    ([1, 1, 1, 1], 443),
    ([8, 8, 8, 8], 443),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// 路由器：局域网这一跳。
    Gateway,
    /// 系统配置的第一个 DNS 服务器。
    Dns,
    /// 公网：拿几个全球可达、稳定的地址代表「能上网」。
    Internet,
}

/// 要量的一个对象。`addrs` 里的地址同时量，读数取最快有回应的那个。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub kind: TargetKind,
    pub addrs: Vec<SocketAddr>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub kind: TargetKind,
    /// 读数来自哪个地址（公网目标有多个地址）。
    pub address: String,
    /// 往返耗时（毫秒）；`None` 是没有回应。
    pub rtt_ms: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencySample {
    pub timestamp_ms: u64,
    pub readings: Vec<Reading>,
}

impl LatencySample {
    pub fn reading(&self, kind: TargetKind) -> Option<&Reading> {
        self.readings.iter().find(|reading| reading.kind == kind)
    }

    /// 公网是否可达：量了公网、并且有回应。
    pub fn online(&self) -> Option<bool> {
        self.reading(TargetKind::Internet)
            .map(|reading| reading.rtt_ms.is_some())
    }
}

/// 一串往返读数的统计。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Quality {
    pub latest_ms: Option<f32>,
    pub average_ms: Option<f32>,
    /// 相邻两次耗时之差的平均值：越小越稳。
    pub jitter_ms: Option<f32>,
    /// 没有回应的占比（百分数）。
    pub loss_percent: f32,
    pub samples: usize,
}

/// 统计一串读数，按时间从旧到新给出。
pub fn quality(readings: impl IntoIterator<Item = Option<f32>>) -> Quality {
    let readings: Vec<Option<f32>> = readings.into_iter().collect();
    let answered: Vec<f32> = readings.iter().flatten().copied().collect();
    let average =
        (!answered.is_empty()).then(|| answered.iter().sum::<f32>() / answered.len() as f32);
    // 只拿「前后两次都有回应」的相邻对算抖动：丢包是丢包，不该算成抖动。
    let steps: Vec<f32> = readings
        .windows(2)
        .filter_map(|pair| Some((pair[0]? - pair[1]?).abs()))
        .collect();
    let jitter = (!steps.is_empty()).then(|| steps.iter().sum::<f32>() / steps.len() as f32);
    let lost = readings.len() - answered.len();
    Quality {
        latest_ms: readings.last().copied().flatten(),
        average_ms: average,
        jitter_ms: jitter,
        loss_percent: if readings.is_empty() {
            0.0
        } else {
            lost as f32 / readings.len() as f32 * 100.0
        },
        samples: readings.len(),
    }
}

/// 值得量的 DNS 服务器：不是本机转发器（systemd-resolved 的 127.0.0.53 之类）、不是链路本地地址，
/// 也不是 198.18.0.0/15——虚拟网卡代理拿这一段当假地址池，把系统 DNS 指到那里，实际在物理网卡上连不通。
fn usable_dns(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_link_local()
        || (a == 198 && (b == 18 || b == 19)))
}

/// 根据当前网络配置决定量哪几个对象。
pub fn targets(gateway: Option<&str>, dns: &[String]) -> Vec<Target> {
    let mut out = Vec::new();
    let gateway_ip = gateway.and_then(|gateway| gateway.parse::<Ipv4Addr>().ok());
    if let Some(ip) = gateway_ip {
        out.push(Target {
            kind: TargetKind::Gateway,
            addrs: vec![SocketAddr::new(ip.into(), 80)],
        });
    }
    // 和网关是同一个地址（家用路由器常见）的 DNS 不重复量。
    let dns_ip = dns
        .iter()
        .filter_map(|server| server.parse::<Ipv4Addr>().ok())
        .find(|ip| usable_dns(*ip) && Some(*ip) != gateway_ip);
    if let Some(ip) = dns_ip {
        out.push(Target {
            kind: TargetKind::Dns,
            addrs: vec![SocketAddr::new(IpAddr::V4(ip), 53)],
        });
    }
    out.push(Target {
        kind: TargetKind::Internet,
        addrs: PUBLIC
            .into_iter()
            .map(|(ip, port)| SocketAddr::from((ip, port)))
            .collect(),
    });
    out
}

/// 同时量所有对象，每个对象各用一个线程，整体耗时就是最慢的那一个。
/// `interface` 是网关所在物理网卡的序号（见 [`gateway_interface`]），给了就从那块网卡出去。
pub fn measure(
    targets: &[Target],
    interface: Option<NonZeroU32>,
    timestamp_ms: u64,
) -> LatencySample {
    let readings = std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .iter()
            .map(|target| scope.spawn(move || probe(target, interface)))
            .collect();
        handles
            .into_iter()
            .zip(targets)
            .map(|(handle, target)| {
                handle.join().unwrap_or_else(|_| Reading {
                    kind: target.kind,
                    address: String::new(),
                    rtt_ms: None,
                })
            })
            .collect()
    });
    LatencySample {
        timestamp_ms,
        readings,
    }
}

fn probe(target: &Target, interface: Option<NonZeroU32>) -> Reading {
    let found = if target.kind == TargetKind::Gateway {
        // 网关就在本网段，绑不绑网卡都不会被代理接走，也不必退而求其次。
        fastest(&target.addrs, interface)
    } else {
        // 从物理网卡直接出去量；这条路不通（比如只放行 VPN 隧道的网络）再退回普通连接，
        // 但普通连接里被代理在本机接住的读数不算数。
        interface
            .and_then(|index| fastest(&target.addrs, Some(index)))
            .or_else(|| fastest(&target.addrs, None).filter(|(_, rtt)| *rtt >= MIN_PLAIN_RTT_MS))
    };
    let address = |index: usize| {
        target
            .addrs
            .get(index)
            .map(|addr| addr.ip().to_string())
            .unwrap_or_default()
    };
    match found {
        Some((index, rtt)) => Reading {
            kind: target.kind,
            address: address(index),
            rtt_ms: Some(rtt),
        },
        None => Reading {
            kind: target.kind,
            address: address(0),
            rtt_ms: None,
        },
    }
}

/// 同时连这几个地址，返回最快有回应的那个（下标，耗时毫秒）。
fn fastest(addrs: &[SocketAddr], interface: Option<NonZeroU32>) -> Option<(usize, f32)> {
    let (tx, rx) = std::sync::mpsc::channel();
    for (index, addr) in addrs.iter().copied().enumerate() {
        let tx = tx.clone();
        // 起不来线程就当这个地址没量，别 panic；线程出了问题 `tx` 随之释放，不会卡住下面的接收。
        let _ = std::thread::Builder::new()
            .name("pc-specs-ping".into())
            .spawn(move || {
                let _ = tx.send((index, connect_rtt(&addr, interface)));
            });
    }
    drop(tx);
    // 第一个有回应的就是最快的，不必等还在超时的那几个（它们自己会在超时后退出）。
    rx.iter().find_map(|(index, rtt)| Some((index, rtt?)))
}

fn connect_rtt(addr: &SocketAddr, interface: Option<NonZeroU32>) -> Option<f32> {
    let socket = Socket::new(
        Domain::for_address(*addr),
        Type::STREAM,
        Some(Protocol::TCP),
    )
    .ok()?;
    if let Some(index) = interface {
        // 绑定失败（权限不够、系统不支持）就当没绑：量得出来总比量不出来好。
        let _ = bind_to_interface(&socket, index);
    }
    let started = Instant::now();
    match socket.connect_timeout(&SockAddr::from(*addr), TIMEOUT) {
        Ok(()) => {}
        // 端口没开：对方的拒绝也是一个来回。
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset
            ) => {}
        Err(_) => return None,
    }
    Some(started.elapsed().as_secs_f32() * 1000.0)
}

#[cfg(unix)]
fn bind_to_interface(socket: &Socket, index: NonZeroU32) -> std::io::Result<()> {
    socket.bind_device_by_index_v4(Some(index))
}

#[cfg(windows)]
fn bind_to_interface(socket: &Socket, index: NonZeroU32) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;
    use windows::Win32::Networking::WinSock::{IP_UNICAST_IF, IPPROTO_IP, SOCKET, setsockopt};
    // IPv4 的 IP_UNICAST_IF 要网络字节序的网卡序号。
    let value = index.get().to_be_bytes();
    // SAFETY: 套接字由 `socket` 持有、在整个调用期间有效；`value` 是栈上的 4 个字节。
    let result = unsafe {
        setsockopt(
            SOCKET(socket.as_raw_socket() as usize),
            IPPROTO_IP.0,
            IP_UNICAST_IF,
            Some(&value),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// 网关所在的那块物理网卡的序号：网关一定在这块网卡的网段里，开着虚拟网卡代理也一样。
pub fn gateway_interface(gateway: &str) -> Option<NonZeroU32> {
    let gateway: Ipv4Addr = gateway.parse().ok()?;
    #[cfg(unix)]
    {
        let networks = sysinfo::Networks::new_with_refreshed_list().ok()?;
        networks.iter().find_map(|(name, data)| {
            let on_link = data.ip_networks().iter().any(|net| match net.addr {
                IpAddr::V4(addr) => same_subnet(addr, gateway, net.prefix),
                IpAddr::V6(_) => false,
            });
            let name = std::ffi::CString::new(name.to_str()?).ok()?;
            // SAFETY: `name` 是以 NUL 结尾的有效 C 字符串。
            on_link
                .then(|| NonZeroU32::new(unsafe { libc::if_nametoindex(name.as_ptr()) }))
                .flatten()
        })
    }
    #[cfg(windows)]
    {
        use windows::Win32::NetworkManagement::IpHelper::GetBestInterface;
        let mut index = 0u32;
        // SAFETY: `index` 在调用期间是有效的输出地址；目标地址按网络字节序传入。
        let status = unsafe { GetBestInterface(u32::from_ne_bytes(gateway.octets()), &mut index) };
        if status == 0 {
            NonZeroU32::new(index)
        } else {
            None
        }
    }
}

#[cfg(unix)]
fn same_subnet(a: Ipv4Addr, b: Ipv4Addr, prefix: u8) -> bool {
    let mask = match prefix {
        0 => 0,
        1..=31 => u32::MAX << (32 - u32::from(prefix)),
        _ => u32::MAX,
    };
    u32::from(a) & mask == u32::from(b) & mask
}

// ---------------------------------------------------------------------------
// Wi-Fi

/// 当前 Wi-Fi 连接的细节。各系统能给出的项不一样，给不出的就是 `None`。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WifiInfo {
    pub interface: String,
    /// 网络名。macOS 14 之后没有定位授权就拿不到，那时是 `None`。
    pub ssid: Option<String>,
    pub bssid: Option<String>,
    pub signal_dbm: Option<i32>,
    pub noise_dbm: Option<i32>,
    /// 系统只给了百分比（Windows、NetworkManager）时用它。
    pub signal_percent: Option<u8>,
    pub channel: Option<u32>,
    pub band: Option<String>,
    pub width_mhz: Option<u32>,
    /// 协议，比如 `802.11ax`。
    pub standard: Option<String>,
    /// 当前连接速率（Mbps）。
    pub rate_mbps: Option<u32>,
    pub security: Option<String>,
}

impl WifiInfo {
    /// 信号质量 0–100。系统给了百分比就用它，否则由 dBm 折算：-50 dBm 及更强算 100，-100 dBm 及更弱算 0。
    pub fn quality(&self) -> Option<u8> {
        if let Some(percent) = self.signal_percent {
            return Some(percent.min(100));
        }
        let dbm = self.signal_dbm?;
        Some(((dbm + 100) * 2).clamp(0, 100) as u8)
    }

    /// 信噪比（dB），信号和底噪都知道时才有。
    pub fn snr_db(&self) -> Option<i32> {
        Some(self.signal_dbm? - self.noise_dbm?)
    }

    /// 协议对应的代际名称：`Wi-Fi 6` 之类。
    pub fn generation(&self) -> Option<&'static str> {
        let standard = self.standard.as_deref()?;
        let six_ghz = self.band.as_deref() == Some("6 GHz");
        Some(match standard.trim_start_matches("802.11") {
            "be" => "Wi-Fi 7",
            "ax" if six_ghz => "Wi-Fi 6E",
            "ax" => "Wi-Fi 6",
            "ac" => "Wi-Fi 5",
            "n" => "Wi-Fi 4",
            _ => return None,
        })
    }
}

/// 读取当前的 Wi-Fi 连接；没连 Wi-Fi、机器没有无线网卡、系统没给出信息都是 `None`。
/// `interfaces` 是 Wi-Fi 网卡的名字（Linux 要逐个问，另外两个系统自己会找）。
/// 会起外部命令，耗时从几十毫秒（Windows）到一秒多（macOS），不要在界面线程调用。
pub fn wifi(interfaces: &[String]) -> Option<WifiInfo> {
    #[cfg(target_os = "macos")]
    {
        let _ = interfaces;
        let json = crate::util::run_stdout_timeout(
            "/usr/sbin/system_profiler",
            &["SPAirPortDataType", "-json"],
            Duration::from_secs(8),
        )?;
        parse_system_profiler(&json)
    }
    #[cfg(target_os = "windows")]
    {
        let _ = interfaces;
        // 先切到 UTF-8 代码页，本地化的中文标签才不会变成乱码。
        let output = crate::util::run_shell(
            "chcp 65001 >nul & netsh wlan show interfaces",
            &[],
            Duration::from_secs(5),
        )?;
        parse_netsh(&output.stdout)
    }
    #[cfg(target_os = "linux")]
    {
        linux_wifi(interfaces)
    }
}

#[cfg(target_os = "linux")]
fn linux_wifi(interfaces: &[String]) -> Option<WifiInfo> {
    use crate::util::run_stdout_timeout;
    let timeout = Duration::from_secs(3);
    for name in interfaces {
        let text = ["iw", "/usr/sbin/iw", "/sbin/iw"]
            .iter()
            .find_map(|iw| run_stdout_timeout(iw, &["dev", name, "link"], timeout));
        if let Some(info) = text.and_then(|text| parse_iw(name, &text)) {
            return Some(info);
        }
    }
    // 没有 iw：问 NetworkManager，它只给信号百分比。
    let text = run_stdout_timeout(
        "nmcli",
        &[
            "-t",
            "-f",
            "ACTIVE,SSID,BSSID,CHAN,FREQ,RATE,SIGNAL,SECURITY",
            "dev",
            "wifi",
        ],
        timeout,
    )?;
    let name = interfaces.first().cloned().unwrap_or_default();
    parse_nmcli(&name, &text)
}

/// 去掉系统给的「不可用」占位（macOS 在没有定位授权时把网络名写成 `<redacted>`）。
fn usable(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty() && text != "<redacted>").then(|| text.to_string())
}

/// 信道号 → 频段。6 GHz 的信道号和 5 GHz 重叠，只凭信道号分不开，这里按 5 GHz 算。
fn band_of_channel(channel: u32) -> Option<&'static str> {
    match channel {
        1..=14 => Some("2.4 GHz"),
        32..=177 => Some("5 GHz"),
        _ => None,
    }
}

/// 频率（MHz）→ (信道号, 频段)。
fn channel_of_frequency(mhz: u32) -> Option<(u32, &'static str)> {
    match mhz {
        2484 => Some((14, "2.4 GHz")),
        2412..=2472 => Some(((mhz - 2407) / 5, "2.4 GHz")),
        5160..=5885 => Some(((mhz - 5000) / 5, "5 GHz")),
        5955..=7115 => Some(((mhz - 5950) / 5, "6 GHz")),
        _ => None,
    }
}

// ---- macOS: system_profiler SPAirPortDataType -json ----

/// macOS 14 之后没有 `airport` 命令，`networksetup` 和 `ipconfig` 也把网络名遮掉了；
/// 只有 `system_profiler` 还能给出信道、速率、信号和底噪。
#[cfg_attr(not(any(target_os = "macos", test)), allow(dead_code))]
pub(crate) fn parse_system_profiler(json: &str) -> Option<WifiInfo> {
    let root: serde_json::Value = serde_json::from_str(json).ok()?;
    let interfaces = root
        .get("SPAirPortDataType")?
        .as_array()?
        .iter()
        .filter_map(|card| card.get("spairport_airport_interfaces")?.as_array())
        .flatten();
    for interface in interfaces {
        let connected = interface
            .get("spairport_status_information")
            .and_then(|status| status.as_str())
            == Some("spairport_status_connected");
        let Some(current) = interface.get("spairport_current_network_information") else {
            continue;
        };
        if !connected {
            continue;
        }
        let text = |key: &str| current.get(key).and_then(|value| value.as_str());
        let (channel, band, width_mhz) = text("spairport_network_channel")
            .map(parse_channel_text)
            .unwrap_or_default();
        let (signal_dbm, noise_dbm) = text("spairport_signal_noise")
            .map(parse_signal_noise)
            .unwrap_or_default();
        return Some(WifiInfo {
            interface: interface
                .get("_name")
                .and_then(|name| name.as_str())
                .unwrap_or_default()
                .to_string(),
            ssid: text("_name").and_then(usable),
            bssid: None,
            signal_dbm,
            noise_dbm,
            signal_percent: None,
            channel,
            band,
            width_mhz,
            standard: text("spairport_network_phymode").and_then(usable),
            rate_mbps: current
                .get("spairport_network_rate")
                .and_then(|rate| rate.as_u64())
                .and_then(|rate| u32::try_from(rate).ok()),
            security: text("spairport_security_mode").map(macos_security),
        });
    }
    None
}

/// `40 (5GHz, 160MHz)` → (40, 5 GHz, 160)。
fn parse_channel_text(text: &str) -> (Option<u32>, Option<String>, Option<u32>) {
    let text = text.trim();
    let channel = text.split_whitespace().next().and_then(|n| n.parse().ok());
    let inner = text
        .split_once('(')
        .and_then(|(_, rest)| rest.trim_end().strip_suffix(')'));
    let (mut band, mut width) = (None, None);
    for part in inner.into_iter().flat_map(|inner| inner.split(',')) {
        let part = part.trim();
        if let Some(ghz) = part.strip_suffix("GHz") {
            band = Some(match ghz.trim() {
                "2" | "2.4" => "2.4 GHz".to_string(),
                other => format!("{other} GHz"),
            });
        } else if let Some(mhz) = part.strip_suffix("MHz") {
            width = mhz.trim().parse().ok();
        }
    }
    (channel, band, width)
}

/// `-39 dBm / -91 dBm` → (-39, -91)。
fn parse_signal_noise(text: &str) -> (Option<i32>, Option<i32>) {
    let mut numbers = text.split('/').map(|part| {
        part.trim()
            .trim_end_matches("dBm")
            .trim()
            .parse::<i32>()
            .ok()
    });
    (numbers.next().flatten(), numbers.next().flatten())
}

/// `spairport_security_mode_wpa2_personal` → `WPA2 Personal`。
fn macos_security(raw: &str) -> String {
    let raw = raw
        .strip_prefix("spairport_security_mode_")
        .unwrap_or(raw)
        .trim();
    if raw == "none" {
        return "Open".to_string();
    }
    raw.split('_')
        .map(|word| {
            if word.starts_with("wpa") || word.starts_with("wep") || matches!(word, "psk" | "eap") {
                word.to_uppercase()
            } else {
                let mut chars = word.chars();
                chars
                    .next()
                    .map(|first| first.to_uppercase().chain(chars).collect())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

// ---- Windows: netsh wlan show interfaces ----

/// 按「名称 : 值」逐行读，英文和中文系统的标签都认。`netsh` 一次会列出所有无线网卡，取已连接的那块。
#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
pub(crate) fn parse_netsh(text: &str) -> Option<WifiInfo> {
    let mut cards: Vec<WifiInfo> = Vec::new();
    let mut connected: Vec<bool> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_lowercase(), value.trim());
        let has = |names: &[&str]| names.iter().any(|name| key.contains(name));
        if has(&["name", "名称"]) && !has(&["ssid", "profile", "配置"]) {
            cards.push(WifiInfo {
                interface: value.to_string(),
                ..WifiInfo::default()
            });
            connected.push(false);
            continue;
        }
        let (Some(card), Some(is_connected)) = (cards.last_mut(), connected.last_mut()) else {
            continue;
        };
        let number = || {
            value
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<f32>().ok())
        };
        if key == "ssid" {
            card.ssid = usable(value);
            *is_connected = card.ssid.is_some();
        } else if key == "bssid" {
            card.bssid = usable(value);
        } else if has(&["radio type", "无线电类型"]) {
            card.standard = usable(value);
        } else if has(&["authentication", "身份验证"]) {
            card.security = usable(value);
        } else if has(&["channel", "信道"]) {
            card.channel = value.parse().ok();
            card.band = card.channel.and_then(band_of_channel).map(str::to_string);
        } else if has(&["transmit rate", "传输速率", "发送速率"]) {
            card.rate_mbps = number().map(|rate| rate.round() as u32);
        } else if has(&["receive rate", "接收速率"]) {
            // 只有没读到发送速率时才用接收速率。
            if card.rate_mbps.is_none() {
                card.rate_mbps = number().map(|rate| rate.round() as u32);
            }
        } else if has(&["signal", "信号"]) {
            card.signal_percent = value
                .trim_end_matches('%')
                .trim()
                .parse::<u8>()
                .ok()
                .map(|percent| percent.min(100));
        }
    }
    cards
        .into_iter()
        .zip(connected)
        .find_map(|(card, connected)| connected.then_some(card))
}

// ---- Linux: iw dev <iface> link / nmcli ----

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn parse_iw(interface: &str, text: &str) -> Option<WifiInfo> {
    let mut lines = text.lines();
    let first = lines.next()?.trim();
    // 没连上时只有一行 `Not connected.`。
    let bssid = first.strip_prefix("Connected to ")?;
    let bssid = bssid.split_whitespace().next().map(str::to_string);
    let mut info = WifiInfo {
        interface: interface.to_string(),
        bssid,
        ..WifiInfo::default()
    };
    for line in lines {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "SSID" => info.ssid = usable(value),
            "freq" => {
                if let Some((channel, band)) = value
                    .split_whitespace()
                    .next()
                    .and_then(|mhz| mhz.parse::<f32>().ok())
                    .and_then(|mhz| channel_of_frequency(mhz as u32))
                {
                    info.channel = Some(channel);
                    info.band = Some(band.to_string());
                }
            }
            "signal" => {
                info.signal_dbm = value
                    .split_whitespace()
                    .next()
                    .and_then(|dbm| dbm.parse::<f32>().ok())
                    .map(|dbm| dbm.round() as i32);
            }
            "tx bitrate" => {
                let mut parts = value.split_whitespace();
                info.rate_mbps = parts
                    .next()
                    .and_then(|rate| rate.parse::<f32>().ok())
                    .map(|rate| rate.round() as u32);
                for token in parts {
                    if let Some(width) = token.strip_suffix("MHz") {
                        info.width_mhz = width.parse().ok();
                    }
                    // 速率后面跟着调制方式，借它判断协议。
                    let standard = if token.starts_with("EHT-") {
                        Some("802.11be")
                    } else if token.starts_with("HE-") {
                        Some("802.11ax")
                    } else if token.starts_with("VHT-") {
                        Some("802.11ac")
                    } else if token.starts_with("MCS") {
                        Some("802.11n")
                    } else {
                        None
                    };
                    if let Some(standard) = standard {
                        info.standard = Some(standard.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    Some(info)
}

/// `nmcli -t` 的字段用冒号分隔，值里的冒号写成 `\:`（BSSID 里就有）。
fn split_nmcli(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        let literal = match c {
            ':' => {
                fields.push(String::new());
                continue;
            }
            '\\' => chars.next(),
            c => Some(c),
        };
        if let (Some(c), Some(field)) = (literal, fields.last_mut()) {
            field.push(c);
        }
    }
    fields
}

#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn parse_nmcli(interface: &str, text: &str) -> Option<WifiInfo> {
    let fields = text
        .lines()
        .map(split_nmcli)
        .find(|fields| fields.first().is_some_and(|active| active == "yes"))?;
    let field = |index: usize| fields.get(index).map(String::as_str).unwrap_or_default();
    let number = |index: usize| {
        field(index)
            .split_whitespace()
            .next()
            .and_then(|n| n.parse::<f32>().ok())
    };
    let channel = field(3).parse().ok();
    Some(WifiInfo {
        interface: interface.to_string(),
        ssid: usable(field(1)),
        bssid: usable(field(2)),
        signal_percent: number(6).map(|percent| (percent as u8).min(100)),
        band: number(4)
            .and_then(|mhz| channel_of_frequency(mhz as u32))
            .map(|(_, band)| band.to_string())
            .or_else(|| channel.and_then(band_of_channel).map(str::to_string)),
        channel,
        rate_mbps: number(5).map(|rate| rate.round() as u32),
        security: usable(field(7)),
        ..WifiInfo::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_ignores_lost_probes_and_loss_counts_them() {
        let q = quality([Some(10.0), Some(20.0), None, Some(20.0), Some(30.0)]);
        assert_eq!(q.samples, 5);
        assert_eq!(q.latest_ms, Some(30.0));
        assert_eq!(q.average_ms, Some(20.0));
        // 只有 (10,20) 和 (20,30) 两对相邻读数都有回应。
        assert_eq!(q.jitter_ms, Some(10.0));
        assert_eq!(q.loss_percent, 20.0);
        assert_eq!(quality([]), Quality::default());
        assert_eq!(quality([None, None]).loss_percent, 100.0);
    }

    #[test]
    fn targets_skip_unusable_and_duplicate_dns() {
        let dns = vec!["127.0.0.53".to_string(), "192.168.1.1".to_string()];
        let found = targets(Some("192.168.1.1"), &dns);
        // 网关、公网；DNS 要么是回环、要么和网关重复，都不重复量。
        let kinds: Vec<_> = found.iter().map(|target| target.kind).collect();
        assert_eq!(kinds, [TargetKind::Gateway, TargetKind::Internet]);
        // 虚拟网卡代理的假地址池（198.18.0.0/15）连不通，跳到下一个。
        let dns = vec!["198.18.0.2".to_string(), "10.0.0.2".to_string()];
        let found = targets(None, &dns);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].kind, TargetKind::Dns);
        assert_eq!(found[0].addrs[0].ip().to_string(), "10.0.0.2");
        let last = targets(None, &[]);
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].kind, TargetKind::Internet);
        assert_eq!(last[0].addrs.len(), PUBLIC.len());
    }

    #[test]
    fn handshakes_the_local_proxy_answers_are_not_trusted() {
        // 本机回环的握手只要几十微秒：对网关是真实读数，对 DNS / 公网只会是代理接住了握手。
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addrs = vec![listener.local_addr().unwrap()];
        let gateway = probe(
            &Target {
                kind: TargetKind::Gateway,
                addrs: addrs.clone(),
            },
            None,
        );
        assert!(gateway.rtt_ms.is_some_and(|rtt| rtt < MIN_PLAIN_RTT_MS));
        let dns = probe(
            &Target {
                kind: TargetKind::Dns,
                addrs,
            },
            None,
        );
        assert_eq!(dns.rtt_ms, None);
        assert_eq!(dns.address, "127.0.0.1");
    }

    #[test]
    fn a_closed_port_still_counts_as_a_round_trip() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        assert!(connect_rtt(&addr, None).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn subnet_matching_follows_the_prefix() {
        let ip = |s: &str| s.parse::<Ipv4Addr>().unwrap();
        assert!(same_subnet(ip("192.168.5.20"), ip("192.168.5.1"), 24));
        assert!(!same_subnet(ip("192.168.6.20"), ip("192.168.5.1"), 24));
        assert!(same_subnet(ip("10.1.2.3"), ip("10.200.0.1"), 8));
        assert!(same_subnet(ip("1.2.3.4"), ip("200.2.3.4"), 0));
        assert!(!same_subnet(ip("1.2.3.4"), ip("1.2.3.5"), 32));
    }

    #[test]
    fn reads_the_connected_card_from_system_profiler() {
        let json = r#"{"SPAirPortDataType":[{"spairport_airport_interfaces":[
            {"_name":"en0","spairport_status_information":"spairport_status_connected",
             "spairport_current_network_information":{"_name":"<redacted>",
               "spairport_network_channel":"40 (5GHz, 160MHz)","spairport_network_phymode":"802.11ax",
               "spairport_network_rate":2401,"spairport_security_mode":"spairport_security_mode_wpa2_personal",
               "spairport_signal_noise":"-39 dBm / -91 dBm"}},
            {"_name":"awdl0","spairport_current_network_information":{"spairport_network_type":"spairport_network_type_station"}}
        ]}]}"#;
        let info = parse_system_profiler(json).unwrap();
        assert_eq!(info.interface, "en0");
        assert_eq!(info.ssid, None);
        assert_eq!(info.channel, Some(40));
        assert_eq!(info.band.as_deref(), Some("5 GHz"));
        assert_eq!(info.width_mhz, Some(160));
        assert_eq!(info.rate_mbps, Some(2401));
        assert_eq!(info.signal_dbm, Some(-39));
        assert_eq!(info.snr_db(), Some(52));
        assert_eq!(info.security.as_deref(), Some("WPA2 Personal"));
        assert_eq!(info.generation(), Some("Wi-Fi 6"));
        assert_eq!(info.quality(), Some(100));
        let off = r#"{"SPAirPortDataType":[{"spairport_airport_interfaces":[
            {"_name":"en0","spairport_status_information":"spairport_status_disconnected"}]}]}"#;
        assert!(parse_system_profiler(off).is_none());
    }

    #[test]
    fn reads_english_and_chinese_netsh_output() {
        let english = "\
There is 1 interface on the system:

    Name                   : Wi-Fi
    Description            : Intel(R) Wi-Fi 6 AX201 160MHz
    Physical address       : aa:bb:cc:dd:ee:ff
    State                  : connected
    SSID                   : Home
    BSSID                  : 11:22:33:44:55:66
    Network type           : Infrastructure
    Radio type             : 802.11ax
    Authentication         : WPA2-Personal
    Cipher                 : CCMP
    Channel                : 36
    Receive rate (Mbps)    : 1201
    Transmit rate (Mbps)   : 1080
    Signal                 : 82%
    Profile                : Home
";
        let info = parse_netsh(english).unwrap();
        assert_eq!(info.interface, "Wi-Fi");
        assert_eq!(info.ssid.as_deref(), Some("Home"));
        assert_eq!(info.bssid.as_deref(), Some("11:22:33:44:55:66"));
        assert_eq!(info.standard.as_deref(), Some("802.11ax"));
        assert_eq!(info.channel, Some(36));
        assert_eq!(info.band.as_deref(), Some("5 GHz"));
        assert_eq!(info.rate_mbps, Some(1080));
        assert_eq!(info.signal_percent, Some(82));
        assert_eq!(info.security.as_deref(), Some("WPA2-Personal"));

        let chinese = "\
    名称                   : WLAN
    状态                   : 已连接
    SSID                   : 家里
    BSSID                  : 11:22:33:44:55:66
    无线电类型             : 802.11n
    身份验证               : WPA2 - 个人
    信道                   : 6
    接收速率(Mbps)         : 144
    传输速率(Mbps)         : 130
    信号                   : 60%
    配置文件               : 家里
";
        let info = parse_netsh(chinese).unwrap();
        assert_eq!(info.ssid.as_deref(), Some("家里"));
        assert_eq!(info.band.as_deref(), Some("2.4 GHz"));
        assert_eq!(info.rate_mbps, Some(130));
        assert_eq!(info.signal_percent, Some(60));

        let disconnected =
            "    Name                   : Wi-Fi\n    State                  : disconnected\n";
        assert!(parse_netsh(disconnected).is_none());
    }

    #[test]
    fn reads_iw_and_nmcli_output() {
        let iw = "Connected to 11:22:33:44:55:66 (on wlan0)\n\tSSID: Home\n\tfreq: 5180\n\tsignal: -52 dBm\n\trx bitrate: 866.7 MBit/s VHT-MCS 9 80MHz short GI VHT-NSS 2\n\ttx bitrate: 780.0 MBit/s VHT-MCS 8 80MHz short GI VHT-NSS 2\n";
        let info = parse_iw("wlan0", iw).unwrap();
        assert_eq!(info.ssid.as_deref(), Some("Home"));
        assert_eq!(info.channel, Some(36));
        assert_eq!(info.band.as_deref(), Some("5 GHz"));
        assert_eq!(info.signal_dbm, Some(-52));
        assert_eq!(info.rate_mbps, Some(780));
        assert_eq!(info.width_mhz, Some(80));
        assert_eq!(info.standard.as_deref(), Some("802.11ac"));
        assert!(parse_iw("wlan0", "Not connected.\n").is_none());

        let nmcli = "no:Other:AA\\:BB\\:CC\\:DD\\:EE\\:01:1:2412 MHz:54 Mbit/s:30:WPA2\nyes:Home:11\\:22\\:33\\:44\\:55\\:66:149:5745 MHz:540 Mbit/s:77:WPA2 WPA3\n";
        let info = parse_nmcli("wlan0", nmcli).unwrap();
        assert_eq!(info.ssid.as_deref(), Some("Home"));
        assert_eq!(info.bssid.as_deref(), Some("11:22:33:44:55:66"));
        assert_eq!(info.channel, Some(149));
        assert_eq!(info.band.as_deref(), Some("5 GHz"));
        assert_eq!(info.rate_mbps, Some(540));
        assert_eq!(info.signal_percent, Some(77));
        assert_eq!(info.security.as_deref(), Some("WPA2 WPA3"));
    }
}

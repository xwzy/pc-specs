//! 展示用格式化：容量 / 速率 / 温度 / 时长 / 频率，全部受用户设置的单位影响。

use crate::settings::{NetUnit, Settings, SizeUnit, TempUnit};
use pc_specs_core::model::{CpuInfo, OsInfo, SensorReading};
use rust_i18n::t;

/// 数值位数随大小自适应：>=100 不带小数，>=10 一位，其余两位。
fn scaled(value: f64) -> String {
    if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

pub fn bytes(n: u64, unit: SizeUnit) -> String {
    let (base, names): (f64, [&str; 5]) = match unit {
        SizeUnit::Binary => (1024.0, ["B", "KiB", "MiB", "GiB", "TiB"]),
        SizeUnit::Decimal => (1000.0, ["B", "KB", "MB", "GB", "TB"]),
    };
    let mut value = n as f64;
    let mut idx = 0;
    while value >= base && idx < names.len() - 1 {
        value /= base;
        idx += 1;
    }
    if idx == 0 {
        format!("{n} B")
    } else {
        format!("{} {}", scaled(value), names[idx])
    }
}

/// 网速。字节模式按 1024 进制显示 KB/s、MB/s；比特模式按 1000 进制显示 Kbps、Mbps。
pub fn rate(bytes_per_sec: u64, unit: NetUnit) -> String {
    match unit {
        NetUnit::BytesPerSec => {
            let names = ["B/s", "KB/s", "MB/s", "GB/s"];
            let mut value = bytes_per_sec as f64;
            let mut idx = 0;
            while value >= 1024.0 && idx < names.len() - 1 {
                value /= 1024.0;
                idx += 1;
            }
            if idx == 0 {
                format!("{bytes_per_sec} B/s")
            } else {
                format!("{} {}", scaled(value), names[idx])
            }
        }
        NetUnit::BitsPerSec => {
            let names = ["bps", "Kbps", "Mbps", "Gbps"];
            let mut value = bytes_per_sec as f64 * 8.0;
            let mut idx = 0;
            while value >= 1000.0 && idx < names.len() - 1 {
                value /= 1000.0;
                idx += 1;
            }
            if idx == 0 {
                format!("{value:.0} bps")
            } else {
                format!("{} {}", scaled(value), names[idx])
            }
        }
    }
}

/// 图表上速率的单位：按这一段时间里的峰值挑个读起来舒服的量级，
/// 几十 KB/s 的流量就不必在纵轴上写成 `0.03 MB/s` 了。
#[derive(Clone, Copy)]
pub struct RateScale {
    /// 一个单位等于多少字节每秒。
    per_unit: f64,
    pub name: &'static str,
}

impl RateScale {
    pub fn new(peak_bytes_per_sec: u64, unit: NetUnit) -> Self {
        // 比特按 1000 进制：1 Kbps = 125 B/s。
        let tiers: [(f64, &str); 3] = match unit {
            NetUnit::BytesPerSec => [
                (1024.0, "KB/s"),
                (1024.0 * 1024.0, "MB/s"),
                (1024.0 * 1024.0 * 1024.0, "GB/s"),
            ],
            NetUnit::BitsPerSec => [
                (125.0, "Kbps"),
                (125_000.0, "Mbps"),
                (125_000_000.0, "Gbps"),
            ],
        };
        let peak = peak_bytes_per_sec as f64;
        let (per_unit, name) = tiers
            .iter()
            .rev()
            .find(|(per_unit, _)| peak >= *per_unit)
            .copied()
            .unwrap_or(tiers[0]);
        Self { per_unit, name }
    }

    pub fn value(self, bytes_per_sec: u64) -> f64 {
        bytes_per_sec as f64 / self.per_unit
    }
}

/// 电池容量：毫瓦时换成瓦时，「76739 mWh」不如「76.7 Wh」好读。
pub fn watt_hours(mwh: u64) -> String {
    format!("{:.1} Wh", mwh as f64 / 1000.0)
}

/// 「8 性能核 + 4 能效核 · 12 线程」；不分大小核的就是「8 核 · 16 线程」。
pub fn core_summary(cpu: &CpuInfo) -> String {
    let threads = cpu.logical_cores.max(1);
    let hybrid = cpu
        .topology
        .as_ref()
        .and_then(|t| t.p_cores.zip(t.e_cores))
        .filter(|(p, e)| *p > 0 && *e > 0);
    match hybrid {
        Some((p, e)) => t!("bar_core_hybrid", p = p, e = e, threads = threads),
        None => t!(
            "bar_core_count",
            cores = cpu.physical_cores.max(1),
            threads = threads
        ),
    }
    .into_owned()
}

/// 纵轴刻度的文字：数值大就不带小数，小就多留一位。
pub fn axis_label(value: f64) -> String {
    if value <= 0.0 {
        "0".to_string()
    } else if value >= 100.0 {
        format!("{value:.0}")
    } else if value >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

/// 把 B/s 换算成固定的 MB/s 或 Mbps，只给不显示刻度的迷你走势用。
pub fn rate_chart_value(bytes_per_sec: u64, unit: NetUnit) -> f64 {
    match unit {
        NetUnit::BytesPerSec => bytes_per_sec as f64 / (1024.0 * 1024.0),
        NetUnit::BitsPerSec => bytes_per_sec as f64 * 8.0 / 1_000_000.0,
    }
}

/// 紧凑速率（托盘 / 悬浮窗用）：`2.4M`、`312K`。数字最多三位：`1023K` 直接进位成 `1.0M`，
/// 菜单栏上的位图宽度才不会因为这一两个读数突然撑大。
pub fn rate_compact(bytes_per_sec: u64, unit: NetUnit) -> String {
    let (mut value, names, base) = match unit {
        NetUnit::BytesPerSec => (bytes_per_sec as f64, ["B", "K", "M", "G"], 1024.0),
        NetUnit::BitsPerSec => (bytes_per_sec as f64 * 8.0, ["b", "K", "M", "G"], 1000.0),
    };
    let mut idx = 0;
    while value >= 999.5 && idx < names.len() - 1 {
        value /= base;
        idx += 1;
    }
    if idx == 0 || value >= 99.95 {
        format!("{value:.0}{}", names[idx])
    } else {
        format!("{value:.1}{}", names[idx])
    }
}

pub fn temp(celsius: f32, unit: TempUnit) -> String {
    match unit {
        TempUnit::Celsius => format!("{celsius:.0}°C"),
        TempUnit::Fahrenheit => format!("{:.0}°F", celsius * 9.0 / 5.0 + 32.0),
    }
}

pub fn temp_unit_name(unit: TempUnit) -> &'static str {
    match unit {
        TempUnit::Celsius => "°C",
        TempUnit::Fahrenheit => "°F",
    }
}

/// 摄氏度 → 图表所用单位的数值。
pub fn temp_chart_value(celsius: f32, unit: TempUnit) -> f64 {
    match unit {
        TempUnit::Celsius => celsius as f64,
        TempUnit::Fahrenheit => celsius as f64 * 9.0 / 5.0 + 32.0,
    }
}

pub fn percent(value: f32) -> String {
    format!("{value:.0}%")
}

/// 往返延迟：个位数毫秒留一位小数（`3.2 ms`），再大就不必了（`18 ms`）。
pub fn latency(ms: f32) -> String {
    if ms < 10.0 {
        format!("{ms:.1} ms")
    } else {
        format!("{ms:.0} ms")
    }
}

/// 占比（0..=100）。total 为 0 时返回 0。
pub fn ratio(used: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64 * 100.0) as f32
    }
}

/// 频率。Apple Silicon 等不暴露频率的平台传 0，显示为 "—"。
pub fn freq(hz: u64) -> String {
    if hz == 0 {
        "—".into()
    } else if hz >= 1_000_000_000 {
        format!("{:.2} GHz", hz as f64 / 1e9)
    } else {
        format!("{} MHz", hz / 1_000_000)
    }
}

/// 时长，跟随界面语言：`3d 9h 20m` / `3 天 9 小时 20 分`。
pub fn duration(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    let text = if d > 0 {
        t!("dur_dhm", d = d, h = h, m = m)
    } else if h > 0 {
        t!("dur_hm", h = h, m = m)
    } else if m > 0 {
        t!("dur_ms", m = m, s = s)
    } else {
        t!("dur_s", s = s)
    };
    text.into_owned()
}

/// 存储介质的显示名。没识别出来的类型返回 `None`，界面上干脆不显示，免得冒出一句英文的 Unknown。
pub fn storage_kind(kind: &str) -> Option<String> {
    match kind {
        "" | "Unknown" => None,
        "Removable" => Some(t!("storage_kind_removable").into_owned()),
        other => Some(other.to_string()),
    }
}

/// 系统名称 + 版本。有的平台名称里已经带了版本号（`macOS 26.2 Tahoe`），不要再接一遍。
pub fn os_title(os: &OsInfo) -> String {
    let (name, version) = (os.name.trim(), os.version.trim());
    if version.is_empty() || name.contains(version) {
        name.to_string()
    } else {
        format!("{name} {version}")
    }
}

/// 电池状态的显示名；不认识的状态原样显示。
pub fn battery_state(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "charging" => t!("bat_state_charging"),
        "discharging" => t!("bat_state_discharging"),
        "full" => t!("bat_state_full"),
        "empty" => t!("bat_state_empty"),
        "unknown" => t!("bat_state_unknown"),
        _ => return raw.to_string(),
    }
    .into_owned()
}

/// 网卡类型的显示名；不认识的类型原样显示。
pub fn net_kind(kind: &str) -> String {
    match kind {
        "wifi" => t!("net_kind_wifi"),
        "ethernet" => t!("net_kind_ethernet"),
        "bluetooth" => t!("net_kind_bluetooth"),
        "virtual" => t!("net_kind_virtual"),
        "loopback" => t!("net_kind_loopback"),
        "other" => t!("net_kind_other"),
        other => return other.to_string(),
    }
    .into_owned()
}

/// Unix 秒 → 本地时间 `YYYY-MM-DD HH:MM`。
pub fn datetime(unix_secs: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(unix_secs as i64, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

/// Unix 毫秒 → 本地时间 `HH:MM:SS`。
pub fn clock(unix_millis: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(unix_millis as i64)
        .single()
        .map(|t| t.format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

/// 自然排序：`tdie2` 排在 `tdie10` 前面，大小写不敏感。
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    let number = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
        let mut n: u64 = 0;
        while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
            n = n
                .saturating_mul(10)
                .saturating_add(u64::from(c as u8 - b'0'));
            it.next();
        }
        n
    };
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (nx, ny) = (number(&mut a), number(&mut b));
                if nx != ny {
                    return nx.cmp(&ny);
                }
            }
            (Some(x), Some(y)) => {
                let (lx, ly) = (x.to_ascii_lowercase(), y.to_ascii_lowercase());
                if lx != ly {
                    return lx.cmp(&ly);
                }
                a.next();
                b.next();
            }
        }
    }
}

/// 热压力等级（0 正常、1 中等、2 较重、3 严重、4 休眠）的显示名。
pub fn thermal_pressure(level: u8) -> String {
    match level {
        0 => t!("thermal_nominal"),
        1 => t!("thermal_moderate"),
        2 => t!("thermal_heavy"),
        _ => t!("thermal_critical"),
    }
    .into_owned()
}

/// macOS SMC 里带序号的传感器名（风扇、CPU / GPU 逐核温度）对应的翻译键和序号。
fn smc_numbered_label(label: &str) -> Option<(&'static str, u32)> {
    [
        ("Fan ", "sensor_fan_n"),
        ("CPU efficiency core ", "sensor_cpu_efficiency_core"),
        ("CPU performance core ", "sensor_cpu_performance_core"),
        ("CPU super core ", "sensor_cpu_super_core"),
        ("GPU ", "sensor_gpu_n"),
    ]
    .into_iter()
    .find_map(|(prefix, key)| Some((key, label.strip_prefix(prefix)?.parse().ok()?)))
}

/// 传感器的显示名：macOS SMC 那几个固定的名字翻译成界面语言，硬件自己报的名字原样显示。
pub fn sensor_label(r: &SensorReading) -> String {
    if r.source != "smc" {
        return r.label.clone();
    }
    match r.label.as_str() {
        "System Total" => t!("sensor_system_total").into_owned(),
        "DC In" => t!("sensor_dc_in").into_owned(),
        "Battery" => t!("sensor_battery").into_owned(),
        other => match smc_numbered_label(other) {
            Some((key, n)) => t!(key, n = n).into_owned(),
            None => other.to_string(),
        },
    }
}

/// 风扇 / 功率 / 电压 / 电流这类读数带单位的显示（温度要照用户设置换算，不走这里）。
pub fn sensor_value(r: &SensorReading) -> String {
    match r.unit.as_str() {
        "RPM" => format!("{:.0} RPM", r.value),
        "V" => format!("{:.2} V", r.value),
        "A" => format!("{:.2} A", r.value),
        unit => format!("{:.1} {unit}", r.value),
    }
}

/// 空字符串显示为 "—"。
pub fn or_dash(s: &str) -> String {
    if s.trim().is_empty() {
        "—".into()
    } else {
        s.to_string()
    }
}

impl Settings {
    pub fn bytes(&self, n: u64) -> String {
        bytes(n, self.size_unit)
    }

    pub fn rate(&self, bytes_per_sec: u64) -> String {
        rate(bytes_per_sec, self.net_unit)
    }

    pub fn temp(&self, celsius: f32) -> String {
        temp(celsius, self.temp_unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smc_sensor_names_with_a_number_are_translated() {
        assert_eq!(smc_numbered_label("Fan 2"), Some(("sensor_fan_n", 2)));
        assert_eq!(
            smc_numbered_label("CPU performance core 12"),
            Some(("sensor_cpu_performance_core", 12))
        );
        assert_eq!(
            smc_numbered_label("CPU efficiency core 1"),
            Some(("sensor_cpu_efficiency_core", 1))
        );
        assert_eq!(
            smc_numbered_label("CPU super core 4"),
            Some(("sensor_cpu_super_core", 4))
        );
        assert_eq!(smc_numbered_label("GPU 3"), Some(("sensor_gpu_n", 3)));
        // 没有序号或者序号不是数字的，原样显示。
        assert_eq!(smc_numbered_label("GPU MTR Temp Sensor0"), None);
        assert_eq!(smc_numbered_label("Fan"), None);
        assert_eq!(smc_numbered_label("CPU performance core x"), None);
        assert_eq!(smc_numbered_label("PMU tdie0"), None);
    }

    #[test]
    fn every_numbered_sensor_name_has_both_translations() {
        for (key, n) in [
            ("sensor_fan_n", 1),
            ("sensor_cpu_efficiency_core", 1),
            ("sensor_cpu_performance_core", 1),
            ("sensor_cpu_super_core", 1),
            ("sensor_gpu_n", 1),
        ] {
            for locale in ["en", "zh-CN"] {
                let text = t!(key, locale = locale, n = n);
                assert!(
                    text.contains('1') && !text.contains("sensor_"),
                    "{key} is not translated for {locale}: {text}"
                );
            }
        }
    }

    #[test]
    fn rate_scale_follows_the_peak() {
        let kb = RateScale::new(300 * 1024, NetUnit::BytesPerSec);
        assert_eq!(kb.name, "KB/s");
        assert!((kb.value(512 * 1024) - 512.0).abs() < 1e-9);

        let mb = RateScale::new(3 * 1024 * 1024, NetUnit::BytesPerSec);
        assert_eq!(mb.name, "MB/s");
        assert!((mb.value(3 * 1024 * 1024) - 3.0).abs() < 1e-9);

        // 空闲时也要有个单位，别写成 0 B/s。
        assert_eq!(RateScale::new(0, NetUnit::BytesPerSec).name, "KB/s");
        assert_eq!(RateScale::new(0, NetUnit::BitsPerSec).name, "Kbps");
    }

    #[test]
    fn rate_scale_in_bits_is_decimal() {
        // 1 MB/s = 8 Mbps。
        let scale = RateScale::new(1_000_000, NetUnit::BitsPerSec);
        assert_eq!(scale.name, "Mbps");
        assert!((scale.value(1_000_000) - 8.0).abs() < 1e-9);
        assert_eq!(
            RateScale::new(2_000_000_000, NetUnit::BitsPerSec).name,
            "Gbps"
        );
    }

    #[test]
    fn compact_rates_never_exceed_three_digits() {
        let compact = |bytes| rate_compact(bytes, NetUnit::BytesPerSec);
        assert_eq!(compact(512), "512B");
        assert_eq!(compact(999 * 1024), "999K");
        assert_eq!(compact(1000 * 1024), "1.0M");
        assert_eq!(compact(1023 * 1024), "1.0M");
        assert_eq!(compact(99 * 1024 * 1024 + 1024 * 1000), "100M");
        assert_eq!(rate_compact(125_000, NetUnit::BitsPerSec), "1.0M");
    }

    #[test]
    fn axis_labels_keep_few_digits() {
        assert_eq!(axis_label(0.0), "0");
        assert_eq!(axis_label(563.4), "563");
        assert_eq!(axis_label(14.73), "14.7");
        assert_eq!(axis_label(2.236), "2.24");
    }

    #[test]
    fn battery_capacity_reads_in_watt_hours() {
        assert_eq!(watt_hours(76_739), "76.7 Wh");
    }
}

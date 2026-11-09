use super::config::{Metric, Show};
use crate::{
    fmt,
    settings::{NetUnit, Settings},
};
use pc_specs_core::model::{MonitorTick, ProcessInfo, StorageInfo, SystemSnapshot};

/// 一个读数拆成「箭头 / 数字 / 单位」：位图里数字大、单位小，位置紧挨着。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Value {
    pub lead: &'static str,
    pub num: String,
    pub unit: String,
    /// 位置紧张（数字已经三位）或设置里关掉单位时用的短单位。
    pub short: String,
}

impl Value {
    fn new(num: impl Into<String>, unit: impl Into<String>, short: impl Into<String>) -> Self {
        Self {
            lead: "",
            num: num.into(),
            unit: unit.into(),
            short: short.into(),
        }
    }

    fn lead(mut self, lead: &'static str) -> Self {
        self.lead = lead;
        self
    }

    fn missing(lead: &'static str) -> Self {
        Self::new("—", "", "").lead(lead)
    }

    /// 完整的一段文字，托盘菜单、提示和无障碍标签用。
    pub fn plain(&self) -> String {
        format!("{}{}{}", self.lead, self.num, self.unit)
    }

    /// 菜单栏位图里实际画出来的单位。
    pub fn bar_unit(&self, show_units: bool) -> &str {
        let digits = self.num.chars().filter(char::is_ascii_digit).count();
        if show_units && digits < 3 {
            &self.unit
        } else {
            &self.short
        }
    }
}

/// 温度走势的纵轴：20–100 °C，菜单栏、面板里的图都用这一个。
pub const CELSIUS_FLOOR: f64 = 20.;
pub const CELSIUS_SPAN: f64 = 80.;
/// 速率走势的量程下限：网速这类数据空闲时只有一点噪声，别把噪声放大成满格。
pub const RATE_FLOOR: f64 = 1024.;

/// 走势图的纵轴。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    /// 占用率：固定 0–100。
    Percent,
    /// 摄氏度：固定 20–100。
    Celsius,
    /// 速率：按最近的最大值缩放。
    Rate,
}

impl Scale {
    /// 把一个读数放到 0..=1 的纵轴上。`window` 是同一张图里的全部读数，只有 `Rate` 要看它。
    pub fn fraction(self, value: f64, window: &[f64]) -> f32 {
        let unit = match self {
            Self::Percent => value / 100.,
            Self::Celsius => (value - CELSIUS_FLOOR) / CELSIUS_SPAN,
            Self::Rate => value / window.iter().copied().fold(RATE_FLOOR, f64::max),
        };
        unit.clamp(0., 1.) as f32
    }
}

/// 面板里走势图最多保留多少个采样。
pub const HISTORY: usize = 120;

/// 会被长期记录的曲线。顺序就是落盘文件里的列顺序：改动之后旧文件会因为列数对不上被丢弃重来。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Cpu,
    Memory,
    Temperature,
    Gpu,
    Battery,
    NetDown,
    NetUp,
    DiskRead,
    DiskWrite,
}

impl Channel {
    pub const ALL: [Self; 9] = [
        Self::Cpu,
        Self::Memory,
        Self::Temperature,
        Self::Gpu,
        Self::Battery,
        Self::NetDown,
        Self::NetUp,
        Self::DiskRead,
        Self::DiskWrite,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn scale(self) -> Scale {
        match self {
            Self::Temperature => Scale::Celsius,
            Self::NetDown | Self::NetUp | Self::DiskRead | Self::DiskWrite => Scale::Rate,
            _ => Scale::Percent,
        }
    }
}

/// 面板走势图里的一个采样，只留画图要的数，不带单位。
/// 和「实时监控」页的历史分开存：那边暂停、清空、换保留时长，都不该影响菜单栏。
#[derive(Clone, Copy, Debug, Default)]
pub struct Sample {
    /// 采样时刻，Unix 毫秒。
    pub timestamp: u64,
    numbers: [Option<f64>; Metric::ALL.len()],
    pub net_down: u64,
    pub net_up: u64,
    pub disk_read: u64,
    pub disk_write: u64,
}

impl Sample {
    pub fn new(tick: &MonitorTick, snapshot: &SystemSnapshot, settings: &Settings) -> Self {
        Self {
            timestamp: tick.timestamp,
            numbers: Metric::ALL
                .map(|m| reading(m, Show::Auto, Some(tick), snapshot, settings).number),
            net_down: tick.net_rx_bps,
            net_up: tick.net_tx_bps,
            disk_read: tick.disk_read_bps,
            disk_write: tick.disk_write_bps,
        }
    }

    pub fn number(&self, metric: Metric) -> Option<f64> {
        self.numbers[metric.index()]
    }

    /// 某条曲线在这个采样里的值，`None` 表示那一刻没有读数。
    pub fn value(&self, channel: Channel) -> Option<f64> {
        match channel {
            Channel::Cpu => self.number(Metric::Cpu),
            Channel::Memory => self.number(Metric::Memory),
            Channel::Temperature => self.number(Metric::Temperature),
            Channel::Gpu => self.number(Metric::Gpu),
            Channel::Battery => self.number(Metric::Battery),
            Channel::NetDown => Some(self.net_down as f64),
            Channel::NetUp => Some(self.net_up as f64),
            Channel::DiskRead => Some(self.disk_read as f64),
            Channel::DiskWrite => Some(self.disk_write as f64),
        }
    }
}

pub struct Reading {
    pub value: Value,
    /// 第二行，只有同时展示两个数字（上下行、读写）时才有。
    pub second: Option<Value>,
    /// 走势图用的数值，跟展示哪个读数无关：始终是这个指标最有代表性的那条曲线。
    pub number: Option<f64>,
    pub scale: Scale,
}

impl Reading {
    fn percent(number: Option<f64>) -> Self {
        Self {
            value: number.map_or_else(|| Value::missing(""), percent),
            second: None,
            number,
            scale: Scale::Percent,
        }
    }

    pub fn plain(&self) -> String {
        match &self.second {
            Some(second) => format!("{}  {}", self.value.plain(), second.plain()),
            None => self.value.plain(),
        }
    }
}

fn percent(n: f64) -> Value {
    Value::new(format!("{n:.0}"), "%", "")
}

fn degrees(celsius: f32, settings: &Settings) -> Value {
    Value::new(
        format!("{:.0}", fmt::temp_chart_value(celsius, settings.temp_unit)),
        fmt::temp_unit_name(settings.temp_unit),
        "°",
    )
}

/// 转得最快的那把风扇（rpm）。
fn fastest_fan(tick: &MonitorTick) -> Option<f64> {
    tick.other_sensors
        .iter()
        .filter(|r| r.kind == "fan" && r.value.is_finite())
        .map(|r| f64::from(r.value))
        .max_by(f64::total_cmp)
}

fn fan_speed(rpm: f64) -> Value {
    Value::new(format!("{rpm:.0}"), "rpm", "")
}

fn rate(lead: &'static str, bytes_per_sec: u64, settings: &Settings) -> Value {
    let text = fmt::rate_compact(bytes_per_sec, settings.net_unit);
    let split = text
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    let (num, letter) = text.split_at(split);
    let bits = settings.net_unit == NetUnit::BitsPerSec && letter != "b";
    let scale = if bits {
        format!("{letter}b")
    } else {
        letter.to_string()
    };
    Value::new(num, format!("{scale}/s"), scale).lead(lead)
}

fn amount(bytes: u64, settings: &Settings) -> Value {
    let text = fmt::bytes(bytes, settings.size_unit);
    let (num, name) = text.split_once(' ').unwrap_or((&text, ""));
    let letter: String = name.chars().take(1).collect();
    Value::new(num, letter.clone(), letter)
}

fn remaining(secs: u64) -> Value {
    let (hours, minutes) = (secs / 3600, secs % 3600 / 60);
    if hours > 0 {
        Value::new(format!("{hours}:{minutes:02}"), "h", "h")
    } else {
        Value::new(minutes.to_string(), "m", "m")
    }
}

/// 系统卷是用户最关心的容量；没有系统卷时取第一块有效卷，不把重复挂载相加。
pub fn system_volume(snapshot: &SystemSnapshot) -> Option<&StorageInfo> {
    snapshot
        .storages
        .iter()
        .filter(|d| d.total_bytes > 0)
        .min_by_key(|d| {
            let root = d
                .mount_point
                .as_deref()
                .is_some_and(|m| m == "/" || m.eq_ignore_ascii_case("C:\\"));
            u8::from(!root)
        })
}

pub fn reading(
    metric: Metric,
    show: Show,
    tick: Option<&MonitorTick>,
    snapshot: &SystemSnapshot,
    settings: &Settings,
) -> Reading {
    // 数字不可用时一律显示「—」，不能把「没有传感器」画成 0。
    let finite = |n: f64| n.is_finite().then_some(n);
    match metric {
        Metric::Cpu => Reading::percent(tick.and_then(|t| finite(t.cpu_overall as f64))),
        Metric::Gpu => Reading::percent(
            tick.and_then(|t| {
                t.gpus
                    .iter()
                    .filter_map(|g| g.utilization)
                    .max_by(f32::total_cmp)
            })
            .and_then(|n| finite(n as f64)),
        ),
        Metric::Battery => {
            let battery = snapshot.battery.as_ref();
            let mut reading = Reading::percent(battery.and_then(|b| finite(b.percentage as f64)));
            if show == Show::Remaining
                && let Some(secs) =
                    battery.and_then(|b| b.time_to_empty_secs.or(b.time_to_full_secs))
            {
                reading.value = remaining(secs);
            }
            reading
        }
        Metric::Memory => {
            let Some(t) = tick.filter(|t| t.mem_total_bytes > 0) else {
                return Reading::percent(None);
            };
            let mut reading = Reading::percent(finite(fmt::ratio(
                t.mem_used_bytes,
                t.mem_total_bytes,
            ) as f64));
            match show {
                Show::Used => reading.value = amount(t.mem_used_bytes, settings),
                Show::Free => {
                    reading.value =
                        amount(t.mem_total_bytes.saturating_sub(t.mem_used_bytes), settings)
                }
                _ => {}
            }
            reading
        }
        Metric::Storage => {
            if show == Show::Io {
                return match tick {
                    Some(t) => Reading {
                        value: rate("↓", t.disk_read_bps, settings),
                        second: Some(rate("↑", t.disk_write_bps, settings)),
                        number: finite((t.disk_read_bps.saturating_add(t.disk_write_bps)) as f64),
                        scale: Scale::Rate,
                    },
                    None => Reading {
                        value: Value::missing("↓"),
                        second: Some(Value::missing("↑")),
                        number: None,
                        scale: Scale::Rate,
                    },
                };
            }
            let Some(disk) = system_volume(snapshot) else {
                return Reading::percent(None);
            };
            let mut reading =
                Reading::percent(finite(fmt::ratio(disk.used_bytes, disk.total_bytes) as f64));
            match show {
                Show::Used => reading.value = amount(disk.used_bytes, settings),
                Show::Free => {
                    reading.value =
                        amount(disk.total_bytes.saturating_sub(disk.used_bytes), settings)
                }
                _ => {}
            }
            reading
        }
        Metric::Temperature => {
            let celsius = tick.and_then(|t| match show {
                Show::Hottest => t.max_temperature(),
                _ => t.cpu_temperature().or_else(|| t.max_temperature()),
            });
            let number = celsius.and_then(|c| finite(c as f64));
            // 走势和仪表始终是温度；只有菜单栏上那段文字可以换成风扇转速。
            let value = if show == Show::Fan {
                tick.and_then(fastest_fan)
                    .map_or_else(|| Value::missing(""), fan_speed)
            } else {
                number.map_or_else(|| Value::missing(""), |c| degrees(c as f32, settings))
            };
            Reading {
                value,
                second: None,
                number,
                scale: Scale::Celsius,
            }
        }
        Metric::Network => {
            let Some(t) = tick else {
                return Reading {
                    value: Value::missing("↓"),
                    second: (show == Show::Auto).then(|| Value::missing("↑")),
                    number: None,
                    scale: Scale::Rate,
                };
            };
            let down = rate("↓", t.net_rx_bps, settings);
            let up = rate("↑", t.net_tx_bps, settings);
            match show {
                Show::Down => Reading {
                    value: down,
                    second: None,
                    number: Some(t.net_rx_bps as f64),
                    scale: Scale::Rate,
                },
                Show::Up => Reading {
                    value: up,
                    second: None,
                    number: Some(t.net_tx_bps as f64),
                    scale: Scale::Rate,
                },
                _ => Reading {
                    value: down,
                    second: Some(up),
                    number: Some(t.net_rx_bps as f64),
                    scale: Scale::Rate,
                },
            }
        }
    }
}

/// 同一版式下可能出现的最宽读数（第一行，第二行）。位图宽度按它固定，数值跳动时
/// 菜单栏上的其它项不会跟着左右抖动。
#[cfg(any(target_os = "macos", test))]
pub fn widest(metric: Metric, show: Show, settings: &Settings) -> Vec<(Value, Option<Value>)> {
    // 每一档单位里最宽的两种读数：数字两位时带 `/s`（`9.0M/s`），三位时不带（`888M`）。
    let rates = |lines: [&'static str; 2], both: bool| -> Vec<(Value, Option<Value>)> {
        let units: [f64; 3] = match settings.net_unit {
            NetUnit::BytesPerSec => [1024., 1024. * 1024., 1024. * 1024. * 1024.],
            NetUnit::BitsPerSec => [125., 125_000., 125_000_000.],
        };
        units
            .into_iter()
            .flat_map(|unit| [9., 888.].map(|n| (n * unit) as u64))
            .map(|bps| {
                (
                    rate(lines[0], bps, settings),
                    both.then(|| rate(lines[1], bps, settings)),
                )
            })
            .collect()
    };
    match (metric, show) {
        (Metric::Network, Show::Down) => rates(["↓", "↓"], false),
        (Metric::Network, Show::Up) => rates(["↑", "↑"], false),
        (Metric::Network, _) | (Metric::Storage, Show::Io) => rates(["↓", "↑"], true),
        (Metric::Temperature, Show::Fan) => vec![(fan_speed(8888.), None)],
        (Metric::Temperature, _) => vec![
            (degrees(88., settings), None),
            (degrees(100., settings), None),
        ],
        // 容量的三种位数（`9.00`、`99.0`、`888`）各档单位都试一遍，`8.88M` 这种最宽。
        (Metric::Memory | Metric::Storage, Show::Used | Show::Free) => {
            use crate::settings::SizeUnit;
            let base = match settings.size_unit {
                SizeUnit::Binary => 1024_f64,
                SizeUnit::Decimal => 1000_f64,
            };
            (2..=4)
                .flat_map(|power| [9., 99., 888.].map(|n| (n * base.powi(power)) as u64))
                .map(|bytes| (amount(bytes, settings), None))
                .collect()
        }
        (Metric::Battery, Show::Remaining) => vec![
            (remaining(8 * 3600 + 59 * 60), None),
            (remaining(59 * 60), None),
        ],
        _ => vec![(percent(88.), None), (percent(100.), None)],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessSort {
    Cpu,
    Memory,
    Io,
    /// 网络流量：下行加上行，只列出此刻有流量的进程。
    Net,
}

impl ProcessSort {
    /// 一行里有两个数（读 / 写、下行 / 上行）的排行。
    pub fn is_pair(self) -> bool {
        matches!(self, Self::Io | Self::Net)
    }
}

/// 进程此刻的网络流量（下行加上行）。
pub fn net_traffic(p: &ProcessInfo) -> u64 {
    p.net_rx_bps.saturating_add(p.net_tx_bps)
}

pub fn ranked_processes(
    processes: &[ProcessInfo],
    sort: ProcessSort,
    descending: bool,
    limit: usize,
) -> Vec<&ProcessInfo> {
    let mut rows: Vec<_> = processes
        .iter()
        // 流量排行只看正在收发数据的进程，一长串零没有意义。
        .filter(|p| sort != ProcessSort::Net || net_traffic(p) > 0)
        .collect();
    rows.sort_unstable_by(|a, b| {
        let order = match sort {
            ProcessSort::Cpu => a.cpu_percent.total_cmp(&b.cpu_percent),
            ProcessSort::Memory => a.mem_bytes.cmp(&b.mem_bytes),
            ProcessSort::Io => a
                .read_bps
                .saturating_add(a.write_bps)
                .cmp(&b.read_bps.saturating_add(b.write_bps)),
            ProcessSort::Net => net_traffic(a).cmp(&net_traffic(b)),
        };
        (if descending { order.reverse() } else { order }).then_with(|| a.pid.cmp(&b.pid))
    });
    rows.truncate(limit);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_and_io_panels_rank_actual_resource_usage() {
        let rows = vec![
            ProcessInfo {
                pid: 3,
                mem_bytes: 200,
                cpu_percent: 2.,
                read_bps: 100,
                ..Default::default()
            },
            ProcessInfo {
                pid: 1,
                mem_bytes: 900,
                cpu_percent: 10.,
                write_bps: 80,
                ..Default::default()
            },
            ProcessInfo {
                pid: 2,
                mem_bytes: 900,
                cpu_percent: 30.,
                read_bps: 90,
                write_bps: 50,
                ..Default::default()
            },
        ];
        let pids = |sort, desc, limit| {
            ranked_processes(&rows, sort, desc, limit)
                .iter()
                .map(|p| p.pid)
                .collect::<Vec<_>>()
        };
        assert_eq!(pids(ProcessSort::Memory, true, 2), [1, 2]);
        assert_eq!(pids(ProcessSort::Cpu, true, 3), [2, 1, 3]);
        assert_eq!(pids(ProcessSort::Io, true, 3), [2, 3, 1]);
        assert_eq!(pids(ProcessSort::Memory, false, 3), [3, 1, 2]);
    }

    #[test]
    fn network_panel_ranks_active_processes_by_total_traffic() {
        let process = |pid, rx, tx| ProcessInfo {
            pid,
            net_rx_bps: rx,
            net_tx_bps: tx,
            ..Default::default()
        };
        // 3 号下载最多但总量不是最大：排序看的是上下行之和；5 号没有流量，不进榜；2 号和 6 号一样大，按 pid。
        let rows = vec![
            process(3, 900, 0),
            process(1, 100, 1000),
            process(6, 1000, 0),
            process(2, 500, 500),
            process(5, 0, 0),
            process(4, 0, 50),
        ];
        let pids = |limit| {
            ranked_processes(&rows, ProcessSort::Net, true, limit)
                .iter()
                .map(|p| p.pid)
                .collect::<Vec<_>>()
        };
        assert_eq!(pids(10), [1, 2, 6, 3, 4]);
        assert_eq!(pids(2), [1, 2]);
    }

    #[test]
    fn unavailable_hardware_is_not_reported_as_zero() {
        let settings = Settings::default();
        let snapshot = SystemSnapshot::default();
        for metric in Metric::ALL {
            for &show in metric.shows() {
                let reading = reading(metric, show, None, &snapshot, &settings);
                assert_eq!(reading.value.num, "—");
                assert_eq!(reading.number, None);
            }
        }
    }

    #[test]
    fn each_choice_reads_its_own_number() {
        let settings = Settings::default();
        let snapshot = SystemSnapshot::default();
        let tick = MonitorTick {
            mem_used_bytes: 45 << 30,
            mem_total_bytes: 100 << 30,
            net_rx_bps: 393 << 10,
            net_tx_bps: 3 << 10,
            ..Default::default()
        };
        let text = |metric, show| reading(metric, show, Some(&tick), &snapshot, &settings).plain();
        assert_eq!(text(Metric::Memory, Show::Auto), "45%");
        assert_eq!(text(Metric::Memory, Show::Used), "45.0G");
        assert_eq!(text(Metric::Memory, Show::Free), "55.0G");
        assert_eq!(text(Metric::Network, Show::Auto), "↓393K/s  ↑3.0K/s");
        assert_eq!(text(Metric::Network, Show::Down), "↓393K/s");
        assert_eq!(text(Metric::Network, Show::Up), "↑3.0K/s");
    }

    #[test]
    fn units_shrink_when_space_is_tight() {
        let wide = Value::new("22", "%", "");
        assert_eq!(wide.bar_unit(true), "%");
        assert_eq!(wide.bar_unit(false), "");
        assert_eq!(Value::new("100", "%", "").bar_unit(true), "");
        assert_eq!(Value::new("393", "K/s", "K").bar_unit(true), "K");
    }
}

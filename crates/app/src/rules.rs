//! 规则：某个读数越过设定的线、并且持续了一段时间，就发通知、推 Webhook 或运行一条命令。
//!
//! 本模块只有「读什么、怎么判断、怎么措辞」这些不碰界面的逻辑，可以单独测试；
//! 结果怎么送到用户手里（通知 / Webhook / 命令）在 `state.rs`，设置界面在 `pages/rules.rs`。

use crate::fmt;
use crate::menubar::data::system_volume;
use crate::settings::{NetUnit, Settings, SizeUnit};
use pc_specs_core::model::{MonitorTick, ProcessInfo, SystemSnapshot};
use pc_specs_core::netprobe::{LatencySample, TargetKind};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 规则条数上限：够用就好，也免得设置文件被越写越大。
pub const MAX_RULES: usize = 40;
/// 「最近触发」保留的条数。
pub const MAX_EVENTS: usize = 50;
const MAX_COMMAND_LEN: usize = 1000;
const MAX_URL_LEN: usize = 2000;

/// 条件要连续成立多久才触发（秒）、触发之后隔多久才会再触发（分钟）的可选档位。
pub const HOLD_CHOICES: [u32; 9] = [0, 5, 10, 30, 60, 120, 300, 600, 1800];
pub const COOLDOWN_CHOICES: [u32; 8] = [1, 5, 10, 15, 30, 60, 360, 1440];

// ---------------------------------------------------------------------------
// 规则本身

/// 能拿来设规则的读数。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    CpuUsage,
    MemoryUsage,
    MemoryPressure,
    SwapUsage,
    /// 最满的那块磁盘的占用率。
    DiskUsage,
    /// 系统盘的剩余空间。
    DiskFree,
    /// 所有温度传感器里最高的一个。
    Temperature,
    /// 转得最快的风扇。
    FanSpeed,
    GpuUsage,
    BatteryLevel,
    /// 事件：电脑改用电池供电。
    OnBattery,
    NetDownload,
    NetUpload,
    /// 到公网的往返延迟。
    NetLatency,
    /// 事件：公网连不上了。
    Offline,
    /// CPU 占用最高的那个进程。
    ProcessCpu,
}

impl Metric {
    pub const ALL: [Metric; 16] = [
        Metric::CpuUsage,
        Metric::MemoryUsage,
        Metric::MemoryPressure,
        Metric::SwapUsage,
        Metric::DiskUsage,
        Metric::DiskFree,
        Metric::Temperature,
        Metric::FanSpeed,
        Metric::GpuUsage,
        Metric::BatteryLevel,
        Metric::OnBattery,
        Metric::NetDownload,
        Metric::NetUpload,
        Metric::NetLatency,
        Metric::Offline,
        Metric::ProcessCpu,
    ];

    /// 稳定的英文标识，Webhook 的 `metric` 字段和命令的环境变量里用它。
    pub fn key(self) -> &'static str {
        match self {
            Metric::CpuUsage => "cpu_usage",
            Metric::MemoryUsage => "memory_usage",
            Metric::MemoryPressure => "memory_pressure",
            Metric::SwapUsage => "swap_usage",
            Metric::DiskUsage => "disk_usage",
            Metric::DiskFree => "disk_free",
            Metric::Temperature => "temperature",
            Metric::FanSpeed => "fan_speed",
            Metric::GpuUsage => "gpu_usage",
            Metric::BatteryLevel => "battery_level",
            Metric::OnBattery => "on_battery",
            Metric::NetDownload => "net_download",
            Metric::NetUpload => "net_upload",
            Metric::NetLatency => "net_latency",
            Metric::Offline => "offline",
            Metric::ProcessCpu => "process_cpu",
        }
    }

    pub fn name(self) -> String {
        match self {
            Metric::CpuUsage => t!("rule_metric_cpu_usage"),
            Metric::MemoryUsage => t!("rule_metric_memory_usage"),
            Metric::MemoryPressure => t!("rule_metric_memory_pressure"),
            Metric::SwapUsage => t!("rule_metric_swap_usage"),
            Metric::DiskUsage => t!("rule_metric_disk_usage"),
            Metric::DiskFree => t!("rule_metric_disk_free"),
            Metric::Temperature => t!("rule_metric_temperature"),
            Metric::FanSpeed => t!("rule_metric_fan_speed"),
            Metric::GpuUsage => t!("rule_metric_gpu_usage"),
            Metric::BatteryLevel => t!("rule_metric_battery_level"),
            Metric::OnBattery => t!("rule_metric_on_battery"),
            Metric::NetDownload => t!("rule_metric_net_download"),
            Metric::NetUpload => t!("rule_metric_net_upload"),
            Metric::NetLatency => t!("rule_metric_net_latency"),
            Metric::Offline => t!("rule_metric_offline"),
            Metric::ProcessCpu => t!("rule_metric_process_cpu"),
        }
        .into_owned()
    }

    /// 事件只关心「发生了没有」：没有比较方向和阈值，条件由不成立变成成立才触发一次。
    pub fn is_event(self) -> bool {
        matches!(self, Metric::OnBattery | Metric::Offline)
    }

    /// 阈值可选的档位（用户看得懂的整数）。
    pub fn presets(self) -> &'static [f64] {
        match self {
            Metric::CpuUsage | Metric::MemoryUsage | Metric::SwapUsage | Metric::GpuUsage => {
                &[50., 60., 70., 80., 85., 90., 95., 100.]
            }
            Metric::MemoryPressure => &[30., 40., 50., 60., 70., 80., 90.],
            Metric::DiskUsage => &[70., 80., 85., 90., 95., 98.],
            Metric::DiskFree => &[1., 2., 5., 10., 20., 50., 100., 200., 500.],
            Metric::Temperature => &[60., 70., 75., 80., 85., 88., 90., 95., 100.],
            Metric::FanSpeed => &[2000., 3000., 4000., 5000., 6000., 7000.],
            Metric::BatteryLevel => &[5., 10., 15., 20., 30., 40., 50.],
            Metric::NetDownload | Metric::NetUpload => {
                &[1., 2., 5., 10., 20., 50., 100., 200., 500.]
            }
            Metric::NetLatency => &[20., 50., 100., 200., 300., 500., 1000.],
            Metric::ProcessCpu => &[50., 100., 150., 200., 300., 400., 800.],
            Metric::OnBattery | Metric::Offline => &[],
        }
    }

    pub fn default_threshold(self) -> f64 {
        match self {
            Metric::CpuUsage | Metric::MemoryUsage | Metric::DiskUsage => 90.,
            Metric::MemoryPressure => 70.,
            Metric::SwapUsage => 80.,
            Metric::DiskFree => 10.,
            Metric::Temperature => 90.,
            Metric::FanSpeed => 5000.,
            Metric::GpuUsage => 95.,
            Metric::BatteryLevel => 20.,
            Metric::NetDownload | Metric::NetUpload => 50.,
            Metric::NetLatency => 200.,
            Metric::ProcessCpu => 100.,
            Metric::OnBattery | Metric::Offline => 0.,
        }
    }

    /// 剩余量类的读数「低于」才是坏事，其余「达到」才是。
    pub fn default_compare(self) -> Compare {
        match self {
            Metric::DiskFree | Metric::BatteryLevel => Compare::Below,
            _ => Compare::Above,
        }
    }

    /// 这个读数要不要读传感器（温度、风扇）。传感器很重，没人看的时候不读。
    pub fn needs_sensors(self) -> bool {
        matches!(self, Metric::Temperature | Metric::FanSpeed)
    }

    /// 这个读数要不要采进程列表。
    pub fn needs_processes(self) -> bool {
        matches!(self, Metric::ProcessCpu)
    }

    /// 这个读数要不要探测网络延迟（每隔几秒向网关、DNS 和公网各发一次连接）。
    pub fn needs_network(self) -> bool {
        matches!(self, Metric::NetLatency | Metric::Offline)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Compare {
    /// 达到或超过阈值。
    #[default]
    Above,
    /// 低于阈值。
    Below,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    /// 在整份设置里唯一，增删规则时不重排。
    pub id: u32,
    pub enabled: bool,
    pub metric: Metric,
    pub compare: Compare,
    pub threshold: f64,
    /// 条件要连续成立多少秒才触发；0 表示一成立就触发。
    pub for_secs: u32,
    /// 触发之后至少隔多少分钟才会再触发。
    pub cooldown_mins: u32,
    /// 发系统通知（主窗口开着时是窗口里的提示）。
    pub notify: bool,
    /// 向设置里的 Webhook 地址推送。
    pub webhook: bool,
    /// 触发时运行的命令行；空表示不运行。
    pub command: String,
}

impl Default for Rule {
    fn default() -> Self {
        Self::new(0, Metric::CpuUsage)
    }
}

impl Rule {
    /// 一条新规则：阈值和方向按读数的常见用法给出，只发通知。
    pub fn new(id: u32, metric: Metric) -> Self {
        Self {
            id,
            enabled: true,
            metric,
            compare: metric.default_compare(),
            threshold: metric.default_threshold(),
            for_secs: 60,
            cooldown_mins: 15,
            notify: true,
            webhook: false,
            command: String::new(),
        }
    }

    /// 读数 `value` 现在算不算触发条件。
    pub fn holds(&self, value: f64) -> bool {
        if self.metric.is_event() {
            return value >= 0.5;
        }
        match self.compare {
            Compare::Above => value >= self.threshold,
            Compare::Below => value < self.threshold,
        }
    }

    pub fn has_command(&self) -> bool {
        !self.command.trim().is_empty()
    }

    /// 触发之后做的事，给列表里的次要说明用。
    pub fn actions_text(&self) -> String {
        let mut actions = Vec::new();
        if self.notify {
            actions.push(t!("rule_action_notify").into_owned());
        }
        if self.webhook {
            actions.push(t!("rule_action_webhook").into_owned());
        }
        if self.has_command() {
            actions.push(t!("rule_action_command").into_owned());
        }
        if actions.is_empty() {
            return t!("rule_action_none").into_owned();
        }
        actions.join(" · ")
    }

    /// 一句话概括这条规则，比如「CPU 占用达到 90%，持续 1 分钟」。
    pub fn summary(&self, settings: &Settings) -> String {
        let name = self.metric.name();
        let when = if self.metric.is_event() {
            name
        } else {
            let limit = format_value(self.metric, self.threshold, settings);
            match self.compare {
                Compare::Above => t!("rule_summary_above", metric = name, limit = limit),
                Compare::Below => t!("rule_summary_below", metric = name, limit = limit),
            }
            .into_owned()
        };
        if self.for_secs == 0 {
            when
        } else {
            t!(
                "rule_summary_for",
                when = when,
                duration = duration_text(self.for_secs)
            )
            .into_owned()
        }
    }
}

/// 第一次运行（以及「恢复默认」）时的规则：旧版内置的那四项告警。温度的阈值放宽到了 100 度：
/// 处理器核心满载时九十几度是正常的，一百度上下才是快要降频了。
pub fn defaults() -> Vec<Rule> {
    let rule = |id, metric, threshold| Rule {
        threshold,
        for_secs: 5,
        cooldown_mins: 5,
        ..Rule::new(id, metric)
    };
    vec![
        rule(1, Metric::CpuUsage, 95.),
        rule(2, Metric::MemoryUsage, 95.),
        rule(3, Metric::DiskUsage, 95.),
        rule(4, Metric::Temperature, 100.),
    ]
}

/// 手工改坏的规则收敛回合法状态：编号唯一、数值有限、长度有上限。
pub fn sanitize(rules: &mut Vec<Rule>) {
    rules.truncate(MAX_RULES);
    let mut taken = HashSet::new();
    let mut next = rules.iter().map(|rule| rule.id).max().unwrap_or(0);
    for rule in rules.iter_mut() {
        if !taken.insert(rule.id) {
            next = next.saturating_add(1);
            rule.id = next;
            taken.insert(next);
        }
        if !rule.threshold.is_finite() {
            rule.threshold = rule.metric.default_threshold();
        }
        rule.for_secs = rule.for_secs.min(86_400);
        // 数值类规则的冷却为 0 会在条件成立期间每个采样都触发一次。
        rule.cooldown_mins = rule.cooldown_mins.clamp(1, 7 * 24 * 60);
        if rule.command.chars().count() > MAX_COMMAND_LEN {
            rule.command = rule.command.chars().take(MAX_COMMAND_LEN).collect();
        }
    }
}

/// 新规则的编号。
pub fn next_id(rules: &[Rule]) -> u32 {
    rules
        .iter()
        .map(|rule| rule.id)
        .max()
        .map_or(1, |id| id.saturating_add(1))
}

pub fn clean_url(url: &str) -> String {
    url.trim().chars().take(MAX_URL_LEN).collect()
}

/// 启用的规则里有没有要读传感器（温度、风扇）的。总开关关着就没有。
pub fn needs_sensors(settings: &Settings) -> bool {
    settings.alerts
        && settings
            .rules
            .iter()
            .any(|rule| rule.enabled && rule.metric.needs_sensors())
}

/// 启用的规则里有没有要采进程列表的。
pub fn needs_processes(settings: &Settings) -> bool {
    settings.alerts
        && settings
            .rules
            .iter()
            .any(|rule| rule.enabled && rule.metric.needs_processes())
}

/// 启用的规则里有没有要探测网络延迟的。
pub fn needs_network_probe(settings: &Settings) -> bool {
    settings.alerts
        && settings
            .rules
            .iter()
            .any(|rule| rule.enabled && rule.metric.needs_network())
}

// ---------------------------------------------------------------------------
// Webhook

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookFormat {
    /// 通用 JSON；同时带有 `text` 和 `content` 字段，Slack、Discord、Mattermost 的入站地址可以直接用。
    #[default]
    Generic,
    Feishu,
    DingTalk,
}

/// 推送给 Webhook 的 JSON 正文。
pub fn webhook_body(
    format: WebhookFormat,
    firing: &Firing,
    summary: &str,
    message: &str,
    host: &str,
    timestamp_ms: u64,
) -> String {
    let text = if host.is_empty() {
        message.to_string()
    } else {
        format!("{host}: {message}")
    };
    let body = match format {
        WebhookFormat::Generic => serde_json::json!({
            "event": "rule_triggered",
            "rule": summary,
            "metric": firing.rule.metric.key(),
            "value": firing.reading.value,
            "threshold": firing.rule.threshold,
            "detail": firing.reading.detail,
            "message": message,
            "host": host,
            "timestamp_ms": timestamp_ms,
            "text": text,
            "content": text,
        }),
        WebhookFormat::Feishu => serde_json::json!({
            "msg_type": "text",
            "content": { "text": text },
        }),
        WebhookFormat::DingTalk => serde_json::json!({
            "msgtype": "text",
            "text": { "content": text },
        }),
    };
    body.to_string()
}

// ---------------------------------------------------------------------------
// 读数

/// 评估规则时能看到的全部数据。
pub struct Inputs<'a> {
    pub settings: &'a Settings,
    pub tick: Option<&'a MonitorTick>,
    pub snapshot: &'a SystemSnapshot,
    /// 进程列表；采样没在跑的时候传空，免得拿旧数据下结论。
    pub processes: &'a [ProcessInfo],
    /// 最近一次网络探测；探测没在跑、或者结果已经旧了就是 `None`。
    pub latency: Option<&'a LatencySample>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub value: f64,
    /// 读数来自哪里（哪块磁盘、哪个进程、哪个风扇），消息里一并说出来。
    pub detail: Option<String>,
}

fn percent(used: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| used as f64 / total as f64 * 100.0)
}

/// 容量类阈值的 1 GB 是多少字节：跟着界面上显示容量的进制走，设的数和看到的数一致。
fn gigabyte(unit: SizeUnit) -> f64 {
    match unit {
        SizeUnit::Binary => 1024.0 * 1024.0 * 1024.0,
        SizeUnit::Decimal => 1_000_000_000.0,
    }
}

/// 网速阈值的数值：字节模式是 MB/s（1024 进制），比特模式是 Mbps（1000 进制），和 `fmt::rate` 一致。
fn net_value(bytes_per_sec: u64, unit: NetUnit) -> f64 {
    match unit {
        NetUnit::BytesPerSec => bytes_per_sec as f64 / (1024.0 * 1024.0),
        NetUnit::BitsPerSec => bytes_per_sec as f64 * 8.0 / 1_000_000.0,
    }
}

/// 读出一个指标现在的值；当前拿不到（没有电池、传感器没数据……）时是 `None`。
pub fn read(metric: Metric, inputs: &Inputs) -> Option<Reading> {
    let tick = inputs.tick;
    let plain = |value: f64| {
        Some(Reading {
            value,
            detail: None,
        })
    };
    let reading = match metric {
        Metric::CpuUsage => plain(f64::from(tick?.cpu_overall)),
        Metric::MemoryUsage => {
            let tick = tick?;
            plain(percent(tick.mem_used_bytes, tick.mem_total_bytes)?)
        }
        Metric::MemoryPressure => plain(f64::from(tick?.memory.as_ref()?.pressure_percent?)),
        Metric::SwapUsage => {
            let tick = tick?;
            plain(percent(tick.swap_used_bytes, tick.swap_total_bytes)?)
        }
        Metric::DiskUsage => {
            // 只看最满的那一块，多卷时不会一次响好几条。
            let (disk, value) = inputs
                .snapshot
                .storages
                .iter()
                .filter_map(|disk| Some((disk, percent(disk.used_bytes, disk.total_bytes)?)))
                .max_by(|a, b| a.1.total_cmp(&b.1))?;
            Some(Reading {
                value,
                detail: Some(
                    disk.mount_point
                        .clone()
                        .unwrap_or_else(|| disk.name.clone()),
                ),
            })
        }
        Metric::DiskFree => {
            let disk = system_volume(inputs.snapshot)?;
            let free = disk.total_bytes.saturating_sub(disk.used_bytes);
            Some(Reading {
                value: free as f64 / gigabyte(inputs.settings.size_unit),
                detail: Some(
                    disk.mount_point
                        .clone()
                        .unwrap_or_else(|| disk.name.clone()),
                ),
            })
        }
        Metric::Temperature => plain(f64::from(tick?.max_temperature()?)),
        Metric::FanSpeed => {
            let fan = tick?
                .other_sensors
                .iter()
                .filter(|sensor| sensor.kind == "fan" && sensor.value.is_finite())
                .max_by(|a, b| a.value.total_cmp(&b.value))?;
            Some(Reading {
                value: f64::from(fan.value),
                detail: Some(fmt::sensor_label(fan)),
            })
        }
        Metric::GpuUsage => {
            let (gpu, value) = tick?
                .gpus
                .iter()
                .filter_map(|gpu| Some((gpu, gpu.utilization?)))
                .max_by(|a, b| a.1.total_cmp(&b.1))?;
            Some(Reading {
                value: f64::from(value),
                detail: Some(gpu.name.clone()),
            })
        }
        Metric::BatteryLevel => plain(f64::from(inputs.snapshot.battery.as_ref()?.percentage)),
        Metric::OnBattery => {
            let battery = inputs.snapshot.battery.as_ref()?;
            plain(f64::from(u8::from(
                battery.state.eq_ignore_ascii_case("discharging"),
            )))
        }
        Metric::NetDownload => plain(net_value(tick?.net_rx_bps, inputs.settings.net_unit)),
        Metric::NetUpload => plain(net_value(tick?.net_tx_bps, inputs.settings.net_unit)),
        Metric::NetLatency => {
            // 丢了包没有「延迟」可言，那是 `Offline` 的事：这里只在有回应时才有读数。
            let reading = inputs.latency?.reading(TargetKind::Internet)?;
            Some(Reading {
                value: f64::from(reading.rtt_ms?),
                detail: Some(reading.address.clone()),
            })
        }
        Metric::Offline => plain(f64::from(u8::from(!inputs.latency?.online()?))),
        Metric::ProcessCpu => {
            let process = inputs
                .processes
                .iter()
                .filter(|process| process.cpu_percent.is_finite())
                .max_by(|a, b| a.cpu_percent.total_cmp(&b.cpu_percent))?;
            Some(Reading {
                value: f64::from(process.cpu_percent),
                detail: Some(process.name.clone()),
            })
        }
    }?;
    reading.value.is_finite().then_some(reading)
}

// ---------------------------------------------------------------------------
// 引擎

/// 一次触发：哪条规则、当时读到什么。
#[derive(Clone, Debug)]
pub struct Firing {
    pub rule: Rule,
    pub reading: Reading,
}

#[derive(Default)]
struct RuleState {
    /// 评估过至少一次。
    seen: bool,
    /// 条件开始连续成立的时刻。
    since: Option<Instant>,
    last_fired: Option<Instant>,
    /// 事件类规则：条件不成立的状态已经被看到过，下一次由不成立变成立才算「发生」。
    armed: bool,
    /// 条件此刻是否成立（不管有没有触发）。
    active: bool,
}

/// 逐条跟踪规则的状态。时间由调用方传入，测试里不必真等。
#[derive(Default)]
pub struct Engine {
    states: HashMap<u32, RuleState>,
}

impl Engine {
    /// 评估所有启用的规则，返回这一次该触发的。
    ///
    /// - 数值类：条件连续成立达到 `for_secs` 就触发，之后每隔 `cooldown_mins` 在条件仍成立时再触发。
    /// - 事件类：只在条件由不成立变成成立时触发；程序启动时已经成立的不算。
    /// - 读数暂时拿不到按「不成立」处理，计时从头来过。
    pub fn evaluate(&mut self, rules: &[Rule], inputs: &Inputs, now: Instant) -> Vec<Firing> {
        // 删除或停用的规则丢掉状态，重新启用时从头计时。
        self.states
            .retain(|id, _| rules.iter().any(|rule| rule.enabled && rule.id == *id));
        let mut fired = Vec::new();
        for rule in rules.iter().filter(|rule| rule.enabled) {
            let reading = read(rule.metric, inputs);
            let holds = reading.as_ref().map(|reading| rule.holds(reading.value));
            let state = self.states.entry(rule.id).or_default();
            if !state.seen {
                state.seen = true;
                state.armed = holds != Some(true);
            }
            let Some(true) = holds else {
                state.since = None;
                state.active = false;
                if holds == Some(false) {
                    state.armed = true;
                }
                continue;
            };
            state.active = true;
            let since = *state.since.get_or_insert(now);
            let held = now.saturating_duration_since(since)
                >= Duration::from_secs(u64::from(rule.for_secs));
            let cooled = state.last_fired.is_none_or(|at| {
                now.saturating_duration_since(at)
                    >= Duration::from_secs(u64::from(rule.cooldown_mins) * 60)
            });
            let edge = !rule.metric.is_event() || state.armed;
            if held
                && cooled
                && edge
                && let Some(reading) = reading
            {
                state.last_fired = Some(now);
                state.armed = false;
                fired.push(Firing {
                    rule: rule.clone(),
                    reading,
                });
            }
        }
        fired
    }

    /// 这条规则的条件此刻是否成立（界面上标「进行中」用）。
    pub fn is_active(&self, id: u32) -> bool {
        self.states.get(&id).is_some_and(|state| state.active)
    }

    /// 总开关关掉时调用：再打开从头计时，不会因为旧状态立刻触发。
    pub fn reset(&mut self) {
        self.states.clear();
    }
}

// ---------------------------------------------------------------------------
// 措辞

/// 把读数或阈值写成带单位的文字。
pub fn format_value(metric: Metric, value: f64, settings: &Settings) -> String {
    // 整数不带小数，其余留一位。
    let number = |value: f64| {
        if (value - value.round()).abs() < 0.05 {
            format!("{value:.0}")
        } else {
            format!("{value:.1}")
        }
    };
    match metric {
        Metric::Temperature => fmt::temp(value as f32, settings.temp_unit),
        Metric::FanSpeed => format!("{value:.0} RPM"),
        Metric::DiskFree => {
            let unit = match settings.size_unit {
                SizeUnit::Binary => "GiB",
                SizeUnit::Decimal => "GB",
            };
            format!("{} {unit}", number(value))
        }
        Metric::NetDownload | Metric::NetUpload => {
            let unit = match settings.net_unit {
                NetUnit::BytesPerSec => "MB/s",
                NetUnit::BitsPerSec => "Mbps",
            };
            format!("{} {unit}", number(value))
        }
        Metric::NetLatency => format!("{} ms", number(value)),
        Metric::OnBattery | Metric::Offline => String::new(),
        _ => format!("{value:.0}%"),
    }
}

/// 「立即」「30 秒」「5 分钟」「6 小时」。
pub fn duration_text(secs: u32) -> String {
    if secs == 0 {
        t!("rule_immediately").into_owned()
    } else if secs.is_multiple_of(3600) {
        t!("rule_hours", n = secs / 3600).into_owned()
    } else if secs.is_multiple_of(60) {
        t!("rule_mins", n = secs / 60).into_owned()
    } else {
        t!("rule_secs", n = secs).into_owned()
    }
}

/// 触发时展示给用户的消息，比如「CPU 占用为 97%」。
pub fn message(rule: &Rule, reading: &Reading, settings: &Settings) -> String {
    if rule.metric.is_event() {
        return match rule.metric {
            Metric::Offline => t!("rule_message_offline"),
            _ => t!("rule_message_on_battery"),
        }
        .into_owned();
    }
    let metric = rule.metric.name();
    let value = format_value(rule.metric, reading.value, settings);
    match &reading.detail {
        Some(detail) => t!(
            "rule_message_detail",
            metric = metric,
            value = value,
            detail = detail
        ),
        None => t!("rule_message", metric = metric, value = value),
    }
    .into_owned()
}

// ---------------------------------------------------------------------------
// 触发记录

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleEvent {
    pub timestamp_ms: u64,
    pub rule_id: u32,
    pub message: String,
}

fn events_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("pc-specs").join("rule_events.json"))
}

/// 读取上次保存的触发记录；文件不存在或损坏就当没有。
pub fn load_events() -> VecDeque<RuleEvent> {
    let events: Vec<RuleEvent> = events_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let skip = events.len().saturating_sub(MAX_EVENTS);
    events.into_iter().skip(skip).collect()
}

/// 写入失败只记日志：丢一份历史记录不该打断使用。
pub fn save_events(events: &VecDeque<RuleEvent>) {
    let Some(path) = events_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("json.tmp");
    let result = serde_json::to_string(events)
        .map_err(std::io::Error::other)
        .and_then(|text| std::fs::write(&tmp, text))
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = result {
        tracing::warn!("failed to save the rule log: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct World {
        settings: Settings,
        tick: MonitorTick,
        snapshot: SystemSnapshot,
        latency: Option<LatencySample>,
    }

    impl World {
        fn new() -> Self {
            Self {
                settings: Settings::default(),
                tick: MonitorTick::default(),
                snapshot: SystemSnapshot::default(),
                latency: None,
            }
        }

        fn inputs(&self) -> Inputs<'_> {
            Inputs {
                settings: &self.settings,
                tick: Some(&self.tick),
                snapshot: &self.snapshot,
                processes: &[],
                latency: self.latency.as_ref(),
            }
        }

        /// 公网这一项有回应（给了延迟）或者没有回应（`None`）的一次探测。
        fn internet(&mut self, rtt_ms: Option<f32>) {
            self.latency = Some(LatencySample {
                timestamp_ms: 0,
                readings: vec![
                    pc_specs_core::netprobe::Reading {
                        kind: TargetKind::Gateway,
                        address: "192.168.1.1".into(),
                        rtt_ms: Some(3.0),
                    },
                    pc_specs_core::netprobe::Reading {
                        kind: TargetKind::Internet,
                        address: "223.5.5.5".into(),
                        rtt_ms,
                    },
                ],
            });
        }

        fn on_battery(&mut self, on: bool) {
            let battery = self.snapshot.battery.get_or_insert_with(Default::default);
            battery.state = if on { "discharging" } else { "charging" }.into();
        }
    }

    fn rule(metric: Metric, threshold: f64, for_secs: u32, cooldown_mins: u32) -> Rule {
        Rule {
            threshold,
            for_secs,
            cooldown_mins,
            ..Rule::new(1, metric)
        }
    }

    fn at(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    #[test]
    fn fires_once_the_condition_has_held_long_enough() {
        let mut world = World::new();
        world.tick.cpu_overall = 97.0;
        let rules = [rule(Metric::CpuUsage, 95., 5, 5)];
        let mut engine = Engine::default();
        let base = Instant::now();
        for secs in 0..5 {
            assert!(
                engine
                    .evaluate(&rules, &world.inputs(), at(base, secs))
                    .is_empty()
            );
        }
        assert!(engine.is_active(1));
        let fired = engine.evaluate(&rules, &world.inputs(), at(base, 5));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].reading.value, f64::from(97.0_f32));
    }

    #[test]
    fn a_dip_below_the_line_restarts_the_timer() {
        let mut world = World::new();
        let rules = [rule(Metric::CpuUsage, 95., 5, 5)];
        let mut engine = Engine::default();
        let base = Instant::now();
        world.tick.cpu_overall = 99.0;
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 0))
                .is_empty()
        );
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 4))
                .is_empty()
        );
        world.tick.cpu_overall = 10.0;
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 5))
                .is_empty()
        );
        assert!(!engine.is_active(1));
        world.tick.cpu_overall = 99.0;
        // 从这里重新数 5 秒。
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 6))
                .is_empty()
        );
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 10))
                .is_empty()
        );
        assert_eq!(
            engine.evaluate(&rules, &world.inputs(), at(base, 11)).len(),
            1
        );
    }

    #[test]
    fn the_cooldown_spaces_out_repeats() {
        let mut world = World::new();
        world.tick.cpu_overall = 99.0;
        let rules = [rule(Metric::CpuUsage, 95., 0, 5)];
        let mut engine = Engine::default();
        let base = Instant::now();
        assert_eq!(
            engine.evaluate(&rules, &world.inputs(), at(base, 0)).len(),
            1
        );
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 299))
                .is_empty()
        );
        assert_eq!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 300))
                .len(),
            1
        );
    }

    #[test]
    fn rules_that_fall_below_a_line_and_missing_readings_behave() {
        let mut world = World::new();
        let rules = [rule(Metric::BatteryLevel, 20., 0, 5)];
        let mut engine = Engine::default();
        let base = Instant::now();
        // 没有电池：读不到，不触发也不报错。
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 0))
                .is_empty()
        );
        let battery = world.snapshot.battery.get_or_insert_with(Default::default);
        battery.percentage = 20.0;
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 1))
                .is_empty()
        );
        world.snapshot.battery.as_mut().unwrap().percentage = 19.0;
        assert_eq!(
            engine.evaluate(&rules, &world.inputs(), at(base, 2)).len(),
            1
        );
    }

    #[test]
    fn events_fire_on_the_edge_only() {
        let mut world = World::new();
        world.on_battery(true);
        let rules = [rule(Metric::OnBattery, 0., 0, 1)];
        let mut engine = Engine::default();
        let base = Instant::now();
        // 启动时已经在用电池：不算「刚发生」。
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 0))
                .is_empty()
        );
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 120))
                .is_empty()
        );
        world.on_battery(false);
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 121))
                .is_empty()
        );
        world.on_battery(true);
        assert_eq!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 122))
                .len(),
            1
        );
        // 一直在用电池不会反复触发。
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 400))
                .is_empty()
        );
    }

    #[test]
    fn disabled_rules_do_nothing_and_forget_their_state() {
        let mut world = World::new();
        world.tick.cpu_overall = 99.0;
        let mut rules = [rule(Metric::CpuUsage, 95., 0, 5)];
        let mut engine = Engine::default();
        let base = Instant::now();
        rules[0].enabled = false;
        assert!(
            engine
                .evaluate(&rules, &world.inputs(), at(base, 0))
                .is_empty()
        );
        rules[0].enabled = true;
        assert_eq!(
            engine.evaluate(&rules, &world.inputs(), at(base, 1)).len(),
            1
        );
    }

    #[test]
    fn latency_needs_a_reply_and_going_offline_fires_on_the_edge() {
        let mut world = World::new();
        let slow = [rule(Metric::NetLatency, 200., 0, 5)];
        let base = Instant::now();

        // 还没探测过：没有读数，不触发。
        let mut engine = Engine::default();
        assert!(
            engine
                .evaluate(&slow, &world.inputs(), at(base, 0))
                .is_empty()
        );
        world.internet(Some(80.0));
        assert!(
            engine
                .evaluate(&slow, &world.inputs(), at(base, 1))
                .is_empty()
        );
        world.internet(Some(320.0));
        let fired = engine.evaluate(&slow, &world.inputs(), at(base, 2));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].reading.detail.as_deref(), Some("223.5.5.5"));
        // 丢了包没有延迟读数：不算「慢」，那是「断网」规则的事。
        let mut engine = Engine::default();
        world.internet(None);
        assert!(
            engine
                .evaluate(&slow, &world.inputs(), at(base, 3))
                .is_empty()
        );

        // 一开始就是断的当作基线；由通变断触发一次，断着的期间不重复。
        let offline = [rule(Metric::Offline, 0., 0, 5)];
        let mut engine = Engine::default();
        assert!(
            engine
                .evaluate(&offline, &world.inputs(), at(base, 0))
                .is_empty()
        );
        world.internet(Some(10.0));
        assert!(
            engine
                .evaluate(&offline, &world.inputs(), at(base, 1))
                .is_empty()
        );
        world.internet(None);
        let fired = engine.evaluate(&offline, &world.inputs(), at(base, 2));
        assert_eq!(fired.len(), 1);
        assert_eq!(
            message(&fired[0].rule, &fired[0].reading, &world.settings),
            t!("rule_message_offline")
        );
        assert!(
            engine
                .evaluate(&offline, &world.inputs(), at(base, 3))
                .is_empty()
        );
    }

    #[test]
    fn sanitize_repairs_ids_and_numbers() {
        let mut rules = vec![
            Rule {
                id: 7,
                threshold: f64::NAN,
                cooldown_mins: 0,
                ..Rule::default()
            },
            Rule {
                id: 7,
                ..Rule::default()
            },
        ];
        sanitize(&mut rules);
        assert_ne!(rules[0].id, rules[1].id);
        assert!(rules[0].threshold.is_finite());
        assert_eq!(rules[0].cooldown_mins, 1);
    }

    #[test]
    fn old_settings_without_rules_get_the_four_builtin_alerts() {
        let settings: Settings = serde_json::from_str(r#"{"alerts":true}"#).unwrap();
        assert_eq!(settings.rules, defaults());
        assert_eq!(defaults().len(), 4);
    }

    #[test]
    fn webhook_bodies_follow_each_services_shape() {
        let firing = Firing {
            rule: Rule::default(),
            reading: Reading {
                value: 97.0,
                detail: None,
            },
        };
        let body = |format| {
            let text = webhook_body(format, &firing, "rule", "CPU is 97%", "mac", 5);
            serde_json::from_str::<serde_json::Value>(&text).unwrap()
        };
        let generic = body(WebhookFormat::Generic);
        assert_eq!(generic["metric"], "cpu_usage");
        assert_eq!(generic["text"], "mac: CPU is 97%");
        assert_eq!(generic["content"], generic["text"]);
        assert_eq!(
            body(WebhookFormat::Feishu)["content"]["text"],
            "mac: CPU is 97%"
        );
        assert_eq!(
            body(WebhookFormat::DingTalk)["text"]["content"],
            "mac: CPU is 97%"
        );
    }
}

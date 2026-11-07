//! 大屏上的各个组件：每个组件从应用状态里取自己的读数，交给 `widgets` 里的版面去摆。

use super::config::{Rank, WidgetKind};
use super::data::{self, Point};
use super::layout::Rect;
use super::widgets::{self, Body, Ctx, Gauge, Rates, Side};
use crate::menubar::data::{CELSIUS_FLOOR, CELSIUS_SPAN, ProcessSort, ranked_processes};
use crate::menubar::widgets::tabular;
use crate::{fmt, health};
use chrono::{DateTime, Datelike as _, Local, Weekday};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, Div, Hsla, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _,
};
use pc_specs_core::model::{BatteryInfo, CpuInfo, StorageInfo};
use rust_i18n::t;
use std::collections::HashSet;

pub fn render(c: &Ctx, kind: WidgetKind, rect: Rect) -> AnyElement {
    match kind {
        WidgetKind::Clock => clock(c, rect),
        WidgetKind::Cpu => cpu(c, rect),
        WidgetKind::Memory => memory(c, rect),
        WidgetKind::Gpu => gpu(c, rect),
        WidgetKind::Network => network(c, rect),
        WidgetKind::DiskIo => disk_io(c, rect),
        WidgetKind::Storage => storage(c, rect),
        WidgetKind::Temperature => temperature(c, rect),
        WidgetKind::Battery => battery(c, rect),
        WidgetKind::Processes => processes(c, rect),
        WidgetKind::System => system(c, rect),
    }
}

fn dash() -> String {
    "—".to_string()
}

/// 「1.23 MB/s」拆成数值和单位，数值放大、单位缩小分开排。
fn split_unit(text: String) -> (String, String) {
    match text.split_once(' ') {
        Some((value, unit)) => (value.to_string(), unit.to_string()),
        None => (text, String::new()),
    }
}

fn percent_value(percent: Option<f32>) -> (String, String) {
    match percent {
        Some(value) => (format!("{value:.0}"), "%".to_string()),
        None => (dash(), String::new()),
    }
}

// ---- 时钟 -----------------------------------------------------------------------

pub fn clock_text(now: &DateTime<Local>, hour24: bool, seconds: bool) -> String {
    let pattern = match (hour24, seconds) {
        (true, true) => "%H:%M:%S",
        (true, false) => "%H:%M",
        (false, true) => "%-I:%M:%S %p",
        (false, false) => "%-I:%M %p",
    };
    now.format(pattern).to_string()
}

pub fn date_text(now: &DateTime<Local>) -> String {
    let weekday = match now.weekday() {
        Weekday::Mon => t!("bs_wd_mon"),
        Weekday::Tue => t!("bs_wd_tue"),
        Weekday::Wed => t!("bs_wd_wed"),
        Weekday::Thu => t!("bs_wd_thu"),
        Weekday::Fri => t!("bs_wd_fri"),
        Weekday::Sat => t!("bs_wd_sat"),
        Weekday::Sun => t!("bs_wd_sun"),
    };
    t!(
        "bs_date",
        weekday = weekday,
        year = now.year(),
        month = now.month(),
        day = now.day(),
        month_name = now.format("%b").to_string()
    )
    .into_owned()
}

fn clock(c: &Ctx, rect: Rect) -> AnyElement {
    let time = clock_text(&c.now, c.cfg.hour24, c.cfg.show_seconds);
    let date = date_text(&c.now);
    widgets::tile(c, WidgetKind::Clock, rect, None, |b| {
        let chars = time.chars().count() as f32;
        let with_date = b.h >= 84.;
        let size = (b.w / (chars * 0.6))
            .min(b.h * if with_date { 0.5 } else { 0.78 })
            .clamp(14., 110.);
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap(c.px(size * 0.14))
            .child(c.number(time, size, c.pal.text))
            .when(with_date, |column| {
                column.child(c.text(date, (size * 0.3).clamp(11., 26.), c.pal.muted))
            })
            .into_any_element()
    })
}

// ---- CPU / 内存 / GPU -----------------------------------------------------------

fn cores_text(info: &CpuInfo) -> Option<String> {
    if info.logical_cores == 0 {
        return None;
    }
    let hybrid = info
        .topology
        .as_ref()
        .and_then(|topology| topology.p_cores.zip(topology.e_cores))
        .filter(|(p, e)| *p > 0 && *e > 0);
    Some(match hybrid {
        Some((p, e)) => format!("{p}P + {e}E · {}T", info.logical_cores),
        None if info.physical_cores > 0 => {
            format!("{}C / {}T", info.physical_cores, info.logical_cores)
        }
        None => format!("{}T", info.logical_cores),
    })
}

fn cpu(c: &Ctx, rect: Rect) -> AnyElement {
    let app = c.app;
    let info = &app.snapshot.cpu;
    let tick = app.latest.as_ref();
    let usage = tick.map(|tick| tick.cpu_overall);
    let mut facts = Vec::new();
    if let Some(text) = cores_text(info) {
        facts.push((t!("spec_cores").into_owned(), text));
    }
    if let Some(temp) = tick.and_then(|tick| tick.cpu_temperature()) {
        facts.push((t!("spec_temp").into_owned(), app.settings.temp(temp)));
    }
    let top_speed = if info.max_frequency_hz > 0 {
        info.max_frequency_hz
    } else {
        info.base_frequency_hz
    };
    if top_speed > 0 {
        facts.push((t!("spec_freq_max").into_owned(), fmt::freq(top_speed)));
    }
    let cores = if c.cfg.per_core {
        tick.map(|tick| {
            tick.cpu_per_core
                .iter()
                .map(|&load| (load / 100., c.pal.usage(load, c.pal.cpu)))
                .collect()
        })
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let (value, unit) = percent_value(usage);
    let gauge = Gauge {
        fraction: usage.map(|usage| usage / 100.),
        color: usage.map_or(c.pal.cpu, |usage| c.pal.usage(usage, c.pal.cpu)),
        value,
        unit,
        facts,
        chart: Some(Box::new(move |width| {
            (
                vec![c.percent_line(width, c.pal.cpu, |p| Some(p.cpu))],
                None,
            )
        })),
        cores,
    };
    let aside = (!info.brand.trim().is_empty()).then(|| info.brand.trim().to_string());
    widgets::tile(c, WidgetKind::Cpu, rect, aside, |b| {
        widgets::gauge_panel(c, b, gauge)
    })
}

fn memory(c: &Ctx, rect: Rect) -> AnyElement {
    let app = c.app;
    let tick = app.latest.as_ref();
    let total = tick.map_or(app.snapshot.memory.total_bytes, |tick| tick.mem_total_bytes);
    let used = tick.map_or(0, |tick| tick.mem_used_bytes);
    let percent = tick.filter(|_| total > 0).map(|_| fmt::ratio(used, total));
    let mut facts = vec![
        (t!("spec_used").into_owned(), app.settings.bytes(used)),
        (
            t!("spec_available").into_owned(),
            app.settings.bytes(total.saturating_sub(used)),
        ),
    ];
    if let Some(tick) = tick.filter(|tick| tick.swap_total_bytes > 0) {
        facts.push((
            t!("mem_swap").into_owned(),
            format!(
                "{} / {}",
                app.settings.bytes(tick.swap_used_bytes),
                app.settings.bytes(tick.swap_total_bytes)
            ),
        ));
    }
    let (value, unit) = percent_value(percent);
    let gauge = Gauge {
        fraction: percent.map(|percent| percent / 100.),
        color: percent.map_or(c.pal.mem, |percent| c.pal.usage(percent, c.pal.mem)),
        value,
        unit,
        facts: if total > 0 { facts } else { Vec::new() },
        chart: Some(Box::new(move |width| {
            (
                vec![c.percent_line(width, c.pal.mem, |p| Some(p.mem))],
                None,
            )
        })),
        cores: Vec::new(),
    };
    let aside = (total > 0).then(|| app.settings.bytes(total));
    widgets::tile(c, WidgetKind::Memory, rect, aside, |b| {
        widgets::gauge_panel(c, b, gauge)
    })
}

fn gpu(c: &Ctx, rect: Rect) -> AnyElement {
    let app = c.app;
    let card = data::active_gpu(app);
    let live = data::live_gpu(app, card);
    // 占用和显存用实时采样；快照里的温度、功耗只在启动时读过一次，挂在常亮的大屏上就是错的，所以不放。
    let usage = live.and_then(|tick| tick.utilization);
    let mut facts = Vec::new();
    if let Some(total) = card
        .and_then(|card| card.vram_total_bytes)
        .filter(|total| *total > 0)
    {
        let used = live
            .and_then(|tick| tick.vram_used_bytes)
            .or_else(|| card.and_then(|card| card.vram_used_bytes));
        // 分成两行：「已用 / 总量」写在一行里，窄磁贴上放不下。
        if let Some(used) = used {
            facts.push((t!("bs_f_vram_used").into_owned(), app.settings.bytes(used)));
        }
        facts.push((t!("bs_f_vram").into_owned(), app.settings.bytes(total)));
    }
    if let Some(cores) = card.and_then(|card| card.cores) {
        facts.push((t!("spec_cores").into_owned(), cores.to_string()));
    }
    let (value, unit) = percent_value(usage);
    let gauge = Gauge {
        fraction: usage.map(|usage| usage / 100.),
        color: usage.map_or(c.pal.gpu, |usage| c.pal.usage(usage, c.pal.gpu)),
        value,
        unit,
        facts,
        chart: Some(Box::new(move |width| {
            (vec![c.percent_line(width, c.pal.gpu, |p| p.gpu)], None)
        })),
        cores: Vec::new(),
    };
    let aside = card
        .map(|card| card.name.clone())
        .or_else(|| live.map(|tick| tick.name.clone()));
    widgets::tile(c, WidgetKind::Gpu, rect, aside, |b| {
        widgets::gauge_panel(c, b, gauge)
    })
}

// ---- 网络 / 磁盘读写 ------------------------------------------------------------

fn down(point: &Point) -> Option<f32> {
    Some(point.down)
}

fn up(point: &Point) -> Option<f32> {
    Some(point.up)
}

fn read(point: &Point) -> Option<f32> {
    Some(point.read)
}

fn write(point: &Point) -> Option<f32> {
    Some(point.write)
}

fn side(c: &Ctx, arrow: &'static str, label: String, bytes_per_sec: u64, color: Hsla) -> Side {
    let (value, unit) = split_unit(c.app.settings.rate(bytes_per_sec));
    Side {
        arrow,
        label,
        value,
        unit,
        color,
    }
}

fn network(c: &Ctx, rect: Rect) -> AnyElement {
    let app = c.app;
    let tick = app.latest.as_ref();
    let (rx, tx) = tick.map_or((0, 0), |tick| (tick.net_rx_bps, tick.net_tx_bps));
    let interface = data::active_interface(app);
    let mut footer = Vec::new();
    if let Some(interface) = interface {
        let kind = match interface.kind.as_str() {
            "wifi" => t!("net_kind_wifi").into_owned(),
            "ethernet" => t!("net_kind_ethernet").into_owned(),
            "bluetooth" => t!("net_kind_bluetooth").into_owned(),
            "virtual" => t!("net_kind_virtual").into_owned(),
            _ => String::new(),
        };
        let mut line: Vec<String> = Vec::new();
        if !kind.is_empty() {
            line.push(kind);
        }
        line.push(interface.name.clone());
        if let Some(address) = interface.ipv4.first() {
            line.push(address.to_string());
        }
        if let Some(speed) = interface.link_speed_mbps.filter(|speed| *speed > 0) {
            line.push(if speed >= 1000 {
                format!("{} Gbps", speed as f64 / 1000.)
            } else {
                format!("{speed} Mbps")
            });
        }
        footer.push(line.join(" · "));
        footer.push(format!(
            "↓ {}   ↑ {}",
            app.settings.bytes(interface.rx_total_bytes),
            app.settings.bytes(interface.tx_total_bytes)
        ));
    }
    let rates = Rates {
        sides: [
            side(c, "↓", t!("bar_download").into_owned(), rx, c.pal.down),
            side(c, "↑", t!("bar_upload").into_owned(), tx, c.pal.up),
        ],
        footer,
        chart: Box::new(move |width| {
            let (lines, peak) = c.rate_lines(width, [(down, c.pal.down), (up, c.pal.up)]);
            (lines, Some(app.settings.rate(peak as u64)))
        }),
    };
    widgets::tile(c, WidgetKind::Network, rect, None, |b| {
        widgets::rate_panel(c, b, rates)
    })
}

fn disk_io(c: &Ctx, rect: Rect) -> AnyElement {
    let app = c.app;
    let tick = app.latest.as_ref();
    let (rd, wr) = tick.map_or((0, 0), |tick| (tick.disk_read_bps, tick.disk_write_bps));
    // 多块盘时，把此刻最忙的那块点出来。
    let busiest = tick.and_then(|tick| {
        tick.per_disk
            .iter()
            .filter(|disk| disk.read_bps + disk.write_bps > 0)
            .max_by_key(|disk| disk.read_bps + disk.write_bps)
    });
    let footer = busiest
        .filter(|_| tick.is_some_and(|tick| tick.per_disk.len() > 1))
        .map(|disk| vec![disk.mount_point.clone()])
        .unwrap_or_default();
    let rates = Rates {
        sides: [
            side(c, "R", t!("bar_read").into_owned(), rd, c.pal.disk),
            side(c, "W", t!("bar_write").into_owned(), wr, c.pal.up),
        ],
        footer,
        chart: Box::new(move |width| {
            let (lines, peak) = c.rate_lines(width, [(read, c.pal.disk), (write, c.pal.up)]);
            (lines, Some(app.settings.rate(peak as u64)))
        }),
    };
    widgets::tile(c, WidgetKind::DiskIo, rect, None, |b| {
        widgets::rate_panel(c, b, rates)
    })
}

// ---- 温度 / 电池 ----------------------------------------------------------------

fn temperature(c: &Ctx, rect: Rect) -> AnyElement {
    let app = c.app;
    let tick = app.latest.as_ref();
    let hottest = tick.and_then(data::headline_temperature);
    let unit = app.settings.temp_unit;
    let mut sensors: Vec<(&str, f32)> = tick
        .map(|tick| {
            tick.temperatures
                .iter()
                .filter(|sensor| sensor.value.is_finite())
                .map(|sensor| {
                    let name = if sensor.label.trim().is_empty() {
                        sensor.source.as_str()
                    } else {
                        sensor.label.as_str()
                    };
                    (name, sensor.value)
                })
                .collect()
        })
        .unwrap_or_default();
    sensors.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut seen = HashSet::new();
    let facts = sensors
        .into_iter()
        .filter(|(name, _)| seen.insert(*name))
        .take(8)
        .map(|(name, value)| (name.to_string(), app.settings.temp(value)))
        .collect();
    let gauge = Gauge {
        fraction: hottest
            .map(|value| ((value - CELSIUS_FLOOR as f32) / CELSIUS_SPAN as f32).clamp(0., 1.)),
        color: hottest.map_or(c.pal.temp, |value| {
            c.pal.graded(health::temperature_level(value), c.pal.temp)
        }),
        value: hottest.map_or_else(dash, |value| {
            format!("{:.0}", fmt::temp_chart_value(value, unit))
        }),
        unit: hottest
            .map(|_| fmt::temp_unit_name(unit).to_string())
            .unwrap_or_default(),
        facts,
        chart: Some(Box::new(move |width| {
            (vec![c.celsius_line(width, c.pal.temp, |p| p.temp)], None)
        })),
        cores: Vec::new(),
    };
    // 读数优先取 CPU / SoC 的传感器；没有的时候才是全机最高，标签要和读数对得上。
    let aside = hottest.map(|_| {
        if tick.is_some_and(|tick| tick.cpu_temperature().is_some()) {
            t!("bs_temp_cpu").into_owned()
        } else {
            t!("monitor_hottest").into_owned()
        }
    });
    widgets::tile(c, WidgetKind::Temperature, rect, aside, |b| {
        widgets::gauge_panel(c, b, gauge)
    })
}

fn battery_state(raw: &str) -> String {
    match raw.to_ascii_lowercase().as_str() {
        "charging" => t!("bat_state_charging").into_owned(),
        "discharging" => t!("bat_state_discharging").into_owned(),
        "full" => t!("bat_state_full").into_owned(),
        "empty" => t!("bat_state_empty").into_owned(),
        "unknown" => t!("bat_state_unknown").into_owned(),
        _ => raw.to_string(),
    }
}

fn battery_facts(c: &Ctx, battery: &BatteryInfo) -> Vec<(String, String)> {
    let mut facts = Vec::new();
    let minutes = |secs: Option<u64>| secs.filter(|secs| *secs > 0).map(fmt::duration);
    if let Some(left) = minutes(battery.time_to_empty_secs) {
        facts.push((t!("bat_time_to_empty").into_owned(), left));
    } else if let Some(left) = minutes(battery.time_to_full_secs) {
        facts.push((t!("bat_time_to_full").into_owned(), left));
    }
    if let Some(power) = battery.power_now_mw.filter(|power| *power != 0) {
        facts.push((
            t!("bat_power").into_owned(),
            format!("{:.1} W", power.unsigned_abs() as f64 / 1000.),
        ));
    }
    if let (Some(full), Some(design)) = (battery.full_capacity_mwh, battery.design_capacity_mwh)
        && design > 0
    {
        facts.push((
            t!("bat_health").into_owned(),
            fmt::percent(full as f32 / design as f32 * 100.),
        ));
    }
    if let Some(cycles) = battery.cycle_count {
        facts.push((t!("bat_cycle").into_owned(), cycles.to_string()));
    }
    if let Some(temp) = battery.temperature_c {
        facts.push((t!("spec_temp").into_owned(), c.app.settings.temp(temp)));
    }
    facts
}

fn battery(c: &Ctx, rect: Rect) -> AnyElement {
    let Some(info) = c.app.snapshot.battery.as_ref() else {
        return widgets::tile(c, WidgetKind::Battery, rect, None, |_| {
            widgets::empty(c, t!("bat_no_battery").into_owned())
        });
    };
    let percent = info.percentage.clamp(0., 100.);
    let gauge = Gauge {
        fraction: Some(percent / 100.),
        color: c.pal.graded(health::battery_level(percent), c.pal.battery),
        value: format!("{percent:.0}"),
        unit: "%".to_string(),
        facts: battery_facts(c, info),
        chart: None,
        cores: Vec::new(),
    };
    let aside = Some(battery_state(&info.state));
    widgets::tile(c, WidgetKind::Battery, rect, aside, |b| {
        widgets::gauge_panel(c, b, gauge)
    })
}

// ---- 存储 / 进程 / 系统 ---------------------------------------------------------

/// 一个卷占的设计高度：名称那一行加进度条。
const VOLUME_ROW: f32 = 34.;

fn volume_row(c: &Ctx, volume: &StorageInfo) -> Div {
    let percent = fmt::ratio(volume.used_bytes, volume.total_bytes);
    let color = c.pal.usage(percent, c.pal.disk);
    let name = [
        volume.name.trim(),
        volume.mount_point.as_deref().unwrap_or(""),
    ]
    .into_iter()
    .find(|name| !name.is_empty())
    .unwrap_or("—")
    .to_string();
    v_flex()
        .w_full()
        .gap(c.px(4.))
        .child(
            h_flex()
                .w_full()
                .items_baseline()
                .gap(c.px(8.))
                .child(c.text(name, 12., c.pal.text).flex_1().min_w_0().truncate())
                .child(
                    tabular(c.text(
                        format!(
                            "{} / {}",
                            c.app.settings.bytes(volume.used_bytes),
                            c.app.settings.bytes(volume.total_bytes)
                        ),
                        11.,
                        c.pal.muted,
                    ))
                    .flex_none(),
                )
                .child(c.number(fmt::percent(percent), 12., color).flex_none()),
        )
        .child(widgets::bar(c, percent / 100., color, 6.))
}

fn storage(c: &Ctx, rect: Rect) -> AnyElement {
    let volumes = data::volumes(c.app);
    let aside = (!volumes.is_empty()).then(|| t!("count_found", n = volumes.len()).into_owned());
    widgets::tile(c, WidgetKind::Storage, rect, aside, |b| {
        if volumes.is_empty() {
            return widgets::empty(c, t!("storage_no_disks").into_owned());
        }
        let columns = if b.w >= 560. { 2 } else { 1 };
        let rows = (((b.h + 8.) / (VOLUME_ROW + 8.)).floor() as usize).max(1);
        let shown: Vec<&StorageInfo> = volumes.iter().copied().take(rows * columns).collect();
        // 卷不多时，行与行之间松一点，别都挤在最上面。
        let per_column = shown.len().div_ceil(columns).max(1);
        let spacing = if per_column > 1 {
            ((b.h - per_column as f32 * VOLUME_ROW) / (per_column - 1) as f32).clamp(8., 22.)
        } else {
            8.
        };
        h_flex()
            .size_full()
            .gap(c.px(widgets::GAP * 1.4))
            .items_start()
            .children(shown.chunks(per_column).map(|chunk| {
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap(c.px(spacing))
                    .justify_center()
                    .children(chunk.iter().map(|volume| volume_row(c, volume)))
            }))
            .into_any_element()
    })
}

fn process_row(
    c: &Ctx,
    process: &pc_specs_core::model::ProcessInfo,
    sort: ProcessSort,
    scale: f64,
    color: Hsla,
) -> Div {
    let (amount, text) = match sort {
        ProcessSort::Memory => (
            process.mem_bytes as f64,
            c.app.settings.bytes(process.mem_bytes),
        ),
        _ => (
            f64::from(process.cpu_percent),
            format!("{:.1}%", process.cpu_percent),
        ),
    };
    // 没有占用的行调暗，眼睛才会落在真正在干活的进程上。
    let idle = amount <= 0.05;
    h_flex()
        .w_full()
        .h(c.px(20.))
        .items_center()
        .gap(c.px(8.))
        .child(
            c.text(
                process.name.clone(),
                12.,
                if idle { c.pal.muted } else { c.pal.text },
            )
            .flex_1()
            .min_w_0()
            .truncate(),
        )
        .child(div().flex_none().w(c.px(56.)).child(widgets::bar(
            c,
            (amount / scale) as f32,
            color,
            6.,
        )))
        .child(
            tabular(
                c.text(text, 12., if idle { c.pal.muted } else { c.pal.text })
                    .text_right(),
            )
            .flex_none()
            .w(c.px(64.)),
        )
}

fn processes(c: &Ctx, rect: Rect) -> AnyElement {
    let app = c.app;
    let (sort, color) = match c.cfg.rank {
        Rank::Cpu => (ProcessSort::Cpu, c.pal.cpu),
        Rank::Memory => (ProcessSort::Memory, c.pal.mem),
    };
    let aside =
        (!app.processes.is_empty()).then(|| t!("proc_count", n = app.processes.len()).into_owned());
    widgets::tile(c, WidgetKind::Processes, rect, aside, |b| {
        if app.processes.is_empty() {
            return widgets::empty(c, t!("monitor_no_data").into_owned());
        }
        let rows = (((b.h + 4.) / 24.).floor() as usize).max(1);
        let top = ranked_processes(&app.processes, sort, true, rows);
        // 以榜首为满格，CPU 至少按单核 100%，免得空闲时一点杂音也顶满。
        let peak = top.first().map_or(0., |process| match sort {
            ProcessSort::Memory => process.mem_bytes as f64,
            _ => f64::from(process.cpu_percent),
        });
        let scale = match sort {
            ProcessSort::Memory => peak.max(1.),
            _ => peak.max(100.),
        };
        v_flex()
            .size_full()
            .gap(c.px(4.))
            .children(
                top.iter()
                    .map(|process| process_row(c, process, sort, scale, color)),
            )
            .into_any_element()
    })
}

fn system_facts(c: &Ctx) -> Vec<(String, String)> {
    let app = c.app;
    let snapshot = &app.snapshot;
    let mut rows = Vec::new();
    let mut push = |label: String, value: String| {
        if !value.trim().is_empty() {
            rows.push((label, value));
        }
    };
    push(
        t!("spec_hostname").into_owned(),
        snapshot.host.hostname.clone(),
    );
    push(
        t!("nav_os").into_owned(),
        format!("{} {}", snapshot.os.name, snapshot.os.version),
    );
    push(t!("spec_kernel").into_owned(), snapshot.os.kernel.clone());
    push(t!("spec_arch").into_owned(), snapshot.os.arch.clone());
    let uptime = if snapshot.host.boot_time > 0 {
        (c.now.timestamp().max(0) as u64).saturating_sub(snapshot.host.boot_time)
    } else {
        snapshot.host.uptime_secs
    };
    if uptime > 0 {
        push(t!("spec_uptime").into_owned(), fmt::duration(uptime));
    }
    if let Some(address) = data::active_interface(app).and_then(|interface| interface.ipv4.first())
    {
        push(t!("bs_f_ip").into_owned(), address.to_string());
    }
    if let Some(address) = snapshot.network.public_ip.as_ref() {
        push(t!("net_public_prefix").into_owned(), address.to_string());
    }
    push(t!("spec_user").into_owned(), snapshot.host.username.clone());
    rows
}

fn system(c: &Ctx, rect: Rect) -> AnyElement {
    let rows = system_facts(c);
    widgets::tile(c, WidgetKind::System, rect, None, |b: Body| {
        if rows.is_empty() {
            return widgets::empty(c, t!("monitor_no_data").into_owned());
        }
        let columns = if b.w >= 560. { 2 } else { 1 };
        let capacity = widgets::rows_fitting(b.h).max(1);
        let shown: Vec<(String, String)> = rows.into_iter().take(capacity * columns).collect();
        let per_column = shown.len().div_ceil(columns).max(1);
        h_flex()
            .size_full()
            .gap(c.px(widgets::GAP * 1.6))
            .items_start()
            .children(shown.chunks(per_column).map(|chunk| {
                div()
                    .flex_1()
                    .min_w_0()
                    .child(widgets::facts(c, chunk.to_vec()))
            }))
            .into_any_element()
    })
}

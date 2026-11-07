//! 监控面板里每个指标的内容。每一块都带着自己的高度，所有块的高度加起来就是面板的高度，
//! 所以面板既不需要滚动，也不需要先排版再量尺寸。
use super::{
    color,
    config::{Metric, Show},
    data::{self, ProcessSort, Value},
    widgets::{self, Palette},
};
use crate::{
    fmt,
    health::{self, Level},
    state::AppState,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Div, Hsla, IntoElement, ParentElement as _, SharedString, Styled as _, div,
    prelude::FluentBuilder as _, px, relative,
};
use pc_specs_core::model::{MonitorTick, ProcessInfo, SensorReading, StorageInfo};
use rust_i18n::t;

pub const WIDTH: f32 = 340.;
pub const PAD: f32 = 16.;
/// 去掉 1px 边框和左右留白之后，内容实际可用的宽度。
const INNER: f32 = WIDTH - 2. - PAD * 2.;
/// 走势图的列数：按默认 1 秒一个采样，正好是一分钟。
const COLS: usize = 60;
const ROW: f32 = 26.;
const TITLE: f32 = 26.;
/// 「名称 + 一条格子」这一行（含下方留白）。
const METER_ROW: f32 = 40.;
const LINK_ROW: f32 = 40.;
const GPU_ROW: f32 = METER_ROW + 16.;
/// 每一节下面的留白。
const GAP: f32 = 10.;
/// 温度走势和温度条的纵轴，和菜单栏上的走势用同一个量程。
const TEMP_FLOOR: f32 = data::CELSIUS_FLOOR as f32;
const TEMP_SPAN: f32 = data::CELSIUS_SPAN as f32;
/// 走势图上方的标题行、下方的横轴行。
const CHART_HEAD: f32 = 20.;
const CHART_AXIS: f32 = 14.;
/// 两张走势图排在一起时，速率低于这个值就当作没有流量，免得把杂音放大成满格。
const NET_FLOOR: f64 = 100. * 1024.;
const DISK_FLOOR: f64 = 1024. * 1024.;
/// 二级页面里最多列出的进程数。
const FLYOUT_ROWS: usize = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// 鼠标悬浮在面板里的某一块上时，在旁边展开的二级页面。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flyout {
    Processes(ProcessSort),
    Volumes,
    Sensors,
    Interfaces,
}

pub struct Block {
    pub height: f32,
    pub element: AnyElement,
    pub trigger: Option<Flyout>,
}

impl Block {
    fn new(height: f32, element: impl IntoElement) -> Self {
        Self {
            height,
            element: div()
                .w_full()
                .h(px(height))
                .flex_shrink_0()
                .child(element)
                .into_any_element(),
            trigger: None,
        }
    }

    fn trigger(mut self, flyout: Flyout) -> Self {
        self.trigger = Some(flyout);
        self
    }
}

/// 面板里用到的几种颜色，按指标取主色。
#[derive(Clone, Copy)]
struct Ink {
    fg: Hsla,
    muted: Hsla,
    chip: Hsla,
    /// 各节之间的细线。
    rule: Hsla,
    accent: Hsla,
    warning: Hsla,
    danger: Hsla,
    cyan: Hsla,
    magenta: Hsla,
    yellow: Hsla,
}

impl Ink {
    fn new(metric: Metric, cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            fg: theme.foreground,
            muted: theme.muted_foreground,
            chip: theme.foreground.opacity(0.07),
            rule: theme.foreground.opacity(0.09),
            accent: color(metric, cx),
            warning: theme.warning,
            danger: theme.danger,
            cyan: theme.cyan,
            magenta: theme.magenta,
            yellow: theme.yellow,
        }
    }

    fn tone(&self, level: Level) -> Hsla {
        match level {
            Level::Healthy => self.accent,
            Level::Warning => self.warning,
            Level::Critical => self.danger,
        }
    }

    fn palette(&self, cx: &App) -> Palette {
        Palette::new(self.accent, cx)
    }

    fn temperature_palette(&self, cx: &App) -> Palette {
        self.palette(cx).levels(
            (75. - TEMP_FLOOR) / TEMP_SPAN,
            (88. - TEMP_FLOOR) / TEMP_SPAN,
        )
    }
}

/// 行高固定为字号的 1.3 倍，各块的高度才算得准。
fn label(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div()
        .text_size(px(size))
        .line_height(relative(1.3))
        .text_color(color)
        .child(text.into())
}

/// 读数旁边的小标签：图标说明它是什么，颜色和下面的曲线对得上。
struct Chip {
    icon: IconName,
    tint: Hsla,
    text: String,
}

impl Chip {
    fn new(icon: IconName, tint: Hsla, text: impl Into<String>) -> Self {
        Self {
            icon,
            tint,
            text: text.into(),
        }
    }
}

fn chip(ink: &Ink, chip: Chip) -> Div {
    widgets::tabular(
        h_flex()
            .h(px(20.))
            .px(px(7.))
            .gap(px(4.))
            .items_center()
            .rounded(px(6.))
            .bg(ink.chip)
            .text_size(px(11.))
            .text_color(ink.fg)
            .child(Icon::new(chip.icon).size(px(11.)).text_color(chip.tint))
            .child(chip.text),
    )
}

fn usage_tone(ink: &Ink, percent: Option<f64>) -> Hsla {
    ink.tone(percent.map_or(Level::Healthy, |n| health::usage_level(n as f32)))
}

fn title_row(ink: &Ink, title: String, hint: Option<(String, Side)>) -> Div {
    let chevron = |name| Icon::new(name).size(px(11.)).text_color(ink.muted);
    h_flex()
        .h(px(TITLE))
        .w_full()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_size(px(10.5))
                .font_semibold()
                .text_color(ink.muted)
                .child(title.to_uppercase()),
        )
        .when_some(hint, |this, (hint, side)| {
            this.child(
                h_flex()
                    .items_center()
                    .gap(px(1.))
                    .text_size(px(10.5))
                    .text_color(ink.muted)
                    .when(side == Side::Left, |this| {
                        this.child(chevron(IconName::ChevronLeft))
                    })
                    .child(hint)
                    .when(side == Side::Right, |this| {
                        this.child(chevron(IconName::ChevronRight))
                    }),
            )
        })
}

/// 标题 + 一组内容。`body_height` 是内容本身的高度。
fn section(
    ink: &Ink,
    title: String,
    hint: Option<(String, Side)>,
    body_height: f32,
    body: impl IntoElement,
) -> Block {
    Block::new(
        TITLE + body_height + GAP,
        v_flex()
            .relative()
            // 和上一块之间的细线，画在上一块下面的留白里，不占高度。
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(1.))
                    .bg(ink.rule),
            )
            .child(title_row(ink, title, hint))
            .child(div().w_full().h(px(body_height)).child(body)),
    )
}

/// 标题右边的「全部 N」，提示鼠标悬停就能在旁边展开完整列表。
fn all_hint(side: Side, count: usize) -> Option<(String, Side)> {
    let text = if count > 0 {
        t!("bar_all_count", n = count)
    } else {
        t!("bar_all")
    };
    Some((text.into_owned(), side))
}

/// 把读数缩放到 0..=1。
fn scaled(values: Vec<Option<f64>>, floor: f64, span: f64) -> Vec<Option<f32>> {
    values
        .into_iter()
        .map(|v| v.map(|v| ((v - floor) / span).clamp(0., 1.) as f32))
        .collect()
}

/// 走势图横轴最左边的文字：「1 分钟前」。
fn history_span(s: &AppState) -> String {
    let seconds = COLS as u64 * s.settings.interval_ms / 1000;
    let span = if seconds >= 120 {
        t!("unit_minutes", n = seconds / 60)
    } else {
        t!("unit_seconds", n = seconds)
    };
    t!("bar_ago", span = span).into_owned()
}

/// 「平均 … · 峰值 …」，没有任何读数时留空。
fn summary(series: &[f64], show: impl Fn(f64) -> String) -> String {
    if series.is_empty() {
        return String::new();
    }
    let average = series.iter().sum::<f64>() / series.len() as f64;
    let peak = series.iter().copied().fold(f64::MIN, f64::max);
    t!("bar_avg_peak", avg = show(average), peak = show(peak)).into_owned()
}

fn hero(ink: &Ink, tone: Hsla, value: &Value, caption: String, chips: Vec<Chip>) -> Block {
    let number = h_flex()
        .items_baseline()
        .gap(px(3.))
        .child(widgets::tabular(
            div()
                .text_size(px(40.))
                .line_height(px(44.))
                .font_semibold()
                .text_color(tone)
                .child(format!("{}{}", value.lead, value.num)),
        ))
        .child(
            div()
                .text_size(px(16.))
                .font_semibold()
                .text_color(ink.muted)
                .child(value.unit.clone()),
        );
    Block::new(
        76.,
        h_flex()
            .size_full()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(number)
                    .when(!caption.is_empty(), |column| {
                        column.child(label(caption, 12., ink.muted).truncate())
                    }),
            )
            .child(
                v_flex()
                    .items_end()
                    .gap(px(4.))
                    .children(chips.into_iter().map(|text| chip(ink, text))),
            ),
    )
}

/// 一张走势图：上面是名字和统计，下面是一格一格的曲线和横轴。
struct Trend {
    title: String,
    /// 标题的颜色：几张图排在一起时，用来和上面的读数对上号。
    tint: Hsla,
    /// 右上角的「平均 … · 峰值 …」。
    summary: String,
    values: Vec<Option<f32>>,
    rows: usize,
    height: f32,
    palette: Palette,
    /// 横轴文字（「1 分钟前 … 现在」）。几张图叠在一起时只在最下面一张写。
    axis: bool,
}

fn chart(s: &AppState, ink: &Ink, trend: Trend) -> Block {
    let axis = trend.axis.then(|| {
        h_flex()
            .h(px(CHART_AXIS))
            .items_center()
            .justify_between()
            .text_size(px(10.))
            .text_color(ink.muted)
            .child(history_span(s))
            .child(t!("bar_now").into_owned())
    });
    let height = CHART_HEAD + 5. + trend.height + if axis.is_some() { 5. + CHART_AXIS } else { 0. };
    Block::new(
        height + 8.,
        v_flex()
            .w_full()
            .gap(px(5.))
            .child(
                h_flex()
                    .h(px(CHART_HEAD))
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(10.5))
                            .font_semibold()
                            .text_color(trend.tint)
                            .child(trend.title.to_uppercase()),
                    )
                    .child(widgets::tabular(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(ink.muted)
                            .child(trend.summary),
                    )),
            )
            .child(div().w_full().h(px(trend.height)).child(widgets::columns(
                trend.values,
                COLS,
                trend.rows,
                (1.5, 1.5),
                trend.palette,
            )))
            .children(axis),
    )
}

/// 占用率类指标的走势图：纵轴固定 0–100%。
fn usage_chart(s: &AppState, ink: &Ink, metric: Metric, cx: &App) -> Block {
    let series = super::history(s, metric, COLS);
    let known: Vec<f64> = series.iter().flatten().copied().collect();
    chart(
        s,
        ink,
        Trend {
            title: t!("bar_show_usage").into_owned(),
            tint: ink.muted,
            summary: summary(&known, |v| fmt::percent(v as f32)),
            values: scaled(series, 0., 100.),
            rows: 10,
            height: 64.,
            palette: ink.palette(cx),
            axis: true,
        },
    )
}

/// 速率类指标的走势：纵轴按这一段时间里的峰值缩放，`floor` 是量程下限。
/// 版式（行数、高度、要不要横轴）用结构体更新语法覆盖。
fn rate_trend(
    s: &AppState,
    title: String,
    tint: Hsla,
    series: Vec<f64>,
    floor: f64,
    cx: &App,
) -> Trend {
    let scale = series.iter().copied().fold(floor, f64::max);
    Trend {
        title,
        tint,
        summary: summary(&series, |v| s.settings.rate(v as u64)),
        values: series
            .iter()
            .map(|v| Some((v / scale).clamp(0., 1.) as f32))
            .collect(),
        rows: 5,
        height: 30.,
        palette: Palette::new(tint, cx).plain(),
        axis: true,
    }
}

fn process_value(p: &ProcessInfo, sort: ProcessSort) -> f64 {
    match sort {
        ProcessSort::Cpu => p.cpu_percent as f64,
        ProcessSort::Memory => p.mem_bytes as f64,
        ProcessSort::Io => p.read_bps.saturating_add(p.write_bps) as f64,
    }
}

fn process_text(s: &AppState, p: &ProcessInfo, sort: ProcessSort) -> String {
    match sort {
        ProcessSort::Cpu => format!("{:.1}%", p.cpu_percent),
        ProcessSort::Memory => s.settings.bytes(p.mem_bytes),
        ProcessSort::Io => s.settings.rate(p.read_bps.saturating_add(p.write_bps)),
    }
}

/// 排行里每一行的条形满量程：都以榜首为满格，让排名一目了然；
/// CPU 至少按单核 100%、I/O 至少 1 MB/s，免得空闲时几 KB/s 的杂音也顶满。
fn process_scale(top: &[&ProcessInfo], sort: ProcessSort) -> f64 {
    let peak = top.first().map_or(0., |p| process_value(p, sort));
    match sort {
        ProcessSort::Cpu => peak.max(100.),
        ProcessSort::Memory => peak.max(1.),
        ProcessSort::Io => peak.max(1_048_576.),
    }
}

fn process_row(
    s: &AppState,
    ink: &Ink,
    p: &ProcessInfo,
    sort: ProcessSort,
    scale: f64,
    palette: Palette,
) -> Div {
    h_flex()
        .h(px(ROW))
        .w_full()
        .items_center()
        .gap(px(8.))
        .child(
            label(p.name.clone(), 12., ink.fg)
                .flex_1()
                .min_w_0()
                .truncate(),
        )
        .child(div().w(px(52.)).h(px(6.)).child(widgets::meter(
            (process_value(p, sort) / scale) as f32,
            10,
            1.5,
            palette,
        )))
        .child(widgets::tabular(
            label(
                process_text(s, p, sort),
                12.,
                // 没有占用的行调暗，眼睛才会落在真正在干活的进程上。
                if process_value(p, sort) > 0. {
                    ink.fg
                } else {
                    ink.muted
                },
            )
            .w(px(66.))
            .font_medium()
            .text_right(),
        ))
}

/// 进程数据还没到时占位的一行：骨架和真正的一行等高，数据一到不会跳。
fn skeleton_row(ink: &Ink, index: usize, palette: Palette) -> Div {
    // 名字的长短错落一些，看上去像排好的版。
    const NAMES: [f32; 8] = [104., 72., 124., 88., 112., 64., 96., 80.];
    let bar = |width: f32| div().h(px(8.)).w(px(width)).rounded(px(4.)).bg(ink.chip);
    h_flex()
        .h(px(ROW))
        .w_full()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().child(bar(NAMES[index % NAMES.len()])))
        .child(
            div()
                .w(px(52.))
                .h(px(6.))
                .child(widgets::meter(0., 10, 1.5, palette)),
        )
        .child(div().w(px(66.)).flex().justify_end().child(bar(38.)))
}

/// 进程排行，固定占 `rows` 行，不够的留白，这样高度不随进程数变化。
fn process_list(s: &AppState, ink: &Ink, sort: ProcessSort, rows: usize, cx: &App) -> Div {
    let ranked = data::ranked_processes(&s.processes, sort, true, rows);
    let scale = process_scale(&ranked, sort);
    let palette = ink.palette(cx).plain();
    let loading = s.process_rev == 0;
    let mut list = v_flex().w_full().h(px(rows as f32 * ROW));
    for index in 0..rows {
        list = list.child(match ranked.get(index) {
            Some(p) => process_row(s, ink, p, sort, scale, palette),
            None if loading => skeleton_row(ink, index, palette),
            None if index == 0 => h_flex().h(px(ROW)).items_center().child(label(
                t!("proc_empty").into_owned(),
                12.,
                ink.muted,
            )),
            None => div().h(px(ROW)),
        });
    }
    list
}

fn processes(
    s: &AppState,
    ink: &Ink,
    sort: ProcessSort,
    side: Side,
    rows: usize,
    cx: &App,
) -> Block {
    section(
        ink,
        t!("bar_top_processes").into_owned(),
        all_hint(side, s.processes.len()),
        rows as f32 * ROW,
        process_list(s, ink, sort, rows, cx),
    )
    .trigger(Flyout::Processes(sort))
}

/// 一行「名称 ……… 数值」加下面一条格子，自带下方间距。
fn meter_row(ink: &Ink, name: String, value: String, fraction: f32, palette: Palette) -> Div {
    v_flex()
        .w_full()
        .h(px(METER_ROW))
        .gap(px(6.))
        .child(
            h_flex()
                .justify_between()
                .items_baseline()
                .gap(px(10.))
                .child(label(name, 12., ink.fg).min_w_0().truncate())
                .child(widgets::tabular(
                    label(value, 11., ink.muted).flex_shrink_0(),
                )),
        )
        .child(
            div()
                .w_full()
                .h(px(7.))
                .child(widgets::meter(fraction, 44, 1.5, palette)),
        )
}
fn volumes(s: &AppState) -> Vec<&StorageInfo> {
    let mut disks: Vec<_> = s
        .snapshot
        .storages
        .iter()
        .filter(|d| d.total_bytes > 0)
        .collect();
    if let Some(system) = data::system_volume(&s.snapshot) {
        disks.sort_by_key(|d| !std::ptr::eq(*d, system));
    }
    disks
}

/// 卷的名字：优先用「Macintosh HD」这样的卷标，没有再退回挂载点（`C:\`、`/`）。
fn volume_name(disk: &StorageInfo) -> String {
    if disk.name.trim().is_empty() {
        disk.mount_point.clone().unwrap_or_default()
    } else {
        disk.name.clone()
    }
}

fn free_of(s: &AppState, disk: &StorageInfo) -> String {
    t!(
        "bar_free_of",
        free = s
            .settings
            .bytes(disk.total_bytes.saturating_sub(disk.used_bytes)),
        total = s.settings.bytes(disk.total_bytes)
    )
    .into_owned()
}

fn volume_rows(s: &AppState, ink: &Ink, disks: &[&StorageInfo], cx: &App) -> Vec<Div> {
    let palette = ink.palette(cx);
    disks
        .iter()
        .map(|d| {
            meter_row(
                ink,
                volume_name(d),
                free_of(s, d),
                fmt::ratio(d.used_bytes, d.total_bytes) / 100.,
                palette,
            )
        })
        .collect()
}

/// 有读数的温度传感器，由热到冷。
fn temperatures(s: &AppState) -> Vec<&SensorReading> {
    let mut list: Vec<&SensorReading> =
        match s.latest.as_ref().filter(|t| !t.temperatures.is_empty()) {
            Some(tick) => tick.temperatures.iter().collect(),
            None => s
                .snapshot
                .sensors
                .iter()
                .filter(|r| r.kind == "temperature")
                .collect(),
        };
    list.retain(|r| r.value.is_finite());
    list.sort_by(|a, b| b.value.total_cmp(&a.value));
    list
}

fn sensor_rows(s: &AppState, ink: &Ink, list: &[&SensorReading], cx: &App) -> Vec<Div> {
    let palette = ink.temperature_palette(cx);
    list.iter()
        .map(|r| {
            let tone = ink.tone(health::temperature_level(r.value));
            h_flex()
                .h(px(ROW))
                .w_full()
                .items_center()
                .gap(px(8.))
                .child(
                    label(r.label.clone(), 12., ink.fg)
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                )
                .child(div().w(px(72.)).h(px(6.)).child(widgets::meter(
                    (r.value - TEMP_FLOOR) / TEMP_SPAN,
                    14,
                    1.5,
                    palette,
                )))
                .child(widgets::tabular(
                    label(s.settings.temp(r.value), 12., tone)
                        .w(px(52.))
                        .font_medium()
                        .text_right(),
                ))
        })
        .collect()
}

struct Link<'a> {
    name: &'a str,
    kind: &'a str,
    ipv4: Option<&'a str>,
    rx: u64,
    tx: u64,
}

impl Link<'_> {
    /// 接口类型的名字，不认识的类型就直接用接口名。
    fn kind_label(&self) -> String {
        match self.kind {
            "wifi" => t!("net_kind_wifi"),
            "ethernet" => t!("net_kind_ethernet"),
            "bluetooth" => t!("net_kind_bluetooth"),
            "virtual" => t!("net_kind_virtual"),
            _ => return self.name.to_string(),
        }
        .into_owned()
    }

    /// 真正连着外面的接口排在前面：开着 VPN 时，流量最大的是隧道，但用户要看的是 Wi-Fi / 网线。
    fn rank(&self) -> u8 {
        match self.kind {
            "wifi" | "ethernet" => 0,
            "virtual" => 2,
            _ => 1,
        }
    }
}

/// 正在用的网络接口：不含回环地址，也不含没有地址、没有流量的虚拟接口（`awdl0`、`utun0` 这类系统自带的）。
fn interfaces(s: &AppState) -> Vec<Link<'_>> {
    let Some(tick) = s.latest.as_ref() else {
        return Vec::new();
    };
    let mut links: Vec<_> = tick
        .per_interface
        .iter()
        .filter_map(|net| {
            let info = s
                .snapshot
                .network
                .interfaces
                .iter()
                .find(|i| i.name == net.name);
            if info.is_some_and(|i| i.is_loopback || !i.is_up) {
                return None;
            }
            let link = Link {
                name: &net.name,
                kind: info.map_or("other", |i| i.kind.as_str()),
                ipv4: info.and_then(|i| i.ipv4.first()).map(String::as_str),
                rx: net.rx_bps,
                tx: net.tx_bps,
            };
            let idle_virtual = link.rank() == 2 && link.ipv4.is_none() && link.rx + link.tx == 0;
            (!idle_virtual).then_some(link)
        })
        .collect();
    links.sort_by(|a, b| {
        a.rank()
            .cmp(&b.rank())
            .then_with(|| b.rx.saturating_add(b.tx).cmp(&a.rx.saturating_add(a.tx)))
            .then_with(|| fmt::natural_cmp(a.name, b.name))
    });
    links
}

fn link_rows(s: &AppState, ink: &Ink, links: &[Link], count: usize) -> Vec<Div> {
    links
        .iter()
        .take(count)
        .map(|link| {
            let detail = match link.ipv4 {
                Some(ip) => format!("{} · {ip}", link.kind_label()),
                None => link.kind_label(),
            };
            h_flex()
                .w_full()
                .h(px(LINK_ROW))
                .items_center()
                .justify_between()
                .gap(px(10.))
                .child(
                    v_flex()
                        .min_w_0()
                        .child(label(link.name.to_string(), 12., ink.fg).truncate())
                        .child(label(detail, 10.5, ink.muted).truncate()),
                )
                .child(widgets::tabular(
                    v_flex()
                        .flex_shrink_0()
                        .items_end()
                        .child(label(
                            format!("↓ {}", s.settings.rate(link.rx)),
                            11.,
                            ink.cyan,
                        ))
                        .child(label(
                            format!("↑ {}", s.settings.rate(link.tx)),
                            11.,
                            ink.magenta,
                        )),
                ))
        })
        .collect()
}

/// 每块显卡一行，连同这些行加起来的高度；没有显存数据（比如统一内存）的不留那一行字的位置。
fn gpu_rows(s: &AppState, ink: &Ink, palette: Palette, tick: &MonitorTick) -> (Vec<Div>, f32) {
    let (rows, heights): (Vec<_>, Vec<_>) = tick
        .gpus
        .iter()
        .take(2)
        .map(|gpu| {
            let vram = match (gpu.vram_used_bytes, gpu.vram_total_bytes) {
                (Some(used), Some(total)) if total > 0 => {
                    format!(
                        "VRAM {} / {}",
                        s.settings.bytes(used),
                        s.settings.bytes(total)
                    )
                }
                (Some(used), _) => format!("VRAM {}", s.settings.bytes(used)),
                _ => String::new(),
            };
            let height = if vram.is_empty() { METER_ROW } else { GPU_ROW };
            let row = v_flex().w_full().h(px(height)).child(meter_row(
                ink,
                gpu.name.clone(),
                gpu.utilization.map_or("—".into(), fmt::percent),
                gpu.utilization.map_or(0., |u| u / 100.),
                palette,
            ));
            let row = if vram.is_empty() {
                row
            } else {
                row.child(label(vram, 10.5, ink.muted))
            };
            (row, height)
        })
        .unzip();
    (rows, heights.iter().sum())
}

/// 头部副标题。
pub fn subtitle(s: &AppState, metric: Metric) -> String {
    let settings = &s.settings;
    match metric {
        Metric::Cpu => {
            let brand = &s.snapshot.cpu.brand;
            if brand.is_empty() {
                t!("nav_cpu").into_owned()
            } else {
                brand.clone()
            }
        }
        Metric::Memory => s.latest.as_ref().map_or_else(
            || t!("nav_memory").into_owned(),
            |t| settings.bytes(t.mem_total_bytes),
        ),
        Metric::Storage => data::system_volume(&s.snapshot).map_or_else(
            || t!("nav_storage").into_owned(),
            |d| format!("{} · {}", volume_name(d), settings.bytes(d.total_bytes)),
        ),
        Metric::Temperature => t!("bar_sensor_count", n = temperatures(s).len()).into_owned(),
        Metric::Network => interfaces(s).first().map_or_else(
            || t!("nav_network").into_owned(),
            |l| match l.ipv4 {
                Some(ip) => format!("{} · {ip}", l.kind_label()),
                None => l.kind_label(),
            },
        ),
        Metric::Gpu => s
            .snapshot
            .gpus
            .first()
            .map_or_else(|| t!("nav_gpu").into_owned(), |g| g.name.clone()),
        Metric::Battery => s.snapshot.battery.as_ref().map_or_else(
            || t!("nav_battery").into_owned(),
            |b| battery_state(&b.state),
        ),
    }
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

/// 每个核心一根竖条，核心不多时在下面标上序号。
fn cores(ink: &Ink, usage: &[f32], cx: &App) -> Block {
    let count = usage.len();
    let gap = if count > 16 { 2. } else { 4. };
    let width = (INNER - gap * (count - 1) as f32) / count as f32;
    let values = usage.iter().map(|v| Some(v / 100.)).collect();
    let numbers = (count <= 16).then(|| {
        h_flex()
            .gap(px(gap))
            .pt(px(4.))
            .children((1..=count).map(|n| {
                label(n.to_string(), 9., ink.muted)
                    .w(px(width))
                    .text_center()
            }))
    });
    let body = 44. + if numbers.is_some() { 17. } else { 0. };
    section(
        ink,
        t!("cpu_per_core").into_owned(),
        None,
        body,
        v_flex()
            .child(div().w_full().h(px(44.)).child(widgets::columns(
                values,
                count,
                8,
                (gap, 2.),
                ink.palette(cx),
            )))
            .children(numbers),
    )
}

/// 体温计图标的颜色：正常时用温度这个指标的黄色，偏高变橙，过热变红。
fn heat(ink: &Ink, celsius: f32) -> Hsla {
    match health::temperature_level(celsius) {
        Level::Healthy => ink.yellow,
        Level::Warning => ink.warning,
        Level::Critical => ink.danger,
    }
}

/// 面板主体：从上到下的各个块。`rows` 是进程排行的行数。
pub fn blocks(s: &AppState, metric: Metric, side: Side, rows: usize, cx: &App) -> Vec<Block> {
    let ink = Ink::new(metric, cx);
    let tick = s.latest.as_ref();
    let settings = &s.settings;
    let reading = data::reading(metric, Show::Auto, tick, &s.snapshot, settings);
    let number = reading.number;
    let mut out = Vec::new();
    match metric {
        Metric::Cpu => {
            let cpu = &s.snapshot.cpu;
            let mut chips = Vec::new();
            if let Some(temp) = tick.and_then(MonitorTick::cpu_temperature) {
                chips.push(Chip::new(
                    IconName::Thermometer,
                    heat(&ink, temp),
                    settings.temp(temp),
                ));
            }
            if cpu.current_frequency_hz > 0 {
                chips.push(Chip::new(
                    IconName::Gauge,
                    ink.muted,
                    fmt::freq(cpu.current_frequency_hz),
                ));
            }
            out.push(hero(
                &ink,
                usage_tone(&ink, number),
                &reading.value,
                fmt::core_summary(cpu),
                chips,
            ));
            out.push(usage_chart(s, &ink, metric, cx));
            if let Some(tick) = tick.filter(|t| !t.cpu_per_core.is_empty()) {
                out.push(cores(&ink, &tick.cpu_per_core, cx));
            }
            out.push(processes(s, &ink, ProcessSort::Cpu, side, rows, cx));
        }
        Metric::Memory => {
            let mut chips = Vec::new();
            let mut caption = String::new();
            if let Some(t) = tick.filter(|t| t.mem_total_bytes > 0) {
                caption = t!(
                    "bar_free_of",
                    free = settings.bytes(t.mem_total_bytes.saturating_sub(t.mem_used_bytes)),
                    total = settings.bytes(t.mem_total_bytes)
                )
                .into_owned();
                if t.swap_used_bytes > 0 {
                    chips.push(Chip::new(
                        IconName::ArrowLeftRight,
                        ink.muted,
                        format!("Swap {}", settings.bytes(t.swap_used_bytes)),
                    ));
                }
            }
            out.push(hero(
                &ink,
                usage_tone(&ink, number),
                &reading.value,
                caption,
                chips,
            ));
            out.push(usage_chart(s, &ink, metric, cx));
            if let Some(t) = tick.filter(|t| t.mem_total_bytes > 0) {
                let palette = ink.palette(cx);
                let mut list = vec![meter_row(
                    &ink,
                    t!("spec_used").into_owned(),
                    format!(
                        "{} / {}",
                        settings.bytes(t.mem_used_bytes),
                        settings.bytes(t.mem_total_bytes)
                    ),
                    fmt::ratio(t.mem_used_bytes, t.mem_total_bytes) / 100.,
                    palette,
                )];
                if t.swap_total_bytes > 0 {
                    list.push(meter_row(
                        &ink,
                        t!("mem_swap").into_owned(),
                        format!(
                            "{} / {}",
                            settings.bytes(t.swap_used_bytes),
                            settings.bytes(t.swap_total_bytes)
                        ),
                        fmt::ratio(t.swap_used_bytes, t.swap_total_bytes) / 100.,
                        palette,
                    ));
                }
                out.push(section(
                    &ink,
                    t!("nav_memory").into_owned(),
                    None,
                    list.len() as f32 * METER_ROW,
                    v_flex().children(list),
                ));
            }
            out.push(processes(s, &ink, ProcessSort::Memory, side, rows, cx));
        }
        Metric::Storage => {
            let mut chips = Vec::new();
            if let Some(t) = tick {
                chips.push(Chip::new(
                    IconName::ArrowDown,
                    ink.accent,
                    settings.rate(t.disk_read_bps),
                ));
                chips.push(Chip::new(
                    IconName::ArrowUp,
                    ink.yellow,
                    settings.rate(t.disk_write_bps),
                ));
            }
            let caption = data::system_volume(&s.snapshot)
                .map(|d| free_of(s, d))
                .unwrap_or_default();
            out.push(hero(
                &ink,
                usage_tone(&ink, number),
                &reading.value,
                caption,
                chips,
            ));
            let reads = super::rates(s, COLS, |sample| sample.disk_read);
            let writes = super::rates(s, COLS, |sample| sample.disk_write);
            out.push(chart(
                s,
                &ink,
                Trend {
                    axis: false,
                    ..rate_trend(
                        s,
                        format!("↓ {}", t!("bar_read")),
                        ink.accent,
                        reads,
                        DISK_FLOOR,
                        cx,
                    )
                },
            ));
            out.push(chart(
                s,
                &ink,
                rate_trend(
                    s,
                    format!("↑ {}", t!("bar_write")),
                    ink.yellow,
                    writes,
                    DISK_FLOOR,
                    cx,
                ),
            ));
            let disks = volumes(s);
            if !disks.is_empty() {
                let shown = disks.len().min(3);
                let more = disks.len() > shown;
                let block = section(
                    &ink,
                    t!("bar_volumes").into_owned(),
                    more.then(|| all_hint(side, disks.len())).flatten(),
                    shown as f32 * METER_ROW,
                    v_flex().children(volume_rows(s, &ink, &disks[..shown], cx)),
                );
                out.push(if more {
                    block.trigger(Flyout::Volumes)
                } else {
                    block
                });
            }
            out.push(processes(s, &ink, ProcessSort::Io, side, rows, cx));
        }
        Metric::Temperature => {
            let list = temperatures(s);
            let mut chips = Vec::new();
            if let Some(max) = tick.and_then(MonitorTick::max_temperature)
                && number.is_none_or(|n| (n - max as f64).abs() > 0.5)
            {
                chips.push(Chip::new(
                    IconName::Flame,
                    heat(&ink, max),
                    format!("{} {}", t!("bar_max"), settings.temp(max)),
                ));
            }
            let tone = number.map_or(Level::Healthy, |n| health::temperature_level(n as f32));
            out.push(hero(
                &ink,
                ink.tone(tone),
                &reading.value,
                t!("bar_show_cpu_temp").into_owned(),
                chips,
            ));
            let series = super::history(s, metric, COLS);
            let known: Vec<f64> = series.iter().flatten().copied().collect();
            out.push(chart(
                s,
                &ink,
                Trend {
                    title: t!("spec_temp").into_owned(),
                    tint: ink.muted,
                    summary: summary(&known, |v| settings.temp(v as f32)),
                    values: scaled(series, TEMP_FLOOR as f64, TEMP_SPAN as f64),
                    rows: 10,
                    height: 64.,
                    palette: ink.temperature_palette(cx),
                    axis: true,
                },
            ));
            if list.is_empty() {
                out.push(section(
                    &ink,
                    t!("bar_sensors").into_owned(),
                    None,
                    ROW,
                    label(t!("bar_no_sensor").into_owned(), 12., ink.muted),
                ));
            } else {
                let shown = list.len().min(5);
                let more = list.len() > shown;
                let block = section(
                    &ink,
                    t!("bar_sensors").into_owned(),
                    more.then(|| all_hint(side, list.len())).flatten(),
                    shown as f32 * ROW,
                    v_flex().children(sensor_rows(s, &ink, &list[..shown], cx)),
                );
                out.push(if more {
                    block.trigger(Flyout::Sensors)
                } else {
                    block
                });
            }
        }
        Metric::Network => {
            let (down, up) = match tick {
                Some(t) => (settings.rate(t.net_rx_bps), settings.rate(t.net_tx_bps)),
                None => ("—".into(), "—".into()),
            };
            let column = |arrow: &str, caption: String, value: String, color: Hsla| {
                let (num, unit) = value.split_once(' ').unwrap_or((&value, ""));
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(label(format!("{arrow} {caption}"), 11., ink.muted))
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap(px(3.))
                            .child(widgets::tabular(
                                div()
                                    .text_size(px(28.))
                                    .line_height(px(32.))
                                    .font_semibold()
                                    .text_color(color)
                                    .child(num.to_string()),
                            ))
                            .child(label(unit.to_string(), 12., ink.muted).font_semibold()),
                    )
            };
            out.push(Block::new(
                76.,
                h_flex()
                    .size_full()
                    .items_center()
                    .gap(px(12.))
                    .child(column("↓", t!("bar_download").into_owned(), down, ink.cyan))
                    .child(column("↑", t!("bar_upload").into_owned(), up, ink.magenta)),
            ));
            let downs = super::rates(s, COLS, |sample| sample.net_down);
            let ups = super::rates(s, COLS, |sample| sample.net_up);
            out.push(chart(
                s,
                &ink,
                Trend {
                    rows: 7,
                    height: 44.,
                    axis: false,
                    ..rate_trend(
                        s,
                        format!("↓ {}", t!("bar_download")),
                        ink.cyan,
                        downs,
                        NET_FLOOR,
                        cx,
                    )
                },
            ));
            out.push(chart(
                s,
                &ink,
                Trend {
                    rows: 7,
                    height: 44.,
                    ..rate_trend(
                        s,
                        format!("↑ {}", t!("bar_upload")),
                        ink.magenta,
                        ups,
                        NET_FLOOR,
                        cx,
                    )
                },
            ));
            let links = interfaces(s);
            if !links.is_empty() {
                let shown = links.len().min(3);
                let more = links.len() > shown;
                let block = section(
                    &ink,
                    t!("net_interfaces").into_owned(),
                    more.then(|| all_hint(side, links.len())).flatten(),
                    shown as f32 * LINK_ROW,
                    v_flex().children(link_rows(s, &ink, &links, shown)),
                );
                out.push(if more {
                    block.trigger(Flyout::Interfaces)
                } else {
                    block
                });
            }
        }
        Metric::Gpu => {
            let info = s.snapshot.gpus.first();
            let mut chips = Vec::new();
            if let Some(temp) = info.and_then(|g| g.temperature_c) {
                chips.push(Chip::new(
                    IconName::Thermometer,
                    heat(&ink, temp),
                    settings.temp(temp),
                ));
            }
            if let Some(power) = info.and_then(|g| g.power_w) {
                chips.push(Chip::new(IconName::Zap, ink.muted, format!("{power:.1} W")));
            }
            let caption = tick
                .and_then(|t| t.gpus.first())
                .and_then(|g| g.vram_used_bytes)
                .map_or_else(String::new, |used| format!("VRAM {}", settings.bytes(used)));
            out.push(hero(
                &ink,
                usage_tone(&ink, number),
                &reading.value,
                caption,
                chips,
            ));
            out.push(usage_chart(s, &ink, metric, cx));
            let (list, height) = tick
                .map(|t| gpu_rows(s, &ink, ink.palette(cx), t))
                .unwrap_or_default();
            if list.is_empty() {
                out.push(section(
                    &ink,
                    t!("nav_gpu").into_owned(),
                    None,
                    ROW,
                    label(t!("bar_unavailable").into_owned(), 12., ink.muted),
                ));
            } else {
                out.push(section(
                    &ink,
                    t!("nav_gpu").into_owned(),
                    None,
                    height,
                    v_flex().children(list),
                ));
            }
        }
        Metric::Battery => {
            let Some(battery) = s.snapshot.battery.as_ref() else {
                out.push(Block::new(
                    150.,
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap(px(8.))
                        .child(
                            Icon::new(IconName::BatteryWarning)
                                .size(px(28.))
                                .text_color(ink.muted),
                        )
                        .child(label(t!("bat_no_battery").into_owned(), 13., ink.fg))
                        .child(label(
                            t!("bat_no_battery_hint").into_owned(),
                            11.,
                            ink.muted,
                        )),
                ));
                return out;
            };
            let tone = ink.tone(health::battery_level(battery.percentage));
            let eta = battery
                .time_to_empty_secs
                .map(|secs| (t!("bat_time_to_empty"), secs))
                .or(battery
                    .time_to_full_secs
                    .map(|secs| (t!("bat_time_to_full"), secs)))
                .filter(|(_, secs)| *secs > 0)
                .map(|(name, secs)| format!(" · {name} {}", fmt::duration(secs)))
                .unwrap_or_default();
            let mut chips = Vec::new();
            if let Some(mw) = battery.power_now_mw {
                chips.push(Chip::new(
                    IconName::Zap,
                    ink.muted,
                    format!("{:.1} W", mw.unsigned_abs() as f64 / 1000.),
                ));
            }
            out.push(hero(
                &ink,
                tone,
                &reading.value,
                format!("{}{eta}", battery_state(&battery.state)),
                chips,
            ));
            out.push(Block::new(
                14. + GAP + 10.,
                div().w_full().h(px(14.)).child(widgets::meter(
                    battery.percentage / 100.,
                    40,
                    2.,
                    Palette::new(tone, cx).plain(),
                )),
            ));
            let health = match (battery.full_capacity_mwh, battery.design_capacity_mwh) {
                (Some(full), Some(design)) if design > 0 => {
                    Some(full as f32 / design as f32 * 100.)
                }
                _ => None,
            };
            let stat = |name: String, value: Option<String>| {
                v_flex()
                    .w(px((INNER - 12.) / 2.))
                    .h(px(42.))
                    .gap(px(1.))
                    .child(label(name, 10.5, ink.muted))
                    .child(widgets::tabular(
                        label(value.unwrap_or_else(|| "—".into()), 13., ink.fg).font_semibold(),
                    ))
            };
            let cells = vec![
                stat(t!("bat_health").into_owned(), health.map(fmt::percent)),
                stat(
                    t!("bat_cycle").into_owned(),
                    battery.cycle_count.map(|c| c.to_string()),
                ),
                stat(
                    t!("spec_temp").into_owned(),
                    battery.temperature_c.map(|c| settings.temp(c)),
                ),
                stat(
                    t!("bat_full_cap").into_owned(),
                    battery.full_capacity_mwh.map(fmt::watt_hours),
                ),
            ];
            out.push(section(
                &ink,
                t!("bar_details").into_owned(),
                None,
                2. * 42.,
                h_flex().flex_wrap().gap_x(px(12.)).children(cells),
            ));
        }
    }
    out
}

/// 二级页面的四周留白：上边框和内边距、标题、下方留白。
const FLYOUT_CHROME: f32 = 2. + 8. + TITLE + GAP + 4.;

/// 二级页面：宽度、高度和内容。`max_height` 是所在显示器能容纳的最大高度。
pub fn flyout(
    s: &AppState,
    metric: Metric,
    kind: Flyout,
    max_height: f32,
    cx: &App,
) -> (f32, f32, AnyElement) {
    let ink = Ink::new(metric, cx);
    // 最多能放几行：列表太长时按屏幕高度截断，而不是让窗口超出屏幕。
    let fit = |row: f32, extra: f32, wanted: usize| {
        let room = ((max_height - FLYOUT_CHROME - extra) / row).floor().max(1.) as usize;
        wanted.min(room)
    };
    let (title, rows, body) = match kind {
        Flyout::Processes(sort) => {
            let value = match sort {
                ProcessSort::Cpu => t!("proc_col_cpu"),
                ProcessSort::Memory => t!("proc_col_mem"),
                ProcessSort::Io => "I/O".into(),
            };
            let header = h_flex()
                .h(px(20.))
                .w_full()
                .items_center()
                .gap(px(8.))
                .text_size(px(10.5))
                .font_semibold()
                .text_color(ink.muted)
                .child(div().flex_1().child(t!("proc_col_name").into_owned()))
                .child(div().w(px(52.)))
                .child(div().w(px(66.)).text_right().child(value.into_owned()));
            let count = fit(ROW, 20., FLYOUT_ROWS);
            (
                t!("proc_title").into_owned(),
                20. + count as f32 * ROW,
                v_flex()
                    .child(header)
                    .child(process_list(s, &ink, sort, count, cx)),
            )
        }
        Flyout::Volumes => {
            let disks = volumes(s);
            let shown = disks.len().min(fit(METER_ROW, 0., 8));
            (
                t!("bar_volumes").into_owned(),
                shown as f32 * METER_ROW,
                v_flex().children(volume_rows(s, &ink, &disks[..shown], cx)),
            )
        }
        Flyout::Sensors => {
            let list = temperatures(s);
            let shown = list.len().min(fit(ROW, 0., 16));
            (
                t!("bar_sensors").into_owned(),
                shown as f32 * ROW,
                v_flex().children(sensor_rows(s, &ink, &list[..shown], cx)),
            )
        }
        Flyout::Interfaces => {
            let links = interfaces(s);
            let shown = links.len().min(fit(LINK_ROW, 0., 8));
            (
                t!("net_interfaces").into_owned(),
                shown as f32 * LINK_ROW,
                v_flex().children(link_rows(s, &ink, &links, shown)),
            )
        }
    };
    (
        WIDTH,
        FLYOUT_CHROME + rows,
        v_flex()
            .size_full()
            .px(px(PAD))
            .pt(px(8.))
            .child(title_row(&ink, title, None))
            .child(div().w_full().h(px(rows)).child(body))
            .into_any_element(),
    )
}

//! 监控面板里每个指标的内容。
//!
//! 一个指标就是一摞卡片：先是带环形仪表的概览，再是走势图、分项和进程排行。卡片的外框（底色、
//! 边框、悬停效果）由 `panel` 统一画，这里只管框里的东西。每一块都带着自己的高度，所有块的高度
//! 加起来就是面板的高度，所以面板既不需要滚动，也不需要先排版再量尺寸。
use super::{
    COLUMNS,
    archive::Series,
    color,
    config::{Metric, Show, TimeRange},
    data::{self, Channel, ProcessSort, Value},
    series,
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
    AnyElement, App, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px, relative,
};
use pc_specs_core::model::{GpuTick, MonitorTick, ProcessInfo, SensorReading, StorageInfo};
use rust_i18n::t;

pub const WIDTH: f32 = 340.;
/// 卡片和面板左右边缘的距离。
pub const PAD: f32 = 10.;
/// 卡片之间的间距；第一张卡片和头部、最后一张卡片和底栏之间也留这么多。
pub const SPACE: f32 = 8.;
pub const RADIUS: f32 = 10.;
/// 卡片自己的上下边框（各 1px）。
const BORDER: f32 = 2.;
/// 卡片内边距：左右、标题上方、内容下方。
const CARD_X: f32 = 12.;
const CARD_TOP: f32 = 9.;
const CARD_BOTTOM: f32 = 10.;
/// 卡片标题行的高度，以及它和内容之间的空隙。
const HEAD: f32 = 18.;
const HEAD_GAP: f32 = 7.;
/// 去掉面板边框、卡片边距、卡片边框和内边距之后，内容实际可用的宽度。
const INNER: f32 = WIDTH - 2. - 2. * PAD - BORDER - 2. * CARD_X;

/// 概览卡片里的环形仪表：直径、环宽，以及右边每行读数、说明文字的高度。
const GAUGE: f32 = 76.;
const RING: f32 = 8.;
const STAT: f32 = 19.;
const CAPTION: f32 = 14.;

/// 排行、传感器这类单行列表的行高。
const ROW: f32 = 22.;
/// 「名称 + 一条格子」这一项的高度。
const METER: f32 = 27.;
/// 「接口名 + 说明 | 上下行」这一项的高度。
const LINK: f32 = 30.;
/// 两个分项之间的空隙。
const LIST_GAP: f32 = 8.;
/// 进程排行里「读 / 写」列头的高度。
const PROC_HEAD: f32 = 16.;

/// 走势图：单条曲线的高度、对称图（上行 / 下行）每一半的高度，以及横轴。
const CHART: f32 = 56.;
const LANE: f32 = 28.;
const LANE_GAP: f32 = 2.;
const AXIS: f32 = 13.;
const AXIS_GAP: f32 = 5.;
/// 逐核心占用的柱子高度。
const CORES: f32 = 40.;

/// 温度走势和温度条的纵轴，和菜单栏上的走势用同一个量程。
const TEMP_FLOOR: f32 = data::CELSIUS_FLOOR as f32;
const TEMP_SPAN: f32 = data::CELSIUS_SPAN as f32;
/// 两条曲线排在一起时，速率低于这个值就当作没有流量，免得把杂音放大成满格。
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
    /// 含边框的卡片高度。
    pub height: f32,
    pub element: AnyElement,
    pub trigger: Option<Flyout>,
}

impl Block {
    /// 一张卡片。`inner` 是框里面（不含 1px 边框）的高度。
    fn card(inner: f32, element: impl IntoElement) -> Self {
        Self {
            height: inner + BORDER,
            element: div().size_full().child(element).into_any_element(),
            trigger: None,
        }
    }

    fn trigger(mut self, flyout: Flyout) -> Self {
        self.trigger = Some(flyout);
        self
    }
}

/// 卡片的外框：底色、边框和圆角。内容由各个 `Block` 提供；悬停、选中这类状态由用到它的地方叠加。
pub fn frame(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .mx(px(PAD))
        .rounded(px(RADIUS))
        .border_1()
        .border_color(theme.foreground.opacity(0.08))
        .bg(theme.foreground.opacity(0.04))
        .overflow_hidden()
}

/// 面板里用到的几种颜色，按指标取主色。
#[derive(Clone, Copy)]
struct Ink {
    fg: Hsla,
    muted: Hsla,
    /// 控件和占位的浅底色。
    chip: Hsla,
    /// 控件里被选中的那一格。
    chosen: Hsla,
    /// 环形仪表和柱子的底轨。
    track: Hsla,
    /// 图例里「剩余」的那个点。
    faint: Hsla,
    /// 盖在图上的文字的底。
    veil: Hsla,
    /// 卡片里的分隔线。
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
            chosen: theme.foreground.opacity(0.14),
            track: theme.foreground.opacity(0.1),
            faint: theme.foreground.opacity(0.3),
            veil: theme.popover.opacity(0.88),
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

fn usage_tone(ink: &Ink, percent: Option<f64>) -> Hsla {
    ink.tone(percent.map_or(Level::Healthy, |n| health::usage_level(n as f32)))
}

/// 体温计图标的颜色：正常时用温度这个指标的黄色，偏高变橙，过热变红。
fn heat(ink: &Ink, celsius: f32) -> Hsla {
    match health::temperature_level(celsius) {
        Level::Healthy => ink.yellow,
        Level::Warning => ink.warning,
        Level::Critical => ink.danger,
    }
}

// ---- 卡片 -----------------------------------------------------------------

fn title_text(ink: &Ink, text: impl Into<String>) -> Div {
    div()
        .text_size(px(10.5))
        .font_semibold()
        .text_color(ink.muted)
        .child(text.into().to_uppercase())
}

/// 标题行：左边标题，右边放切换控件或「全部 N」。
fn head(title: impl IntoElement, aside: Option<AnyElement>) -> Div {
    h_flex()
        .h(px(HEAD))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .child(title)
        .children(aside)
}

/// 带标题的卡片。`body_height` 是内容本身的高度。
fn card(
    title: impl IntoElement,
    aside: Option<AnyElement>,
    body_height: f32,
    body: impl IntoElement,
) -> Block {
    Block::card(
        CARD_TOP + HEAD + HEAD_GAP + body_height + CARD_BOTTOM,
        v_flex()
            .size_full()
            .px(px(CARD_X))
            .pt(px(CARD_TOP))
            .gap(px(HEAD_GAP))
            .child(head(title, aside))
            .child(
                div()
                    .w_full()
                    .h(px(body_height))
                    .flex_shrink_0()
                    .child(body),
            ),
    )
}

/// 标题右边的「全部 N」，提示鼠标悬停就能在旁边展开完整列表。
fn more(ink: &Ink, side: Side, count: usize) -> AnyElement {
    let text = if count > 0 {
        t!("bar_all_count", n = count)
    } else {
        t!("bar_all")
    };
    let chevron = |name| Icon::new(name).size(px(11.)).text_color(ink.muted);
    h_flex()
        .items_center()
        .gap(px(1.))
        .text_size(px(10.5))
        .text_color(ink.muted)
        .when(side == Side::Left, |this| {
            this.child(chevron(IconName::ChevronLeft))
        })
        .child(text.into_owned())
        .when(side == Side::Right, |this| {
            this.child(chevron(IconName::ChevronRight))
        })
        .into_any_element()
}

/// 一组分项：每项固定高度，项与项之间留 `LIST_GAP`。
fn stack(count: usize, item: f32) -> f32 {
    count as f32 * item + count.saturating_sub(1) as f32 * LIST_GAP
}

// ---- 概览：环形仪表 + 几行读数 --------------------------------------------------

/// 读数行开头的标记：圆点对应仪表上的颜色，图标说明这是什么。
enum Lead {
    Dot(Hsla),
    Icon(IconName, Hsla),
}

/// 概览里的一行读数。
struct Stat {
    lead: Lead,
    label: String,
    value: String,
    /// 数值的颜色，默认是前景色。
    tone: Option<Hsla>,
}

impl Stat {
    fn dot(tint: Hsla, label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            lead: Lead::Dot(tint),
            label: label.into(),
            value: value.into(),
            tone: None,
        }
    }

    fn icon(
        icon: IconName,
        tint: Hsla,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            lead: Lead::Icon(icon, tint),
            label: label.into(),
            value: value.into(),
            tone: None,
        }
    }
}

fn stat_row(ink: &Ink, stat: Stat) -> Div {
    let lead = match stat.lead {
        Lead::Dot(tint) => div()
            .size(px(7.))
            .rounded_full()
            .bg(tint)
            .into_any_element(),
        Lead::Icon(icon, tint) => Icon::new(icon)
            .size(px(12.))
            .text_color(tint)
            .into_any_element(),
    };
    h_flex()
        .h(px(STAT))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap(px(7.))
        .child(
            div()
                .w(px(12.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .child(lead),
        )
        .child(
            label(stat.label, 11.5, ink.muted)
                .flex_1()
                .min_w_0()
                .truncate(),
        )
        .child(widgets::tabular(
            label(stat.value, 12., stat.tone.unwrap_or(ink.fg))
                .flex_shrink_0()
                .font_semibold(),
        ))
}

/// 环形仪表，当前读数写在环中间。
fn gauge(ink: &Ink, fraction: f32, tone: Hsla, value: &Value) -> Div {
    let number = h_flex()
        .items_baseline()
        .gap(px(1.))
        .child(widgets::tabular(
            div()
                .text_size(px(24.))
                .line_height(px(28.))
                .font_semibold()
                .text_color(ink.fg)
                .child(format!("{}{}", value.lead, value.num)),
        ))
        .child(label(value.unit.clone(), 11., ink.muted).font_semibold());
    div()
        .relative()
        .size(px(GAUGE))
        .flex_shrink_0()
        .child(widgets::ring(fraction, GAUGE, RING, tone, ink.track))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(number),
        )
}

/// 每个指标最上面的概览：左边是环形仪表，右边是说明文字和最多三行读数。
fn hero(
    ink: &Ink,
    fraction: f32,
    tone: Hsla,
    value: &Value,
    caption: Option<String>,
    stats: Vec<Stat>,
) -> Block {
    let side = v_flex()
        .flex_1()
        .min_w_0()
        .h(px(GAUGE))
        .justify_center()
        .gap(px(2.))
        .children(caption.map(|text| {
            label(text, 11., ink.muted)
                .h(px(CAPTION))
                .flex_shrink_0()
                .truncate()
        }))
        .children(stats.into_iter().map(|stat| stat_row(ink, stat)));
    Block::card(
        CARD_TOP + GAUGE + CARD_TOP,
        h_flex()
            .size_full()
            .px(px(CARD_X))
            .items_center()
            .gap(px(16.))
            .child(gauge(ink, fraction, tone, value))
            .child(side),
    )
}

/// 把读数放到 0..=1 的纵轴上，没有读数当作 0。
fn fraction(number: Option<f64>, scale: data::Scale) -> f32 {
    number.map_or(0., |n| scale.fraction(n, &[]))
}

// ---- 走势图 ----------------------------------------------------------------

/// 一张走势图要放进卡片的东西。
struct Chart {
    title: AnyElement,
    graph: AnyElement,
    height: f32,
    /// 横轴中间的统计文字。
    summary: String,
    /// 这段时间里一个读数也没有。
    empty: bool,
}

/// 对称图里的一条：上行画在中线上面，下行画在下面。
struct Lane {
    arrow: &'static str,
    name: String,
    tint: Hsla,
    series: Series,
}

/// 把读数缩放到 0..=1。
fn scaled(values: &[Option<f64>], floor: f64, span: f64) -> Vec<Option<f32>> {
    values
        .iter()
        .map(|v| v.map(|v| ((v - floor) / span).clamp(0., 1.) as f32))
        .collect()
}

/// 右上角的切换控件：所有走势图共用这一个时间跨度。
fn range_switch(ink: &Ink, current: TimeRange) -> AnyElement {
    let fg = ink.fg;
    h_flex()
        .h(px(HEAD))
        .p(px(2.))
        .gap(px(1.))
        .items_center()
        .rounded(px(6.))
        .bg(ink.chip)
        .children(
            TimeRange::ALL
                .into_iter()
                .enumerate()
                .map(|(index, range)| {
                    let on = range == current;
                    div()
                        .id(("range", index))
                        .h_full()
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .text_size(px(10.))
                        .font_medium()
                        .when(on, |this| this.bg(ink.chosen).text_color(fg))
                        .when(!on, |this| {
                            this.text_color(ink.muted)
                                .cursor_pointer()
                                .hover(move |style| style.text_color(fg))
                                .on_click(move |_, _, cx| {
                                    AppState::global(cx)
                                        .update(cx, |s, cx| s.set_panel_range(range, cx));
                                })
                        })
                        .child(range.label())
                }),
        )
        .into_any_element()
}

/// 横轴最左边的文字：「1 分钟前」。
fn span_text(s: &AppState, range: TimeRange) -> String {
    let span = match range {
        TimeRange::Live => {
            let seconds = COLUMNS as u64 * s.settings.interval_ms / 1000;
            if seconds >= 120 {
                t!("unit_minutes", n = seconds / 60)
            } else {
                t!("unit_seconds", n = seconds)
            }
        }
        TimeRange::Hour => t!("unit_minutes", n = 60),
        TimeRange::Day => t!("unit_hours", n = 24),
        TimeRange::Week => t!("unit_days", n = 7),
        TimeRange::Month => t!("unit_days", n = 30),
    };
    t!("bar_ago", span = span).into_owned()
}

/// 横轴：最左边是时间跨度，最右边是「现在」，中间放这段时间的统计。
fn axis(s: &AppState, ink: &Ink, range: TimeRange, summary: String) -> Div {
    h_flex()
        .h(px(AXIS))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap(px(8.))
        .child(label(span_text(s, range), 10., ink.muted).flex_shrink_0())
        .child(widgets::tabular(
            label(summary, 10., ink.muted)
                .flex_1()
                .min_w_0()
                .truncate()
                .text_center(),
        ))
        .child(label(t!("bar_now").into_owned(), 10., ink.muted).flex_shrink_0())
}

fn history_card(s: &AppState, ink: &Ink, chart: Chart) -> Block {
    let range = s.settings.panel_range;
    // 刚开始记的时候，长时间跨度的图还是空的：说一声，免得以为坏了。
    let notice = (chart.empty && range != TimeRange::Live).then(|| {
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                label(t!("panel_collecting").into_owned(), 10.5, ink.muted)
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .bg(ink.veil),
            )
    });
    card(
        chart.title,
        Some(range_switch(ink, range)),
        chart.height + AXIS_GAP + AXIS,
        v_flex()
            .gap(px(AXIS_GAP))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(chart.height))
                    .flex_shrink_0()
                    .child(chart.graph)
                    .children(notice),
            )
            .child(axis(s, ink, range, chart.summary)),
    )
}

/// 「平均 … · 峰值 …」，没有任何读数时留空。
fn summary(data: &Series, show: impl Fn(f64) -> String) -> String {
    match (data.mean, data.peak) {
        (Some(mean), Some(peak)) => {
            t!("bar_avg_peak", avg = show(mean), peak = show(peak)).into_owned()
        }
        _ => String::new(),
    }
}

/// 占用率、温度这类有固定量程的曲线。
fn trend(
    s: &AppState,
    ink: &Ink,
    title: String,
    channel: Channel,
    palette: Palette,
    show: impl Fn(f64) -> String,
) -> Block {
    let data = series(s, s.settings.panel_range, channel);
    let scale = channel.scale();
    let bars = data
        .columns
        .iter()
        .map(|v| v.map(|v| scale.fraction(v, &[])))
        .collect();
    let chart = Chart {
        title: title_text(ink, title).into_any_element(),
        graph: div()
            .w_full()
            .h(px(CHART))
            .child(widgets::columns(bars, COLUMNS, 10, (1.5, 1.5), palette))
            .into_any_element(),
        height: CHART,
        summary: summary(&data, show),
        empty: data.mean.is_none(),
    };
    history_card(s, ink, chart)
}

/// 上下对称的速率图（下载 / 上传、读 / 写）：两条共用一个量程，高低才有可比性。
fn traffic(s: &AppState, ink: &Ink, up: Lane, down: Lane, floor: f64, cx: &App) -> Block {
    let top = [&up, &down]
        .into_iter()
        .flat_map(|lane| lane.series.columns.iter().flatten())
        .copied()
        .fold(floor, f64::max);
    let half = |height: f32, graph: AnyElement| div().w_full().h(px(height)).child(graph);
    let graph = v_flex()
        .gap(px(LANE_GAP))
        .child(half(
            LANE,
            widgets::columns(
                scaled(&up.series.columns, 0., top),
                COLUMNS,
                7,
                (1.5, 1.),
                Palette::new(up.tint, cx).plain(),
            )
            .into_any_element(),
        ))
        .child(half(
            LANE,
            widgets::columns_down(
                scaled(&down.series.columns, 0., top),
                COLUMNS,
                7,
                (1.5, 1.),
                Palette::new(down.tint, cx).plain(),
            )
            .into_any_element(),
        ));
    let peaks = match (up.series.peak, down.series.peak) {
        (Some(a), Some(b)) => t!(
            "panel_peak",
            value = format!(
                "{} {} · {} {}",
                up.arrow,
                s.settings.rate(a as u64),
                down.arrow,
                s.settings.rate(b as u64)
            )
        )
        .into_owned(),
        _ => String::new(),
    };
    let title = h_flex()
        .gap(px(10.))
        .children([&up, &down].into_iter().map(|lane| {
            div()
                .text_size(px(10.5))
                .font_semibold()
                .text_color(lane.tint)
                .child(format!("{} {}", lane.arrow, lane.name).to_uppercase())
        }))
        .into_any_element();
    let chart = Chart {
        title,
        graph: graph.into_any_element(),
        height: 2. * LANE + LANE_GAP,
        summary: peaks,
        empty: up.series.mean.is_none() && down.series.mean.is_none(),
    };
    history_card(s, ink, chart)
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
    let body = CORES + if numbers.is_some() { 17. } else { 0. };
    card(
        title_text(ink, t!("cpu_per_core").into_owned()),
        None,
        body,
        v_flex()
            .child(div().w_full().h(px(CORES)).child(widgets::columns(
                values,
                count,
                8,
                (gap, 2.),
                ink.palette(cx),
            )))
            .children(numbers),
    )
}

// ---- 进程排行 ----------------------------------------------------------------

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

/// 名称之后的数值列宽。读写分两列时每列窄一点。
const VALUE_W: f32 = 62.;
const METER_W: f32 = 48.;
const IO_W: f32 = 58.;

fn process_row(
    s: &AppState,
    ink: &Ink,
    p: &ProcessInfo,
    sort: ProcessSort,
    scale: f64,
    palette: Palette,
) -> Div {
    let busy = |value: f64| if value > 0. { ink.fg } else { ink.muted };
    let row = h_flex()
        .h(px(ROW))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap(px(8.))
        .child(
            label(p.name.clone(), 12., ink.fg)
                .flex_1()
                .min_w_0()
                .truncate(),
        );
    if sort == ProcessSort::Io {
        let cell = |bps: u64| {
            widgets::tabular(
                label(s.settings.rate(bps), 11.5, busy(bps as f64))
                    .w(px(IO_W))
                    .font_medium()
                    .text_right(),
            )
        };
        return row.child(cell(p.read_bps)).child(cell(p.write_bps));
    }
    let value = process_value(p, sort);
    row.child(div().w(px(METER_W)).h(px(6.)).child(widgets::meter(
        (value / scale) as f32,
        10,
        1.5,
        palette,
    )))
    .child(widgets::tabular(
        // 没有占用的行调暗，眼睛才会落在真正在干活的进程上。
        label(process_text(s, p, sort), 12., busy(value))
            .w(px(VALUE_W))
            .font_medium()
            .text_right(),
    ))
}

/// 进程数据还没到时占位的一行：骨架和真正的一行等高，数据一到不会跳。
fn skeleton_row(ink: &Ink, index: usize, sort: ProcessSort, palette: Palette) -> Div {
    // 名字的长短错落一些，看上去像排好的版。
    const NAMES: [f32; 8] = [104., 72., 124., 88., 112., 64., 96., 80.];
    let bar = |width: f32| div().h(px(8.)).w(px(width)).rounded(px(4.)).bg(ink.chip);
    let row = h_flex()
        .h(px(ROW))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap(px(8.))
        .child(div().flex_1().child(bar(NAMES[index % NAMES.len()])));
    if sort == ProcessSort::Io {
        let cell = || div().w(px(IO_W)).flex().justify_end().child(bar(36.));
        return row.child(cell()).child(cell());
    }
    row.child(
        div()
            .w(px(METER_W))
            .h(px(6.))
            .child(widgets::meter(0., 10, 1.5, palette)),
    )
    .child(div().w(px(VALUE_W)).flex().justify_end().child(bar(38.)))
}

/// 列头。卡片里只有读写分两列时才需要（`name` 为假），二级页面每种排行都有。
fn process_head(ink: &Ink, sort: ProcessSort, name: bool) -> Div {
    let head = |text: String, width: f32, tint: Hsla| {
        div()
            .w(px(width))
            .text_right()
            .text_size(px(10.))
            .font_semibold()
            .text_color(tint)
            .child(text.to_uppercase())
    };
    let row = h_flex()
        .h(px(PROC_HEAD))
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .flex_1()
                .text_size(px(10.))
                .font_semibold()
                .text_color(ink.muted)
                .when(name, |this| this.child(t!("proc_col_name").to_uppercase())),
        );
    match sort {
        ProcessSort::Io => row
            .child(head(t!("bar_read").into_owned(), IO_W, ink.accent))
            .child(head(t!("bar_write").into_owned(), IO_W, ink.yellow)),
        ProcessSort::Cpu => row.child(div().w(px(METER_W))).child(head(
            t!("proc_col_cpu").into_owned(),
            VALUE_W,
            ink.muted,
        )),
        ProcessSort::Memory => row.child(div().w(px(METER_W))).child(head(
            t!("proc_col_mem").into_owned(),
            VALUE_W,
            ink.muted,
        )),
    }
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
            None if loading => skeleton_row(ink, index, sort, palette),
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
    // 读写分两列时，卡片里也要有列头，不然看不出哪列是读哪列是写。
    let head = (sort == ProcessSort::Io).then(|| process_head(ink, sort, false));
    let head_height = if head.is_some() { PROC_HEAD } else { 0. };
    card(
        title_text(ink, t!("bar_top_processes").into_owned()),
        Some(more(ink, side, s.processes.len())),
        head_height + rows as f32 * ROW,
        v_flex()
            .children(head)
            .child(process_list(s, ink, sort, rows, cx)),
    )
    .trigger(Flyout::Processes(sort))
}

// ---- 分项 ----------------------------------------------------------------------

/// 一行「名称 ……… 数值」加下面一条格子。
fn meter_row(ink: &Ink, name: String, value: String, fraction: f32, palette: Palette) -> Div {
    v_flex()
        .w_full()
        .h(px(METER))
        .flex_shrink_0()
        .gap(px(5.))
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
                .h(px(6.))
                .child(widgets::meter(fraction, 48, 1.5, palette)),
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
                .flex_shrink_0()
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
                .h(px(LINK))
                .flex_shrink_0()
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

/// 「VRAM 已用 / 总量」；没有总量（比如统一内存）时只写已用，什么都没有就返回 `None`。
fn vram_text(s: &AppState, gpu: &GpuTick) -> Option<String> {
    match (gpu.vram_used_bytes, gpu.vram_total_bytes) {
        (Some(used), Some(total)) if total > 0 => Some(format!(
            "VRAM {} / {}",
            s.settings.bytes(used),
            s.settings.bytes(total)
        )),
        (Some(used), _) => Some(format!("VRAM {}", s.settings.bytes(used))),
        _ => None,
    }
}

/// 每块显卡一项，连同这些项加起来的高度；没有显存数据的不留那一行字的位置。
fn gpu_rows(s: &AppState, ink: &Ink, palette: Palette, tick: &MonitorTick) -> (Vec<Div>, f32) {
    let (rows, heights): (Vec<_>, Vec<_>) = tick
        .gpus
        .iter()
        .take(2)
        .map(|gpu| {
            let vram = vram_text(s, gpu);
            let height = if vram.is_some() { METER + 15. } else { METER };
            let row = v_flex()
                .w_full()
                .h(px(height))
                .flex_shrink_0()
                .child(meter_row(
                    ink,
                    gpu.name.clone(),
                    gpu.utilization.map_or("—".into(), fmt::percent),
                    gpu.utilization.map_or(0., |u| u / 100.),
                    palette,
                ));
            (
                row.children(vram.map(|text| label(text, 10.5, ink.muted))),
                height,
            )
        })
        .unzip();
    let total = heights.iter().sum::<f32>() + heights.len().saturating_sub(1) as f32 * LIST_GAP;
    (rows, total)
}

// ---- 头部副标题 ------------------------------------------------------------------

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

// ---- 面板主体 ----------------------------------------------------------------------

/// 面板主体：从上到下的各张卡片。`rows` 是进程排行的行数。
pub fn blocks(s: &AppState, metric: Metric, side: Side, rows: usize, cx: &App) -> Vec<Block> {
    let ink = Ink::new(metric, cx);
    let tick = s.latest.as_ref();
    let settings = &s.settings;
    let reading = data::reading(metric, Show::Auto, tick, &s.snapshot, settings);
    let number = reading.number;
    let gauge_fraction = fraction(number, reading.scale);
    let mut out = Vec::new();
    match metric {
        Metric::Cpu => {
            let cpu = &s.snapshot.cpu;
            let mut stats = Vec::new();
            if let Some(temp) = tick.and_then(MonitorTick::cpu_temperature) {
                stats.push(Stat::icon(
                    IconName::Thermometer,
                    heat(&ink, temp),
                    t!("spec_temp"),
                    settings.temp(temp),
                ));
            }
            if cpu.current_frequency_hz > 0 {
                stats.push(Stat::icon(
                    IconName::Gauge,
                    ink.muted,
                    t!("spec_freq"),
                    fmt::freq(cpu.current_frequency_hz),
                ));
            }
            let busiest = tick.and_then(|t| {
                t.cpu_per_core
                    .iter()
                    .copied()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(&b.1))
            });
            if let Some((index, load)) = busiest {
                stats.push(Stat::icon(
                    IconName::Activity,
                    ink.muted,
                    t!("panel_busiest_core", n = index + 1),
                    fmt::percent(load),
                ));
            }
            out.push(hero(
                &ink,
                gauge_fraction,
                usage_tone(&ink, number),
                &reading.value,
                Some(fmt::core_summary(cpu)),
                stats,
            ));
            out.push(trend(
                s,
                &ink,
                t!("bar_show_usage").into_owned(),
                Channel::Cpu,
                ink.palette(cx),
                |v| fmt::percent(v as f32),
            ));
            if let Some(tick) = tick.filter(|t| !t.cpu_per_core.is_empty()) {
                out.push(cores(&ink, &tick.cpu_per_core, cx));
            }
            out.push(processes(s, &ink, ProcessSort::Cpu, side, rows, cx));
        }
        Metric::Memory => {
            let mut stats = Vec::new();
            if let Some(t) = tick.filter(|t| t.mem_total_bytes > 0) {
                stats.push(Stat::dot(
                    ink.accent,
                    t!("spec_used"),
                    settings.bytes(t.mem_used_bytes),
                ));
                stats.push(Stat::dot(
                    ink.faint,
                    t!("spec_available"),
                    settings.bytes(t.mem_total_bytes.saturating_sub(t.mem_used_bytes)),
                ));
                if t.swap_total_bytes > 0 {
                    stats.push(Stat::icon(
                        IconName::ArrowLeftRight,
                        ink.muted,
                        t!("mem_swap"),
                        settings.bytes(t.swap_used_bytes),
                    ));
                }
            }
            out.push(hero(
                &ink,
                gauge_fraction,
                usage_tone(&ink, number),
                &reading.value,
                None,
                stats,
            ));
            out.push(trend(
                s,
                &ink,
                t!("bar_show_usage").into_owned(),
                Channel::Memory,
                ink.palette(cx),
                |v| fmt::percent(v as f32),
            ));
            out.push(processes(s, &ink, ProcessSort::Memory, side, rows, cx));
        }
        Metric::Storage => {
            let mut stats = Vec::new();
            if let Some(disk) = data::system_volume(&s.snapshot) {
                stats.push(Stat::dot(
                    ink.faint,
                    t!("spec_available"),
                    settings.bytes(disk.total_bytes.saturating_sub(disk.used_bytes)),
                ));
            }
            if let Some(t) = tick {
                stats.push(Stat::icon(
                    IconName::ArrowDown,
                    ink.accent,
                    t!("bar_read"),
                    settings.rate(t.disk_read_bps),
                ));
                stats.push(Stat::icon(
                    IconName::ArrowUp,
                    ink.yellow,
                    t!("bar_write"),
                    settings.rate(t.disk_write_bps),
                ));
            }
            out.push(hero(
                &ink,
                gauge_fraction,
                usage_tone(&ink, number),
                &reading.value,
                None,
                stats,
            ));
            let range = settings.panel_range;
            out.push(traffic(
                s,
                &ink,
                Lane {
                    arrow: "↓",
                    name: t!("bar_read").into_owned(),
                    tint: ink.accent,
                    series: series(s, range, Channel::DiskRead),
                },
                Lane {
                    arrow: "↑",
                    name: t!("bar_write").into_owned(),
                    tint: ink.yellow,
                    series: series(s, range, Channel::DiskWrite),
                },
                DISK_FLOOR,
                cx,
            ));
            let disks = volumes(s);
            if !disks.is_empty() {
                let shown = disks.len().min(3);
                let more_disks = disks.len() > shown;
                let block = card(
                    title_text(&ink, t!("bar_volumes").into_owned()),
                    more_disks.then(|| more(&ink, side, disks.len())),
                    stack(shown, METER),
                    v_flex()
                        .gap(px(LIST_GAP))
                        .children(volume_rows(s, &ink, &disks[..shown], cx)),
                );
                out.push(if more_disks {
                    block.trigger(Flyout::Volumes)
                } else {
                    block
                });
            }
            out.push(processes(s, &ink, ProcessSort::Io, side, rows, cx));
        }
        Metric::Temperature => {
            let list = temperatures(s);
            let mut stats = Vec::new();
            if let Some(max) = tick.and_then(MonitorTick::max_temperature)
                && number.is_none_or(|n| (n - max as f64).abs() > 0.5)
            {
                stats.push(Stat::icon(
                    IconName::Flame,
                    heat(&ink, max),
                    t!("bar_max"),
                    settings.temp(max),
                ));
            }
            if !list.is_empty() {
                let mean = list.iter().map(|r| r.value).sum::<f32>() / list.len() as f32;
                stats.push(Stat::icon(
                    IconName::Activity,
                    ink.muted,
                    t!("panel_average"),
                    settings.temp(mean),
                ));
                stats.push(Stat::icon(
                    IconName::Layers,
                    ink.muted,
                    t!("bar_sensors"),
                    list.len().to_string(),
                ));
            }
            let level = number.map_or(Level::Healthy, |n| health::temperature_level(n as f32));
            out.push(hero(
                &ink,
                gauge_fraction,
                ink.tone(level),
                &reading.value,
                Some(t!("bar_show_cpu_temp").into_owned()),
                stats,
            ));
            out.push(trend(
                s,
                &ink,
                t!("spec_temp").into_owned(),
                Channel::Temperature,
                ink.temperature_palette(cx),
                |v| settings.temp(v as f32),
            ));
            if list.is_empty() {
                out.push(card(
                    title_text(&ink, t!("bar_sensors").into_owned()),
                    None,
                    ROW,
                    label(t!("bar_no_sensor").into_owned(), 12., ink.muted),
                ));
            } else {
                let shown = list.len().min(5);
                let more_sensors = list.len() > shown;
                let block = card(
                    title_text(&ink, t!("bar_sensors").into_owned()),
                    more_sensors.then(|| more(&ink, side, list.len())),
                    shown as f32 * ROW,
                    v_flex().children(sensor_rows(s, &ink, &list[..shown], cx)),
                );
                out.push(if more_sensors {
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
            let column = |arrow: IconName, caption: String, value: String, tint: Hsla| {
                let (num, unit) = value.split_once(' ').unwrap_or((&value, ""));
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(4.))
                            .child(Icon::new(arrow).size(px(11.)).text_color(tint))
                            .child(label(caption, 11., ink.muted)),
                    )
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap(px(3.))
                            .child(widgets::tabular(
                                div()
                                    .text_size(px(26.))
                                    .line_height(px(30.))
                                    .font_semibold()
                                    .text_color(tint)
                                    .child(num.to_string()),
                            ))
                            .child(label(unit.to_string(), 12., ink.muted).font_semibold()),
                    )
            };
            out.push(Block::card(
                CARD_TOP + 48. + CARD_TOP,
                h_flex()
                    .size_full()
                    .px(px(CARD_X))
                    .items_center()
                    .gap(px(12.))
                    .child(column(
                        IconName::ArrowDown,
                        t!("bar_download").into_owned(),
                        down,
                        ink.cyan,
                    ))
                    .child(div().w(px(1.)).h(px(40.)).bg(ink.rule))
                    .child(column(
                        IconName::ArrowUp,
                        t!("bar_upload").into_owned(),
                        up,
                        ink.magenta,
                    )),
            ));
            let range = settings.panel_range;
            out.push(traffic(
                s,
                &ink,
                Lane {
                    arrow: "↓",
                    name: t!("bar_download").into_owned(),
                    tint: ink.cyan,
                    series: series(s, range, Channel::NetDown),
                },
                Lane {
                    arrow: "↑",
                    name: t!("bar_upload").into_owned(),
                    tint: ink.magenta,
                    series: series(s, range, Channel::NetUp),
                },
                NET_FLOOR,
                cx,
            ));
            let links = interfaces(s);
            if !links.is_empty() {
                let shown = links.len().min(3);
                let more_links = links.len() > shown;
                let block = card(
                    title_text(&ink, t!("net_interfaces").into_owned()),
                    more_links.then(|| more(&ink, side, links.len())),
                    stack(shown, LINK),
                    v_flex()
                        .gap(px(LIST_GAP))
                        .children(link_rows(s, &ink, &links, shown)),
                );
                out.push(if more_links {
                    block.trigger(Flyout::Interfaces)
                } else {
                    block
                });
            }
        }
        Metric::Gpu => {
            let info = s.snapshot.gpus.first();
            let mut stats = Vec::new();
            if let Some(temp) = info.and_then(|g| g.temperature_c) {
                stats.push(Stat::icon(
                    IconName::Thermometer,
                    heat(&ink, temp),
                    t!("spec_temp"),
                    settings.temp(temp),
                ));
            }
            if let Some(power) = info.and_then(|g| g.power_w) {
                stats.push(Stat::icon(
                    IconName::Zap,
                    ink.muted,
                    t!("panel_power"),
                    format!("{power:.1} W"),
                ));
            }
            let caption = tick
                .and_then(|t| t.gpus.first())
                .and_then(|gpu| vram_text(s, gpu));
            out.push(hero(
                &ink,
                gauge_fraction,
                usage_tone(&ink, number),
                &reading.value,
                caption,
                stats,
            ));
            out.push(trend(
                s,
                &ink,
                t!("bar_show_usage").into_owned(),
                Channel::Gpu,
                ink.palette(cx),
                |v| fmt::percent(v as f32),
            ));
            let (list, height) = tick
                .map(|t| gpu_rows(s, &ink, ink.palette(cx), t))
                .unwrap_or_default();
            if list.is_empty() {
                out.push(card(
                    title_text(&ink, t!("nav_gpu").into_owned()),
                    None,
                    ROW,
                    label(t!("bar_unavailable").into_owned(), 12., ink.muted),
                ));
            } else if list.len() > 1 {
                // 只有一块显卡时，概览里已经有它的占用、温度和显存，不再重复列一遍。
                out.push(card(
                    title_text(&ink, t!("nav_gpu").into_owned()),
                    None,
                    height,
                    v_flex().gap(px(LIST_GAP)).children(list),
                ));
            }
        }
        Metric::Battery => {
            let Some(battery) = s.snapshot.battery.as_ref() else {
                out.push(Block::card(
                    130.,
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
            let mut stats = Vec::new();
            let eta = battery
                .time_to_empty_secs
                .map(|secs| (t!("bat_time_to_empty"), secs))
                .or(battery
                    .time_to_full_secs
                    .map(|secs| (t!("bat_time_to_full"), secs)))
                .filter(|(_, secs)| *secs > 0);
            if let Some((name, secs)) = eta {
                stats.push(Stat::icon(
                    IconName::Timer,
                    ink.muted,
                    name,
                    fmt::duration(secs),
                ));
            }
            if let Some(mw) = battery.power_now_mw {
                stats.push(Stat::icon(
                    IconName::Zap,
                    ink.muted,
                    t!("bat_power"),
                    format!("{:.1} W", mw.unsigned_abs() as f64 / 1000.),
                ));
            }
            out.push(hero(
                &ink,
                battery.percentage / 100.,
                tone,
                &reading.value,
                Some(battery_state(&battery.state)),
                stats,
            ));
            out.push(trend(
                s,
                &ink,
                t!("bar_show_level").into_owned(),
                Channel::Battery,
                Palette::new(tone, cx).plain(),
                |v| fmt::percent(v as f32),
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
                    .h(px(34.))
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
            out.push(card(
                title_text(&ink, t!("bar_details").into_owned()),
                None,
                2. * 34. + 8.,
                h_flex()
                    .flex_wrap()
                    .gap_x(px(12.))
                    .gap_y(px(8.))
                    .children(cells),
            ));
        }
    }
    out
}

/// 二级页面的四周留白：面板边框、上下留白、卡片的边框、内边距和标题。
const FLYOUT_CHROME: f32 = 2. + 2. * SPACE + BORDER + CARD_TOP + HEAD + HEAD_GAP + CARD_BOTTOM;

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
        let room = ((max_height - FLYOUT_CHROME - extra + LIST_GAP) / (row + LIST_GAP))
            .floor()
            .max(1.) as usize;
        wanted.min(room)
    };
    let (title, rows, body) = match kind {
        Flyout::Processes(sort) => {
            let count = ((max_height - FLYOUT_CHROME - PROC_HEAD) / ROW)
                .floor()
                .max(1.) as usize;
            let count = count.min(FLYOUT_ROWS);
            (
                t!("proc_title").into_owned(),
                PROC_HEAD + count as f32 * ROW,
                v_flex()
                    .child(process_head(&ink, sort, true))
                    .child(process_list(s, &ink, sort, count, cx)),
            )
        }
        Flyout::Volumes => {
            let disks = volumes(s);
            let shown = disks.len().min(fit(METER, 0., 8));
            (
                t!("bar_volumes").into_owned(),
                stack(shown, METER),
                v_flex()
                    .gap(px(LIST_GAP))
                    .children(volume_rows(s, &ink, &disks[..shown], cx)),
            )
        }
        Flyout::Sensors => {
            let list = temperatures(s);
            let count = (((max_height - FLYOUT_CHROME) / ROW).floor().max(1.) as usize)
                .min(list.len())
                .min(16);
            (
                t!("bar_sensors").into_owned(),
                count as f32 * ROW,
                v_flex().children(sensor_rows(s, &ink, &list[..count], cx)),
            )
        }
        Flyout::Interfaces => {
            let links = interfaces(s);
            let shown = links.len().min(fit(LINK, 0., 8));
            (
                t!("net_interfaces").into_owned(),
                stack(shown, LINK),
                v_flex()
                    .gap(px(LIST_GAP))
                    .children(link_rows(s, &ink, &links, shown)),
            )
        }
    };
    let block = card(title_text(&ink, title), None, rows, body);
    (
        WIDTH,
        2. + 2. * SPACE + block.height,
        v_flex()
            .size_full()
            .py(px(SPACE))
            .child(frame(cx).h(px(block.height)).child(block.element))
            .into_any_element(),
    )
}

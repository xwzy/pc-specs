//! Retina 菜单栏位图。模板图只有透明度有意义，颜色由系统按菜单栏明暗染色，不依赖应用主题。
//!
//! 版面尽量紧凑：左右各留设置里的间距的一半，宽度按版式里可能出现的最宽读数固定下来，
//! 数值跳动时菜单栏上的其它项不会左右抖动。
use super::config::{ItemStyle, Metric, Show};
use super::data::{self, Reading, Scale, Value};
use crate::{
    health::{self, Level},
    settings::Settings,
};
use resvg::{tiny_skia, usvg};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
};

/// 栅格化好的位图。像素是预乘 alpha 的 RGBA（`tiny-skia` 和 `NSBitmapImageRep` 的原生格式），
/// 可以原样交给 AppKit；PNG 之类要直通 alpha 的地方用 [`StatusImage::straight_rgba`]。
pub struct StatusImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl StatusImage {
    /// 位图是 2 倍分辨率，菜单栏项的长度按点计算。
    pub fn points(&self) -> f64 {
        self.width as f64 / 2.
    }

    /// 直通（非预乘）alpha 的像素。
    #[cfg(target_os = "macos")]
    pub fn straight_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.rgba.len());
        for px in self.rgba.chunks_exact(4) {
            let alpha = px[3] as u32;
            if alpha == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                for &channel in &px[..3] {
                    out.push(((channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8);
                }
                out.push(px[3]);
            }
        }
        out
    }
}

pub struct Item<'a> {
    pub metric: Metric,
    pub style: ItemStyle,
    pub show: Show,
    pub reading: &'a Reading,
    pub history: &'a [f64],
    /// 不是第一项时，在左边画一条发丝线。
    pub separator: bool,
}

const HEIGHT: f32 = 22.;
const SPARK_WIDTH: f32 = 24.;
const SPARK_GAP: f32 = 4.;
/// 环形仪表的外径，环宽。
const RING_SIZE: f32 = 16.;
const RING_STROKE: f32 = 2.6;
const FONT: &str = "Helvetica Neue, Arial, sans-serif";

#[derive(Clone)]
enum Piece {
    Text {
        text: String,
        size: f32,
        weight: u16,
        dim: bool,
        spacing: f32,
    },
    Arrow {
        down: bool,
        size: f32,
    },
    Gap(f32),
}

struct Line(Vec<Piece>);

impl Line {
    fn width(&self) -> f32 {
        self.0
            .iter()
            .map(|piece| match piece {
                Piece::Text {
                    text,
                    size,
                    weight,
                    spacing,
                    ..
                } => text_width(text, *size, *weight, *spacing),
                Piece::Arrow { size, .. } => arrow_width(*size),
                Piece::Gap(w) => *w,
            })
            .sum()
    }

    fn draw(&self, svg: &mut String, x: f32, baseline: f32) {
        let mut x = x;
        for piece in &self.0 {
            match piece {
                Piece::Text {
                    text,
                    size,
                    weight,
                    dim,
                    spacing,
                } => {
                    svg.push_str(&format!(
                        r#"<text x="{x:.2}" y="{baseline:.2}" font-size="{size}" font-weight="{weight}" letter-spacing="{spacing}"{}>{}</text>"#,
                        if *dim { r#" opacity="0.72""# } else { "" },
                        escape(text)
                    ));
                    x += text_width(text, *size, *weight, *spacing);
                }
                Piece::Arrow { down, size } => {
                    let height = size * 0.72;
                    let head = height * 0.36;
                    let cx = x + head + 0.5;
                    let (top, bottom) = (baseline - height, baseline);
                    let (from, to, wing) = if *down {
                        (top, bottom, bottom - head)
                    } else {
                        (bottom, top, top + head)
                    };
                    svg.push_str(&format!(
                        r#"<path d="M{cx:.2} {from:.2}L{cx:.2} {to:.2}M{:.2} {wing:.2}L{cx:.2} {to:.2}L{:.2} {wing:.2}" fill="none" stroke="{INK}" stroke-width="{:.2}" stroke-linecap="round" stroke-linejoin="round"/>"#,
                        cx - head,
                        cx + head,
                        size * 0.13
                    ));
                    x += arrow_width(*size);
                }
                Piece::Gap(w) => x += w,
            }
        }
    }
}

/// 位图里所有笔画都用这个占位色，最后整体替换成真正的颜色。
const INK: &str = "@ink";

fn arrow_width(size: f32) -> f32 {
    size * 0.72 * 0.36 * 2. + 0.5 + 0.5 + 1.6
}

/// 菜单栏位图里只会出现 ASCII、`°` 和 `—`，一套无衬线字体就够了。
///
/// 不把整个系统字体库扫进来：那要打开几百个字体文件，元数据还会一直占着内存；
/// 更糟的是 `fontdb` 默认每次取字形都重新 `open`+`mmap` 一遍字体文件，菜单栏一秒刷新一次，
/// 这条路径曾占去主线程近一半的 CPU。所以字体只映射一次，之后一直共用同一份映射。
fn fonts() -> &'static Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut fonts = usvg::fontdb::Database::new();
        #[cfg(target_os = "macos")]
        let _ = fonts.load_font_file("/System/Library/Fonts/HelveticaNeue.ttc");
        if fonts.is_empty() {
            fonts.load_system_fonts();
        }
        #[cfg(target_os = "macos")]
        share_used_faces(&mut fonts);
        Arc::new(fonts)
    })
}

/// 把 `FONT` 实际会选中的字体文件映射成共享的一份。
///
/// 只对系统卷上的文件这么做：macOS 的系统卷是只读、带签名的，文件在应用运行期间不会被改写，
/// 映射不会因为文件被截断而触发 SIGBUS。其它位置的字体（回退到用户字体时）维持原样，
/// 每次按需打开，慢一点，但绝不会因此崩溃。
#[cfg(target_os = "macos")]
fn share_used_faces(fonts: &mut usvg::fontdb::Database) {
    use usvg::fontdb::{Family, Query, Stretch, Style, Weight};
    let families = [
        Family::Name("Helvetica Neue"),
        Family::Name("Arial"),
        Family::SansSerif,
    ];
    for weight in [600, 700] {
        let query = Query {
            families: &families,
            weight: Weight(weight),
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let Some(id) = fonts.query(&query) else {
            continue;
        };
        let on_system_volume = fonts.face(id).is_some_and(|face| match &face.source {
            usvg::fontdb::Source::File(path) => path.starts_with("/System/"),
            _ => false,
        });
        if on_system_volume {
            // SAFETY: 文件在只读的系统卷上，运行期间不会被修改（见上）。
            let _ = unsafe { fonts.make_shared_face_data(id) };
        }
    }
}

fn options() -> &'static usvg::Options<'static> {
    static OPTIONS: OnceLock<usvg::Options<'static>> = OnceLock::new();
    OPTIONS.get_or_init(|| usvg::Options {
        fontdb: fonts().clone(),
        ..Default::default()
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 一段文字的宽度（从原点到最后一个字形的右缘），按字体量出来再缓存。
///
/// 缓存按字体参数分层，命中时不需要为了查表而分配 `String`；
/// 条目数有上限，万一出现没见过的文字组合也不会无限增长。
fn text_width(text: &str, size: f32, weight: u16, spacing: f32) -> f32 {
    type Font = (u32, u16, i32);
    const MAX_PER_FONT: usize = 512;
    static CACHE: OnceLock<Mutex<HashMap<Font, HashMap<String, f32>>>> = OnceLock::new();
    let font = ((size * 10.) as u32, weight, (spacing * 10.) as i32);
    let cache = CACHE.get_or_init(Default::default);
    if let Some(width) = lock(cache).get(&font).and_then(|c| c.get(text)).copied() {
        return width;
    }
    let width = measure(text, size, weight, spacing);
    let mut cache = lock(cache);
    let entries = cache.entry(font).or_default();
    if entries.len() >= MAX_PER_FONT {
        entries.clear();
    }
    entries.insert(text.to_string(), width);
    width
}

fn measure(text: &str, size: f32, weight: u16, spacing: f32) -> f32 {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="60"><text x="0" y="40" font-family="{FONT}" font-size="{size}" font-weight="{weight}" letter-spacing="{spacing}">{}</text></svg>"#,
        escape(text)
    );
    usvg::Tree::from_str(&svg, options())
        .ok()
        .map(|tree| tree.root().bounding_box())
        .filter(|b| b.width() > 0.)
        .map_or(text.chars().count() as f32 * size * 0.58, |b| {
            b.right() + 0.3
        })
}

fn text(text: &str, size: f32, weight: u16) -> Piece {
    Piece::Text {
        text: text.to_string(),
        size,
        weight,
        dim: false,
        spacing: 0.,
    }
}

fn value_line(value: &Value, size: f32, units: bool) -> Line {
    let mut line = Vec::new();
    match value.lead {
        "↓" => line.push(Piece::Arrow { down: true, size }),
        "↑" => line.push(Piece::Arrow { down: false, size }),
        _ => {}
    }
    line.push(text(&value.num, size, 700));
    let unit = value.bar_unit(units);
    if !unit.is_empty() {
        line.push(Piece::Gap(0.3));
        line.push(Piece::Text {
            text: unit.to_string(),
            size: size * 0.74,
            weight: 600,
            dim: true,
            spacing: 0.,
        });
    }
    Line(line)
}

fn label_line(label: &str) -> Line {
    Line(vec![Piece::Text {
        text: label.to_string(),
        size: 7.,
        weight: 600,
        dim: true,
        spacing: 0.5,
    }])
}

/// 竖排的一组行，以及每一行的基线。
fn stacked(
    value: &Value,
    second: Option<&Value>,
    label: Option<&str>,
    units: bool,
) -> Vec<(Line, f32)> {
    match (second, label) {
        (Some(second), _) => vec![
            (value_line(value, 10., units), 9.4),
            (value_line(second, 10., units), 19.4),
        ],
        (None, Some(label)) => vec![
            (value_line(value, 11.5, units), 10.6),
            (label_line(label), 19.4),
        ],
        (None, None) => vec![(value_line(value, 13., units), 15.7)],
    }
}

/// 横排：标签 数值 [第二个数值]。
fn inline(
    value: &Value,
    second: Option<&Value>,
    label: Option<&str>,
    units: bool,
) -> Vec<(Line, f32)> {
    let mut line = Vec::new();
    if let Some(label) = label {
        line.push(Piece::Text {
            text: label.to_string(),
            size: 8.,
            weight: 600,
            dim: true,
            spacing: 0.4,
        });
        line.push(Piece::Gap(3.5));
    }
    line.extend(value_line(value, 12., units).0);
    if let Some(second) = second {
        line.push(Piece::Gap(5.));
        line.extend(value_line(second, 12., units).0);
    }
    vec![(Line(line), 15.4)]
}

fn block(
    style: ItemStyle,
    value: &Value,
    second: Option<&Value>,
    label: Option<&str>,
    units: bool,
) -> Vec<(Line, f32)> {
    match style {
        ItemStyle::Inline => inline(value, second, label, units),
        ItemStyle::Stacked | ItemStyle::Graph | ItemStyle::Ring => {
            stacked(value, second, label, units)
        }
    }
}

fn width_of(block: &[(Line, f32)]) -> f32 {
    block
        .iter()
        .map(|(line, _)| line.width())
        .fold(0., f32::max)
}

/// 读数现在算不算「有问题」。只看占用率、温度和电量，网速这类没有好坏之分。
fn level(item: &Item) -> Level {
    let Some(number) = item.reading.number.map(|n| n as f32) else {
        return Level::Healthy;
    };
    match item.metric {
        Metric::Cpu | Metric::Memory | Metric::Storage | Metric::Gpu
            if item.reading.scale == Scale::Percent =>
        {
            health::usage_level(number)
        }
        Metric::Temperature => health::temperature_level(number),
        Metric::Battery => health::battery_level(number),
        _ => Level::Healthy,
    }
}

/// 墨色。模板图只有透明度有意义，用黑色；彩色模式下平时是各指标自己的颜色，
/// 读数进入警告 / 严重区间就换成橙 / 红，一眼能看出哪一项不对劲。
fn color(item: &Item, settings: &Settings) -> &'static str {
    if !settings.menu_bar.colorful {
        return "#000000";
    }
    match level(item) {
        Level::Critical => "#ef4444",
        Level::Warning => "#f97316",
        Level::Healthy => item.metric.accent(),
    }
}

fn spark(svg: &mut String, history: &[f64], scale: Scale, x: f32) {
    let (top, bottom) = (4.5_f32, 17.5_f32);
    let samples = &history[history.len().saturating_sub(24)..];
    svg.push_str(&format!(
        r#"<rect x="{x}" y="{:.2}" width="{SPARK_WIDTH}" height="0.6" fill="{INK}" opacity="0.28"/>"#,
        bottom
    ));
    if samples.len() < 2 {
        return;
    }
    let points: Vec<_> = samples
        .iter()
        .enumerate()
        .map(|(i, value)| {
            (
                x + SPARK_WIDTH * i as f32 / (samples.len() - 1) as f32,
                bottom - scale.fraction(*value, samples) * (bottom - top),
            )
        })
        .collect();
    let line = points
        .iter()
        .map(|(x, y)| format!("{x:.2},{y:.2}"))
        .collect::<Vec<_>>()
        .join(" ");
    svg.push_str(&format!(
        r#"<polygon points="{x:.2},{bottom:.2} {line} {:.2},{bottom:.2}" fill="{INK}" opacity="0.22"/>"#,
        x + SPARK_WIDTH
    ));
    svg.push_str(&format!(
        r#"<polyline points="{line}" fill="none" stroke="{INK}" stroke-width="1.2" stroke-linejoin="round" stroke-linecap="round"/>"#
    ));
}

/// 左边一个小环形仪表：弧越长读数越高，没有读数时只剩底轨。从 12 点钟方向顺时针画。
fn ring(svg: &mut String, item: &Item, x: f32) {
    let fraction = item
        .reading
        .number
        .map_or(0., |n| item.reading.scale.fraction(n, item.history));
    let radius = (RING_SIZE - RING_STROKE) / 2.;
    let (cx, cy) = (x + RING_SIZE / 2., HEIGHT / 2.);
    let circumference = std::f32::consts::TAU * radius;
    svg.push_str(&format!(
        r#"<circle cx="{cx:.2}" cy="{cy:.2}" r="{radius:.2}" fill="none" stroke="{INK}" stroke-width="{RING_STROKE}" opacity="0.26"/>"#
    ));
    if fraction <= 0. {
        return;
    }
    let (arc, cap) = if fraction >= 0.995 {
        (circumference, "butt")
    } else {
        // 圆头让弧线两端各多出半个笔画宽，扣掉它，弧长才和读数对得上。
        ((circumference * fraction - RING_STROKE).max(0.01), "round")
    };
    svg.push_str(&format!(
        r#"<circle cx="{cx:.2}" cy="{cy:.2}" r="{radius:.2}" fill="none" stroke="{INK}" stroke-width="{RING_STROKE}" stroke-linecap="{cap}" stroke-dasharray="{arc:.2} {circumference:.2}" transform="rotate(-90 {cx:.2} {cy:.2})"/>"#
    ));
}

/// 两行都是数值时（上下行、读写）本身就说明了含义，不再加标签。
fn tagged(label: Option<&'static str>, has_second: bool) -> Option<&'static str> {
    label.filter(|_| !has_second)
}

/// 这个版式在当前设置下可能出现的最宽读数（不含左右留白）。
/// 只取决于设置，不取决于此刻的读数，所以算一次就记下来，不必每秒把几十种读数重排一遍。
fn widest(item: &Item, settings: &Settings) -> f32 {
    type Key = (
        Metric,
        Show,
        ItemStyle,
        bool,
        bool,
        crate::settings::NetUnit,
        crate::settings::SizeUnit,
        crate::settings::TempUnit,
    );
    const MAX_ENTRIES: usize = 4096;
    static CACHE: OnceLock<Mutex<HashMap<Key, f32>>> = OnceLock::new();
    let config = &settings.menu_bar;
    let key = (
        item.metric,
        item.show,
        item.style,
        config.show_labels,
        config.show_units,
        settings.net_unit,
        settings.size_unit,
        settings.temp_unit,
    );
    let cache = CACHE.get_or_init(Default::default);
    if let Some(width) = lock(cache).get(&key).copied() {
        return width;
    }
    let label = config
        .show_labels
        .then_some(item.show.bar_label(item.metric));
    let widest = data::widest(item.metric, item.show, settings)
        .into_iter()
        .map(|(value, second)| {
            width_of(&block(
                item.style,
                &value,
                second.as_ref(),
                tagged(label, second.is_some()),
                config.show_units,
            ))
        })
        .fold(0., f32::max);
    let mut cache = lock(cache);
    if cache.len() >= MAX_ENTRIES {
        cache.clear();
    }
    cache.insert(key, widest);
    widest
}

/// 返回 SVG 和以点为单位的宽度。
fn build(item: &Item, settings: &Settings) -> (String, f32) {
    let config = &settings.menu_bar;
    let units = config.show_units;
    let label = config
        .show_labels
        .then_some(item.show.bar_label(item.metric));

    let current = block(
        item.style,
        &item.reading.value,
        item.reading.second.as_ref(),
        tagged(label, item.reading.second.is_some()),
        units,
    );
    // 当前读数万一比预估的还宽（极端数值），以实际为准，宁可多撑一点也不裁切。
    let content = widest(item, settings).max(width_of(&current));
    let lead = match item.style {
        ItemStyle::Graph => SPARK_WIDTH + SPARK_GAP,
        ItemStyle::Ring => RING_SIZE + SPARK_GAP,
        ItemStyle::Stacked | ItemStyle::Inline => 0.,
    };
    let pad = config.padding();
    let width = ((pad * 2. + lead + content) * 2.).ceil() / 2.;

    let ink = color(item, settings);
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{HEIGHT}" viewBox="0 0 {width} {HEIGHT}"><g fill="{INK}" font-family="{FONT}">"#
    );
    if item.separator && config.separators {
        svg.push_str(&format!(
            r#"<rect x="0" y="4" width="0.6" height="14" fill="{INK}" opacity="0.32"/>"#
        ));
    }
    match item.style {
        ItemStyle::Graph => spark(&mut svg, item.history, item.reading.scale, pad),
        ItemStyle::Ring => ring(&mut svg, item, pad),
        ItemStyle::Stacked | ItemStyle::Inline => {}
    }
    let left = pad + lead;
    // 两行都是数值时靠左对齐，上下箭头排成一列；其余居中。
    let aligned = item.reading.second.is_some() && item.style != ItemStyle::Inline;
    for (line, baseline) in &current {
        let x = if aligned {
            left
        } else {
            left + (content - line.width()) / 2.
        };
        line.draw(&mut svg, x, *baseline);
    }
    svg.push_str("</g></svg>");
    (svg.replace(INK, ink), width)
}

/// 一个菜单栏项此刻该画成什么样。只是 SVG 描述，还没有栅格化，生成它很便宜；
/// 内容和上一次一样就不用再栅格化，更不用去碰 AppKit。
pub struct Drawing {
    svg: String,
    width: f32,
}

impl Drawing {
    // 只有 macOS 的菜单栏项拿它们比较「内容变没变」；其它平台只在测试里编译这个模块。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn svg(&self) -> &str {
        &self.svg
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn into_svg(self) -> String {
        self.svg
    }

    /// 按 2 倍分辨率栅格化。
    pub fn rasterize(&self) -> Option<StatusImage> {
        let tree = usvg::Tree::from_str(&self.svg, options()).ok()?;
        let mut pixmap = tiny_skia::Pixmap::new((self.width * 2.) as u32, (HEIGHT * 2.) as u32)?;
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(2., 2.),
            &mut pixmap.as_mut(),
        );
        Some(StatusImage {
            width: pixmap.width(),
            height: pixmap.height(),
            rgba: pixmap.take(),
        })
    }
}

pub fn draw(item: &Item, settings: &Settings) -> Drawing {
    let (svg, width) = build(item, settings);
    Drawing { svg, width }
}

#[cfg(test)]
pub fn render(item: &Item, settings: &Settings) -> Option<StatusImage> {
    draw(item, settings).rasterize()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item<'a>(
        metric: Metric,
        style: ItemStyle,
        show: Show,
        reading: &'a Reading,
        history: &'a [f64],
    ) -> Item<'a> {
        Item {
            metric,
            style,
            show,
            reading,
            history,
            separator: true,
        }
    }

    fn sample(metric: Metric, show: Show) -> Reading {
        let tick = pc_specs_core::model::MonitorTick {
            cpu_overall: 22.,
            mem_used_bytes: 60,
            mem_total_bytes: 100,
            net_rx_bps: 393 << 10,
            net_tx_bps: 3 << 10,
            disk_read_bps: 5 << 20,
            disk_write_bps: 800 << 10,
            ..Default::default()
        };
        data::reading(
            metric,
            show,
            Some(&tick),
            &Default::default(),
            &Settings::default(),
        )
    }

    // 这个模块只在 macOS 上用，字体写的是 Helvetica Neue / Arial。Linux 上没有 Arial，`sans-serif` 解析不到字体，
    // 纯文字的样式会渲染成空位图，所以这条断言在 Linux 上不成立。
    #[test]
    #[cfg_attr(
        target_os = "linux",
        ignore = "Linux 上没有 Helvetica Neue / Arial，文字渲染不出来"
    )]
    fn all_styles_render_at_retina_resolution() {
        let settings = Settings::default();
        for metric in Metric::ALL {
            for style in [
                ItemStyle::Stacked,
                ItemStyle::Inline,
                ItemStyle::Graph,
                ItemStyle::Ring,
            ] {
                for &show in metric.shows() {
                    let reading = sample(metric, show);
                    let history = [5., 45., 20.];
                    let image =
                        render(&item(metric, style, show, &reading, &history), &settings).unwrap();
                    assert_eq!(image.height, 44);
                    assert_eq!(image.rgba.len(), (image.width * image.height * 4) as usize);
                    assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0));
                }
            }
        }
    }

    #[test]
    fn width_is_stable_while_numbers_change() {
        let settings = Settings::default();
        let width = |value: &str| {
            let mut reading = sample(Metric::Cpu, Show::Auto);
            reading.value.num = value.into();
            render(
                &item(Metric::Cpu, ItemStyle::Stacked, Show::Auto, &reading, &[]),
                &settings,
            )
            .unwrap()
            .width
        };
        assert_eq!(width("1"), width("88"));
        assert_eq!(width("7"), width("100"));
    }

    #[test]
    fn rate_widths_do_not_jitter() {
        use crate::settings::NetUnit;
        use pc_specs_core::model::MonitorTick;
        for unit in [NetUnit::BytesPerSec, NetUnit::BitsPerSec] {
            let settings = Settings {
                net_unit: unit,
                ..Default::default()
            };
            let width = |bps: u64| {
                let tick = MonitorTick {
                    net_rx_bps: bps,
                    net_tx_bps: bps / 3,
                    ..Default::default()
                };
                let reading = data::reading(
                    Metric::Network,
                    Show::Auto,
                    Some(&tick),
                    &Default::default(),
                    &settings,
                );
                let item = item(
                    Metric::Network,
                    ItemStyle::Stacked,
                    Show::Auto,
                    &reading,
                    &[],
                );
                render(&item, &settings).unwrap().width
            };
            let reference = width(0);
            for bps in [
                1_u64,
                999,
                1023,
                5 << 10,
                99 << 10,
                100 << 10,
                888 << 10,
                999 << 10,
                1000 << 10,
                1023 << 10,
                1 << 20,
                12 << 20,
                99 << 20,
                100 << 20,
                888 << 20,
                999 << 20,
                1000 << 20,
                1023 << 20,
                1 << 30,
                9 << 30,
            ] {
                assert_eq!(width(bps), reference, "{unit:?} at {bps} B/s");
            }
        }
    }

    #[test]
    fn temperature_widths_do_not_jitter() {
        use crate::settings::TempUnit;
        use pc_specs_core::model::{MonitorTick, SensorReading};
        for unit in [TempUnit::Celsius, TempUnit::Fahrenheit] {
            let settings = Settings {
                temp_unit: unit,
                ..Default::default()
            };
            let width = |celsius: f32| {
                let tick = MonitorTick {
                    temperatures: vec![SensorReading {
                        source: "sysinfo".into(),
                        label: "CPU Package".into(),
                        kind: "temperature".into(),
                        value: celsius,
                        unit: "C".into(),
                    }],
                    ..Default::default()
                };
                let reading = data::reading(
                    Metric::Temperature,
                    Show::Auto,
                    Some(&tick),
                    &Default::default(),
                    &settings,
                );
                let item = item(
                    Metric::Temperature,
                    ItemStyle::Stacked,
                    Show::Auto,
                    &reading,
                    &[],
                );
                render(&item, &settings).unwrap().width
            };
            let reference = width(50.);
            for celsius in (0..=110).map(|c| c as f32) {
                assert_eq!(width(celsius), reference, "{unit:?} at {celsius} C");
            }
        }
    }

    #[test]
    fn amount_widths_do_not_jitter() {
        use crate::settings::SizeUnit;
        use pc_specs_core::model::MonitorTick;
        for unit in [SizeUnit::Binary, SizeUnit::Decimal] {
            let settings = Settings {
                size_unit: unit,
                ..Default::default()
            };
            let width = |used: u64| {
                let tick = MonitorTick {
                    mem_used_bytes: used,
                    mem_total_bytes: 64 << 40,
                    ..Default::default()
                };
                let reading = data::reading(
                    Metric::Memory,
                    Show::Used,
                    Some(&tick),
                    &Default::default(),
                    &settings,
                );
                let item = item(
                    Metric::Memory,
                    ItemStyle::Stacked,
                    Show::Used,
                    &reading,
                    &[],
                );
                render(&item, &settings).unwrap().width
            };
            let reference = width(0);
            for used in [
                1_u64 << 20,
                5 << 20,
                99 << 20,
                888 << 20,
                1 << 30,
                9 << 30,
                15 << 30,
                64 << 30,
                99 << 30,
                128 << 30,
                512 << 30,
                1 << 40,
                9 << 40,
            ] {
                assert_eq!(width(used), reference, "{unit:?} at {used} B");
            }
        }
    }

    #[test]
    fn compact_items_are_narrow() {
        let settings = Settings::default();
        let reading = sample(Metric::Cpu, Show::Auto);
        let image = render(
            &item(Metric::Cpu, ItemStyle::Stacked, Show::Auto, &reading, &[]),
            &settings,
        )
        .unwrap();
        assert!(image.points() < 36., "{}", image.points());
    }
}

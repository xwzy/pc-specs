//! macOS 系统菜单栏。所有 NSStatusItem 都在 GPUI 主线程创建和更新。
use super::{
    Anchor,
    config::{ItemStyle, Metric, Show},
    data,
    status_image::{self, Item, StatusImage},
};
use crate::settings::Settings;
use objc2::{AllocAnyThread, MainThreadMarker, msg_send, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{
    NSApplication, NSBitmapImageRep, NSDeviceRGBColorSpace, NSEvent, NSImage, NSScreen, NSWindow,
};
use objc2_foundation::{NSDictionary, NSNumber, NSSize, NSString, NSUserDefaults, ns_string};
use pc_specs_core::model::{MonitorTick, SystemSnapshot};
use std::{
    collections::{HashMap, VecDeque},
    sync::Once,
    time::{Duration, Instant},
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

/// 主图标是 22pt 的图案，给它固定一个宽度，左右各留一点呼吸的空间。
pub const MAIN_ICON_LENGTH: f64 = 28.;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetOnlineDisplayList(max_displays: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
}

/// 所有显示器是不是都在休眠。
///
/// 台式机、接着外接屏的笔记本经常是「屏幕黑着、系统醒着」（下载、编译、挂机）：
/// 菜单栏没有人看，重绘它只是白白耗电，而重绘正是这个程序最大的开销。
/// 拿不到显示器列表就当作没睡着：宁可多画一帧。
pub fn displays_asleep() -> bool {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    // SAFETY: `ids` 能放下传入的个数，`count` 返回实际写入的个数。
    let status = unsafe { CGGetOnlineDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    let count = (count as usize).min(ids.len());
    status == 0
        && count > 0
        // SAFETY: 只是询问显示器的电源状态。
        && ids[..count]
            .iter()
            .all(|&id| unsafe { CGDisplayIsAsleep(id) } != 0)
}

/// 系统默认给每个菜单栏项左右各留 8pt，几个读数排在一起就占掉一大截。
/// 位图自己带了留白，所以把间距登记成 0。只登记默认值：用户用 `defaults` 自己设过就以用户为准。
/// 必须在创建第一个菜单栏项之前调用。
pub fn prepare() {
    // 选中态改成矩形底色，同样要赶在第一个状态项出现之前。
    super::selection::install();
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let defaults = NSUserDefaults::standardUserDefaults();
        let keys = [
            ns_string!("NSStatusItemSpacing"),
            ns_string!("NSStatusItemSelectionPadding"),
        ];
        let unset: Vec<&NSString> = keys
            .into_iter()
            .filter(|key| defaults.objectForKey(key).is_none())
            .collect();
        if unset.is_empty() {
            return;
        }
        let zeros: Vec<Retained<AnyObject>> = unset
            .iter()
            .map(|_| unsafe { Retained::cast_unchecked(NSNumber::new_i32(0)) })
            .collect();
        let registration = NSDictionary::from_retained_objects(&unset, &zeros);
        // SAFETY: 字典的键是字符串、值是 NSNumber，都是 `registerDefaults:` 接受的属性列表类型。
        unsafe { defaults.registerDefaults(&registration) };
    });
}

fn display_id(screen: &NSScreen) -> Option<u64> {
    let number = screen
        .deviceDescription()
        .objectForKey(ns_string!("NSScreenNumber"))?;
    let id: u32 = unsafe { msg_send![&*number, unsignedIntValue] };
    Some(u64::from(id))
}

/// 状态项窗口在它所在显示器上的位置。窗口坐标以主屏左下角为原点、y 向上，
/// 这里换成「所在显示器左上角为原点」，和 GPUI 的窗口坐标一致。
fn anchor_of(window: &NSWindow) -> Option<Anchor> {
    let screen = window.screen()?;
    let (screen_frame, bar) = (screen.frame(), window.frame());
    Some(Anchor {
        display: display_id(&screen)?,
        left: (bar.origin.x - screen_frame.origin.x) as f32,
        right: (bar.origin.x + bar.size.width - screen_frame.origin.x) as f32,
        bottom: (screen_frame.origin.y + screen_frame.size.height - bar.origin.y) as f32,
    })
}

/// 菜单栏会复制到每一块显示器，每个副本都是一个独立的状态项窗口。
/// 点击发生时，鼠标正下方的那个窗口就是用户点的那个，弹窗要跟着它所在的显示器走。
pub fn click_anchor() -> Option<Anchor> {
    let mtm = MainThreadMarker::new()?;
    let mouse = NSEvent::mouseLocation();
    let windows = NSApplication::sharedApplication(mtm).windows();
    let (distance, window) = windows
        .iter()
        .filter(|window| {
            window
                .class()
                .name()
                .to_string_lossy()
                .contains("StatusBar")
        })
        .map(|window| {
            let frame = window.frame();
            let dx = (frame.origin.x - mouse.x)
                .max(mouse.x - (frame.origin.x + frame.size.width))
                .max(0.);
            let dy = (frame.origin.y - mouse.y)
                .max(mouse.y - (frame.origin.y + frame.size.height))
                .max(0.);
            (dx.hypot(dy), window)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    // 松开鼠标时指针稍微滑出一点也算这一项，离得太远就不是菜单栏项的点击了。
    (distance < 24.).then(|| anchor_of(&window))?
}

struct Slot {
    metric: Metric,
    icon: TrayIcon,
    /// 当前菜单栏上这张位图的 SVG 描述。内容没变就什么都不用做：
    /// 不栅格化、不创建 NSImage、不让 AppKit 重绘这个状态项。
    svg: String,
    /// 当前设置给状态项的长度（pt）。
    length: f64,
    highlighted: bool,
    /// 鼠标悬停提示的文字和上次写入的时间。提示只在悬停时才有人看，没必要一秒换一次。
    tooltip: String,
    tooltip_at: Option<Instant>,
    /// 长度变了以后，状态项里负责接收点击的视图要跟着变宽；
    /// 窗口服务器调整状态项是异步的，所以隔几次刷新再对齐一遍。
    resync: u8,
}

/// 走势图最多画几个点。
const TREND_POINTS: usize = 24;

/// 悬停提示至少隔这么久才更新一次。
const TOOLTIP_EVERY: Duration = Duration::from_secs(4);

/// 状态项位图的高度（pt）。
const IMAGE_HEIGHT: f64 = 22.;

#[derive(Default)]
pub struct NativeItems {
    slots: Vec<Slot>,
    /// 每个指标最近的读数，用来画「趋势」版式。放在菜单栏项外面，增减监控项、重建菜单栏项时不会丢；
    /// 展示的读数换了（比如网络从下行改成上行）就重新记，免得两条曲线接在一起。
    trends: HashMap<Metric, (Show, VecDeque<f64>)>,
}

/// 把读数记进走势。
fn record(
    trends: &mut HashMap<Metric, (Show, VecDeque<f64>)>,
    metric: Metric,
    show: Show,
    number: Option<f64>,
) {
    let (shown, history) = trends
        .entry(metric)
        .or_insert_with(|| (show, VecDeque::with_capacity(TREND_POINTS + 1)));
    if *shown != show {
        *shown = show;
        history.clear();
    }
    if let Some(number) = number {
        history.push_back(number);
        if history.len() > TREND_POINTS {
            history.pop_front();
        }
    }
}

/// 把位图直接做成 `NSImage`，不经过 PNG 编码再解码。
///
/// 位图的像素本来就是预乘 alpha 的 RGBA，和 `NSBitmapImageRep` 的原生布局一致，拷一遍就行。
/// 走 PNG 的话每次更新都要压缩一遍、再让系统解码一遍，每次还会让 CoreGraphics 为新解码出来的图
/// 建一份颜色转换表缓存，运行久了这些缓存能累积出二十来 MB。
fn ns_image(image: &StatusImage, template: bool) -> Option<Retained<NSImage>> {
    let (width, height) = (
        isize::try_from(image.width).ok()?,
        isize::try_from(image.height).ok()?,
    );
    let row = width.checked_mul(4)?;
    if image.rgba.len() != usize::try_from(row.checked_mul(height)?).ok()? {
        return None;
    }
    // SAFETY: `planes` 传空指针，像素缓冲由 AppKit 分配；下面只在 `bytesPerRow * height`
    // 的范围内写入，且先核对过 AppKit 实际给的行距。
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            row,
            32,
        )
    }?;
    if rep.bytesPerRow() != row {
        return None;
    }
    let pixels = rep.bitmapData();
    if pixels.is_null() {
        return None;
    }
    // SAFETY: 缓冲至少有 `row * height == image.rgba.len()` 字节，且和 `image.rgba` 不重叠。
    unsafe { std::ptr::copy_nonoverlapping(image.rgba.as_ptr(), pixels, image.rgba.len()) };
    // 位图是 2 倍分辨率：像素尺寸不变，逻辑尺寸减半。
    let size = NSSize::new(image.points(), IMAGE_HEIGHT);
    rep.setSize(size);
    let nsimage = NSImage::initWithSize(NSImage::alloc(), size);
    nsimage.addRepresentation(&rep);
    nsimage.setTemplate(template);
    Some(nsimage)
}

impl Slot {
    /// 把新的一帧交给菜单栏。长度没变就直接换按钮上的图；长度变了还要同步状态项的长度，
    /// 这种事一天也没几次，走 `tray-icon` 自带的路径，由它负责把点击区域一并调整好。
    fn show(&mut self, image: &StatusImage, colorful: bool) -> bool {
        let mtm = MainThreadMarker::new();
        let length = image.points();
        let same_length = (length - self.length).abs() < 0.01;
        if same_length
            && let Some(mtm) = mtm
            && let Some(button) = self.icon.ns_status_item().and_then(|item| item.button(mtm))
            && let Some(nsimage) = ns_image(image, !colorful)
        {
            button.setImage(Some(&nsimage));
            return true;
        }
        let Ok(icon) = Icon::from_rgba(image.straight_rgba(), image.width, image.height) else {
            return false;
        };
        let result = if colorful {
            self.icon.set_icon(Some(icon))
        } else {
            self.icon.set_icon_templated(Some(icon))
        };
        if result.is_err() {
            return false;
        }
        if let Some(native) = self.icon.ns_status_item() {
            native.setLength(length);
        }
        self.length = length;
        self.resync = 3;
        true
    }

    /// 悬停提示：内容变了、并且离上次更新够久才写给 AppKit。
    fn update_tooltip(&mut self, text: impl FnOnce() -> String) {
        let resync = self.resync > 0;
        self.resync = self.resync.saturating_sub(1);
        let due = self
            .tooltip_at
            .is_none_or(|at| at.elapsed() >= TOOLTIP_EVERY);
        if !(due || resync) {
            return;
        }
        if due {
            self.tooltip_at = Some(Instant::now());
            let text = text();
            if text != self.tooltip {
                self.tooltip = text;
            } else if !resync {
                return;
            }
        }
        // `set_tooltip` 顺带会让接收点击的视图按按钮当前的大小重新对齐。
        let _ = self.icon.set_tooltip(Some(self.tooltip.as_str()));
    }
}

impl NativeItems {
    pub fn sync(
        &mut self,
        settings: &Settings,
        tick: Option<&MonitorTick>,
        snapshot: &SystemSnapshot,
    ) {
        prepare();
        let desired: Vec<_> = settings
            .menu_bar
            .visible()
            .filter(|_| settings.menu_bar.enabled)
            .map(|i| i.metric)
            .collect();
        let current: Vec<_> = self.slots.iter().map(|i| i.metric).collect();
        if desired != current {
            self.slots.clear();
            // 新插入的 NSStatusItem 在已有项左边，因此反向创建，保持配置的从左到右顺序。
            for (index, &metric) in desired.iter().enumerate().rev() {
                let (style, show) = settings.menu_bar.look(metric);
                let reading = data::reading(metric, show, tick, snapshot, settings);
                // 这一帧马上会被下面的 `refresh` 换掉，走势留给它去记。
                let drawing = status_image::draw(
                    &Item {
                        metric,
                        style,
                        show,
                        reading: &reading,
                        history: &[],
                        separator: index > 0,
                    },
                    settings,
                );
                let Some(image) = drawing.rasterize() else {
                    continue;
                };
                let length = image.points();
                let Ok(icon) = Icon::from_rgba(image.straight_rgba(), image.width, image.height)
                else {
                    continue;
                };
                let builder = TrayIconBuilder::new()
                    .with_id(format!("pc-specs.metric.{}", metric.key()))
                    .with_autosave_name(format!("pc-specs.monitor.slot.{index}"))
                    .with_menu(Box::new(super::super::tray::quick_menu()))
                    .with_menu_on_left_click(false)
                    .with_tooltip(metric.title());
                let builder = if settings.menu_bar.colorful {
                    builder.with_icon(icon)
                } else {
                    builder.with_icon_templated(icon)
                };
                match builder.build() {
                    Ok(icon) => {
                        if let Some(native) = icon.ns_status_item() {
                            native.setLength(length);
                        }
                        self.slots.push(Slot {
                            metric,
                            icon,
                            svg: drawing.into_svg(),
                            length,
                            highlighted: false,
                            tooltip: String::new(),
                            tooltip_at: None,
                            resync: 3,
                        });
                    }
                    Err(error) => tracing::warn!("could not create menu bar item: {error}"),
                }
            }
            self.slots.reverse();
        }
        self.refresh(settings, tick, snapshot);
        tracing::debug!(?desired, "configured system monitor items");
    }

    pub fn refresh(
        &mut self,
        settings: &Settings,
        tick: Option<&MonitorTick>,
        snapshot: &SystemSnapshot,
    ) {
        let colorful = settings.menu_bar.colorful;
        for (index, slot) in self.slots.iter_mut().enumerate() {
            let metric = slot.metric;
            let (style, show) = settings.menu_bar.look(metric);
            let reading = data::reading(metric, show, tick, snapshot, settings);
            record(&mut self.trends, metric, show, reading.number);
            let history: Vec<f64> = match self.trends.get(&metric) {
                Some((_, history)) if matches!(style, ItemStyle::Graph | ItemStyle::Ring) => {
                    history.iter().copied().collect()
                }
                _ => Vec::new(),
            };
            let drawing = status_image::draw(
                &Item {
                    metric,
                    style,
                    show,
                    reading: &reading,
                    history: &history,
                    separator: index > 0,
                },
                settings,
            );
            // 数值一秒一变，但很多时候版面上的字没变（比如 CPU 还是 3%），这时什么都不用做。
            if drawing.svg() != slot.svg
                && let Some(image) = drawing.rasterize()
                && slot.show(&image, colorful)
            {
                slot.svg = drawing.into_svg();
            }
            slot.update_tooltip(|| format!("{}  {}", metric.title(), reading.plain()));
        }
    }

    /// 弹窗打开期间让对应的菜单栏项保持按下的高亮，像下拉菜单一样。
    pub fn highlight(&mut self, metric: Option<Metric>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        for slot in &mut self.slots {
            let on = metric == Some(slot.metric);
            if slot.highlighted == on {
                continue;
            }
            slot.highlighted = on;
            if let Some(button) = slot.icon.ns_status_item().and_then(|item| item.button(mtm)) {
                button.highlight(on);
            }
        }
    }

    /// 没有点击位置时（比如从主图标菜单进来）退回到主屏上的状态项。
    pub fn anchor(&self, metric: Metric) -> Option<Anchor> {
        let mtm = MainThreadMarker::new()?;
        let slot = self.slots.iter().find(|s| s.metric == metric)?;
        let button = slot.icon.ns_status_item()?.button(mtm)?;
        let window = button.window()?;
        anchor_of(&window)
    }
}

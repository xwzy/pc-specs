//! 空间分析页的四种状态：选位置、扫描中、出错、结果（树图 + 信息栏 + 列表）。

use super::search::{AGES, Query, SIZES};
use super::treemap::{self, Palette, category_color};
use super::{
    ListKind, MapKind, MapScene, Report, Row, Running, Scan, StoragePage, Target, collector,
    duplicates, sunburst, user_path, volume_target,
};
use crate::settings::SizeUnit;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Bounds, Context, Entity, Hsla, InteractiveElement as _, IntoElement as _,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, Pixels, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, prelude::FluentBuilder as _,
    px, relative, rems, transparent_black,
};
use pc_specs_core::space::{self, Category, Kind};
use rust_i18n::t;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;

/// 本页用到的几个主题色，先取出来，省得到处借着 `cx`。
#[derive(Clone, Copy)]
pub(super) struct Colors {
    pub(super) border: Hsla,
    pub(super) muted: Hsla,
    pub(super) text: Hsla,
    pub(super) hover: Hsla,
    pub(super) active: Hsla,
    pub(super) warning: Hsla,
    pub(super) success: Hsla,
    pub(super) radius: Pixels,
    pub(super) radius_lg: Pixels,
    pub(super) dark: bool,
}

impl Colors {
    pub(super) fn new(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            border: theme.border,
            muted: theme.muted_foreground,
            text: theme.foreground,
            hover: theme.list_hover,
            active: theme.list_active,
            warning: theme.warning,
            success: theme.success,
            radius: theme.radius,
            radius_lg: theme.radius_lg,
            dark: theme.mode.is_dark(),
        }
    }
}

/// 按分隔符分组的整数：`1,234,567`。
pub(super) fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `part` 占 `whole` 的百分比：小的保留一位小数，免得全显示成 0%。
fn percent_of(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0%".into();
    }
    let p = part as f64 / whole as f64 * 100.0;
    if p >= 10.0 {
        format!("{p:.0}%")
    } else if p >= 0.1 {
        format!("{p:.1}%")
    } else {
        "<0.1%".into()
    }
}

fn category_name(category: Category) -> String {
    match category {
        Category::Video => t!("sa_cat_video"),
        Category::Audio => t!("sa_cat_audio"),
        Category::Image => t!("sa_cat_image"),
        Category::Document => t!("sa_cat_document"),
        Category::Archive => t!("sa_cat_archive"),
        Category::Program => t!("sa_cat_program"),
        Category::Code => t!("sa_cat_code"),
        Category::Other => t!("sa_cat_other"),
    }
    .into_owned()
}

/// 返回 `(页头右侧的操作, 正文)`。
pub(super) fn body(
    page: &mut StoragePage,
    window: &mut Window,
    cx: &mut Context<StoragePage>,
) -> (Vec<AnyElement>, Vec<AnyElement>) {
    if matches!(page.scan, Scan::Done(_)) {
        let actions = header_actions(&cx.entity());
        return (actions, report(page, window, cx));
    }
    let unit = page.app.read(cx).settings.size_unit;
    let body = match &page.scan {
        Scan::Idle => idle(page, unit, cx),
        Scan::Running(run) => running(run, unit, cx),
        Scan::Failed { target, error } => failed(target, error, cx),
        Scan::Done(_) => Vec::new(),
    };
    (Vec::new(), body)
}

// ---- 选位置 -------------------------------------------------------------------

/// 一张可点的位置卡片：图标 + 名称 + 路径，卷还带一根容量条。
#[allow(clippy::too_many_arguments)]
fn tile(
    id: String,
    icon: IconName,
    title: String,
    detail: String,
    usage: Option<(u64, u64)>,
    unit: SizeUnit,
    colors: Colors,
    on_click: impl Fn(&mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let id = SharedString::from(id);
    v_flex()
        .id(id.clone())
        .gap_2()
        .p_3()
        .min_w_0()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .cursor_pointer()
        .hover(move |style| style.bg(colors.hover))
        .on_click(move |_, _, cx| on_click(cx))
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(Icon::new(icon).text_color(colors.muted))
                .child(div().font_semibold().truncate().child(title)),
        )
        .child(
            div()
                .text_xs()
                .text_color(colors.muted)
                .truncate()
                .child(detail),
        )
        .when_some(usage, |this, (used, total)| {
            this.child(ui::usage_bar(
                SharedString::from(format!("{id}-bar")),
                fmt::ratio(used, total),
                cx,
            ))
            .child(
                ui::muted(
                    t!(
                        "sa_used_of",
                        used = fmt::bytes(used, unit),
                        total = fmt::bytes(total, unit)
                    )
                    .into_owned(),
                    cx,
                )
                .text_xs(),
            )
        })
        .into_any_element()
}

fn idle(page: &StoragePage, unit: SizeUnit, cx: &mut Context<StoragePage>) -> Vec<AnyElement> {
    let entity = cx.entity();
    let colors = Colors::new(cx);
    // 区块左右还有内边距，网格可用的宽度要比页面正文窄一点。
    let cols = ui::columns(page.content_width - 40.0, 250.0, 3);
    let grid = |tiles: Vec<AnyElement>| {
        div()
            .w_full()
            .grid()
            .grid_cols(cols)
            .gap_3()
            .children(tiles)
            .into_any_element()
    };

    let storages = page.app.read(cx).snapshot.storages.clone();
    let mut seen = HashSet::new();
    let mut disks = Vec::new();
    for (i, d) in storages.iter().enumerate() {
        let Some(mount) = d.mount_point.as_deref().filter(|m| !m.is_empty()) else {
            continue;
        };
        let target = volume_target(&d.name, mount);
        if !seen.insert(target.path.clone()) {
            continue;
        }
        // 扫描的起点可能不是挂载点本身（macOS 的数据卷），容量以起点所在的卷为准。
        let usage = space::volume_of(&target.path)
            .map(|v| (v.used, v.total))
            .unwrap_or((d.used_bytes, d.total_bytes));
        let detail = user_path(target.path.clone()).display().to_string();
        let page = entity.clone();
        disks.push(tile(
            format!("sa-disk-{i}"),
            IconName::HardDrive,
            target.label.clone(),
            detail,
            Some(usage),
            unit,
            colors,
            move |cx| {
                let target = target.clone();
                page.update(cx, |page, cx| page.start(target, cx));
            },
            cx,
        ));
    }

    // 最近扫过的文件夹（已经不在了的不列）。
    let mut recent = Vec::new();
    let remembered = page.app.read(cx).settings.storage_recent.clone();
    for (i, path) in remembered.iter().map(PathBuf::from).enumerate() {
        if !path.is_dir() {
            continue;
        }
        let detail = path.display().to_string();
        let target = super::folder_target(path);
        let (title, entity) = (target.label.clone(), entity.clone());
        recent.push(tile(
            format!("sa-recent-{i}"),
            IconName::Clock,
            title,
            detail,
            None,
            unit,
            colors,
            move |cx| {
                let target = target.clone();
                entity.update(cx, |page, cx| page.start(target, cx));
            },
            cx,
        ));
    }

    let mut folders = Vec::new();
    let quick = [
        (
            "home",
            IconName::House,
            t!("sa_target_home").into_owned(),
            dirs::home_dir(),
        ),
        (
            "downloads",
            IconName::Download,
            t!("sa_target_downloads").into_owned(),
            dirs::download_dir(),
        ),
    ];
    for (id, icon, title, path) in quick {
        let Some(path) = path.filter(|p| p.is_dir()) else {
            continue;
        };
        let detail = path.display().to_string();
        let page = entity.clone();
        let mut target = super::folder_target(path);
        target.label = title.clone();
        folders.push(tile(
            format!("sa-folder-{id}"),
            icon,
            title,
            detail,
            None,
            unit,
            colors,
            move |cx| {
                let target = target.clone();
                page.update(cx, |page, cx| page.start(target, cx));
            },
            cx,
        ));
    }
    let page = entity.clone();
    folders.push(tile(
        "sa-folder-choose".into(),
        IconName::FolderSearch,
        t!("sa_target_choose").into_owned(),
        t!("sa_target_choose_desc").into_owned(),
        None,
        unit,
        colors,
        move |cx| page.update(cx, |page, cx| page.choose_folder(cx)),
        cx,
    ));

    let mut body = vec![ui::muted(t!("sa_idle_desc").into_owned(), cx).into_any_element()];
    if !disks.is_empty() {
        body.push(
            ui::section("sa-disks", t!("sa_section_disks").into_owned())
                .child(grid(disks))
                .into_any_element(),
        );
    }
    if !recent.is_empty() {
        body.push(
            ui::section("sa-recent", t!("sa_section_recent").into_owned())
                .child(grid(recent))
                .into_any_element(),
        );
    }
    body.push(
        ui::section("sa-folders", t!("sa_section_folders").into_owned())
            .child(grid(folders))
            .into_any_element(),
    );
    body.push(
        ui::muted(t!("sa_tip_slow").into_owned(), cx)
            .text_xs()
            .into_any_element(),
    );
    body
}

// ---- 扫描中 / 出错 --------------------------------------------------------------

fn running(run: &Running, unit: SizeUnit, cx: &mut Context<StoragePage>) -> Vec<AnyElement> {
    let entity = cx.entity();
    let files = run.progress.files.load(Ordering::Relaxed);
    let dirs = run.progress.dirs.load(Ordering::Relaxed);
    let bytes = run.progress.bytes.load(Ordering::Relaxed);
    // 整块磁盘能按「已扫到的字节 / 卷已用字节」估个进度；文件夹没有参照，只显示在动。
    let percent = (run.target.volume && run.expected > 0)
        .then(|| ((bytes as f64 / run.expected as f64) * 100.0).min(99.0) as f32);
    let bar = match percent {
        Some(value) => Progress::new("sa-progress").value(value),
        None => Progress::new("sa-progress").loading(true),
    };
    let mut elapsed = fmt::duration(run.started.elapsed().as_secs());
    if let Some(value) = percent {
        elapsed = format!(
            "{} · {}",
            t!("sa_scan_progress", pct = value.round()),
            elapsed
        );
    }
    vec![
        ui::section(
            "sa-running",
            t!("sa_scanning", name = run.target.label).into_owned(),
        )
        .child(
            v_flex()
                .gap_3()
                .child(bar)
                .child(
                    h_flex()
                        .justify_between()
                        .flex_wrap()
                        .gap_3()
                        .text_sm()
                        .child(
                            div().child(
                                t!(
                                    "sa_scanning_stats",
                                    files = count(files),
                                    folders = count(dirs),
                                    size = fmt::bytes(bytes, unit)
                                )
                                .into_owned(),
                            ),
                        )
                        .child(ui::muted(elapsed, cx)),
                )
                .child(
                    ui::muted(
                        super::user_path(run.progress.current().into())
                            .display()
                            .to_string(),
                        cx,
                    )
                    .text_xs()
                    .truncate(),
                )
                .child(
                    h_flex().child(
                        Button::new("sa-cancel")
                            .small()
                            .outline()
                            .icon(IconName::X)
                            .label(t!("sa_cancel").into_owned())
                            .on_click(move |_, _, cx| entity.update(cx, |p, cx| p.cancel(cx))),
                    ),
                ),
        )
        .into_any_element(),
    ]
}

fn failed(target: &Target, error: &str, cx: &mut Context<StoragePage>) -> Vec<AnyElement> {
    let (retry, other) = (cx.entity(), cx.entity());
    vec![
        ui::section(
            "sa-failed",
            t!("sa_failed_title", name = target.label).into_owned(),
        )
        .child(
            v_flex()
                .gap_3()
                .child(ui::muted(error.to_string(), cx))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("sa-retry")
                                .small()
                                .outline()
                                .icon(IconName::RefreshCw)
                                .label(t!("sa_retry").into_owned())
                                .on_click(move |_, _, cx| retry.update(cx, |p, cx| p.rescan(cx))),
                        )
                        .child(
                            Button::new("sa-other")
                                .small()
                                .ghost()
                                .label(t!("sa_new_scan").into_owned())
                                .on_click(move |_, _, cx| other.update(cx, |p, cx| p.close(cx))),
                        ),
                ),
        )
        .into_any_element(),
    ]
}

// ---- 结果 ---------------------------------------------------------------------

fn report(
    page: &mut StoragePage,
    window: &mut Window,
    cx: &mut Context<StoragePage>,
) -> Vec<AnyElement> {
    let entity = cx.entity();
    let colors = Colors::new(cx);
    let unit = page.app.read(cx).settings.size_unit;
    let busy = page.trashing;
    // 树图上面还有标题、工具栏、汇总、面包屑和信息栏，约占 `ABOVE_MAP` 这么高：
    // 留出这些之后，树图正好在不滚动的情况下完整露出来。
    let height = (window.viewport_size().height.as_f32() - ABOVE_MAP).clamp(320.0, 760.0);
    // 列表要先算好（会更新缓存，需要 `&mut`），之后才能共享借用整个 `report`。
    let (query, search) = (page.query.clone(), page.search.clone());
    let rows = page
        .report_mut()
        .map(|report| report.rows(&query))
        .unwrap_or_default();
    let Some(report) = page.report() else {
        return Vec::new();
    };
    vec![
        summary(report, unit, &entity, colors, cx),
        navigation(report, &entity, colors, cx),
        info_bar(report, unit, &entity, colors, cx),
        canvas_view(report, height, &entity, colors, cx),
        collector::bar(report, unit, busy, &entity, cx),
        list(report, &rows, &query, &search, unit, &entity, colors, cx),
    ]
}

/// 树图上方所有东西加起来大约的高度（含窗口标题栏、底部状态栏和页面边距），
/// 外加树图下面那一条收集箱：这样树图和收集箱正好在不滚动的情况下一起露出来。
const ABOVE_MAP: f32 = 410.0 + collector::BAR_HEIGHT + 8.0;

/// 页头右侧：导出、换个位置扫、重新扫一遍。
fn header_actions(entity: &Entity<StoragePage>) -> Vec<AnyElement> {
    let (export, new_scan, rescan) = (entity.clone(), entity.clone(), entity.clone());
    vec![
        Button::new("sa-export")
            .small()
            .outline()
            .icon(IconName::FileDown)
            .label(t!("sa_export_csv").into_owned())
            .tooltip(t!("sa_export_csv_tip").into_owned())
            .on_click(move |_, _, cx| export.update(cx, |p, cx| p.export_csv(cx)))
            .into_any_element(),
        Button::new("sa-new")
            .small()
            .outline()
            .icon(IconName::FolderSearch)
            .label(t!("sa_new_scan").into_owned())
            .on_click(move |_, _, cx| new_scan.update(cx, |p, cx| p.close(cx)))
            .into_any_element(),
        Button::new("sa-rescan")
            .small()
            .outline()
            .icon(IconName::RefreshCw)
            .label(t!("sa_rescan").into_owned())
            .on_click(move |_, _, cx| rescan.update(cx, |p, cx| p.rescan(cx)))
            .into_any_element(),
    ]
}

fn summary(
    report: &Report,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &mut Context<StoragePage>,
) -> AnyElement {
    let tree = &report.tree;
    // 数字、占比条和图例说的都是「正在看的这个目录」，和下面的树图对得上。
    let view = report.view_node().unwrap_or(&tree.root);
    let at_root = report.view.is_empty();
    let total = view.size;
    let mut kinds: Vec<(Category, u64)> = Category::ALL
        .iter()
        .map(|&c| (c, view.category_size(c)))
        .filter(|&(_, bytes)| bytes > 0)
        .collect();
    kinds.sort_by_key(|&(_, bytes)| Reverse(bytes));

    // 各类型的占比条。
    let bar = h_flex()
        .w_full()
        .h(px(10.))
        .gap_0p5()
        .children(kinds.iter().map(|&(category, bytes)| {
            div()
                .h_full()
                .w(relative(bytes as f32 / total.max(1) as f32))
                .bg(category_color(category, colors.dark))
        }));
    // 图例同时是过滤器：点一下只突出这个类型，再点一下取消。
    let legend = h_flex()
        .flex_wrap()
        .gap_1()
        .children(kinds.iter().map(|&(category, bytes)| {
            let active = report.filter == Some(category);
            let page = entity.clone();
            let hint = t!("sa_filter_hint").into_owned();
            h_flex()
                .id(("sa-kind", category.index()))
                .items_center()
                .gap_1p5()
                .px_2()
                .py_1()
                .rounded(colors.radius)
                .border_1()
                .border_color(if active {
                    colors.text
                } else {
                    transparent_black()
                })
                .cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                .on_click(move |_, _, cx| {
                    page.update(cx, |page, cx| page.set_filter(Some(category), cx))
                })
                .child(
                    div()
                        .size(px(10.))
                        .flex_shrink_0()
                        .bg(category_color(category, colors.dark)),
                )
                .child(div().text_sm().child(category_name(category)))
                .child(div().text_sm().text_color(colors.muted).child(format!(
                    "{} · {}",
                    fmt::bytes(bytes, unit),
                    percent_of(bytes, total)
                )))
        }));

    // 整个扫描的概况只在最上层有意义；钻进子目录之后只说这个目录里有多少文件。
    let stats = if at_root {
        t!(
            "sa_summary_stats",
            files = count(u64::from(tree.root.files)),
            folders = count(tree.dirs),
            time = fmt::duration(tree.elapsed.as_secs())
        )
        .into_owned()
    } else {
        t!("sa_files_count", n = count(u64::from(view.files))).into_owned()
    };

    // 所在磁盘的整体情况。
    let volume = tree.volume.map(|v| {
        t!(
            "sa_volume_line",
            used = fmt::bytes(v.used, unit),
            total = fmt::bytes(v.total, unit),
            free = fmt::bytes(v.total.saturating_sub(v.used), unit)
        )
        .into_owned()
    });
    // 整块磁盘扫完，总量比卷的已用空间小：差的这部分扫描看不到，单独交代清楚，免得对不上账。
    let whole_disk = at_root && report.target.volume;
    let hidden = tree
        .volume
        .filter(|_| whole_disk)
        .map(|v| v.used.saturating_sub(total))
        .filter(|&gap| gap >= HIDDEN_MIN)
        .map(|gap| {
            let mut text = t!("sa_hidden_desc").into_owned();
            if report.extra.snapshots > 0 {
                text.push_str(&t!("sa_hidden_snapshots", n = report.extra.snapshots));
            }
            note(
                IconName::EyeOff,
                t!("sa_hidden_title", size = fmt::bytes(gap, unit)).into_owned(),
                text,
                colors,
                cx,
            )
        });
    // 可清除空间已经是扫描结果的一部分，这里只是告诉用户其中有多少不用自己动手。
    let purgeable = report.extra.purgeable.filter(|_| whole_disk).map(|bytes| {
        note(
            IconName::Sparkles,
            t!("sa_purgeable_title", size = fmt::bytes(bytes, unit)).into_owned(),
            t!("sa_purgeable_desc").into_owned(),
            colors,
            cx,
        )
    });

    // 有的目录没权限读，大小没算进来：说清楚，macOS 上还给出授权入口。
    let unreadable = (tree.unreadable > 0).then(|| {
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(colors.radius)
            .border_1()
            .border_color(colors.warning.opacity(0.6))
            .child(Icon::new(IconName::TriangleAlert).text_color(colors.warning))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(t!("sa_unreadable", n = count(tree.unreadable)).into_owned()),
            )
            .when(cfg!(target_os = "macos"), |this| {
                this.child(
                    Button::new("sa-fda")
                        .small()
                        .outline()
                        .label(t!("sa_open_fda").into_owned())
                        .on_click(|_, _, cx| {
                            cx.open_url(
                                "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles",
                            )
                        }),
                )
            })
    });

    v_flex()
        .w_full()
        .gap_2()
        .child(
            h_flex()
                .w_full()
                .items_baseline()
                .justify_between()
                .flex_wrap()
                .gap_x_4()
                .gap_y_1()
                .child(
                    h_flex()
                        .items_baseline()
                        .flex_wrap()
                        .gap_x_3()
                        .child(
                            div()
                                .text_2xl()
                                .font_semibold()
                                .child(fmt::bytes(total, unit)),
                        )
                        .child(ui::muted(stats, cx)),
                )
                .children(volume.map(|line| ui::muted(line, cx).text_xs())),
        )
        .child(bar)
        .child(legend)
        .children(hidden)
        .children(purgeable)
        .children(unreadable)
        .into_any_element()
}

/// 差多少以内算误差（文件系统自己的记账、扫描期间的变动），不值得交代。
const HIDDEN_MIN: u64 = 256 << 20;

/// 一条带图标的说明：粗体标题 + 一段灰色解释。
fn note(icon: IconName, title: String, body: String, colors: Colors, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .items_start()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border)
        .child(
            Icon::new(icon)
                .small()
                .flex_shrink_0()
                .mt_0p5()
                .text_color(colors.muted),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_sm().font_semibold().child(title))
                .child(ui::muted(body, cx).text_xs()),
        )
        .into_any_element()
}

/// 「上一级」按钮 + 面包屑。
fn navigation(
    report: &Report,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &mut Context<StoragePage>,
) -> AnyElement {
    let up = entity.clone();
    let levels = entity.clone();
    let map = entity.clone();
    let depth = report.view.len();
    let place = user_path(report.tree.path.clone()).display().to_string();
    let mut crumbs: Vec<AnyElement> = Vec::new();
    for level in 0..=depth {
        let label = if level == 0 {
            report.target.label.clone()
        } else {
            report
                .tree
                .root
                .at(&report.view[..level])
                .map(|node| node.name.to_string())
                .unwrap_or_default()
        };
        if level > 0 {
            crumbs.push(
                Icon::new(IconName::ChevronRight)
                    .small()
                    .text_color(colors.muted)
                    .into_any_element(),
            );
        }
        let page = entity.clone();
        let current = level == depth;
        crumbs.push(
            div()
                .id(("sa-crumb", level))
                .px_1p5()
                .py_0p5()
                .max_w(px(240.))
                .truncate()
                .text_sm()
                .rounded(colors.radius)
                .when(current, |this| this.font_semibold())
                // 根这一级的提示是扫描的完整路径。
                .when_some((level == 0).then(|| place.clone()), |this, tip| {
                    this.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                })
                .when(!current, |this| {
                    this.text_color(colors.muted)
                        .cursor_pointer()
                        .hover(move |style| style.bg(colors.hover))
                        .on_click(move |_, _, cx| page.update(cx, |p, cx| p.crumb(level, cx)))
                })
                .child(label)
                .into_any_element(),
        );
    }
    h_flex()
        .w_full()
        .items_center()
        .gap_2()
        .child(
            Button::new("sa-up")
                .small()
                .ghost()
                .icon(IconName::ArrowUp)
                .accessibility_label(t!("sa_up").into_owned())
                .tooltip(t!("sa_up").into_owned())
                .disabled(depth == 0)
                .on_click(move |_, _, cx| up.update(cx, |p, cx| p.up(cx))),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap_1()
                .children(crumbs),
        )
        .child(ui::segmented(
            "sa-map-kind",
            vec![
                (MapKind::Treemap, t!("sa_map_treemap").into_owned()),
                (MapKind::Sunburst, t!("sa_map_sunburst").into_owned()),
            ],
            report.map,
            move |kind, cx| map.update(cx, |p, cx| p.set_map(kind, cx)),
        ))
        .child(
            h_flex()
                .flex_shrink_0()
                .gap_2()
                .items_center()
                .child(ui::muted(t!("sa_levels").into_owned(), cx))
                .child(ui::segmented(
                    "sa-levels",
                    (1u8..=4).map(|n| (n, n.to_string())).collect(),
                    report.levels,
                    move |n, cx| levels.update(cx, |p, cx| p.set_levels(n, cx)),
                )),
        )
        .into_any_element()
}

/// 树图本体：一块 canvas，鼠标事件换算成「哪一块」之后交给页面。
fn canvas_view(
    report: &Report,
    height: f32,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &mut Context<StoragePage>,
) -> AnyElement {
    let frame = div()
        .id("sa-treemap")
        .relative()
        .w_full()
        .h(px(height))
        .border_1()
        .border_color(colors.border);

    let nothing = report
        .view_node()
        .is_none_or(|node| node.children.iter().all(|child| child.size == 0));
    if nothing {
        return frame
            .child(ui::empty_state(
                IconName::Folder,
                t!("sa_empty_folder").into_owned(),
                "",
            ))
            .into_any_element();
    }

    let paint_page = entity.clone();
    let surface = canvas(
        move |bounds: Bounds<Pixels>, _, cx| {
            paint_page.update(cx, |page, cx| page.scene(bounds, cx))
        },
        |bounds, scene, window, cx| match scene {
            Some(MapScene::Tiles(scene)) => treemap::paint(&scene, bounds, window, cx),
            Some(MapScene::Rings(scene)) => sunburst::paint(&scene, bounds, window, cx),
            None => {}
        },
    )
    .size_full();

    let menu_page = entity.clone();
    frame
        .cursor_pointer()
        .on_mouse_move(
            cx.listener(|this, event: &MouseMoveEvent, _, cx| this.hover_at(event.position, cx)),
        )
        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
            if !*hovered {
                this.leave(cx);
            }
        }))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                window.focus(&this.focus, cx);
                this.press(event.position, event.click_count, false, cx)
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                window.focus(&this.focus, cx);
                this.press(event.position, 1, true, cx)
            }),
        )
        .child(surface)
        // 右键菜单针对右键时选中的那一项；没点在任何块上就是空菜单，不会弹出。
        .context_menu(move |menu, _, cx| {
            let selected = menu_page
                .read(cx)
                .report()
                .and_then(|report| report.selected.clone());
            match selected {
                Some(path) => item_menu(&menu_page, path, menu, cx),
                None => menu,
            }
        })
        .into_any_element()
}

/// 对一项可做的操作，树图和列表的右键菜单共用。
fn item_menu(page: &Entity<StoragePage>, path: Vec<u32>, menu: PopupMenu, cx: &App) -> PopupMenu {
    let Some(item) = page.read(cx).report().and_then(|report| report.item(&path)) else {
        return menu;
    };
    let mut menu = menu;
    if item.kind == Kind::Dir {
        let (page, path) = (page.clone(), path.clone());
        menu = menu.item(
            PopupMenuItem::new(t!("sa_act_open").into_owned())
                .icon(IconName::FolderOpen)
                .on_click(move |_, _, cx| page.update(cx, |p, cx| p.zoom(path.clone(), cx))),
        );
    }
    if item.kind == Kind::File {
        let (page, path) = (page.clone(), path.clone());
        menu = menu.item(
            PopupMenuItem::new(t!("sa_act_open_file").into_owned())
                .icon(IconName::AppWindow)
                .on_click(move |_, _, cx| page.update(cx, |p, cx| p.open_item(&path, cx))),
        );
    }
    if item.kind == Kind::File && cfg!(target_os = "macos") {
        let (page, path) = (page.clone(), path.clone());
        menu = menu.item(
            PopupMenuItem::new(t!("sa_act_preview").into_owned())
                .icon(IconName::Eye)
                .on_click(move |_, _, cx| page.update(cx, |p, cx| p.preview(&path, cx))),
        );
    }
    {
        let (page, path) = (page.clone(), path.clone());
        menu = menu.item(
            PopupMenuItem::new(ui::reveal_label())
                .icon(IconName::ExternalLink)
                .on_click(move |_, _, cx| page.update(cx, |p, cx| p.reveal(&path, cx))),
        );
    }
    {
        let (page, path) = (page.clone(), path.clone());
        menu = menu.item(
            PopupMenuItem::new(t!("sa_act_copy").into_owned())
                .icon(IconName::Copy)
                .on_click(move |_, _, cx| page.update(cx, |p, cx| p.copy_path(&path, cx))),
        );
    }
    if item.kind != Kind::Rest {
        let collected = page
            .read(cx)
            .report()
            .is_some_and(|report| report.is_collected(&path));
        let (page, path) = (page.clone(), path.clone());
        menu = menu.separator().item(
            PopupMenuItem::new(if collected {
                t!("sa_act_uncollect").into_owned()
            } else {
                t!("sa_act_collect").into_owned()
            })
            .icon(IconName::ShoppingBasket)
            .on_click({
                let (page, path) = (page.clone(), path.clone());
                move |_, _, cx| page.update(cx, |p, cx| p.toggle_collect(path.clone(), cx))
            }),
        );
        menu = menu.item(
            PopupMenuItem::new(t!("sa_act_trash", bin = super::bin_name()).into_owned())
                .icon(IconName::Trash)
                .on_click(move |_, window, cx| {
                    page.update(cx, |p, cx| p.ask_trash(path.clone(), window, cx))
                }),
        );
    }
    menu
}

/// 树图上面的信息栏：悬停（没有悬停就是选中）那一项的详情，和对选中项的操作。
/// 高度固定：悬停、选中都不会把下面的树图顶得跳动。
fn info_bar(
    report: &Report,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &mut Context<StoragePage>,
) -> AnyElement {
    let shown = report
        .hover
        .as_ref()
        .or(report.selected.as_ref())
        .and_then(|path| report.item(path));
    let selected = report.selected.as_ref().and_then(|path| report.item(path));
    let view_size = report.view_node().map_or(0, |node| node.size);

    let details = match &shown {
        None => div()
            .text_sm()
            .text_color(colors.muted)
            .truncate()
            .child(match report.map {
                MapKind::Treemap => t!("sa_hint").into_owned(),
                MapKind::Sunburst => t!("sa_hint_rings").into_owned(),
            })
            .into_any_element(),
        Some(item) => {
            let is_rest = item.kind == Kind::Rest;
            let name = if is_rest {
                t!("sa_rest").into_owned()
            } else {
                item.name.clone()
            };
            let mut facts = vec![fmt::bytes(item.size, unit)];
            if view_size > 0 {
                facts.push(t!("sa_share_of", pct = percent_of(item.size, view_size)).into_owned());
            }
            if is_rest {
                facts.push(t!("sa_rest_desc", n = count(u64::from(item.files))).into_owned());
            } else if item.kind == Kind::Dir {
                facts.push(t!("sa_files_count", n = count(u64::from(item.files))).into_owned());
            } else if item.modified > 0 {
                facts.push(
                    t!(
                        "sa_modified",
                        date = fmt::datetime(u64::from(item.modified))
                    )
                    .into_owned(),
                );
            }
            let icon = match item.kind {
                Kind::Dir => IconName::Folder,
                Kind::File => IconName::File,
                Kind::Rest => IconName::Ellipsis,
            };
            v_flex()
                .gap_0p5()
                .min_w_0()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .min_w_0()
                        .child(
                            Icon::new(icon)
                                .small()
                                .flex_shrink_0()
                                .text_color(category_color(item.category, colors.dark)),
                        )
                        .child(div().min_w_0().font_semibold().truncate().child(name))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_sm()
                                .text_color(colors.muted)
                                .child(facts.join(" · ")),
                        ),
                )
                .child(
                    ui::muted(item.full.display().to_string(), cx)
                        .text_xs()
                        .truncate(),
                )
                .into_any_element()
        }
    };

    // 操作针对选中的那一项；只放图标，名字在悬停提示里，免得窄窗口里换行把栏撑高。
    let actions = selected.map(|item| {
        let path = item.path.clone();
        let mut buttons = h_flex().flex_shrink_0().gap_1();
        if item.kind == Kind::Dir {
            let (page, path) = (entity.clone(), path.clone());
            buttons = buttons.child(
                Button::new("sa-open")
                    .small()
                    .outline()
                    .icon(IconName::ZoomIn)
                    .tooltip(t!("sa_act_open").into_owned())
                    .on_click(move |_, _, cx| page.update(cx, |p, cx| p.zoom(path.clone(), cx))),
            );
        }
        if item.kind == Kind::File {
            let (page, path) = (entity.clone(), path.clone());
            buttons = buttons.child(
                Button::new("sa-open-file")
                    .small()
                    .outline()
                    .icon(IconName::AppWindow)
                    .tooltip(t!("sa_act_open_file").into_owned())
                    .on_click(move |_, _, cx| page.update(cx, |p, cx| p.open_item(&path, cx))),
            );
        }
        if item.kind == Kind::File && cfg!(target_os = "macos") {
            let (page, path) = (entity.clone(), path.clone());
            buttons = buttons.child(
                Button::new("sa-preview")
                    .small()
                    .outline()
                    .icon(IconName::Eye)
                    .tooltip(t!("sa_act_preview").into_owned())
                    .on_click(move |_, _, cx| page.update(cx, |p, cx| p.preview(&path, cx))),
            );
        }
        {
            let (page, path) = (entity.clone(), path.clone());
            buttons = buttons.child(
                Button::new("sa-reveal")
                    .small()
                    .outline()
                    .icon(IconName::ExternalLink)
                    .tooltip(ui::reveal_label())
                    .on_click(move |_, _, cx| page.update(cx, |p, cx| p.reveal(&path, cx))),
            );
        }
        {
            let (page, path) = (entity.clone(), path.clone());
            buttons = buttons.child(
                Button::new("sa-copy")
                    .small()
                    .outline()
                    .icon(IconName::Copy)
                    .tooltip(t!("sa_act_copy").into_owned())
                    .on_click(move |_, _, cx| page.update(cx, |p, cx| p.copy_path(&path, cx))),
            );
        }
        if item.kind != Kind::Rest {
            let collected = report.is_collected(&path);
            let (page, path) = (entity.clone(), path.clone());
            buttons = buttons.child(
                Button::new("sa-collect")
                    .small()
                    .outline()
                    .selected(collected)
                    .icon(IconName::ShoppingBasket)
                    .tooltip(if collected {
                        t!("sa_act_uncollect").into_owned()
                    } else {
                        t!("sa_act_collect").into_owned()
                    })
                    .on_click(move |_, _, cx| {
                        page.update(cx, |p, cx| p.toggle_collect(path.clone(), cx))
                    }),
            );
        }
        if item.kind != Kind::Rest {
            let (page, path) = (entity.clone(), item.path.clone());
            buttons = buttons.child(
                Button::new("sa-trash")
                    .small()
                    .outline()
                    .danger()
                    .icon(IconName::Trash)
                    .tooltip(t!("sa_act_trash", bin = super::bin_name()).into_owned())
                    .on_click(move |_, window, cx| {
                        page.update(cx, |p, cx| p.ask_trash(path.clone(), window, cx))
                    }),
            );
        }
        buttons
    });

    h_flex()
        .w_full()
        .h(px(INFO_HEIGHT))
        .px_3()
        .gap_3()
        .items_center()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .child(div().flex_1().min_w_0().child(details))
        .children(actions)
        .into_any_element()
}

/// 信息栏的高度：放得下两行字和一排小按钮。
const INFO_HEIGHT: f32 = 56.0;

/// 带正负号的大小：`+1.2 GB`、`−300 MB`。
fn signed_bytes(delta: i64, unit: SizeUnit) -> String {
    let sign = if delta < 0 { '−' } else { '+' };
    format!("{sign}{}", fmt::bytes(delta.unsigned_abs(), unit))
}

/// 下面的列表：当前目录里最大的文件、当前目录的直接子项、重复文件，或者和上次扫描相比的变化。
#[allow(clippy::too_many_arguments)]
fn list(
    report: &Report,
    rows: &Rc<Vec<Row>>,
    query: &Query,
    search: &Entity<InputState>,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &mut Context<StoragePage>,
) -> AnyElement {
    let page = entity.clone();
    let mut kinds = vec![
        (ListKind::Largest, t!("sa_list_largest").into_owned()),
        (ListKind::Folder, t!("sa_list_folder").into_owned()),
        (ListKind::Duplicates, t!("sa_list_duplicates").into_owned()),
    ];
    // 和上次扫描的对比只有扫过同一个位置才有。
    if report.history.is_some() {
        kinds.push((ListKind::Changes, t!("sa_list_changes").into_owned()));
    }
    let switch = ui::segmented("sa-list-kind", kinds, report.list, move |kind, cx| {
        page.update(cx, |p, cx| p.set_list(kind, cx))
    });

    if report.list == ListKind::Duplicates {
        return v_flex()
            .gap_2()
            .child(h_flex().items_center().child(switch))
            .child(duplicates::view(report, unit, entity, colors, cx))
            .into_any_element();
    }

    // 搜索与筛选只管「最大文件」和「当前目录」这两个列表；设了条件，它们显示的就是搜索结果。
    let searching =
        query.is_active() && matches!(report.list, ListKind::Largest | ListKind::Folder);
    let filters = matches!(report.list, ListKind::Largest | ListKind::Folder)
        .then(|| filter_bar(query, !rows.is_empty(), search, unit, entity, cx));
    // 「变化」列表的开头说明拿什么比的、整体变了多少。
    let compared = report
        .history
        .as_ref()
        .filter(|_| report.list == ListKind::Changes)
        .map(|history| {
            let now = report.tree.root.size;
            ui::muted(
                t!(
                    "sa_changes_summary",
                    date = fmt::datetime(history.since),
                    delta = signed_bytes(now as i64 - history.before as i64, unit),
                    before = fmt::bytes(history.before, unit),
                    after = fmt::bytes(now, unit)
                )
                .into_owned(),
                cx,
            )
        });
    // 进度条以最大的一行为满格，小文件才看得出差别。
    let widest = rows.first().map_or(0, |row| row.size);
    let shown = rows.len().min(report.list_limit);
    let body: Vec<AnyElement> = rows
        .iter()
        .take(shown)
        .enumerate()
        .map(|(index, row)| {
            let selected = report.selected.as_deref() == Some(row.path.as_slice());
            list_row(index, row, widest, selected, unit, entity, colors)
        })
        .collect();

    let more = (rows.len() > shown).then(|| {
        let page = entity.clone();
        h_flex().justify_center().child(
            Button::new("sa-more")
                .small()
                .ghost()
                .label(t!("sa_list_more").into_owned())
                .on_click(move |_, _, cx| page.update(cx, |p, cx| p.show_more(cx))),
        )
    });
    let none = if report.list == ListKind::Changes {
        t!(
            "sa_changes_none",
            size = fmt::bytes(space::CHANGE_MIN, unit)
        )
        .into_owned()
    } else {
        t!("sa_list_none").into_owned()
    };

    v_flex()
        .gap_2()
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .flex_wrap()
                .gap_2()
                .child(switch)
                .child(ui::muted(
                    {
                        let counter =
                            t!("sa_list_count", shown = shown, total = rows.len()).into_owned();
                        match report.filter {
                            _ if searching => {
                                format!("{} · {}", t!("sa_results"), counter)
                            }
                            Some(category) => format!("{} · {}", category_name(category), counter),
                            None => counter,
                        }
                    },
                    cx,
                )),
        )
        .children(compared)
        .children(filters)
        .child(
            v_flex()
                .w_full()
                .p_1()
                .rounded(colors.radius_lg)
                .border_1()
                .border_color(colors.border)
                .when(body.is_empty(), |this| {
                    this.child(div().p_4().text_sm().text_color(colors.muted).child(none))
                })
                .children(body),
        )
        .children(more)
        .into_any_element()
}

/// 列表上方的搜索与筛选：名称、大小、多久没动过；设了条件之后还能把结果一次全加进收集箱。
fn filter_bar(
    query: &Query,
    has_results: bool,
    search: &Entity<InputState>,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    cx: &App,
) -> AnyElement {
    let (size_page, age_page, all_page, clear_page) = (
        entity.clone(),
        entity.clone(),
        entity.clone(),
        entity.clone(),
    );
    let any = t!("sa_filter_any").into_owned();
    let sizes = SIZES
        .iter()
        .map(|&bytes| {
            let label = if bytes == 0 {
                any.clone()
            } else {
                format!("≥ {}", fmt::bytes(bytes, unit))
            };
            (bytes, label)
        })
        .collect();
    let ages = AGES
        .iter()
        .map(|&days| {
            let label = match days {
                0 => any.clone(),
                365 => t!("sa_age_year").into_owned(),
                n => t!("sa_age_days", n = n).into_owned(),
            };
            (days, label)
        })
        .collect();
    h_flex()
        .w_full()
        .flex_wrap()
        .items_center()
        .gap_x_3()
        .gap_y_2()
        .child(
            Input::new(search)
                .small()
                .w(rems(14.))
                .cleanable(true)
                .prefix(Icon::new(IconName::Search).small()),
        )
        .child(
            h_flex()
                .items_center()
                .gap_1p5()
                .child(ui::muted(t!("sa_filter_size").into_owned(), cx))
                .child(ui::segmented(
                    "sa-min-size",
                    sizes,
                    query.min_size,
                    move |bytes, cx| size_page.update(cx, |p, cx| p.set_min_size(bytes, cx)),
                )),
        )
        .child(
            h_flex()
                .items_center()
                .gap_1p5()
                .child(ui::muted(t!("sa_filter_age").into_owned(), cx))
                .child(ui::segmented(
                    "sa-older-than",
                    ages,
                    query.older_days,
                    move |days, cx| age_page.update(cx, |p, cx| p.set_older_than(days, cx)),
                )),
        )
        .when(query.is_active(), |this| {
            this.child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("sa-collect-results")
                            .small()
                            .outline()
                            .icon(IconName::ShoppingBasket)
                            .label(t!("sa_collect_results").into_owned())
                            .disabled(!has_results)
                            .on_click(move |_, _, cx| {
                                all_page.update(cx, |p, cx| p.collect_results(cx))
                            }),
                    )
                    .child(
                        Button::new("sa-clear-query")
                            .small()
                            .ghost()
                            .icon(IconName::X)
                            .label(t!("sa_clear_filters").into_owned())
                            .on_click(move |_, _, cx| {
                                clear_page.update(cx, |p, cx| p.clear_query(cx))
                            }),
                    ),
            )
        })
        .into_any_element()
}

fn list_row(
    index: usize,
    row: &Row,
    widest: u64,
    selected: bool,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
) -> AnyElement {
    let color = category_color(row.category, colors.dark);
    let icon = match row.kind {
        Kind::Dir => IconName::Folder,
        Kind::File => IconName::File,
        Kind::Rest => IconName::Ellipsis,
    };
    let fraction = if widest == 0 {
        0.0
    } else {
        (row.size as f64 / widest as f64) as f32
    };
    let (click, press, menu) = (entity.clone(), entity.clone(), entity.clone());
    let (click_path, press_path, menu_path) =
        (row.path.clone(), row.path.clone(), row.path.clone());
    let is_dir = row.kind == Kind::Dir;
    let folder = row.folder.clone();
    // 「变化」列表里，上次还在、现在已经不在树里的文件夹没有路径：只显示，不能点。
    let live = !row.path.is_empty();

    let row_element = h_flex()
        .id(("sa-row", index))
        .h(px(32.))
        .px_2()
        .gap_3()
        .items_center()
        .rounded(colors.radius)
        .when(selected, |this| this.bg(colors.active))
        .when(live, |this| {
            this.cursor_pointer()
                .hover(move |style| style.bg(colors.hover))
                .on_click(move |event, window, cx| {
                    let double = event.click_count() >= 2;
                    let path = click_path.clone();
                    click.update(cx, |page, cx| {
                        window.focus(&page.focus, cx);
                        if double && is_dir {
                            page.zoom(path, cx);
                        } else {
                            page.focus(path, cx);
                        }
                    });
                })
                .on_mouse_down(MouseButton::Right, move |_, _, cx| {
                    let path = press_path.clone();
                    press.update(cx, |page, cx| page.select(Some(path), cx));
                })
        })
        .child(Icon::new(icon).small().text_color(color))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .when(!live, |this| this.text_color(colors.muted))
                .child(row.name.clone()),
        )
        .when(!folder.is_empty(), |this| {
            this.child(
                div()
                    .w(relative(0.28))
                    .truncate()
                    .text_xs()
                    .text_color(colors.muted)
                    .child(folder),
            )
        })
        .when(!live, |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(colors.muted)
                    .child(t!("sa_changes_gone").into_owned()),
            )
        })
        .child(
            div()
                .w(px(96.))
                .h(px(6.))
                .flex_shrink_0()
                .bg(colors.hover)
                .child(
                    div()
                        .h_full()
                        .w(relative(fraction.clamp(0.0, 1.0)))
                        .bg(color),
                ),
        )
        .child(
            div()
                .w(px(84.))
                .flex_shrink_0()
                .text_sm()
                .text_right()
                // 变大用警示色，变小用正面的颜色。
                .when_some(row.delta, |this, delta| {
                    this.text_color(if delta > 0 {
                        colors.warning
                    } else {
                        colors.success
                    })
                })
                .child(match row.delta {
                    Some(delta) => signed_bytes(delta, unit),
                    None => fmt::bytes(row.size, unit),
                }),
        );
    if live {
        row_element
            .context_menu(move |popup, _, cx| item_menu(&menu, menu_path.clone(), popup, cx))
            .into_any_element()
    } else {
        row_element.into_any_element()
    }
}

impl StoragePage {
    /// 画布每帧在 prepaint 里来要这一帧的内容：按画布的真实尺寸排版，所以不用事先知道页面宽度。
    pub(super) fn scene(&mut self, bounds: Bounds<Pixels>, cx: &App) -> Option<MapScene> {
        let unit = self.app.read(cx).settings.size_unit;
        let theme = cx.theme();
        let palette = Palette {
            dark: theme.mode.is_dark(),
            surface: theme.background,
            text: theme.foreground,
            muted: theme.muted_foreground,
        };
        self.report_mut()?.scene(bounds, unit, palette)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_get_thousands_separators() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_000), "1,000");
        assert_eq!(count(12_345), "12,345");
        assert_eq!(count(5_960_000), "5,960,000");
    }

    #[test]
    fn small_shares_keep_a_decimal() {
        assert_eq!(percent_of(0, 0), "0%");
        assert_eq!(percent_of(50, 100), "50%");
        assert_eq!(percent_of(5, 100), "5.0%");
        assert_eq!(percent_of(1, 10_000), "<0.1%");
    }
}

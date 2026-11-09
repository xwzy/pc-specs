//! 重复文件：在正在看的文件夹里找内容完全相同的文件，让用户只留一份。
//!
//! 真正比内容的是核心库的 `dupes`（只读文件）。这里管后台任务、结果的展示，以及把「多余的」放进收集箱——
//! 不会直接删任何东西，真正移走还是在收集箱里确认；每一组至少留下一份没进收集箱的。

use super::analyzer::{Colors, count};
use super::{Report, StoragePage, bin_name, user_path};
use crate::settings::SizeUnit;
use crate::state::spawn_thread;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::progress::Progress as ProgressBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use pc_specs_core::dupes::{self, Candidate, Stage};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 小于这个大小的文件不参与：几 KB 的重复文件清掉也腾不出什么空间，却要多读很多文件。
pub(super) const MIN_SIZE: u64 = 1 << 20;
/// 最多拿多少个文件来比（取最大的）。
const MAX_FILES: usize = 50_000;
/// 结果一开始摆几组，每点一次「显示更多」多摆几组。
const GROUPS_STEP: usize = 20;
/// 一组里最多摆几份；超过的合并成「还有 N 份」。
const COPIES_SHOWN: usize = 10;

pub(super) enum Dupes {
    /// 还没找过。
    Idle,
    Running(Run),
    Done(Found),
}

pub(super) struct Run {
    id: u64,
    scope: String,
    /// 一共有多少个文件要比。
    files: usize,
    progress: Arc<dupes::Progress>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Run {
    /// 结果换掉了、重新扫描了、回到了选位置的页面：没人再等这次的结果，别让线程白白读下去。
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub(super) struct Found {
    scope: String,
    /// 一共比了多少个文件。
    files: usize,
    elapsed: Duration,
    groups: Vec<Group>,
    /// 已经摆出来几组。
    shown: usize,
}

/// 内容完全一样的一组。
struct Group {
    /// 每份有多大（字节）。
    len: u64,
    /// 至少两份，最老的在前。
    copies: Vec<Copy>,
}

impl Group {
    /// 只留一份的话能省多少。
    fn wasted(&self) -> u64 {
        self.len * (self.copies.len() as u64).saturating_sub(1)
    }
}

struct Copy {
    path: PathBuf,
    /// 从扫描起点往下的名字，是它在树里的身份（同级删掉别的之后下标会变，名字不会）。
    names: Vec<Box<str>>,
    modified: u32,
}

impl Dupes {
    /// 树里摘掉了 `removed`（连同它里面的一切）：里面的文件不再是重复文件的一份了。
    pub(super) fn forget(&mut self, removed: &[Box<str>]) {
        if let Dupes::Done(found) = self {
            for group in &mut found.groups {
                group.copies.retain(|copy| !copy.names.starts_with(removed));
            }
            found.groups.retain(|group| group.copies.len() > 1);
        }
    }
}

/// 这一份已经在收集箱里了，或者它所在的文件夹在。
fn staged(report: &Report, copy: &Copy) -> bool {
    report
        .collector
        .iter()
        .any(|staged| copy.names.starts_with(staged))
}

/// 一组里留下的那一份：没进收集箱的里最老的。全进收集箱了就是 `None`。
fn keeper(report: &Report, group: &Group) -> Option<usize> {
    group.copies.iter().position(|copy| !staged(report, copy))
}

/// 搜索范围的名字：根上是扫描位置的名字，往下是文件夹的名字。
fn scope_name(report: &Report) -> String {
    match report.view_node() {
        Some(view) if !report.view.is_empty() => view.name.to_string(),
        _ => report.target.label.clone(),
    }
}

/// 应用、框架、资源库这类「包」的扩展名：包里的文件互相依赖，内容一样也不能随便删一份。
const PACKAGES: [&str; 17] = [
    "app",
    "framework",
    "bundle",
    "plugin",
    "kext",
    "xpc",
    "appex",
    "prefpane",
    "saver",
    "component",
    "photoslibrary",
    "musiclibrary",
    "tvlibrary",
    "imovielibrary",
    "fcpbundle",
    "logicx",
    "sparsebundle",
];
/// 磁盘根下由系统和包管理器管的顶层目录。
const SYSTEM_ROOTS: [&str; 17] = [
    "System",
    "Library",
    "usr",
    "bin",
    "sbin",
    "private",
    "lib",
    "lib64",
    "etc",
    "boot",
    "snap",
    "opt",
    "var",
    "Windows",
    "Program Files",
    "Program Files (x86)",
    "ProgramData",
];
/// 用户个人文件夹里各个系统放程序数据的地方。
const APP_DATA: [&str; 5] = ["Library", ".cache", ".config", ".local", "AppData"];

/// 不参与查找的位置：应用和资源库的包里面、系统目录、用户的程序数据目录。
/// 这些地方的文件是程序在管，删掉一份内容相同的也可能让程序坏掉；要清理它们另有办法（比如卸载程序）。
fn is_managed(path: &Path, home: Option<&Path>) -> bool {
    let normal = |component: Component<'_>| match component {
        Component::Normal(name) => name.to_str().map(str::to_owned),
        _ => None,
    };
    if path.components().any(|component| {
        matches!(component, Component::Normal(name)
        if Path::new(name).extension().is_some_and(|extension| {
            PACKAGES.iter().any(|package| extension.eq_ignore_ascii_case(package))
        }))
    }) {
        return true;
    }
    // 磁盘根下的第一层（`/usr`、`C:\Windows`）。
    let top = path.components().filter_map(normal).next();
    if top
        .as_deref()
        .is_some_and(|top| SYSTEM_ROOTS.contains(&top))
    {
        return true;
    }
    // 个人文件夹下的第一层（`~/Library`、`~/.cache`）。
    home.and_then(|home| path.strip_prefix(home).ok())
        .and_then(|rest| rest.components().filter_map(normal).next())
        .is_some_and(|first| APP_DATA.contains(&first.as_str()))
}

impl StoragePage {
    /// 在正在看的文件夹里找重复文件。
    pub(super) fn find_duplicates(&mut self, cx: &mut Context<Self>) {
        let Some(report) = self.report() else { return };
        if matches!(report.dupes, Dupes::Running(_)) {
            return;
        }
        let Some(view) = report.view_node() else {
            return;
        };
        let scope = scope_name(report);
        let id = report.id;
        // 最大的文件优先；小的不比，个数也封顶。
        let home = dirs::home_dir();
        let mut names: HashMap<PathBuf, Vec<Box<str>>> = HashMap::new();
        let candidates: Vec<Candidate> = view
            .top_files(MAX_FILES, None)
            .into_iter()
            .take_while(|(_, node)| node.size >= MIN_SIZE)
            .filter_map(|(relative, node)| {
                let mut path = report.view.clone();
                path.extend_from_slice(&relative);
                let full = user_path(report.tree.path_of(&path));
                if is_managed(&full, home.as_deref()) {
                    return None;
                }
                names.insert(full.clone(), report.tree.root.names(&path)?);
                Some(Candidate {
                    path: full,
                    modified: node.modified,
                })
            })
            .collect();
        let files = candidates.len();

        let run_id = self.next_search;
        self.next_search += 1;
        let progress = Arc::new(dupes::Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = async_channel::bounded(1);
        let started = spawn_thread("pc-specs-dupes", {
            let (progress, cancel) = (progress.clone(), cancel.clone());
            move || {
                let begun = Instant::now();
                let found = guarded("duplicate search", || {
                    dupes::find(candidates, &progress, &cancel)
                })
                .unwrap_or_default();
                let groups: Vec<Group> = found
                    .into_iter()
                    .map(|group| Group {
                        len: group.len,
                        copies: group
                            .files
                            .into_iter()
                            .map(|file| Copy {
                                names: names.remove(&file.path).unwrap_or_default(),
                                path: file.path,
                                modified: file.modified,
                            })
                            .collect(),
                    })
                    .collect();
                let _ = tx.send_blocking((groups, begun.elapsed()));
            }
        });
        if !started {
            let message = t!("sa_dup_error_thread").into_owned();
            self.toast(NotificationType::Error, message, cx);
            return;
        }
        if let Some(report) = self.report_mut() {
            report.dupes = Dupes::Running(Run {
                id: run_id,
                scope: scope.clone(),
                files,
                progress,
                cancel,
            });
        }

        cx.spawn(async move |this, cx| {
            let Ok((groups, elapsed)) = rx.recv().await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(report) = this.report_mut()
                    && report.id == id
                    && matches!(&report.dupes, Dupes::Running(run) if run.id == run_id)
                {
                    report.dupes = Dupes::Done(Found {
                        scope,
                        files,
                        elapsed,
                        shown: GROUPS_STEP,
                        groups,
                    });
                    cx.notify();
                }
            });
        })
        .detach();
        // 比的时候每隔一会儿重画一次，让进度动起来。
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let running = this.update(cx, |this, cx| {
                    let running = this.report().is_some_and(|report| {
                        report.id == id
                            && matches!(&report.dupes, Dupes::Running(run) if run.id == run_id)
                    });
                    if running {
                        cx.notify();
                    }
                    running
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// 不比了（换掉 `Run` 就会通知线程停下），回到还没找过的状态。
    pub(super) fn stop_duplicates(&mut self, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut() {
            report.dupes = Dupes::Idle;
            cx.notify();
        }
    }

    pub(super) fn show_more_duplicates(&mut self, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && let Dupes::Done(found) = &mut report.dupes
        {
            found.shown += GROUPS_STEP;
            cx.notify();
        }
    }

    /// 每一组只留一份（没进收集箱的里最老的），其余的都放进收集箱。`group` 为 `None` 就是所有组。
    pub(super) fn collect_extra_copies(&mut self, group: Option<usize>, cx: &mut Context<Self>) {
        let Some(report) = self.report() else { return };
        let Dupes::Done(found) = &report.dupes else {
            return;
        };
        let paths: Vec<Vec<u32>> = found
            .groups
            .iter()
            .enumerate()
            .filter(|&(index, _)| group.is_none_or(|only| only == index))
            .flat_map(|(_, group)| {
                let keep = keeper(report, group);
                group
                    .copies
                    .iter()
                    .enumerate()
                    .filter(move |&(index, copy)| Some(index) != keep && !staged(report, copy))
                    .filter_map(|(_, copy)| report.tree.root.locate(&copy.names))
            })
            .collect();
        self.collect_paths(paths, cx);
    }

    /// 一份文件放进收集箱或拿出来。一组里最后一份没进收集箱的不让放：不能把同一个文件的所有副本都移走。
    fn collect_copy(&mut self, group: usize, copy: usize, cx: &mut Context<Self>) {
        let Some(report) = self.report() else { return };
        let Dupes::Done(found) = &report.dupes else {
            return;
        };
        let Some((group, entry)) = found
            .groups
            .get(group)
            .and_then(|group| Some((group, group.copies.get(copy)?)))
        else {
            return;
        };
        let last = !staged(report, entry)
            && group
                .copies
                .iter()
                .filter(|other| !staged(report, other))
                .count()
                <= 1;
        let name = entry
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let path = report.tree.root.locate(&entry.names);
        if last {
            let message = t!("sa_dup_last_copy", name = name).into_owned();
            self.toast(NotificationType::Warning, message, cx);
        } else if let Some(path) = path {
            self.toggle_collect(path, cx);
        }
    }

    fn reveal_copy(&mut self, group: usize, copy: usize, cx: &mut Context<Self>) {
        if let Some(report) = self.report()
            && let Dupes::Done(found) = &report.dupes
            && let Some(copy) = found
                .groups
                .get(group)
                .and_then(|group| group.copies.get(copy))
        {
            cx.reveal_path(&copy.path);
        }
    }
}

// ---- 界面 ---------------------------------------------------------------------

/// 「重复文件」页签的内容。
pub(super) fn view(
    report: &Report,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &App,
) -> AnyElement {
    match &report.dupes {
        Dupes::Idle => idle(report, unit, entity, colors, cx),
        Dupes::Running(run) => running(run, unit, entity, colors, cx),
        Dupes::Done(found) => done(report, found, unit, entity, colors, cx),
    }
}

fn idle(
    report: &Report,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &App,
) -> AnyElement {
    let page = entity.clone();
    h_flex()
        .w_full()
        .items_center()
        .flex_wrap()
        .gap_3()
        .p_4()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .child(
            Icon::new(IconName::CopyCheck)
                .flex_shrink_0()
                .text_color(colors.muted),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w(px(240.))
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(t!("sa_dup_title", scope = scope_name(report)).into_owned()),
                )
                .child(
                    ui::muted(
                        t!(
                            "sa_dup_desc",
                            size = fmt::bytes(MIN_SIZE, unit),
                            bin = bin_name()
                        )
                        .into_owned(),
                        cx,
                    )
                    .text_xs(),
                ),
        )
        .child(
            Button::new("sa-dup-find")
                .small()
                .outline()
                .icon(IconName::Search)
                .label(t!("sa_dup_find").into_owned())
                .on_click(move |_, _, cx| page.update(cx, |p, cx| p.find_duplicates(cx))),
        )
        .into_any_element()
}

fn running(
    run: &Run,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &App,
) -> AnyElement {
    let page = entity.clone();
    let (done, total) = run.progress.counts();
    let line = match run.progress.stage() {
        Stage::Sizes => t!(
            "sa_dup_stage_sizes",
            done = count(done),
            total = count(total)
        ),
        Stage::Samples => t!(
            "sa_dup_stage_samples",
            done = count(done),
            total = count(total)
        ),
        Stage::Content => t!(
            "sa_dup_stage_content",
            done = fmt::bytes(done, unit),
            total = fmt::bytes(total, unit)
        ),
    }
    .into_owned();
    let bar = if total == 0 {
        ProgressBar::new("sa-dup-progress").loading(true)
    } else {
        ProgressBar::new("sa-dup-progress").value((done as f64 / total as f64 * 100.0) as f32)
    };
    v_flex()
        .w_full()
        .gap_3()
        .p_4()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .child(
            div().text_sm().font_semibold().child(
                t!(
                    "sa_dup_running",
                    scope = run.scope,
                    files = count(run.files as u64)
                )
                .into_owned(),
            ),
        )
        .child(bar)
        .child(ui::muted(line, cx).text_xs())
        .child(
            h_flex().child(
                Button::new("sa-dup-cancel")
                    .small()
                    .outline()
                    .icon(IconName::X)
                    .label(t!("sa_cancel").into_owned())
                    .on_click(move |_, _, cx| page.update(cx, |p, cx| p.stop_duplicates(cx))),
            ),
        )
        .into_any_element()
}

fn done(
    report: &Report,
    found: &Found,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &App,
) -> AnyElement {
    let wasted: u64 = found.groups.iter().map(Group::wasted).sum();
    let (again, collect_all, more) = (entity.clone(), entity.clone(), entity.clone());
    let headline = if found.groups.is_empty() {
        t!("sa_dup_none", scope = found.scope).into_owned()
    } else {
        t!(
            "sa_dup_found",
            n = count(found.groups.len() as u64),
            size = fmt::bytes(wasted, unit)
        )
        .into_owned()
    };
    let mut detail = t!(
        "sa_dup_searched",
        files = count(found.files as u64),
        min = fmt::bytes(MIN_SIZE, unit),
        scope = found.scope,
        time = fmt::duration(found.elapsed.as_secs().max(1))
    )
    .into_owned();
    if found.files >= MAX_FILES {
        detail.push(' ');
        detail.push_str(&t!("sa_dup_capped", n = count(MAX_FILES as u64)));
    }

    let header = h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .flex_wrap()
        .gap_3()
        .child(
            v_flex()
                .flex_1()
                .min_w(px(240.))
                .gap_0p5()
                .child(div().text_sm().font_semibold().child(headline))
                .child(ui::muted(detail, cx).text_xs()),
        )
        .child(
            h_flex()
                .gap_1()
                .when(!found.groups.is_empty(), |this| {
                    this.child(
                        Button::new("sa-dup-collect-all")
                            .small()
                            .outline()
                            .icon(IconName::ShoppingBasket)
                            .label(t!("sa_dup_collect_all").into_owned())
                            .on_click(move |_, _, cx| {
                                collect_all.update(cx, |p, cx| p.collect_extra_copies(None, cx))
                            }),
                    )
                })
                .child(
                    Button::new("sa-dup-again")
                        .small()
                        .ghost()
                        .icon(IconName::RefreshCw)
                        .label(t!("sa_dup_again").into_owned())
                        .on_click(move |_, _, cx| again.update(cx, |p, cx| p.find_duplicates(cx))),
                ),
        );

    let shown = found.groups.len().min(found.shown);
    let groups = found
        .groups
        .iter()
        .take(shown)
        .enumerate()
        .map(|(index, group)| group_card(report, index, group, unit, entity, colors));
    let more = (found.groups.len() > shown).then(|| {
        h_flex().justify_center().child(
            Button::new("sa-dup-more")
                .small()
                .ghost()
                .label(t!("sa_list_more").into_owned())
                .on_click(move |_, _, cx| more.update(cx, |p, cx| p.show_more_duplicates(cx))),
        )
    });
    // 访达的「复制」在 APFS 上是克隆：两份共用同一块空间，移走一份并不会多出这么多。
    let clone_note = (cfg!(target_os = "macos") && !found.groups.is_empty())
        .then(|| ui::muted(t!("sa_dup_clone_note").into_owned(), cx).text_xs());

    v_flex()
        .w_full()
        .gap_3()
        .child(header)
        .children(clone_note)
        .children(groups)
        .children(more)
        .into_any_element()
}

/// 一组重复文件：一行总览，下面每一份一行。
fn group_card(
    report: &Report,
    index: usize,
    group: &Group,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
) -> AnyElement {
    let page = entity.clone();
    let keep = keeper(report, group);
    let name = group
        .copies
        .first()
        .and_then(|copy| copy.path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let hidden = group.copies.len().saturating_sub(COPIES_SHOWN);
    let rows = group
        .copies
        .iter()
        .take(COPIES_SHOWN)
        .enumerate()
        .map(|(at, copy)| copy_row(report, (index, at), copy, keep == Some(at), entity, colors));
    v_flex()
        .w_full()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .overflow_hidden()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .bg(colors.hover)
                .child(Icon::new(IconName::Copy).small().text_color(colors.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_semibold()
                        .child(name),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(colors.muted)
                        .child(
                            t!(
                                "sa_dup_copies",
                                n = group.copies.len(),
                                size = fmt::bytes(group.len, unit)
                            )
                            .into_owned(),
                        ),
                )
                .child(
                    div()
                        .w(px(96.))
                        .flex_shrink_0()
                        .text_sm()
                        .text_right()
                        .child(fmt::bytes(group.wasted(), unit)),
                )
                .child(
                    Button::new(("sa-dup-keep", index))
                        .xsmall()
                        .outline()
                        .icon(IconName::ShoppingBasket)
                        .label(t!("sa_dup_keep_one").into_owned())
                        .on_click(move |_, _, cx| {
                            page.update(cx, |p, cx| p.collect_extra_copies(Some(index), cx))
                        }),
                ),
        )
        .children(rows)
        .when(hidden > 0, |this| {
            this.child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(colors.muted)
                    .child(t!("sa_collect_more", n = hidden).into_owned()),
            )
        })
        .into_any_element()
}

fn copy_row(
    report: &Report,
    (group, at): (usize, usize),
    copy: &Copy,
    keep: bool,
    entity: &Entity<StoragePage>,
    colors: Colors,
) -> AnyElement {
    let (reveal, collect) = (entity.clone(), entity.clone());
    let in_box = staged(report, copy);
    let full = copy.path.display().to_string();
    let tip = SharedString::from(full.clone());
    // 同一个界面里不会有一万份以上的同一个文件，这样编号不会重复。
    let id = group * 10_000 + at;
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .px_3()
        .py_1()
        .child(
            div()
                .w(px(84.))
                .flex_shrink_0()
                .text_xs()
                .text_color(colors.muted)
                .when(keep, |this| this.font_semibold().text_color(colors.text))
                .child(if keep {
                    t!("sa_dup_keep").into_owned()
                } else if in_box {
                    t!("sa_dup_staged").into_owned()
                } else {
                    String::new()
                }),
        )
        .child(
            div()
                .id(("sa-dup-path", id))
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .child(full),
        )
        .child(
            div()
                .w(px(116.))
                .flex_shrink_0()
                .text_xs()
                .text_color(colors.muted)
                .child(if copy.modified > 0 {
                    fmt::datetime(u64::from(copy.modified))
                } else {
                    String::new()
                }),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .gap_1()
                .child(
                    Button::new(("sa-dup-reveal", id))
                        .xsmall()
                        .ghost()
                        .icon(IconName::ExternalLink)
                        .tooltip(ui::reveal_label())
                        .on_click(move |_, _, cx| {
                            reveal.update(cx, |p, cx| p.reveal_copy(group, at, cx))
                        }),
                )
                .child(
                    Button::new(("sa-dup-collect", id))
                        .xsmall()
                        .ghost()
                        .selected(in_box)
                        .icon(IconName::ShoppingBasket)
                        .tooltip(if in_box {
                            t!("sa_act_uncollect").into_owned()
                        } else {
                            t!("sa_act_collect").into_owned()
                        })
                        .on_click(move |_, _, cx| {
                            collect.update(cx, |p, cx| p.collect_copy(group, at, cx))
                        }),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(parts: &[&str]) -> Vec<Box<str>> {
        parts.iter().map(|&part| part.into()).collect()
    }

    fn copy(parts: &[&str]) -> Copy {
        Copy {
            path: PathBuf::from(parts.join("/")),
            names: names(parts),
            modified: 0,
        }
    }

    #[test]
    fn files_the_system_or_an_app_depends_on_are_left_out() {
        let home = Some(Path::new("/Users/me"));
        let managed = |path: &str| is_managed(Path::new(path), home);
        // 应用、框架、资源库的包里面。
        assert!(managed(
            "/Applications/Chat.app/Contents/Frameworks/icudtl.dat"
        ));
        assert!(managed(
            "/Users/me/Pictures/Photos Library.photoslibrary/originals/a.heic"
        ));
        assert!(managed("/Users/me/Dev/Tool.FRAMEWORK/Versions/A/Tool"));
        // 系统目录和程序数据目录。
        assert!(managed("/usr/lib/libz.so"));
        assert!(managed("/System/Library/x"));
        assert!(managed("/Users/me/Library/Caches/com.x/cache.db"));
        assert!(managed("/Users/me/.cache/pip/wheel.whl"));
        assert!(managed("C:\\Windows\\WinSxS\\a.dll") || cfg!(unix));
        // 平常放文件的地方，以及名字只是碰巧相似的。
        assert!(!managed("/Users/me/Downloads/movie.mkv"));
        assert!(!managed("/Users/me/Documents/application/notes.pdf"));
        assert!(!managed("/Users/me/Movies/opt/clip.mov"));
        assert!(!managed("/Users/someone-else/Library/x"));
        assert!(!managed("/Volumes/Backup/Users/me/Library/x"));
        assert!(!managed("/data/library/x"));
    }

    #[test]
    fn removing_files_shrinks_their_groups_and_drops_the_ones_left_alone() {
        let found = Found {
            scope: String::new(),
            files: 5,
            elapsed: Duration::ZERO,
            shown: GROUPS_STEP,
            groups: vec![
                Group {
                    len: 10,
                    copies: vec![
                        copy(&["a", "x.bin"]),
                        copy(&["b", "x.bin"]),
                        copy(&["c", "x.bin"]),
                    ],
                },
                Group {
                    len: 20,
                    copies: vec![copy(&["a", "y.bin"]), copy(&["d", "y.bin"])],
                },
            ],
        };
        let mut dupes = Dupes::Done(found);
        // 整个文件夹 `a` 被移走：x 还剩两份，y 只剩一份，不再是重复。
        dupes.forget(&names(&["a"]));
        let Dupes::Done(found) = &dupes else {
            unreachable!()
        };
        assert_eq!(found.groups.len(), 1);
        assert_eq!(found.groups[0].copies.len(), 2);
        assert_eq!(found.groups[0].wasted(), 10);

        // 只是名字以它开头的不受影响：移走 `bb`，`b` 里的文件照旧。
        dupes.forget(&names(&["bb"]));
        let Dupes::Done(found) = &dupes else {
            unreachable!()
        };
        assert_eq!(found.groups[0].copies.len(), 2);
    }
}

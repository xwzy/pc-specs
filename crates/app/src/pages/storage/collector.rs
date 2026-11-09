//! 收集箱：把想清理的项目一个个攒起来，看清总共能腾出多少，再一次移走。
//! 树图上点一下选中、再点「加入收集箱」；在不同的目录里挑，也都攒在同一个箱子里。

use super::{Item, Report, StoragePage, bin_name, is_protected, move_to_trash};
use crate::settings::SizeUnit;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Entity, InteractiveElement as _, IntoElement as _,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use pc_specs_core::space::Kind;
use rust_i18n::t;
use std::cmp::Reverse;

/// 收集箱里的一项，连同它此刻在树里的位置。
pub(super) struct Collected {
    /// 从根往下的名字，是这一项的身份。
    pub names: Vec<Box<str>>,
    pub item: Item,
}

/// 箱子里最多把几项摆成小标签；多出来的合并成「还有 N 项」。
const CHIPS: usize = 8;

impl Report {
    /// 收集箱里的各项，大的在前；树里已经没有的（比如被别处移走了）不返回。
    pub(super) fn collected(&self) -> Vec<Collected> {
        let mut found: Vec<Collected> = self
            .collector
            .iter()
            .filter_map(|names| {
                let path = self.tree.root.locate(names)?;
                Some(Collected {
                    names: names.clone(),
                    item: self.item(&path)?,
                })
            })
            .collect();
        found.sort_by_key(|entry| Reverse(entry.item.size));
        found
    }

    pub(super) fn is_collected(&self, path: &[u32]) -> bool {
        self.tree
            .root
            .names(path)
            .is_some_and(|names| self.collector.contains(&names))
    }

    /// 加进收集箱；已经在里面就拿出来。`None` 是这一项不能收集（走不通的路径、「小项目」）。
    pub(super) fn toggle_collect(&mut self, path: &[u32]) -> Option<Collect> {
        // 扫描的起点本身不能收集：整个位置都移走不是「腾出空间」。
        if path.is_empty() {
            return None;
        }
        let item = self.item(path)?;
        let names = self.tree.root.names(path)?;
        if let Some(at) = self.collector.iter().position(|staged| *staged == names) {
            self.collector.remove(at);
            return Some(Collect::Removed);
        }
        if is_protected(&item.full) {
            return Some(Collect::Protected(item.name));
        }
        // 它所在的文件夹已经在箱子里了：整个文件夹都会移走，不用再单独放。
        if let Some(folder) = self
            .collector
            .iter()
            .find(|staged| names.starts_with(staged))
        {
            let folder = folder.last().map(|name| name.to_string());
            return Some(Collect::Inside {
                name: item.name,
                folder: folder.unwrap_or_default(),
            });
        }
        // 放进一个文件夹之前，把它里面已经单独放进去的项目收掉，免得同一个文件被算两次。
        self.collector.retain(|staged| !staged.starts_with(&names));
        self.collector.push(names);
        Some(Collect::Added)
    }
}

/// [`Report::toggle_collect`] 的结果。
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Collect {
    Added,
    Removed,
    /// 系统文件夹、磁盘顶层文件夹之类，不能动。
    Protected(String),
    /// 它所在的文件夹已经在箱子里。
    Inside {
        name: String,
        folder: String,
    },
}

/// 要移走的一项：名字用来在树里找回它，路径用来真的去移。
struct Job {
    names: Vec<Box<str>>,
    name: String,
    full: std::path::PathBuf,
    size: u64,
}

impl StoragePage {
    /// 加进收集箱；已经在里面就拿出来。
    pub(super) fn toggle_collect(&mut self, path: Vec<u32>, cx: &mut Context<Self>) {
        let Some(outcome) = self
            .report_mut()
            .and_then(|report| report.toggle_collect(&path))
        else {
            return;
        };
        let note = match outcome {
            Collect::Protected(name) => Some(t!("sa_trash_protected", name = name).into_owned()),
            Collect::Inside { name, folder } => {
                Some(t!("sa_collect_inside", name = name, folder = folder).into_owned())
            }
            Collect::Added | Collect::Removed => None,
        };
        if let Some(message) = note {
            self.toast(NotificationType::Warning, message, cx);
        }
        cx.notify();
    }

    /// 把当前列表（搜索结果）里的项目一次全加进收集箱；已经在里面的、在已收集文件夹里的跳过。
    pub(super) fn collect_results(&mut self, cx: &mut Context<Self>) {
        let paths: Vec<Vec<u32>> = self
            .report()
            .and_then(|report| report.rows.as_ref())
            .map(|rows| rows.rows.iter().map(|row| row.path.clone()).collect())
            .unwrap_or_default();
        self.collect_paths(paths, cx);
    }

    /// 把一批项目加进收集箱；已经在里面的、在已收集文件夹里的、不能动的跳过。
    pub(super) fn collect_paths(&mut self, paths: Vec<Vec<u32>>, cx: &mut Context<Self>) {
        let Some(report) = self.report_mut() else {
            return;
        };
        let mut added = 0usize;
        for path in paths {
            if !report.is_collected(&path) && report.toggle_collect(&path) == Some(Collect::Added) {
                added += 1;
            }
        }
        if added > 0 {
            let message = t!("sa_collect_added", n = added).into_owned();
            self.toast(NotificationType::Success, message, cx);
        }
        cx.notify();
    }

    pub(super) fn uncollect(&mut self, names: &[Box<str>], cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut() {
            report.collector.retain(|staged| staged != names);
            cx.notify();
        }
    }

    pub(super) fn clear_collector(&mut self, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && !report.collector.is_empty()
        {
            report.collector.clear();
            cx.notify();
        }
    }

    /// 先弹确认框，列出要移走的东西；点了确定才真的去移。
    pub(super) fn ask_trash_collected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.trashing {
            return;
        }
        let Some(report) = self.report() else { return };
        let entries = report.collected();
        if entries.is_empty() {
            return;
        }
        let unit = self.app.read(cx).settings.size_unit;
        let bin = bin_name();
        let total: u64 = entries.iter().map(|entry| entry.item.size).sum();
        let files: u64 = entries
            .iter()
            .map(|entry| u64::from(entry.item.files))
            .sum();
        let mut lines: Vec<String> = entries
            .iter()
            .take(6)
            .map(|entry| {
                format!(
                    "• {} ({})",
                    entry.item.name,
                    fmt::bytes(entry.item.size, unit)
                )
            })
            .collect();
        if entries.len() > 6 {
            lines.push(t!("sa_collect_more", n = entries.len() - 6).into_owned());
        }
        let description = format!(
            "{}\n\n{}",
            t!(
                "sa_collect_trash_desc",
                size = fmt::bytes(total, unit),
                files = super::analyzer::count(files),
                bin = bin
            ),
            lines.join("\n")
        );
        let title = t!("sa_collect_trash_title", n = entries.len(), bin = bin).into_owned();
        let ok = t!("sa_trash_ok", bin = bin).into_owned();
        let cancel = t!("sa_cancel").into_owned();
        let page = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let page = page.clone();
            alert
                .confirm()
                .title(title.clone())
                .description(description.clone())
                .ok_text(ok.clone())
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(cancel.clone())
                .on_ok(move |_, _, cx| {
                    page.update(cx, |page, cx| page.trash_collected(cx));
                    true
                })
        });
    }

    /// 在后台一项一项移，回来再统一更新树和提示。
    fn trash_collected(&mut self, cx: &mut Context<Self>) {
        if self.trashing {
            return;
        }
        let Some(report) = self.report() else { return };
        let id = report.id;
        let jobs: Vec<Job> = report
            .collected()
            .into_iter()
            .map(|entry| Job {
                names: entry.names,
                name: entry.item.name,
                full: entry.item.full,
                size: entry.item.size,
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        self.trashing = true;
        let task = cx.background_executor().spawn(async move {
            jobs.into_iter()
                .map(|job| {
                    let result = if is_protected(&job.full) {
                        Err(t!("sa_trash_protected", name = job.name).into_owned())
                    } else {
                        move_to_trash(&job.full)
                    };
                    (job, result)
                })
                .collect::<Vec<_>>()
        });
        cx.spawn(async move |this, cx| {
            let results = task.await;
            let _ = this.update(cx, |this, cx| this.trashed_many(id, results, cx));
        })
        .detach();
        cx.notify();
    }

    fn trashed_many(
        &mut self,
        id: u64,
        results: Vec<(Job, Result<(), String>)>,
        cx: &mut Context<Self>,
    ) {
        self.trashing = false;
        let bin = bin_name();
        let total = results.len();
        let (mut moved, mut freed) = (0usize, 0u64);
        let mut failed: Vec<(String, String)> = Vec::new();
        for (job, result) in results {
            match result {
                Ok(()) => {
                    moved += 1;
                    freed += job.size;
                    // 等的这段时间里用户可能已经重新扫描过：树不是原来那棵了就不去动它。
                    if let Some(report) = self.report_mut()
                        && report.id == id
                        && let Some(path) = report.tree.root.locate(&job.names)
                    {
                        report.remove(&path);
                    }
                }
                Err(error) => failed.push((job.name, error)),
            }
        }
        let unit = self.app.read(cx).settings.size_unit;
        let size = fmt::bytes(freed, unit);
        let (kind, message) = match failed.first() {
            None => (
                NotificationType::Success,
                t!("sa_collect_done", n = moved, size = size, bin = bin).into_owned(),
            ),
            Some((name, error)) if moved == 0 => (
                NotificationType::Error,
                t!("sa_trash_failed", name = name, bin = bin, error = error).into_owned(),
            ),
            Some((name, error)) => (
                NotificationType::Warning,
                t!(
                    "sa_collect_partial",
                    moved = moved,
                    total = total,
                    size = size,
                    name = name,
                    error = error
                )
                .into_owned(),
            ),
        };
        self.toast(kind, message, cx);
        cx.notify();
    }
}

/// 树图下面的收集箱：一直在那里，空的时候说明怎么用，有东西时摆出小标签、总量和操作。
/// 高度只会随小标签折行变高，而它在树图下面，所以不会把树图顶得跳动。
pub(super) fn bar(
    report: &Report,
    unit: SizeUnit,
    busy: bool,
    entity: &Entity<StoragePage>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let (border, muted) = (theme.border, theme.muted_foreground);
    let (hover, radius, radius_lg) = (theme.list_hover, theme.radius, theme.radius_lg);
    let entries = report.collected();
    let total: u64 = entries.iter().map(|entry| entry.item.size).sum();
    let frame = h_flex()
        .id("sa-collector")
        .w_full()
        .min_h(px(BAR_HEIGHT))
        .px_3()
        .py_2()
        .gap_3()
        .items_center()
        .rounded(radius_lg)
        .border_1()
        .border_color(border);

    if entries.is_empty() {
        return frame
            .child(
                Icon::new(IconName::ShoppingBasket)
                    .small()
                    .flex_shrink_0()
                    .text_color(muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(muted)
                    .child(t!("sa_collect_empty", bin = bin_name()).into_owned()),
            )
            .into_any_element();
    }

    let hidden = entries.len().saturating_sub(CHIPS);
    let chips = entries
        .iter()
        .take(CHIPS)
        .enumerate()
        .map(|(index, entry)| {
            let (focus_page, remove_page) = (entity.clone(), entity.clone());
            let (path, names) = (entry.item.path.clone(), entry.names.clone());
            let icon = if entry.item.kind == Kind::Dir {
                IconName::Folder
            } else {
                IconName::File
            };
            let tip = SharedString::from(entry.item.full.display().to_string());
            h_flex()
                .id(("sa-chip", index))
                .items_center()
                .gap_1p5()
                .pl_2()
                .pr_1()
                .py_0p5()
                .rounded(radius)
                .border_1()
                .border_color(border)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .on_click(move |_, _, cx| {
                    let path = path.clone();
                    focus_page.update(cx, |page, cx| page.focus(path, cx));
                })
                .child(Icon::new(icon).small().text_color(muted))
                .child(
                    div()
                        .max_w(px(180.))
                        .truncate()
                        .text_sm()
                        .child(entry.item.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .child(fmt::bytes(entry.item.size, unit)),
                )
                .child(
                    Button::new(("sa-chip-x", index))
                        .xsmall()
                        .ghost()
                        .icon(IconName::X)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let names = names.clone();
                            remove_page.update(cx, |page, cx| page.uncollect(&names, cx));
                        }),
                )
                .into_any_element()
        });

    let (trash_page, clear_page) = (entity.clone(), entity.clone());
    frame
        .child(
            Icon::new(IconName::ShoppingBasket)
                .small()
                .flex_shrink_0()
                .text_color(muted),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap_1()
                .children(chips)
                .when(hidden > 0, |this| {
                    this.child(ui::muted(
                        t!("sa_collect_more", n = hidden).into_owned(),
                        cx,
                    ))
                }),
        )
        .child(
            div().flex_shrink_0().text_sm().font_semibold().child(
                t!(
                    "sa_collect_total",
                    n = entries.len(),
                    size = fmt::bytes(total, unit)
                )
                .into_owned(),
            ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .gap_1()
                .child(
                    Button::new("sa-collect-trash")
                        .small()
                        .outline()
                        .danger()
                        .icon(IconName::Trash)
                        .label(t!("sa_collect_trash", bin = bin_name()).into_owned())
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            trash_page.update(cx, |page, cx| page.ask_trash_collected(window, cx))
                        }),
                )
                .child(
                    Button::new("sa-collect-clear")
                        .small()
                        .ghost()
                        .label(t!("sa_collect_clear").into_owned())
                        .disabled(busy)
                        .on_click(move |_, _, cx| {
                            clear_page.update(cx, |page, cx| page.clear_collector(cx))
                        }),
                ),
        )
        .into_any_element()
}

/// 收集箱一行的最小高度：放得下一排小标签和按钮。
pub(super) const BAR_HEIGHT: f32 = 48.0;

#[cfg(test)]
mod tests {
    use super::super::{Target, hidden};
    use super::*;
    use pc_specs_core::space::{Options, Progress, scan_with};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::AtomicBool;

    /// 在临时目录里造一棵真实的树再扫描，返回报告和要清理的临时目录。
    fn report(tag: &str) -> (Report, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("pcs-collector-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let write = |relative: &str, bytes: usize| {
            let path = dir.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, vec![7u8; bytes]).unwrap();
        };
        write("movies/a.mp4", 900 * 1024);
        write("movies/b.mp4", 600 * 1024);
        write("docs/c.txt", 300 * 1024);
        write("d.bin", 100 * 1024);
        let options = Options {
            base_keep: 0,
            ..Options::default()
        };
        let tree = scan_with(
            &dir,
            &options,
            &Progress::default(),
            &AtomicBool::new(false),
        )
        .expect("scan the sample tree");
        let target = Target {
            label: "sample".into(),
            path: dir.clone(),
            volume: false,
        };
        (
            Report::new(1, target, tree, hidden::Extra::default(), None),
            dir,
        )
    }

    fn path(report: &Report, names: &[&str]) -> Vec<u32> {
        let names: Vec<Box<str>> = names.iter().map(|&name| name.into()).collect();
        report.tree.root.locate(&names).expect("item exists")
    }

    fn names_of(report: &Report) -> Vec<String> {
        report
            .collected()
            .iter()
            .map(|entry| entry.item.name.clone())
            .collect()
    }

    #[test]
    fn collecting_an_item_twice_takes_it_out_again() {
        let (mut report, dir) = report("toggle");
        let a = path(&report, &["movies", "a.mp4"]);
        assert_eq!(report.toggle_collect(&a), Some(Collect::Added));
        assert!(report.is_collected(&a));
        assert_eq!(names_of(&report), ["a.mp4"]);
        assert_eq!(report.toggle_collect(&a), Some(Collect::Removed));
        assert!(report.collected().is_empty());
        // 扫描的起点本身、走不通的路径都不能收集。
        assert_eq!(report.toggle_collect(&[]), None);
        assert_eq!(report.toggle_collect(&[99]), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_swallows_its_contents_and_refuses_them_afterwards() {
        let (mut report, dir) = report("nest");
        let a = path(&report, &["movies", "a.mp4"]);
        let movies = path(&report, &["movies"]);
        let c = path(&report, &["docs", "c.txt"]);
        report.toggle_collect(&a);
        report.toggle_collect(&c);
        // 放进整个文件夹，里面单独放过的 a.mp4 就并进去，c.txt 不受影响。
        assert_eq!(report.toggle_collect(&movies), Some(Collect::Added));
        assert_eq!(report.collected().len(), 2);
        // 反过来，文件夹已经在箱子里，里面的文件就不用再放。
        assert_eq!(
            report.toggle_collect(&a),
            Some(Collect::Inside {
                name: "a.mp4".into(),
                folder: "movies".into()
            })
        );
        let total: u64 = report.collected().iter().map(|e| e.item.size).sum();
        let expected =
            report.tree.root.at(&movies).unwrap().size + report.tree.root.at(&c).unwrap().size;
        assert_eq!(total, expected, "no byte is counted twice");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn items_are_found_again_after_others_are_removed() {
        let (mut report, dir) = report("shift");
        let b = path(&report, &["movies", "b.mp4"]);
        let c = path(&report, &["docs", "c.txt"]);
        report.toggle_collect(&b);
        report.toggle_collect(&c);
        // 摘掉同一个目录里排在前面的 a.mp4：b.mp4 的下标前移了，收集箱仍然指着它。
        let a = path(&report, &["movies", "a.mp4"]);
        report.remove(&a);
        let mut names = names_of(&report);
        names.sort();
        assert_eq!(names, ["b.mp4", "c.txt"]);
        let entry = report
            .collected()
            .into_iter()
            .find(|entry| entry.item.name == "b.mp4")
            .unwrap();
        assert_eq!(entry.item.path, path(&report, &["movies", "b.mp4"]));

        // 整个文件夹被移走，箱子里它里面的东西也跟着消失。
        let movies = path(&report, &["movies"]);
        report.remove(&movies);
        assert_eq!(names_of(&report), ["c.txt"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn disk_roots_and_system_folders_cannot_be_collected() {
        let (mut report, dir) = report("protected");
        // 把扫描起点假装成磁盘根下的一层，里面的第一层项目就在「顶层系统目录」的保护范围里。
        report.tree.path = Path::new("/").to_path_buf();
        let movies = path(&report, &["movies"]);
        assert_eq!(
            report.toggle_collect(&movies),
            Some(Collect::Protected("movies".into()))
        );
        assert!(report.collected().is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}

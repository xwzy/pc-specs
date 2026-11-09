//! 清理：把缓存、日志、开发工具的下载缓存这些「应用会自己重建」的东西一次清掉。
//!
//! 清什么、怎么保证安全，都在核心库的 `clean` 里：只认写死的位置，只动位置里面，不跟随符号链接。
//! 这里管后台任务、结果的展示，以及动手之前的确认。

use super::analyzer::{Colors, count};
use super::{Scan, StoragePage, bin_name, folder_target};
use crate::settings::SizeUnit;
use crate::state::spawn_thread;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::progress::Progress as ProgressBar;
use gpui_kit::component::{
    Disableable as _, Icon, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement as _, ParentElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use pc_specs_core::clean::{self, Cleaned, Kind, Measured, Progress, Spec};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 量完之后过了这么久再切回来，就重新量一遍。
const STALE: Duration = Duration::from_secs(5 * 60);

/// 「清理」页的状态。
#[derive(Default)]
pub(super) struct Cleaner {
    rows: Vec<Row>,
    phase: Phase,
    /// 上一次量完的时间。
    measured_at: Option<Instant>,
    /// 任务编号：旧任务的消息据此丢弃。
    epoch: u64,
    /// 用户对每一类勾选过的选择；重新量、一类消失又出现之后，还是他上次选的样子。
    chosen: HashMap<Kind, bool>,
}

/// 一类东西。
struct Row {
    spec: Spec,
    /// 量出来的结果；还没量完是 `None`。
    measured: Option<Measured>,
    selected: bool,
}

impl Row {
    /// 能清出多少字节；还没量出来是 0。
    fn bytes(&self) -> u64 {
        self.measured.as_ref().map_or(0, |measured| measured.bytes)
    }

    /// 勾了、而且里面真有东西：这样的才会被清。
    fn picked(&self) -> bool {
        self.selected && self.bytes() > 0
    }
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Measuring,
    Cleaning(Cleaning),
}

struct Cleaning {
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
    /// 这次要清的总量（量出来的），进度条的分母。
    expected: u64,
    /// 一共处理了多少项，其中有多少删不掉。
    removed: u64,
    failed: u64,
}

/// 后台线程发回来的消息。
enum Event {
    /// 位置写死的几类。
    Found(Vec<Spec>),
    /// 闲置项目里的构建产物：要走遍放代码的文件夹才找得到，所以最后才报；一处也没有就是 `None`。
    Projects(Option<Spec>),
    Measured(Kind, Measured),
    Cleaned(Cleaned),
}

impl Cleaner {
    pub(super) fn idle(&self) -> bool {
        matches!(self.phase, Phase::Idle)
    }

    fn total(&self) -> u64 {
        self.rows.iter().map(Row::bytes).sum()
    }

    fn picked_total(&self) -> u64 {
        self.rows
            .iter()
            .filter(|row| row.picked())
            .map(Row::bytes)
            .sum()
    }

    /// 一类东西的新一行：勾不勾沿用用户上次的选择，没选过就用默认的。
    fn row(&self, spec: Spec) -> Row {
        Row {
            selected: self
                .chosen
                .get(&spec.kind)
                .copied()
                .unwrap_or_else(|| default_selected(spec.kind)),
            spec,
            measured: None,
        }
    }

    /// 把一行放到它该在的位置（各类有固定的先后），同一类已经有就换掉。
    fn put(&mut self, row: Row) {
        self.rows.retain(|other| other.spec.kind != row.spec.kind);
        let at = self
            .rows
            .iter()
            .position(|other| other.spec.kind as u8 > row.spec.kind as u8)
            .unwrap_or(self.rows.len());
        self.rows.insert(at, row);
    }
}

/// 默认勾上的：悄悄重建、基本无感的几类。开发者缓存和项目产物要联网重下、重建很久，废纸篓清了就找不回来，
/// 让用户自己决定。
fn default_selected(kind: Kind) -> bool {
    !matches!(kind, Kind::Developer | Kind::Projects | Kind::Trash)
}

impl StoragePage {
    /// 切到「清理」：从没量过、或者上一次量完已经隔了一阵，就重新量一遍。
    pub(super) fn refresh_cleanup(&mut self, cx: &mut Context<Self>) {
        let stale = self
            .cleaner
            .measured_at
            .is_none_or(|at| at.elapsed() > STALE);
        if stale {
            self.measure_cleanup(cx);
        }
    }

    /// 在后台把每一类能清多少量一遍，量完一类就报一类。
    pub(super) fn measure_cleanup(&mut self, cx: &mut Context<Self>) {
        if !self.cleaner.idle() {
            return;
        }
        self.cleaner.epoch += 1;
        let epoch = self.cleaner.epoch;
        let (tx, rx) = async_channel::unbounded();
        let started = spawn_thread("pc-specs-clean-measure", move || {
            let _ = guarded("clean measure", || {
                let never = AtomicBool::new(false);
                let specs = clean::specs();
                if tx.send_blocking(Event::Found(specs.clone())).is_err() {
                    return;
                }
                for spec in &specs {
                    let measured = clean::measure(spec, &never);
                    if tx
                        .send_blocking(Event::Measured(spec.kind, measured))
                        .is_err()
                    {
                        return;
                    }
                }
                // 项目里的构建产物放在最后：先找出在哪（几秒），再量（可能是几百 GB，要一两分钟）。
                let projects = clean::projects(&never);
                if tx.send_blocking(Event::Projects(projects.clone())).is_err() {
                    return;
                }
                if let Some(spec) = projects {
                    let measured = clean::measure(&spec, &never);
                    let _ = tx.send_blocking(Event::Measured(spec.kind, measured));
                }
            });
        });
        if !started {
            let message = t!("sa_error_thread").into_owned();
            self.toast(NotificationType::Error, message, cx);
            return;
        }
        self.cleaner.phase = Phase::Measuring;
        self.listen_cleanup(epoch, rx, cx);
        cx.notify();
    }

    /// 在界面线程里接后台线程的消息；线程结束（通道关了）就收尾。
    fn listen_cleanup(
        &mut self,
        epoch: u64,
        rx: async_channel::Receiver<Event>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                if this
                    .update(cx, |this, cx| this.cleanup_event(epoch, event, cx))
                    .is_err()
                {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| this.cleanup_finished(epoch, cx));
        })
        .detach();
        // 清理期间每隔一会儿重画一次，让「已释放」的数字动起来。
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let busy = this.update(cx, |this, cx| {
                    let busy = this.cleaner.epoch == epoch
                        && matches!(this.cleaner.phase, Phase::Cleaning(_));
                    if busy {
                        cx.notify();
                    }
                    busy
                });
                if !matches!(busy, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    fn cleanup_event(&mut self, epoch: u64, event: Event, cx: &mut Context<Self>) {
        if self.cleaner.epoch != epoch {
            return;
        }
        match event {
            Event::Found(specs) => {
                // 重新量的时候，上一次找到的项目产物先留着（再找一遍要几秒），免得这一行闪一下。
                let stale = std::mem::take(&mut self.cleaner.rows)
                    .into_iter()
                    .find(|row| row.spec.kind == Kind::Projects);
                for spec in specs {
                    let row = self.cleaner.row(spec);
                    self.cleaner.put(row);
                }
                if let Some(mut row) = stale {
                    row.measured = None;
                    self.cleaner.put(row);
                }
            }
            Event::Projects(spec) => {
                self.cleaner
                    .rows
                    .retain(|row| row.spec.kind != Kind::Projects);
                if let Some(spec) = spec {
                    let row = self.cleaner.row(spec);
                    self.cleaner.put(row);
                }
            }
            Event::Measured(kind, measured) => {
                if let Some(row) = self
                    .cleaner
                    .rows
                    .iter_mut()
                    .find(|row| row.spec.kind == kind)
                {
                    row.measured = Some(measured);
                }
            }
            Event::Cleaned(done) => {
                if let Phase::Cleaning(run) = &mut self.cleaner.phase {
                    run.removed += done.removed;
                    run.failed += done.failed;
                }
            }
        }
        cx.notify();
    }

    fn cleanup_finished(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if self.cleaner.epoch != epoch {
            return;
        }
        match std::mem::take(&mut self.cleaner.phase) {
            Phase::Measuring => self.cleaner.measured_at = Some(Instant::now()),
            Phase::Cleaning(run) => {
                let unit = self.app.read(cx).settings.size_unit;
                let freed = run.progress.freed.load(Ordering::Relaxed);
                let size = fmt::bytes(freed, unit);
                let (kind, message) = if run.failed == 0 {
                    (
                        NotificationType::Success,
                        t!("sa_clean_done", size = size).into_owned(),
                    )
                } else if freed == 0 {
                    (
                        NotificationType::Warning,
                        t!("sa_clean_nothing_freed", n = count(run.failed)).into_owned(),
                    )
                } else {
                    (
                        NotificationType::Warning,
                        t!("sa_clean_done_skipped", size = size, n = count(run.failed))
                            .into_owned(),
                    )
                };
                self.toast(kind, message, cx);
                // 清完再量一遍，列表上的数字才是现在的样子。
                self.measure_cleanup(cx);
            }
            Phase::Idle => {}
        }
        cx.notify();
    }

    /// 勾上或取消勾选某一类。
    fn pick_cleanup(&mut self, index: usize, selected: bool, cx: &mut Context<Self>) {
        if matches!(self.cleaner.phase, Phase::Cleaning(_)) {
            return;
        }
        if let Some(row) = self.cleaner.rows.get_mut(index) {
            row.selected = selected;
            self.cleaner.chosen.insert(row.spec.kind, selected);
            cx.notify();
        }
    }

    /// 在空间分析里打开这一类里占得最多的位置，看看里面到底是什么。
    fn explore_cleanup(&mut self, index: usize, cx: &mut Context<Self>) {
        let path = self
            .cleaner
            .rows
            .get(index)
            .and_then(|row| row.measured.as_ref()?.main.clone());
        if let Some(path) = path {
            self.start(folder_target(path), cx);
        }
    }

    fn reveal_cleanup(&mut self, index: usize, cx: &mut Context<Self>) {
        let path = self
            .cleaner
            .rows
            .get(index)
            .and_then(|row| row.measured.as_ref()?.main.clone());
        if let Some(path) = path {
            cx.reveal_path(&path);
        }
    }

    /// 先弹确认框，列出要清的几类；点了确定才真的动手。
    fn ask_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.cleaner.idle() {
            return;
        }
        let unit = self.app.read(cx).settings.size_unit;
        let chosen: Vec<(String, u64)> = self
            .cleaner
            .rows
            .iter()
            .filter(|row| row.picked())
            .map(|row| (describe(row.spec.kind).1, row.bytes()))
            .collect();
        if chosen.is_empty() {
            return;
        }
        let total: u64 = chosen.iter().map(|(_, bytes)| bytes).sum();
        let lines: Vec<String> = chosen
            .iter()
            .map(|(title, bytes)| format!("• {title} ({})", fmt::bytes(*bytes, unit)))
            .collect();
        let description = format!(
            "{}\n\n{}",
            t!("sa_clean_confirm_desc", bin = bin_name()),
            lines.join("\n")
        );
        let title = t!("sa_clean_confirm_title", size = fmt::bytes(total, unit)).into_owned();
        let ok = t!("sa_clean_ok").into_owned();
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
                    page.update(cx, |page, cx| page.start_cleanup(cx));
                    true
                })
        });
    }

    /// 在后台一类一类地清；每清完一类报一次，全清完（或被停下）再统一收尾。
    fn start_cleanup(&mut self, cx: &mut Context<Self>) {
        if !self.cleaner.idle() {
            return;
        }
        let jobs: Vec<Spec> = self
            .cleaner
            .rows
            .iter()
            .filter(|row| row.picked())
            .map(|row| row.spec.clone())
            .collect();
        if jobs.is_empty() {
            return;
        }
        let expected = self.cleaner.picked_total();
        self.cleaner.epoch += 1;
        let epoch = self.cleaner.epoch;
        let progress = Arc::new(Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = async_channel::unbounded();
        let started = spawn_thread("pc-specs-clean", {
            let (progress, cancel) = (progress.clone(), cancel.clone());
            move || {
                let _ = guarded("clean", || {
                    for spec in &jobs {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let done = clean::clean(spec, &progress, &cancel);
                        if tx.send_blocking(Event::Cleaned(done)).is_err() {
                            break;
                        }
                    }
                });
            }
        });
        if !started {
            let message = t!("sa_error_thread").into_owned();
            self.toast(NotificationType::Error, message, cx);
            return;
        }
        self.cleaner.phase = Phase::Cleaning(Cleaning {
            progress,
            cancel,
            expected,
            removed: 0,
            failed: 0,
        });
        self.listen_cleanup(epoch, rx, cx);
        cx.notify();
    }

    fn stop_cleanup(&mut self, cx: &mut Context<Self>) {
        if let Phase::Cleaning(run) = &self.cleaner.phase {
            run.cancel.store(true, Ordering::Relaxed);
            cx.notify();
        }
    }
}

/// 每一类的图标、名字和说明。
fn describe(kind: Kind) -> (IconName, String, String) {
    match kind {
        Kind::Caches => (
            IconName::Database,
            t!("sa_clean_caches").into_owned(),
            t!("sa_clean_caches_desc").into_owned(),
        ),
        Kind::Browsers => (
            IconName::Globe,
            t!("sa_clean_browsers").into_owned(),
            t!("sa_clean_browsers_desc").into_owned(),
        ),
        Kind::Logs => (
            IconName::FileText,
            t!("sa_clean_logs").into_owned(),
            t!("sa_clean_logs_desc").into_owned(),
        ),
        Kind::Temp => (
            IconName::FileClock,
            t!("sa_clean_temp").into_owned(),
            t!("sa_clean_temp_desc").into_owned(),
        ),
        Kind::Developer => (
            IconName::Hammer,
            t!("sa_clean_developer").into_owned(),
            t!("sa_clean_developer_desc").into_owned(),
        ),
        Kind::Projects => (
            IconName::FolderCode,
            t!("sa_clean_projects").into_owned(),
            t!("sa_clean_projects_desc").into_owned(),
        ),
        Kind::Trash => (
            IconName::Trash,
            bin_name(),
            t!("sa_clean_trash_desc", bin = bin_name()).into_owned(),
        ),
    }
}

// ---- 界面 ---------------------------------------------------------------------

/// 返回 `(页头右侧的操作, 正文)`。
pub(super) fn body(
    page: &StoragePage,
    cx: &mut Context<StoragePage>,
) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let entity = cx.entity();
    let colors = Colors::new(cx);
    let unit = page.app.read(cx).settings.size_unit;
    let cleaner = &page.cleaner;
    let scanning = matches!(page.scan, Scan::Running(_));

    let again = {
        let page = entity.clone();
        Button::new("sa-clean-again")
            .small()
            .outline()
            .icon(IconName::RefreshCw)
            .label(t!("sa_clean_again").into_owned())
            .disabled(!cleaner.idle())
            .on_click(move |_, _, cx| page.update(cx, |page, cx| page.measure_cleanup(cx)))
            .into_any_element()
    };

    let mut body = vec![summary(cleaner, unit, &entity, colors, cx)];
    let locked = matches!(cleaner.phase, Phase::Cleaning(_));
    for (index, row) in cleaner.rows.iter().enumerate() {
        body.push(row_view(
            index, row, unit, locked, scanning, &entity, colors, cx,
        ));
    }
    if cleaner.rows.is_empty() && cleaner.idle() && cleaner.measured_at.is_some() {
        body.push(
            ui::muted(t!("sa_clean_none").into_owned(), cx)
                .py_6()
                .into_any_element(),
        );
    }
    (vec![again], body)
}

/// 最上面一张卡：总共能清多少，「清理」按钮，以及量 / 清的进度。
fn summary(
    cleaner: &Cleaner,
    unit: SizeUnit,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &App,
) -> AnyElement {
    let headline = match &cleaner.phase {
        Phase::Measuring => t!("sa_clean_measuring").into_owned(),
        Phase::Cleaning(run) => t!(
            "sa_clean_running",
            size = fmt::bytes(run.progress.freed.load(Ordering::Relaxed), unit)
        )
        .into_owned(),
        Phase::Idle => t!("sa_clean_total", size = fmt::bytes(cleaner.total(), unit)).into_owned(),
    };

    let page = entity.clone();
    let action = match &cleaner.phase {
        Phase::Cleaning(run) => Button::new("sa-clean-stop")
            .small()
            .outline()
            .label(t!("sa_clean_stop").into_owned())
            .disabled(run.cancel.load(Ordering::Relaxed))
            .on_click(move |_, _, cx| page.update(cx, |page, cx| page.stop_cleanup(cx))),
        _ => Button::new("sa-clean-go")
            .danger()
            .icon(IconName::BrushCleaning)
            .label(
                t!(
                    "sa_clean_go",
                    size = fmt::bytes(cleaner.picked_total(), unit)
                )
                .into_owned(),
            )
            .disabled(!cleaner.idle() || cleaner.picked_total() == 0)
            .on_click(move |_, window, cx| {
                page.update(cx, |page, cx| page.ask_cleanup(window, cx))
            }),
    };

    let bar = match &cleaner.phase {
        Phase::Idle => None,
        Phase::Measuring => Some(ProgressBar::new("sa-clean-progress").loading(true)),
        Phase::Cleaning(run) if run.expected == 0 => {
            Some(ProgressBar::new("sa-clean-progress").loading(true))
        }
        Phase::Cleaning(run) => {
            let done = run.progress.freed.load(Ordering::Relaxed).min(run.expected);
            Some(
                ProgressBar::new("sa-clean-progress")
                    .value((done as f64 / run.expected as f64 * 100.0) as f32),
            )
        }
    };

    v_flex()
        .w_full()
        .gap_3()
        .p_4()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .child(
            h_flex()
                .w_full()
                .items_center()
                .flex_wrap()
                .gap_3()
                .child(
                    Icon::new(IconName::BrushCleaning)
                        .flex_shrink_0()
                        .text_color(colors.muted),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(240.))
                        .gap_0p5()
                        .child(div().text_lg().font_semibold().child(headline))
                        .child(ui::muted(t!("sa_clean_intro").into_owned(), cx).text_xs()),
                )
                .child(action),
        )
        .children(bar)
        .into_any_element()
}

/// 一类东西占一行：勾选框、说明、占了多少、占得最多的是什么，以及去看看的入口。
#[allow(clippy::too_many_arguments)]
fn row_view(
    index: usize,
    row: &Row,
    unit: SizeUnit,
    locked: bool,
    scanning: bool,
    entity: &Entity<StoragePage>,
    colors: Colors,
    cx: &App,
) -> AnyElement {
    let (icon, title, description) = describe(row.spec.kind);
    let measured = row.measured.as_ref();
    let selectable = row.bytes() > 0 && !locked;

    let page = entity.clone();
    let pick = Checkbox::new(("sa-clean-pick", index))
        .checked(row.picked())
        .disabled(!selectable)
        .on_click(move |checked, _, cx| {
            let checked = *checked;
            page.update(cx, |page, cx| page.pick_cleanup(index, checked, cx));
        });

    // 占得最多的几项，让用户知道清掉的到底是什么。
    let biggest = measured
        .filter(|measured| !measured.biggest.is_empty())
        .map(|measured| {
            let items: Vec<String> = measured
                .biggest
                .iter()
                .map(|(name, bytes)| format!("{name} ({})", fmt::bytes(*bytes, unit)))
                .collect();
            ui::muted(
                t!("sa_clean_biggest", items = items.join(" · ")).into_owned(),
                cx,
            )
            .text_xs()
        });

    let blocked = measured.is_some_and(|measured| measured.blocked).then(|| {
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(colors.warning),
            )
            .child(
                div()
                    .text_xs()
                    .child(t!("sa_clean_blocked").into_owned()),
            )
            .when(cfg!(target_os = "macos"), |this| {
                this.child(
                    Button::new(("sa-clean-fda", index))
                        .xsmall()
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

    let size = match measured {
        Some(measured) => v_flex()
            .items_end()
            .child(
                div()
                    .text_base()
                    .font_semibold()
                    .child(fmt::bytes(measured.bytes, unit)),
            )
            .when(measured.files > 0, |this| {
                this.child(
                    ui::muted(
                        t!("sa_clean_files", n = count(measured.files)).into_owned(),
                        cx,
                    )
                    .text_xs(),
                )
            }),
        None => v_flex().items_end().child(ui::muted("…", cx)),
    };

    let buttons = measured
        .and_then(|measured| measured.main.as_ref())
        .map(|_| {
            let (explore_page, reveal_page) = (entity.clone(), entity.clone());
            h_flex()
                .flex_shrink_0()
                .gap_1()
                .child(
                    Button::new(("sa-clean-explore", index))
                        .small()
                        .ghost()
                        .icon(IconName::ChartPie)
                        .tooltip(t!("sa_clean_explore").into_owned())
                        .disabled(scanning || locked)
                        .on_click(move |_, _, cx| {
                            explore_page.update(cx, |page, cx| page.explore_cleanup(index, cx))
                        }),
                )
                .child(
                    Button::new(("sa-clean-reveal", index))
                        .small()
                        .ghost()
                        .icon(IconName::FolderSearch)
                        .tooltip(ui::reveal_label())
                        .on_click(move |_, _, cx| {
                            reveal_page.update(cx, |page, cx| page.reveal_cleanup(index, cx))
                        }),
                )
        });

    h_flex()
        .w_full()
        .items_start()
        .gap_3()
        .p_3()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .child(div().pt_0p5().child(pick))
        .child(
            Icon::new(icon)
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
                .child(ui::muted(description, cx).text_xs())
                .children(biggest)
                .children(blocked),
        )
        .child(size)
        .children(buttons)
        .into_any_element()
}

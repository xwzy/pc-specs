//! 速度测试：在所选磁盘上写一个临时文件，量顺序读写和 4K 随机读写的速度。
//!
//! 怎么测、怎么绕开系统缓存都在核心库的 `bench` 里。这里管选盘、后台任务、进度和结果的展示，
//! 以及每块盘最近几次的记录（用来说「比上次快了还是慢了」）。

use super::analyzer::Colors;
use super::{Mode, Scan, StoragePage, history, scan_root};
use crate::settings::SizeUnit;
use crate::state::spawn_thread;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::progress::Progress as ProgressBar;
use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use pc_specs_core::bench::{self, Phase, Progress, Random, Report};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 每块盘留最近这么多次的记录。
const KEEP: usize = 8;
/// 测试大小的几档，单位是 GiB（十进制的单位下是 GB）。
const SIZES: [u64; 3] = [1, 4, 16];
/// 两次结果相差不到这么多（百分比），就当没变：测量本身就有几个百分点的起伏。
const NOISE: f64 = 3.0;

/// 「速度测试」页的状态。
#[derive(Default)]
pub(super) struct Speed {
    /// 选中的盘（挂载点）；没选过就是第一块。
    mount: Option<String>,
    /// 测试大小选的是第几档。
    size: usize,
    state: State,
    /// 各块盘的记录，最新的在最后。读过一次就留在这里，不必每次重画都去读文件。
    history: HashMap<String, Vec<Report>>,
    /// 任务编号：结果回来时据此确认它还是当前这一次。
    epoch: u64,
}

#[derive(Default)]
enum State {
    #[default]
    Idle,
    Running(Run),
}

struct Run {
    mount: String,
    disk: String,
    progress: Arc<Progress>,
    cancel: Arc<AtomicBool>,
}

impl Speed {
    pub(super) fn running(&self) -> bool {
        matches!(self.state, State::Running(_))
    }
}

/// 一块可以测的盘。
struct Volume {
    name: String,
    mount: String,
}

/// 后台线程交回来的：这次的结果，以及测完之后这块盘的全部记录。
type Outcome = (Result<Report, bench::Error>, Option<Vec<Report>>);

impl StoragePage {
    fn speed_volumes(&self, cx: &App) -> Vec<Volume> {
        self.app
            .read(cx)
            .snapshot
            .storages
            .iter()
            .filter_map(|disk| {
                let mount = disk.mount_point.clone().filter(|mount| !mount.is_empty())?;
                let name = if disk.name.trim().is_empty() {
                    mount.clone()
                } else {
                    disk.name.clone()
                };
                Some(Volume { name, mount })
            })
            .collect()
    }

    /// 选中的那块盘；没选过、或者选过的那块已经拔掉了，就是第一块。
    fn speed_volume<'a>(&self, volumes: &'a [Volume]) -> Option<&'a Volume> {
        self.speed
            .mount
            .as_ref()
            .and_then(|mount| volumes.iter().find(|volume| &volume.mount == mount))
            .or_else(|| volumes.first())
    }

    /// 在扫描或清理：它们和测速抢同一块盘，测出来的数字会偏低。
    fn speed_blocked(&self) -> bool {
        matches!(self.scan, Scan::Running(_)) || !self.cleaner.idle()
    }

    /// 从概览卡片上点「测速」进来：直接选好那块盘。
    pub(super) fn open_speed(&mut self, mount: String, cx: &mut Context<Self>) {
        if !self.speed.running() {
            self.speed.mount = Some(mount);
        }
        self.set_mode(Mode::Speed, cx);
    }

    /// 进到「速度测试」，或者换了一块盘：读出这块盘的记录。
    pub(super) fn refresh_speed(&mut self, cx: &mut Context<Self>) {
        let volumes = self.speed_volumes(cx);
        if let Some(volume) = self.speed_volume(&volumes) {
            let mount = volume.mount.clone();
            self.speed
                .history
                .entry(mount.clone())
                .or_insert_with(|| load(&mount));
        }
    }

    fn pick_speed_disk(&mut self, mount: String, cx: &mut Context<Self>) {
        if self.speed.running() {
            return;
        }
        self.speed.mount = Some(mount);
        self.refresh_speed(cx);
        cx.notify();
    }

    fn pick_speed_size(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.speed.running() {
            self.speed.size = index.min(SIZES.len() - 1);
            cx.notify();
        }
    }

    fn start_speed(&mut self, cx: &mut Context<Self>) {
        if self.speed.running() || self.speed_blocked() {
            return;
        }
        let volumes = self.speed_volumes(cx);
        let Some(volume) = self.speed_volume(&volumes) else {
            return;
        };
        let (mount, disk) = (volume.mount.clone(), volume.name.clone());
        let unit = self.app.read(cx).settings.size_unit;
        let options = bench::Options {
            size: size_options(unit)[self.speed.size.min(SIZES.len() - 1)].0,
            ..bench::Options::default()
        };
        let dir = test_dir(&mount);

        self.speed.epoch += 1;
        let epoch = self.speed.epoch;
        let progress = Arc::new(Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = async_channel::bounded::<Outcome>(1);
        let started = spawn_thread("pc-specs-speed", {
            let (progress, cancel, mount) = (progress.clone(), cancel.clone(), mount.clone());
            move || {
                let result = guarded("speed test", || {
                    bench::run(&dir, &options, &progress, &cancel)
                })
                .unwrap_or_else(|| Err(bench::Error::Io("the test crashed".into())));
                // 测完马上记下来，和结果一起交回界面。
                let history = result.as_ref().ok().map(|report| record(&mount, report));
                let _ = tx.send_blocking((result, history));
            }
        });
        if !started {
            let message = t!("sa_error_thread").into_owned();
            self.toast(NotificationType::Error, message, cx);
            return;
        }
        self.speed.state = State::Running(Run {
            mount,
            disk,
            progress,
            cancel,
        });
        cx.spawn(async move |this, cx| {
            let outcome = rx.recv().await.unwrap_or_else(|_| {
                (
                    Err(bench::Error::Io("the test stopped unexpectedly".into())),
                    None,
                )
            });
            let _ = this.update(cx, |this, cx| this.speed_finished(epoch, outcome, cx));
        })
        .detach();
        // 测的时候每隔一会儿重画一次，让实时速度和进度动起来。
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let busy = this.update(cx, |this, cx| {
                    let busy = this.speed.epoch == epoch && this.speed.running();
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
        cx.notify();
    }

    fn speed_finished(&mut self, epoch: u64, (result, history): Outcome, cx: &mut Context<Self>) {
        if self.speed.epoch != epoch {
            return;
        }
        let State::Running(run) = std::mem::take(&mut self.speed.state) else {
            return;
        };
        match result {
            Ok(report) => {
                if self.mode != Mode::Speed {
                    // 测的时候用户去了别处：结果出来了要说一声。
                    let unit = self.app.read(cx).settings.size_unit;
                    let message = t!(
                        "sa_speed_done",
                        disk = run.disk,
                        write = rate(report.seq_write, unit),
                        read = rate(report.seq_read, unit)
                    )
                    .into_owned();
                    self.toast(NotificationType::Success, message, cx);
                }
                // 记录存不下（比如没有配置目录）也要把这次的结果摆出来。
                let list = history.unwrap_or_else(|| vec![report]);
                self.speed.history.insert(run.mount, list);
            }
            Err(bench::Error::Cancelled) => {}
            Err(bench::Error::NoSpace) => {
                let message = t!("sa_speed_no_space").into_owned();
                self.toast(NotificationType::Warning, message, cx);
            }
            Err(bench::Error::Io(error)) => {
                let message = t!("sa_speed_failed", error = error).into_owned();
                self.toast(NotificationType::Error, message, cx);
            }
        }
        cx.notify();
    }

    fn stop_speed(&mut self, cx: &mut Context<Self>) {
        if let State::Running(run) = &self.speed.state {
            run.cancel.store(true, Ordering::Relaxed);
            cx.notify();
        }
    }
}

// ---- 测试位置与记录 -------------------------------------------------------------

/// 测试大小的几档：跟着用户选的单位走，二进制是 1 / 4 / 16 GiB，十进制是 1 / 4 / 16 GB。
fn size_options(unit: SizeUnit) -> [(u64, String); 3] {
    let (base, name) = match unit {
        SizeUnit::Binary => (1u64 << 30, "GiB"),
        SizeUnit::Decimal => (1_000_000_000, "GB"),
    };
    SIZES.map(|n| (n * base, format!("{n} {name}")))
}

/// 在这块盘上找个放测试文件的目录。用户自己的目录就在这块盘上的话，放进缓存目录里
/// （系统盘的根目录一般写不进去）；否则放在盘的根（外接盘、数据盘都写得进）。
fn test_dir(mount: &str) -> PathBuf {
    let root = scan_root(mount);
    match dirs::cache_dir() {
        Some(cache) if same_volume(&cache, &root) => cache.join("pc-specs"),
        _ => root,
    }
}

/// 两个路径在不在同一个卷上。
fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        // Windows：盘符相同就是同一个卷（把卷挂到文件夹里的情况少见，这里不管）。
        let drive = |path: &Path| {
            path.components()
                .next()
                .map(|part| part.as_os_str().to_string_lossy().to_uppercase())
        };
        drive(a).is_some() && drive(a) == drive(b)
    }
}

/// 记录放哪：和别的设置、记录一起，在用户配置目录下。
fn history_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("pc-specs").join("speed"))
}

/// 读出这块盘的记录；没有、或者文件坏了，都当作没有记录。
fn load(mount: &str) -> Vec<Report> {
    history_dir().map_or_else(Vec::new, |dir| load_from(&dir, mount))
}

fn load_from(dir: &Path, mount: &str) -> Vec<Report> {
    std::fs::read_to_string(history::file_in(dir, Path::new(mount)))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// 把这次的结果记到这块盘名下，返回记下之后的全部记录（最新的在最后）。
fn record(mount: &str, report: &Report) -> Vec<Report> {
    match history_dir() {
        Some(dir) => record_in(&dir, mount, report),
        None => vec![report.clone()],
    }
}

fn record_in(dir: &Path, mount: &str, report: &Report) -> Vec<Report> {
    let mut list = load_from(dir, mount);
    list.push(report.clone());
    if list.len() > KEEP {
        list.drain(..list.len() - KEEP);
    }
    history::save(&history::file_in(dir, Path::new(mount)), &list);
    list
}

// ---- 数字怎么显示 ---------------------------------------------------------------

fn rate(bytes_per_sec: u64, unit: SizeUnit) -> String {
    format!("{}/s", fmt::bytes(bytes_per_sec, unit))
}

/// 每秒多少次：上万的写成 `12k`，上千的 `1.5k`。
fn iops(n: u32) -> String {
    match n {
        10_000.. => format!("{}k", n / 1000),
        1_000.. => format!("{:.1}k", f64::from(n) / 1000.0),
        _ => n.to_string(),
    }
}

fn micros(us: u32) -> String {
    match us {
        0 => "<1 µs".into(),
        1..=999 => format!("{us} µs"),
        _ => format!("{:.1} ms", f64::from(us) / 1000.0),
    }
}

/// 和上一次比的变化。
struct Delta {
    text: String,
    better: bool,
}

/// 这一次比上一次快了或慢了百分之几；差得不多、或者哪一次没数据，就不说。
fn delta(now: u64, before: u64) -> Option<Delta> {
    if now == 0 || before == 0 {
        return None;
    }
    let change = (now as f64 - before as f64) / before as f64 * 100.0;
    (change.abs() >= NOISE).then(|| Delta {
        text: format!("{change:+.0}%"),
        better: change > 0.0,
    })
}

fn phase_title(phase: Phase) -> String {
    match phase {
        Phase::SeqWrite => t!("sa_speed_seq_write").into_owned(),
        Phase::SeqRead => t!("sa_speed_seq_read").into_owned(),
        Phase::RandRead => t!("sa_speed_rand_read").into_owned(),
        Phase::RandReadParallel => t!("sa_speed_rand_read_mt", n = bench::THREADS).into_owned(),
        Phase::RandWrite => t!("sa_speed_rand_write").into_owned(),
    }
}

fn random(report: &Report, phase: Phase) -> &Random {
    match phase {
        Phase::RandReadParallel => &report.rand_read_parallel,
        Phase::RandWrite => &report.rand_write,
        _ => &report.rand_read,
    }
}

/// 这一阶段拿来比较的数字：顺序读写是字节每秒，随机读写是每秒多少次。越大越好。
fn metric(report: &Report, phase: Phase) -> u64 {
    match phase {
        Phase::SeqWrite => report.seq_write,
        Phase::SeqRead => report.seq_read,
        _ => u64::from(random(report, phase).iops),
    }
}

/// 一个阶段的大数字：顺序读写显示速度，随机读写显示每秒多少次。
fn headline(phase: Phase, value: u64, unit: SizeUnit) -> String {
    if phase.is_sequential() {
        rate(value, unit)
    } else {
        t!(
            "sa_speed_iops",
            n = iops(value.min(u64::from(u32::MAX)) as u32)
        )
        .into_owned()
    }
}

// ---- 界面 -----------------------------------------------------------------------

/// 一个阶段的状态。
#[derive(Clone, Copy)]
enum Status {
    Waiting,
    /// 正在测：进度（千分之几）和此刻的速度。
    Running {
        permille: u32,
        rate: u64,
    },
    Done,
}

/// 要摆出来的结果：正在测的，或者最近一次测完的。
struct View<'a> {
    report: Report,
    /// 每一阶段测完了没有（按 `Phase::ALL` 的顺序）。
    done: [bool; Phase::ALL.len()],
    /// 正在测的阶段，以及它的进度和此刻的速度。
    live: Option<(Phase, u32, u64)>,
    /// 拿来比较的上一次。
    before: Option<&'a Report>,
}

fn shown<'a>(page: &StoragePage, history: &'a [Report]) -> Option<View<'a>> {
    match &page.speed.state {
        State::Running(run) => Some(View {
            report: run.progress.report(),
            done: Phase::ALL.map(|phase| run.progress.is_finished(phase)),
            live: Some((
                run.progress.phase(),
                run.progress.permille(),
                run.progress.rate(),
            )),
            before: history.last(),
        }),
        State::Idle => history.last().map(|report| View {
            report: report.clone(),
            done: [true; Phase::ALL.len()],
            live: None,
            before: history
                .len()
                .checked_sub(2)
                .and_then(|index| history.get(index)),
        }),
    }
}

/// 返回 `(页头右侧的操作, 正文)`。
pub(super) fn body(
    page: &StoragePage,
    cx: &mut Context<StoragePage>,
) -> (Vec<AnyElement>, Vec<AnyElement>) {
    let entity = cx.entity();
    let colors = Colors::new(cx);
    let unit = page.app.read(cx).settings.size_unit;
    let volumes = page.speed_volumes(cx);
    if volumes.is_empty() {
        return (
            Vec::new(),
            vec![ui::empty_state(
                IconName::HardDrive,
                t!("storage_no_disks").into_owned(),
                "",
            )],
        );
    }
    let selected = page.speed_volume(&volumes);
    let history: &[Report] = selected
        .and_then(|volume| page.speed.history.get(&volume.mount))
        .map_or(&[], Vec::as_slice);
    let view = shown(page, history);

    let mut body = vec![control(
        page,
        &volumes,
        selected,
        view.as_ref(),
        &entity,
        unit,
        colors,
        cx,
    )];
    match &view {
        Some(view) => body.extend(results(view, unit, colors, cx)),
        None => body.push(
            ui::muted(t!("sa_speed_empty").into_owned(), cx)
                .py_6()
                .into_any_element(),
        ),
    }
    if !history.is_empty() {
        body.push(history_table(history, unit));
    }
    (Vec::new(), body)
}

/// 最上面一张卡：选盘、选大小、开始或停止，以及整体进度。
#[allow(clippy::too_many_arguments)]
fn control(
    page: &StoragePage,
    volumes: &[Volume],
    selected: Option<&Volume>,
    view: Option<&View>,
    entity: &Entity<StoragePage>,
    unit: SizeUnit,
    colors: Colors,
    cx: &App,
) -> AnyElement {
    let running = match &page.speed.state {
        State::Running(run) => Some(run),
        State::Idle => None,
    };
    let blocked = page.speed_blocked();

    let headline = match running {
        Some(run) => t!("sa_speed_running", disk = run.disk.clone()).into_owned(),
        None => t!("sa_speed_title").into_owned(),
    };

    let action = match running {
        Some(run) => {
            let page = entity.clone();
            Button::new("sa-speed-stop")
                .small()
                .outline()
                .icon(IconName::Square)
                .label(t!("sa_speed_stop").into_owned())
                .disabled(run.cancel.load(Ordering::Relaxed))
                .on_click(move |_, _, cx| page.update(cx, |page, cx| page.stop_speed(cx)))
        }
        None => {
            let page = entity.clone();
            let label = if view.is_some() {
                t!("sa_speed_again")
            } else {
                t!("sa_speed_start")
            };
            Button::new("sa-speed-start")
                .primary()
                .icon(IconName::Play)
                .label(label.into_owned())
                .disabled(blocked || selected.is_none())
                .on_click(move |_, _, cx| page.update(cx, |page, cx| page.start_speed(cx)))
        }
    };

    let disks = h_flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .child(ui::muted(t!("sa_speed_disk").into_owned(), cx).text_sm())
        .children(volumes.iter().enumerate().map(|(index, volume)| {
            let (page, mount) = (entity.clone(), volume.mount.clone());
            Button::new(("sa-speed-disk", index))
                .small()
                .outline()
                .label(volume.name.clone())
                .selected(selected.is_some_and(|chosen| chosen.mount == volume.mount))
                .disabled(running.is_some())
                .on_click(move |_, _, cx| {
                    let mount = mount.clone();
                    page.update(cx, |page, cx| page.pick_speed_disk(mount, cx))
                })
        }));

    let sizes = {
        let target = entity.clone();
        let options = size_options(unit)
            .into_iter()
            .enumerate()
            .map(|(index, (_, label))| (index, label))
            .collect();
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(ui::muted(t!("sa_speed_size").into_owned(), cx).text_sm())
            .child(ui::segmented(
                "sa-speed-size",
                options,
                page.speed.size,
                move |index, cx| target.update(cx, |page, cx| page.pick_speed_size(index, cx)),
            ))
    };

    // 整体进度：已经测完的阶段加上正在测的这一段。
    let bar = running.map(|_| {
        let percent = view.map_or(0.0, |view| {
            let done = view.done.iter().filter(|done| **done).count() as f32;
            let current = view
                .live
                .map_or(0.0, |(_, permille, _)| permille as f32 / 1000.0);
            (done + current).min(Phase::ALL.len() as f32) / Phase::ALL.len() as f32 * 100.0
        });
        ProgressBar::new("sa-speed-progress").value(percent)
    });

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
                    Icon::new(IconName::Gauge)
                        .flex_shrink_0()
                        .text_color(colors.muted),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(240.))
                        .gap_0p5()
                        .child(div().text_lg().font_semibold().child(headline))
                        .child(ui::muted(t!("sa_speed_intro").into_owned(), cx).text_xs()),
                )
                .child(action),
        )
        .child(disks)
        .child(sizes)
        .children(bar)
        .when(blocked && running.is_none(), |this| {
            this.child(ui::muted(t!("sa_speed_busy").into_owned(), cx).text_xs())
        })
        .into_any_element()
}

/// 五个阶段的结果卡片，以及卡片下面的几句说明。
fn results(view: &View, unit: SizeUnit, colors: Colors, cx: &App) -> Vec<AnyElement> {
    let card_for = |phase: Phase| {
        let index = Phase::ALL.iter().position(|p| *p == phase).unwrap_or(0);
        let status = match view.live {
            _ if view.done[index] => Status::Done,
            Some((current, permille, rate)) if current == phase => {
                Status::Running { permille, rate }
            }
            _ => Status::Waiting,
        };
        card(phase, status, view, unit, colors, cx)
    };
    let row = |phases: &[Phase]| {
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_3()
            .children(phases.iter().map(|phase| card_for(*phase)))
            .into_any_element()
    };

    let mut out = vec![
        row(&[Phase::SeqWrite, Phase::SeqRead]),
        row(&[Phase::RandRead, Phase::RandReadParallel, Phase::RandWrite]),
    ];
    if view.report.buffered {
        out.push(
            h_flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    Icon::new(IconName::TriangleAlert)
                        .small()
                        .text_color(colors.warning),
                )
                .child(div().text_xs().child(t!("sa_speed_buffered").into_owned()))
                .into_any_element(),
        );
    }
    if let (Some(before), None) = (view.before, view.live) {
        out.push(
            ui::muted(
                t!("sa_speed_vs", time = fmt::datetime(before.taken)).into_owned(),
                cx,
            )
            .text_xs()
            .into_any_element(),
        );
    }
    out.push(
        ui::muted(t!("sa_speed_note").into_owned(), cx)
            .text_xs()
            .into_any_element(),
    );
    out
}

/// 一个阶段一张卡片：名字、大数字（和上一次比的变化）、补充说明。
fn card(
    phase: Phase,
    status: Status,
    view: &View,
    unit: SizeUnit,
    colors: Colors,
    cx: &App,
) -> AnyElement {
    let report = &view.report;
    let (value, lines) = match status {
        Status::Waiting => ("—".to_string(), vec![t!("sa_speed_waiting").into_owned()]),
        Status::Running { rate, .. } => (headline(phase, rate, unit), Vec::new()),
        Status::Done if phase.is_sequential() => (
            headline(phase, metric(report, phase), unit),
            vec![t!("sa_speed_seq_note", size = fmt::bytes(report.size, unit)).into_owned()],
        ),
        Status::Done => {
            let measured = random(report, phase);
            (
                headline(phase, metric(report, phase), unit),
                vec![
                    rate(measured.bytes_per_sec(), unit),
                    t!(
                        "sa_speed_latency",
                        avg = micros(measured.avg_us),
                        p99 = micros(measured.p99_us)
                    )
                    .into_owned(),
                ],
            )
        }
    };
    let change = match status {
        Status::Done => view
            .before
            .and_then(|before| delta(metric(report, phase), metric(before, phase))),
        _ => None,
    };
    let bar = match status {
        Status::Running { permille, .. } => {
            Some(ProgressBar::new(("sa-speed-phase", phase as usize)).value(permille as f32 / 10.0))
        }
        _ => None,
    };

    v_flex()
        .flex_1()
        .min_w(px(220.))
        .gap_1()
        .p_4()
        .rounded(colors.radius_lg)
        .border_1()
        .border_color(colors.border)
        .when(matches!(status, Status::Running { .. }), |this| {
            this.bg(colors.hover)
        })
        .child(ui::muted(phase_title(phase), cx).text_sm())
        .child(
            h_flex()
                .items_baseline()
                .flex_wrap()
                .gap_2()
                .child(
                    div()
                        .text_2xl()
                        .font_semibold()
                        .when(matches!(status, Status::Waiting), |this| {
                            this.text_color(colors.muted)
                        })
                        .child(value),
                )
                .children(change.map(|change| {
                    div()
                        .text_xs()
                        .text_color(if change.better {
                            colors.success
                        } else {
                            colors.warning
                        })
                        .child(change.text)
                })),
        )
        .children(lines.into_iter().map(|line| ui::muted(line, cx).text_xs()))
        .children(bar)
        .into_any_element()
}

/// 这块盘最近几次的结果，新的在上面。
fn history_table(history: &[Report], unit: SizeUnit) -> AnyElement {
    let cols = vec![
        ui::Col::new(t!("sa_speed_h_time").into_owned()).width(10.0),
        ui::Col::new(t!("sa_speed_h_write").into_owned()).right(),
        ui::Col::new(t!("sa_speed_h_read").into_owned()).right(),
        ui::Col::new(t!("sa_speed_h_rand_read").into_owned()).right(),
        ui::Col::new(t!("sa_speed_h_rand_read_mt", n = bench::THREADS).into_owned()).right(),
        ui::Col::new(t!("sa_speed_h_rand_write").into_owned()).right(),
    ];
    let rows = history
        .iter()
        .rev()
        .map(|report| {
            vec![
                ui::txt(fmt::datetime(report.taken)),
                ui::txt(rate(report.seq_write, unit)),
                ui::txt(rate(report.seq_read, unit)),
                ui::txt(iops(report.rand_read.iops)),
                ui::txt(iops(report.rand_read_parallel.iops)),
                ui::txt(iops(report.rand_write.iops)),
            ]
        })
        .collect();
    ui::plain_section("sa-speed-history", t!("sa_speed_history").into_owned())
        .child(ui::table(cols, rows))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(taken: u64, seq_write: u64) -> Report {
        Report {
            taken,
            size: 1 << 30,
            seq_write,
            ..Report::default()
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pcs-speed-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn each_disk_keeps_its_own_latest_results_in_order() {
        let dir = scratch("history");
        assert!(load_from(&dir, "/").is_empty(), "nothing yet");

        for taken in 1..=KEEP as u64 + 3 {
            record_in(&dir, "/", &report(taken, taken * 100));
        }
        let list = load_from(&dir, "/");
        assert_eq!(list.len(), KEEP, "only the latest few are kept");
        assert_eq!(list.first().map(|r| r.taken), Some(4));
        assert_eq!(list.last().map(|r| r.taken), Some(KEEP as u64 + 3));

        // 另一块盘有自己的记录。
        let other = record_in(&dir, "/Volumes/USB", &report(99, 1));
        assert_eq!(other.len(), 1);
        assert_eq!(load_from(&dir, "/").len(), KEEP);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_record_file_is_treated_as_empty_and_then_replaced() {
        let dir = scratch("damaged");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(history::file_in(&dir, Path::new("/")), "{ not json").unwrap();
        assert!(load_from(&dir, "/").is_empty());
        assert_eq!(record_in(&dir, "/", &report(1, 1)).len(), 1);
        assert_eq!(load_from(&dir, "/").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn records_written_by_a_future_version_still_load() {
        let dir = scratch("future");
        std::fs::create_dir_all(&dir).unwrap();
        // 少了字段、多了字段，都不该让整份记录作废。
        std::fs::write(
            history::file_in(&dir, Path::new("/")),
            r#"[{"taken": 5, "seq_write": 7, "something_new": true}]"#,
        )
        .unwrap();
        let list = load_from(&dir, "/");
        assert_eq!((list.len(), list[0].taken, list[0].seq_write), (1, 5, 7));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changes_smaller_than_the_noise_are_not_reported() {
        assert!(delta(100, 0).is_none() && delta(0, 100).is_none());
        assert!(delta(101, 100).is_none());
        assert!(delta(98, 100).is_none());

        let faster = delta(150, 100).unwrap();
        assert_eq!((faster.text.as_str(), faster.better), ("+50%", true));
        let slower = delta(60, 100).unwrap();
        assert_eq!((slower.text.as_str(), slower.better), ("-40%", false));
    }

    #[test]
    fn numbers_are_shown_compactly() {
        assert_eq!(iops(950), "950");
        assert_eq!(iops(1_500), "1.5k");
        assert_eq!(iops(11_049), "11k");
        assert_eq!(iops(250_000), "250k");
        assert_eq!(micros(0), "<1 µs");
        assert_eq!(micros(90), "90 µs");
        assert_eq!(micros(1_270), "1.3 ms");
    }

    #[test]
    fn the_size_choices_follow_the_unit_setting() {
        let binary = size_options(SizeUnit::Binary);
        assert_eq!(binary[0], (1 << 30, "1 GiB".to_string()));
        assert_eq!(binary[2], (16 << 30, "16 GiB".to_string()));
        let decimal = size_options(SizeUnit::Decimal);
        assert_eq!(decimal[1], (4_000_000_000, "4 GB".to_string()));
    }

    #[test]
    fn a_place_is_on_the_same_volume_as_itself_and_not_as_a_missing_one() {
        let dir = scratch("volume");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(same_volume(&dir, &dir));
        assert!(same_volume(&dir, &std::env::temp_dir()));
        assert!(!same_volume(&dir, &dir.join("missing")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

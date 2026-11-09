//! 应用页：列出装了哪些应用、各占多少，选一个就能看到卸载它之后会留下什么，一并清掉。
//!
//! 怎么认出一个应用的残留、怎么保证不误伤别的应用，都在核心库的 `apps` 里；这里管后台任务、
//! 界面，以及动手之前的确认。macOS 上卸载是把应用和勾选的残留移到废纸篓（随时能放回来）；
//! Windows 上是启动应用自己的卸载程序。
//! 带搜索框、选中项和进行中的任务，所以是独立的 `Entity`。

mod icon;
mod view;

use super::storage::{bin_name, move_to_trash};
use crate::fmt;
use crate::state::{AppState, spawn_thread};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::{
    AppContext as _, Context, Entity, Image, ImageFormat, IntoElement, Render, Subscription,
    Window, div,
};
use pc_specs_core::apps::{self, App, Leftover};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 列表放了这么久，再回到这一页就重新读一遍。
const STALE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sort {
    Size,
    Name,
    /// 最久没用的在前：从来没打开过的最先。
    Unused,
}

/// 列表里的一个应用，连同后台陆续算出来的东西。
struct Row {
    key: String,
    app: App,
    size: Option<u64>,
    icon: Option<Arc<Image>>,
}

/// 选中的应用的残留。
enum Leftovers {
    /// 没有选中，或者这个系统上不找残留。
    None,
    Loading,
    Ready(Vec<Item>),
}

struct Item {
    leftover: Leftover,
    checked: bool,
}

/// 后台任务陆续送回来的结果。下标是它们在 `Listed` 里的位置。
enum Event {
    Listed(Vec<App>),
    Icon(usize, Vec<u8>),
    Size(usize, Option<u64>),
}

/// 一次移除的结果。
#[derive(Default)]
struct Removed {
    moved: usize,
    bytes: u64,
    /// 没能移走的，每项一条原因。
    failed: Vec<String>,
}

enum Failure {
    Running,
    Error(String),
}

pub struct AppsPage {
    state: Entity<AppState>,
    search: Entity<InputState>,
    /// 第一次读完之前是 `None`。
    rows: Option<Vec<Row>>,
    loaded_at: Option<Instant>,
    /// 列表和大小还在后台读、量。
    loading: bool,
    sort: Sort,
    selected: Option<String>,
    leftovers: Leftovers,
    /// 找残留的任务编号：换了选中项，旧任务的结果回来时据此丢弃。
    find_epoch: u64,
    find_cancel: Arc<AtomicBool>,
    /// 卸载正在进行：这期间不接受第二个。
    busy: bool,
    _subscriptions: Vec<Subscription>,
}

/// 一个应用在列表里的唯一标识。
fn key_of(app: &App) -> String {
    format!(
        "{}|{}",
        app.id,
        app.path
            .as_deref()
            .map(Path::to_string_lossy)
            .unwrap_or_default()
    )
}

impl AppsPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("apps_search").into_owned()));
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            state,
            search,
            rows: None,
            loaded_at: None,
            loading: false,
            sort: Sort::Size,
            selected: None,
            leftovers: Leftovers::None,
            find_epoch: 0,
            find_cancel: Arc::new(AtomicBool::new(false)),
            busy: false,
            _subscriptions: subscriptions,
        }
    }

    fn selected_row(&self) -> Option<&Row> {
        let key = self.selected.as_deref()?;
        self.rows.as_ref()?.iter().find(|row| row.key == key)
    }

    fn toast(&self, kind: NotificationType, message: String, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.toast(kind, message, cx));
    }

    /// 在后台把应用读一遍：先是列表，然后是图标，最后一个个量大小（要遍历整个应用包）。
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || !apps::SUPPORTED {
            return;
        }
        let (tx, rx) = async_channel::unbounded();
        let started = spawn_thread("pc-specs-apps", move || {
            let Some(list) = guarded("apps list", apps::list) else {
                return;
            };
            let _ = tx.send_blocking(Event::Listed(list.clone()));
            let cancel = AtomicBool::new(false);
            // 图标几毫秒一个，先送；大小要遍历整个应用包，慢慢来。
            for (index, app) in list.iter().enumerate() {
                if let Some(png) = app.path.as_deref().and_then(icon::png) {
                    let _ = tx.send_blocking(Event::Icon(index, png));
                }
            }
            for (index, app) in list.iter().enumerate() {
                let size = guarded("apps size", || apps::size_of(app, &cancel)).flatten();
                let _ = tx.send_blocking(Event::Size(index, size));
            }
        });
        // 线程起不来也记一笔时间：不然每次重画都会再试一次。
        self.loaded_at = Some(Instant::now());
        if !started {
            return;
        }
        self.loading = true;
        cx.spawn(async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                // 一下子来了好几个就一起处理，只重画一次。
                let mut events = vec![event];
                while let Ok(more) = rx.try_recv() {
                    events.push(more);
                }
                let applied = this.update(cx, |this, cx| {
                    for event in events {
                        this.apply(event);
                    }
                    cx.notify();
                });
                if applied.is_err() {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.loaded_at = Some(Instant::now());
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::Listed(list) => {
                // 重新读的时候沿用上一次的大小和图标，列表不闪；新的结果陆续来了会盖掉它们。
                let mut known: HashMap<String, Row> = self
                    .rows
                    .take()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|row| (row.key.clone(), row))
                    .collect();
                let rows: Vec<Row> = list
                    .into_iter()
                    .map(|app| {
                        let key = key_of(&app);
                        let previous = known.remove(&key);
                        Row {
                            size: previous.as_ref().and_then(|row| row.size),
                            icon: previous.and_then(|row| row.icon),
                            key,
                            app,
                        }
                    })
                    .collect();
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|key| !rows.iter().any(|row| &row.key == key))
                {
                    self.selected = None;
                    self.leftovers = Leftovers::None;
                }
                self.rows = Some(rows);
            }
            Event::Icon(index, png) => {
                if let Some(row) = self.rows.as_mut().and_then(|rows| rows.get_mut(index)) {
                    row.icon = Some(Arc::new(Image::from_bytes(ImageFormat::Png, png)));
                }
            }
            Event::Size(index, size) => {
                if let Some(row) = self.rows.as_mut().and_then(|rows| rows.get_mut(index)) {
                    // 量不出来（没登记大小）不要盖掉已经知道的。
                    row.size = size.or(row.size);
                }
            }
        }
    }

    fn select(&mut self, key: String, cx: &mut Context<Self>) {
        if self.selected.as_deref() == Some(key.as_str()) {
            return;
        }
        self.selected = Some(key);
        self.find_leftovers(cx);
        cx.notify();
    }

    /// 给选中的应用找残留。结果回来时选中项已经换了的话就丢掉。
    fn find_leftovers(&mut self, cx: &mut Context<Self>) {
        self.find_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(AtomicBool::new(false));
        self.find_cancel = cancel.clone();
        self.find_epoch += 1;
        let epoch = self.find_epoch;
        self.leftovers = Leftovers::None;
        let Some((app, need_size)) = self
            .selected_row()
            .map(|row| (row.app.clone(), row.size.is_none()))
        else {
            return;
        };
        if !apps::MOVES_TO_TRASH {
            return;
        }
        let installed: Vec<App> = self
            .rows
            .iter()
            .flatten()
            .map(|row| row.app.clone())
            .collect();
        let (tx, rx) = async_channel::bounded(1);
        let started = spawn_thread("pc-specs-apps-find", move || {
            let found = guarded("apps leftovers", || {
                apps::leftovers(&app, &installed, &cancel)
            })
            .unwrap_or_default();
            // 这个应用的大小还没轮到量：确认框里要用，顺手先量了。
            let size = need_size
                .then(|| guarded("apps size", || apps::size_of(&app, &cancel)).flatten())
                .flatten();
            let _ = tx.send_blocking((found, size));
        });
        if !started {
            self.leftovers = Leftovers::Ready(Vec::new());
            return;
        }
        self.leftovers = Leftovers::Loading;
        cx.spawn(async move |this, cx| {
            let (found, size) = rx.recv().await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                if this.find_epoch == epoch {
                    if let (Some(size), Some(key)) = (size, this.selected.clone())
                        && let Some(row) = this
                            .rows
                            .as_mut()
                            .and_then(|rows| rows.iter_mut().find(|row| row.key == key))
                    {
                        row.size.get_or_insert(size);
                    }
                    // 系统共用位置的只列出来，不勾选。
                    this.leftovers = Leftovers::Ready(
                        found
                            .into_iter()
                            .map(|leftover| Item {
                                checked: !leftover.system,
                                leftover,
                            })
                            .collect(),
                    );
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_checked(&mut self, index: usize, checked: bool, cx: &mut Context<Self>) {
        if let Leftovers::Ready(items) = &mut self.leftovers
            && let Some(item) = items.get_mut(index)
            && !item.leftover.system
        {
            item.checked = checked;
            cx.notify();
        }
    }

    fn set_all_checked(&mut self, checked: bool, cx: &mut Context<Self>) {
        if let Leftovers::Ready(items) = &mut self.leftovers {
            for item in items.iter_mut().filter(|item| !item.leftover.system) {
                item.checked = checked;
            }
            cx.notify();
        }
    }

    /// 勾选了的残留，以及它们一共多大。
    fn chosen(&self) -> (Vec<Leftover>, u64) {
        let items: Vec<Leftover> = match &self.leftovers {
            Leftovers::Ready(items) => items
                .iter()
                .filter(|item| item.checked && !item.leftover.system)
                .map(|item| item.leftover.clone())
                .collect(),
            _ => Vec::new(),
        };
        let bytes = items.iter().map(|item| item.bytes).sum();
        (items, bytes)
    }

    /// 先弹确认框，点了确定才真的去卸载。`app` 为假时只删勾选的残留，应用本身留着。
    fn ask(&mut self, remove_app: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(row) = self.selected_row() else {
            return;
        };
        let name = row.app.name.clone();
        let unit = self.state.read(cx).settings.size_unit;
        let (items, bytes) = self.chosen();
        let (count, size) = (items.len(), fmt::bytes(bytes, unit));
        let bin = bin_name();
        // 应用本身的大小还没量出来（应该很少见）就写个破折号，不要写成 0。
        let app_size = row
            .size
            .map_or_else(|| "—".to_string(), |bytes| fmt::bytes(bytes, unit));
        let all = row.size.map_or_else(
            || size.clone(),
            |app_bytes| fmt::bytes(bytes + app_bytes, unit),
        );

        let (title, description, ok) = if !apps::MOVES_TO_TRASH {
            (
                t!("apps_confirm_title", name = name.as_str()).into_owned(),
                t!("apps_confirm_desc_win", name = name.as_str()).into_owned(),
                t!("apps_uninstall").into_owned(),
            )
        } else if !remove_app {
            (
                t!("apps_confirm_files_title", name = name.as_str()).into_owned(),
                t!(
                    "apps_confirm_files_desc",
                    n = count,
                    size = size.as_str(),
                    bin = bin.as_str()
                )
                .into_owned(),
                t!("apps_remove_files_ok").into_owned(),
            )
        } else if count == 0 {
            (
                t!("apps_confirm_title", name = name.as_str()).into_owned(),
                t!(
                    "apps_confirm_desc_alone",
                    name = name.as_str(),
                    size = app_size.as_str(),
                    bin = bin.as_str()
                )
                .into_owned(),
                t!("apps_uninstall").into_owned(),
            )
        } else {
            (
                t!("apps_confirm_title", name = name.as_str()).into_owned(),
                t!(
                    "apps_confirm_desc",
                    name = name.as_str(),
                    n = count,
                    size = all.as_str(),
                    bin = bin.as_str()
                )
                .into_owned(),
                t!("apps_uninstall").into_owned(),
            )
        };
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
                    page.update(cx, |page, cx| page.uninstall(remove_app, cx));
                    true
                })
        });
    }

    fn uninstall(&mut self, remove_app: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(row) = self.selected_row() else {
            return;
        };
        let (key, name, app, app_size) = (
            row.key.clone(),
            row.app.name.clone(),
            row.app.clone(),
            row.size.unwrap_or(0),
        );
        let (items, _) = self.chosen();
        let (tx, rx) = async_channel::bounded(1);
        let started = spawn_thread("pc-specs-apps-remove", move || {
            let result = guarded("apps remove", || remove(&app, remove_app, &items))
                .unwrap_or_else(|| Err(Failure::Error("the removal crashed".into())));
            let _ = tx.send_blocking(result);
        });
        if !started {
            let message = t!("sa_error_thread").into_owned();
            self.toast(NotificationType::Error, message, cx);
            return;
        }
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = rx
                .recv()
                .await
                .unwrap_or_else(|_| Err(Failure::Error("the removal was interrupted".into())));
            let _ = this.update(cx, |this, cx| {
                this.removed(&key, &name, app_size, remove_app, result, cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn removed(
        &mut self,
        key: &str,
        name: &str,
        app_size: u64,
        remove_app: bool,
        result: Result<Removed, Failure>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        let unit = self.state.read(cx).settings.size_unit;
        let bin = bin_name();
        let (kind, message) = match result {
            Err(Failure::Running) => (
                NotificationType::Warning,
                t!("apps_running_error", name = name).into_owned(),
            ),
            Err(Failure::Error(error)) => {
                let mut message =
                    t!("apps_failed", name = name, error = error.as_str()).into_owned();
                // 系统给的原因不会提到「找管理员」，补一句。
                if error.to_lowercase().contains("permission") {
                    message.push(' ');
                    message.push_str(&t!("apps_hint_admin"));
                }
                (NotificationType::Error, message)
            }
            Ok(_) if !apps::MOVES_TO_TRASH => (
                NotificationType::Info,
                t!("apps_uninstaller_started", name = name).into_owned(),
            ),
            Ok(done) => {
                let size = fmt::bytes(done.bytes + if remove_app { app_size } else { 0 }, unit);
                let mut message = match (remove_app, done.moved) {
                    (true, 0) => t!("apps_done_alone", name = name, size = size, bin = bin),
                    (true, n) => t!("apps_done", name = name, n = n, size = size, bin = bin),
                    (false, n) => t!(
                        "apps_done_files",
                        name = name,
                        n = n,
                        size = size,
                        bin = bin
                    ),
                }
                .into_owned();
                if let Some(first) = done.failed.first() {
                    message.push(' ');
                    message.push_str(&t!(
                        "apps_done_partial",
                        failed = done.failed.len(),
                        error = first.as_str()
                    ));
                }
                let kind = if done.failed.is_empty() {
                    NotificationType::Success
                } else {
                    NotificationType::Warning
                };
                // 应用没了就把它从列表里拿掉；只删了残留就重新找一遍，剩下的才是真的剩下的。
                if remove_app {
                    if let Some(rows) = self.rows.as_mut() {
                        rows.retain(|row| row.key != key);
                    }
                    self.selected = None;
                    self.leftovers = Leftovers::None;
                } else {
                    self.find_leftovers(cx);
                }
                (kind, message)
            }
        };
        self.toast(kind, message, cx);
        cx.notify();
    }
}

/// 在后台线程里真正动手。macOS：先移应用（失败就整个停下，不碰残留），再一项项移残留，
/// 单项失败不影响其余。Windows：启动卸载程序。
fn remove(app: &App, remove_app: bool, items: &[Leftover]) -> Result<Removed, Failure> {
    if !apps::MOVES_TO_TRASH {
        return apps::start_uninstaller(app)
            .map(|()| Removed::default())
            .map_err(Failure::Error);
    }
    if remove_app {
        let path = app
            .path
            .as_deref()
            .filter(|path| safe_app(path))
            .ok_or_else(|| Failure::Error("this app is not in an applications folder".into()))?;
        if apps::is_running(app) {
            return Err(Failure::Running);
        }
        move_to_trash(path).map_err(Failure::Error)?;
    }
    let mut done = Removed::default();
    for item in items {
        if item.system || !safe_leftover(&item.path) {
            done.failed.push("not a removable location".into());
            continue;
        }
        match move_to_trash(&item.path) {
            Ok(()) => {
                done.moved += 1;
                done.bytes += item.bytes;
            }
            Err(error) => done.failed.push(error),
        }
    }
    Ok(done)
}

/// 应用包：必须是 `.app`，而且在 `/Applications` 或 `~/Applications` 里面。
/// 核心库给出的路径本来就是这样；这里再核对一遍，因为接下来是删东西。
fn safe_app(path: &Path) -> bool {
    let is_app = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
    let mut roots = vec![PathBuf::from("/Applications")];
    roots.extend(dirs::home_dir().map(|home| home.join("Applications")));
    is_app
        && roots
            .iter()
            .any(|root| path.starts_with(root) && path != root)
}

/// 残留：必须深入 `~/Library` 里某个目录的里面——不是 `~/Library` 本身，也不是 `Caches` 之类的目录本身。
fn safe_leftover(path: &Path) -> bool {
    dirs::home_dir().is_some_and(|home| {
        path.strip_prefix(home.join("Library"))
            .is_ok_and(|rest| rest.components().count() >= 2)
    })
}

/// `~` 代替用户目录，路径短一些。
fn tilde(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

impl Render for AppsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 第一次打开、或者离开了一阵再回来：重新读一遍。
        if self.loaded_at.is_none_or(|at| at.elapsed() > STALE) {
            self.refresh(cx);
        }
        guarded("render apps", || view::page(self, cx)).unwrap_or_else(|| div().into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_applications_in_the_applications_folders_may_be_moved() {
        assert!(safe_app(Path::new("/Applications/Foo.app")));
        assert!(safe_app(Path::new("/Applications/Utilities/Foo.app")));
        assert!(!safe_app(Path::new("/Applications")));
        assert!(!safe_app(Path::new("/Applications/Foo")));
        assert!(!safe_app(Path::new("/System/Applications/Foo.app")));
        assert!(!safe_app(Path::new("/Users/someone/Documents/Foo.app")));
        if let Some(home) = dirs::home_dir() {
            assert!(safe_app(&home.join("Applications/Foo.app")));
            assert!(!safe_app(&home.join("Applications")));
        }
    }

    #[test]
    fn only_things_inside_a_library_folder_may_be_moved_as_leftovers() {
        let Some(home) = dirs::home_dir() else { return };
        assert!(safe_leftover(&home.join("Library/Caches/com.foo.bar")));
        assert!(safe_leftover(
            &home.join("Library/Preferences/com.foo.bar.plist")
        ));
        assert!(!safe_leftover(&home.join("Library")));
        assert!(!safe_leftover(&home.join("Library/Caches")));
        assert!(!safe_leftover(&home.join("Documents/Library/Caches/x")));
        assert!(!safe_leftover(Path::new("/Library/Caches/com.foo.bar")));
        assert!(!safe_leftover(&home));
    }

    #[test]
    fn paths_under_the_home_folder_are_shortened() {
        let Some(home) = dirs::home_dir() else { return };
        assert_eq!(tilde(&home.join("Library/Caches/x")), "~/Library/Caches/x");
        assert_eq!(tilde(Path::new("/Library/Caches/x")), "/Library/Caches/x");
    }
}

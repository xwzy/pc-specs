//! 存储：概览（每个卷一张卡片）和空间分析（矩形树图，找出到底是什么占满了磁盘）。
//!
//! 概览只是 `AppState` 的只读视图；空间分析要记住扫描进度、下钻到了哪一层、选中了谁，
//! 所以整页是一个独立的 `Entity`，和进程页一样。

mod analyzer;
mod cleaner;
mod collector;
mod duplicates;
mod export;
mod hidden;
mod history;
mod overview;
mod search;
mod speed;
mod sunburst;
mod treemap;

use crate::settings::SizeUnit;
use crate::state::{AppState, spawn_thread};
use crate::{fmt, ui};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::{WindowExt as _, button::ButtonVariant};
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyDownEvent, ParentElement as _, PathPromptOptions, Pixels, Point, Render, SharedString,
    Styled as _, Subscription, Window, div,
};
use pc_specs_core::space::{self, Category, Kind, Node, Tree};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use treemap::{Layout, Palette, Scene};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Overview,
    Analyze,
    /// 一键清理缓存、日志这类能自己重建的东西。
    Clean,
    /// 测磁盘的读写速度。
    Speed,
}

/// 空间图的画法：同一棵树，两种看法。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MapKind {
    /// 矩形树图：一块块矩形，目录里嵌着它的子项。
    Treemap,
    /// 旋转图：同心圆环，从圆心往外一层层展开。
    Sunburst,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ListKind {
    /// 当前目录下最大的若干个文件（不管藏在多深的子目录里）。
    Largest,
    /// 当前目录的直接子项。
    Folder,
    /// 当前目录里内容完全相同的文件。
    Duplicates,
    /// 和上一次扫描相比，哪些文件夹变大了、变小了（只有扫过同一个位置才有）。
    Changes,
}

/// 要扫描的位置。
#[derive(Clone)]
struct Target {
    label: String,
    path: PathBuf,
    /// 是整块磁盘：结果页会拿扫描到的总量和卷的已用空间对比。
    volume: bool,
}

struct Running {
    target: Target,
    progress: Arc<space::Progress>,
    cancel: Arc<AtomicBool>,
    started: Instant,
    /// 起点所在卷已用的字节数，用来估算进度；取不到就是 0。
    expected: u64,
}

enum Scan {
    Idle,
    Running(Running),
    Failed { target: Target, error: String },
    Done(Box<Report>),
}

/// 树里一项的快照，信息栏、右键菜单和确认框都用它。
struct Item {
    path: Vec<u32>,
    name: String,
    kind: Kind,
    category: Category,
    size: u64,
    files: u32,
    modified: u32,
    /// 用户看得懂的完整路径（「小项目」停在它所在的目录）。
    full: PathBuf,
}

/// 列表里的一行。
struct Row {
    path: Vec<u32>,
    name: SharedString,
    /// 所在的子目录，相对当前视图；直接在视图下的为空。
    folder: SharedString,
    /// 列表里显示的大小；「变化」列表里是变化的幅度。
    size: u64,
    category: Category,
    kind: Kind,
    /// 只有「变化」列表有：变大是正数，变小是负数。
    delta: Option<i64>,
}

struct Rows {
    rev: u64,
    view: Vec<u32>,
    filter: Option<Category>,
    kind: ListKind,
    query: search::Query,
    rows: Rc<Vec<Row>>,
}

/// 排好版的空间图。
enum MapLayout {
    Tiles(Arc<Layout>),
    Rings(Arc<sunburst::Rings>),
}

impl MapLayout {
    fn contains(&self, path: &[u32]) -> bool {
        match self {
            MapLayout::Tiles(layout) => layout.find(path).is_some(),
            MapLayout::Rings(rings) => rings.find(path).is_some(),
        }
    }
}

/// 一帧要画的东西，随空间图的画法而不同。
pub enum MapScene {
    Tiles(Scene),
    Rings(sunburst::Scene),
}

struct Laid {
    rev: u64,
    view: Vec<u32>,
    levels: u8,
    map: MapKind,
    /// 画布的整数宽高。
    width: i32,
    height: i32,
    layout: MapLayout,
}

/// 一次扫描的结果，以及围绕它的浏览状态。
struct Report {
    /// 区分不同的结果：异步回来的「移到废纸篓」据此确认树还是原来那一棵。
    id: u64,
    target: Target,
    tree: Tree,
    /// 树被改过几次（移走了东西），缓存据此失效。
    rev: u64,
    /// 当前缩放到的目录，下标路径。
    view: Vec<u32>,
    selected: Option<Vec<u32>>,
    hover: Option<Vec<u32>>,
    filter: Option<Category>,
    levels: u8,
    map: MapKind,
    list: ListKind,
    list_limit: usize,
    laid: Option<Laid>,
    rows: Option<Rows>,
    /// 画布上一次在窗口里的位置，鼠标事件据此换算坐标。
    canvas: Bounds<Pixels>,
    /// 收集箱：攒着准备一起移走的项目，记的是从根往下的名字（同级删掉别的之后下标会变，名字不会）。
    collector: Vec<Vec<Box<str>>>,
    /// 起点所在卷上扫描交代不了的空间（macOS 的可清除空间、本地快照）。
    extra: hidden::Extra,
    /// 重复文件的搜索状态和结果；跟着这一次扫描走，重新扫描就清空。
    dupes: duplicates::Dupes,
    /// 和上一次扫描同一位置的对比；第一次扫描这个位置时没有。
    history: Option<history::History>,
}

/// 列表一开始显示多少行，以及每点一次「显示更多」增加多少。
const LIST_STEP: usize = 40;
/// 列表最多算多少行。
const LIST_MAX: usize = 400;
const DEFAULT_LEVELS: u8 = 3;

impl Report {
    fn new(
        id: u64,
        target: Target,
        tree: Tree,
        extra: hidden::Extra,
        history: Option<history::History>,
    ) -> Report {
        Report {
            id,
            target,
            tree,
            rev: 0,
            view: Vec::new(),
            selected: None,
            hover: None,
            filter: None,
            levels: DEFAULT_LEVELS,
            map: MapKind::Treemap,
            list: ListKind::Largest,
            list_limit: LIST_STEP,
            laid: None,
            rows: None,
            canvas: Bounds::default(),
            collector: Vec::new(),
            extra,
            dupes: duplicates::Dupes::Idle,
            history,
        }
    }

    fn view_node(&self) -> Option<&Node> {
        self.tree.root.at(&self.view)
    }

    /// 换了目录之后：正在过滤的类型在新目录里一个字节也没有，就取消过滤，免得整张图都被压暗。
    fn tidy_filter(&mut self) {
        let empty = match (self.filter, self.view_node()) {
            (Some(category), Some(view)) => view.category_size(category) == 0,
            _ => false,
        };
        if empty {
            self.filter = None;
        }
    }

    fn item(&self, path: &[u32]) -> Option<Item> {
        let node = self.tree.root.at(path)?;
        Some(Item {
            path: path.to_vec(),
            name: node.name.to_string(),
            kind: node.kind,
            category: node.category,
            size: node.size,
            files: node.files,
            modified: node.modified,
            full: user_path(self.tree.path_of(path)),
        })
    }

    /// 窗口坐标换算成相对画布左上角的坐标。
    fn local(&self, position: Point<Pixels>) -> (f32, f32) {
        (
            (position.x - self.canvas.origin.x).as_f32(),
            (position.y - self.canvas.origin.y).as_f32(),
        )
    }

    /// 鼠标（窗口坐标）下面是哪一块。
    fn path_at(&self, position: Point<Pixels>) -> Option<Vec<u32>> {
        let laid = self.laid.as_ref()?;
        let (x, y) = self.local(position);
        match &laid.layout {
            MapLayout::Tiles(layout) => Some(layout.tiles[layout.hit(x, y)?].path.clone()),
            MapLayout::Rings(rings) => Some(rings.slices[rings.hit(x, y)?].path.clone()),
        }
    }

    /// 鼠标是不是在旋转图圆心的空洞里：点一下回到上一级。
    fn in_hole(&self, position: Point<Pixels>) -> bool {
        let (x, y) = self.local(position);
        matches!(
            self.laid.as_ref().map(|laid| &laid.layout),
            Some(MapLayout::Rings(rings)) if rings.in_hole(x, y)
        )
    }

    /// 按当前画布尺寸排版（没变就复用上一次的），并组装这一帧要画的东西。
    fn scene(
        &mut self,
        bounds: Bounds<Pixels>,
        unit: SizeUnit,
        palette: Palette,
    ) -> Option<MapScene> {
        self.canvas = bounds;
        let (width, height) = (bounds.size.width.as_f32(), bounds.size.height.as_f32());
        let (iw, ih) = (width.floor() as i32, height.floor() as i32);
        let fresh = self.laid.as_ref().is_some_and(|laid| {
            laid.rev == self.rev
                && laid.levels == self.levels
                && laid.map == self.map
                && (laid.width, laid.height) == (iw, ih)
                && laid.view == self.view
        });
        if !fresh {
            let (root, view, levels) = (&self.tree.root, &self.view, self.levels);
            let layout = match self.map {
                MapKind::Treemap => {
                    MapLayout::Tiles(Arc::new(treemap::layout(root, view, width, height, levels)))
                }
                MapKind::Sunburst => MapLayout::Rings(Arc::new(sunburst::layout(
                    root, view, width, height, levels,
                ))),
            };
            self.laid = Some(Laid {
                rev: self.rev,
                view: self.view.clone(),
                levels: self.levels,
                map: self.map,
                width: iw,
                height: ih,
                layout,
            });
        }
        let rest_label: SharedString = t!("sa_rest").into_owned().into();
        Some(match &self.laid.as_ref()?.layout {
            MapLayout::Tiles(layout) => {
                let find = |path: &Option<Vec<u32>>| path.as_deref().and_then(|p| layout.find(p));
                MapScene::Tiles(Scene {
                    hover: find(&self.hover),
                    selected: find(&self.selected),
                    layout: layout.clone(),
                    filter: self.filter,
                    unit,
                    palette,
                    rest_label,
                })
            }
            MapLayout::Rings(rings) => {
                let find = |path: &Option<Vec<u32>>| path.as_deref().and_then(|p| rings.find(p));
                let view = self.view_node();
                MapScene::Rings(sunburst::Scene {
                    hover: find(&self.hover),
                    selected: find(&self.selected),
                    rings: rings.clone(),
                    filter: self.filter,
                    unit,
                    palette,
                    rest_label,
                    title: if self.view.is_empty() {
                        self.target.label.clone().into()
                    } else {
                        view.map(|node| node.name.to_string())
                            .unwrap_or_default()
                            .into()
                    },
                    total: view.map_or(0, |node| node.size),
                })
            }
        })
    }

    /// 当前列表的行（没变就复用上一次的）。设了搜索条件就是搜索结果，否则按 `list` 来。
    fn rows(&mut self, query: &search::Query) -> Rc<Vec<Row>> {
        let fresh = self.rows.as_ref().is_some_and(|rows| {
            rows.rev == self.rev
                && rows.kind == self.list
                && rows.filter == self.filter
                && rows.view == self.view
                && rows.query == *query
        });
        if !fresh {
            let rows = self.build_rows(query);
            self.rows = Some(Rows {
                rev: self.rev,
                view: self.view.clone(),
                filter: self.filter,
                kind: self.list,
                query: query.clone(),
                rows: Rc::new(rows),
            });
        }
        self.rows
            .as_ref()
            .map(|r| r.rows.clone())
            .unwrap_or_default()
    }

    fn build_rows(&self, query: &search::Query) -> Vec<Row> {
        let Some(view) = self.view_node() else {
            return Vec::new();
        };
        let absolute = |relative: &[u32]| {
            let mut path = self.view.clone();
            path.extend_from_slice(relative);
            path
        };
        // 节点所在的子目录（相对当前视图）。
        let folder_of = |relative: &[u32]| -> SharedString {
            let mut folders = Vec::new();
            let mut at = view;
            for &index in &relative[..relative.len() - 1] {
                at = &at.children[index as usize];
                folders.push(&*at.name);
            }
            folders.join("/").into()
        };
        if query.is_active() && matches!(self.list, ListKind::Largest | ListKind::Folder) {
            let matcher = query.matcher(unix_now(), self.filter);
            return view
                .search(LIST_MAX, &|node| matcher.score(node))
                .into_iter()
                .map(|(relative, node, score)| Row {
                    path: absolute(&relative),
                    name: node.name.to_string().into(),
                    folder: folder_of(&relative),
                    size: score,
                    category: node.category,
                    kind: node.kind,
                    delta: None,
                })
                .collect();
        }
        match self.list {
            ListKind::Largest => view
                .top_files(LIST_MAX, self.filter)
                .into_iter()
                .map(|(relative, node)| Row {
                    path: absolute(&relative),
                    name: node.name.to_string().into(),
                    folder: folder_of(&relative),
                    size: node.size,
                    category: node.category,
                    kind: node.kind,
                    delta: None,
                })
                .collect(),
            ListKind::Folder => {
                let mut order: Vec<usize> = (0..view.children.len())
                    .filter(|&i| {
                        let child = &view.children[i];
                        child.size > 0 && self.filter.is_none_or(|c| child.category_size(c) > 0)
                    })
                    .collect();
                order.sort_by_key(|&i| std::cmp::Reverse(view.children[i].size));
                order
                    .into_iter()
                    .take(LIST_MAX)
                    .map(|i| {
                        let child = &view.children[i];
                        Row {
                            path: absolute(&[i as u32]),
                            name: if child.kind == Kind::Rest {
                                t!("sa_rest").into_owned().into()
                            } else {
                                child.name.to_string().into()
                            },
                            folder: SharedString::default(),
                            size: match self.filter {
                                Some(c) if child.kind != Kind::File => child.category_size(c),
                                _ => child.size,
                            },
                            category: child.category,
                            kind: child.kind,
                            delta: None,
                        }
                    })
                    .collect()
            }
            ListKind::Duplicates => Vec::new(),
            ListKind::Changes => self.change_rows(),
        }
    }

    /// 「变化」列表：和上一次扫描相比，正在看的文件夹里变得最多的那些文件夹。
    fn change_rows(&self) -> Vec<Row> {
        let Some(history) = &self.history else {
            return Vec::new();
        };
        let here = self.tree.root.names(&self.view).unwrap_or_default();
        history
            .changes
            .iter()
            .filter(|change| change.names.len() > here.len() && change.names.starts_with(&here))
            .filter_map(|change| {
                // 这个文件夹现在还在树里就能点；已经删了（或缩到被裁掉了）就只显示，路径留空。
                let found = self.tree.root.locate(&change.names);
                let category = found
                    .as_ref()
                    .and_then(|path| self.tree.root.at(path))
                    .map_or(Category::Other, |node| node.category);
                let (name, folders) = change.names[here.len()..].split_last()?;
                Some(Row {
                    path: found.unwrap_or_default(),
                    name: name.to_string().into(),
                    folder: folders.join("/").into(),
                    size: change.delta().unsigned_abs(),
                    category,
                    kind: Kind::Dir,
                    delta: Some(change.delta()),
                })
            })
            .collect()
    }

    /// 摘掉一项（已经移到废纸篓），并修正指向它后面兄弟的各条下标路径。
    fn remove(&mut self, path: &[u32]) {
        let names = self.tree.root.names(path);
        if self.tree.remove(path).is_none() {
            return;
        }
        self.rev += 1;
        // 收集箱里被移走的那一项、以及它里面的项目，都不在了。
        if let Some(names) = names {
            self.collector.retain(|staged| !staged.starts_with(&names));
            self.dupes.forget(&names);
        }
        if !shift_after_removal(&mut self.view, path) {
            // 正在看的目录被删了：退回到它的上一级。
            self.view = path[..path.len() - 1].to_vec();
        }
        self.tidy_filter();
        if let Some(selected) = self.selected.as_mut()
            && !shift_after_removal(selected, path)
        {
            self.selected = None;
        }
        self.hover = None;
    }
}

/// 最近扫描过的文件夹最多记几个。
const RECENT_MAX: usize = 6;

/// 把 `path` 放到最近列表的最前面（已有的挪到前面），超出的丢掉最旧的。
fn push_recent(list: &mut Vec<String>, path: &Path) {
    let path = path.display().to_string();
    list.retain(|old| *old != path);
    list.insert(0, path);
    list.truncate(RECENT_MAX);
}

/// 现在的 Unix 秒。
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// 树里摘掉 `removed` 之后，修正 `path`：同一个父目录下排在被摘掉那项后面的兄弟，下标都前移了一位。
/// 返回 `false` 表示 `path` 本来就在被摘掉的那棵子树里（调用方自己决定怎么办）。
fn shift_after_removal(path: &mut [u32], removed: &[u32]) -> bool {
    let Some((&index, parent)) = removed.split_last() else {
        return false;
    };
    if path.len() <= parent.len() || path[..parent.len()] != *parent {
        return true;
    }
    let slot = &mut path[parent.len()];
    match (*slot).cmp(&index) {
        std::cmp::Ordering::Equal => false,
        std::cmp::Ordering::Greater => {
            *slot -= 1;
            true
        }
        std::cmp::Ordering::Less => true,
    }
}

/// 扫描一个卷的起点。macOS 的 `/` 是只读的系统卷，用户的数据在 `/System/Volumes/Data`，
/// 扫它才是用户眼里的「整块磁盘」。
fn scan_root(mount: &str) -> PathBuf {
    #[cfg(target_os = "macos")]
    if mount == "/" {
        let data = Path::new("/System/Volumes/Data");
        if data.is_dir() {
            return data.to_path_buf();
        }
    }
    PathBuf::from(mount)
}

/// 界面上显示、也是真正去操作的路径：macOS 上去掉 `/System/Volumes/Data` 前缀，
/// 用户看到的是熟悉的 `/Users/…`。
fn user_path(path: PathBuf) -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Ok(rest) = path.strip_prefix("/System/Volumes/Data") {
        return Path::new("/").join(rest);
    }
    path
}

/// 不允许从这里删的路径：磁盘根、各卷顶层的系统目录、用户的个人文件夹本身。
/// 别的程序、系统本身要用它们，删掉的后果远不是「腾出空间」。
fn is_protected(path: &Path) -> bool {
    if dirs::home_dir().is_some_and(|home| home == path) {
        return true;
    }
    // 根之下只隔一层（`/usr`、`C:\Windows`、`C:\Users`）。
    path.components()
        .filter(|c| {
            !matches!(
                c,
                std::path::Component::Prefix(_) | std::path::Component::RootDir
            )
        })
        .count()
        <= 1
}

fn volume_target(name: &str, mount: &str) -> Target {
    Target {
        label: if name.trim().is_empty() {
            mount.to_string()
        } else {
            name.to_string()
        },
        path: scan_root(mount),
        volume: true,
    }
}

fn folder_target(path: PathBuf) -> Target {
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| path.display().to_string());
    Target {
        label,
        path,
        volume: false,
    }
}

/// 回收站在各个系统上的叫法。
pub(super) fn bin_name() -> String {
    if cfg!(windows) {
        t!("sa_bin_recycle").into_owned()
    } else {
        t!("sa_bin_trash").into_owned()
    }
}

pub struct StoragePage {
    app: Entity<AppState>,
    mode: Mode,
    scan: Scan,
    /// 扫描任务的编号：被取消或被新扫描顶替的旧任务，结果回来时据此丢弃。
    epoch: u64,
    next_report: u64,
    /// 重复文件搜索的编号：被取消的旧搜索结果回来时据此丢弃。
    next_search: u64,
    /// 「移到废纸篓」正在进行，期间不再接受第二个（树上的下标会错位）。
    trashing: bool,
    /// 「清理」页：各类能清多少、勾选了哪些、清理进行到哪。
    cleaner: cleaner::Cleaner,
    /// 「速度测试」页：选了哪块盘、测到哪、各块盘的历史记录。
    speed: speed::Speed,
    content_width: f32,
    /// 各个卷（按挂载点）上可清除的空间，概览卡片上显示；只有 macOS 会有。
    purgeable: HashMap<String, u64>,
    purgeable_at: Option<Instant>,
    /// 结果页列表上的搜索与筛选。它跟着页面走而不是跟着某一次扫描，输入框里的字才不会和结果对不上。
    query: search::Query,
    search: Entity<InputState>,
    /// 回到选位置的页面之后，输入框要清空；清空需要窗口，留到下一次渲染里做。
    reset_search: bool,
    /// 键盘快捷键要的焦点：点过树图或列表之后，焦点就落在整页上（搜索框里打字不受影响）。
    focus: FocusHandle,
    /// 正在显示的 Quick Look 窗口，换一个文件预览时先关掉旧的。
    #[cfg(target_os = "macos")]
    preview: Option<std::process::Child>,
    _subscriptions: Vec<Subscription>,
}

impl StoragePage {
    pub fn new(app: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("sa_search").into_owned()));
        let subscriptions = vec![
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.query.text = input.read(cx).value().to_string();
                    this.reset_list();
                    cx.notify();
                }
            }),
        ];
        Self {
            app,
            mode: Mode::Overview,
            scan: Scan::Idle,
            epoch: 0,
            next_report: 0,
            next_search: 0,
            trashing: false,
            cleaner: cleaner::Cleaner::default(),
            speed: speed::Speed::default(),
            content_width: 0.0,
            purgeable: HashMap::new(),
            purgeable_at: None,
            query: search::Query::default(),
            search,
            reset_search: false,
            focus: cx.focus_handle(),
            #[cfg(target_os = "macos")]
            preview: None,
            _subscriptions: subscriptions,
        }
    }

    /// 页面正文区的可用宽度，卡片网格据此决定列数。
    pub fn set_content_width(&mut self, width: f32) {
        self.content_width = width;
    }

    fn report(&self) -> Option<&Report> {
        match &self.scan {
            Scan::Done(report) => Some(report),
            _ => None,
        }
    }

    fn report_mut(&mut self) -> Option<&mut Report> {
        match &mut self.scan {
            Scan::Done(report) => Some(report),
            _ => None,
        }
    }

    /// 记下最近扫描过的文件夹，下次在「选位置」页面上一点就能再扫。
    fn remember(&mut self, path: &Path, cx: &mut Context<Self>) {
        let path = path.to_path_buf();
        self.app.update(cx, |state, cx| {
            state.update_settings(|s| push_recent(&mut s.storage_recent, &path), cx)
        });
    }

    /// 隔一阵子（后台）问一遍各个卷的可清除空间：它要问系统，不适合每次重画都问。
    fn refresh_purgeable(&mut self, cx: &mut Context<Self>) {
        if !cfg!(target_os = "macos")
            || self
                .purgeable_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(30))
        {
            return;
        }
        self.purgeable_at = Some(Instant::now());
        let mounts: Vec<String> = self
            .app
            .read(cx)
            .snapshot
            .storages
            .iter()
            .filter_map(|disk| disk.mount_point.clone())
            .filter(|mount| !mount.is_empty())
            .collect();
        let task = cx.background_executor().spawn(async move {
            mounts
                .into_iter()
                .filter_map(|mount| {
                    let bytes = hidden::purgeable(&scan_root(&mount))?;
                    Some((mount, bytes))
                })
                .collect::<HashMap<_, _>>()
        });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| {
                this.purgeable = found;
                cx.notify();
            });
        })
        .detach();
    }

    fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            match mode {
                Mode::Clean => self.refresh_cleanup(cx),
                Mode::Speed => self.refresh_speed(cx),
                Mode::Overview | Mode::Analyze => {}
            }
            cx.notify();
        }
    }

    /// 换掉扫描状态。旧的树可能有几十万个节点，释放放到后台线程，免得界面卡一下。
    fn replace_scan(&mut self, scan: Scan) {
        if let Scan::Done(report) = std::mem::replace(&mut self.scan, scan) {
            // 只把树交给后台线程；报告里其余的缓存很小，留在这里释放就行。
            let Report { tree, .. } = *report;
            std::thread::spawn(move || drop(tree));
        }
    }

    // ---- 扫描 -------------------------------------------------------------

    fn start(&mut self, target: Target, cx: &mut Context<Self>) {
        if matches!(self.scan, Scan::Running(_)) {
            return;
        }
        self.mode = Mode::Analyze;
        if !target.volume {
            self.remember(&target.path, cx);
        }
        self.epoch += 1;
        let epoch = self.epoch;
        let progress = Arc::new(space::Progress::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = async_channel::bounded(1);
        let started = spawn_thread("pc-specs-space", {
            let (root, progress, cancel) = (target.path.clone(), progress.clone(), cancel.clone());
            let volume = target.volume;
            move || {
                let result = guarded("space scan", || {
                    space::scan(&root, &progress, &cancel).map(|tree| {
                        // 整块磁盘才有「看不见的空间」可说；顺手问一下，几十毫秒的事。
                        let extra = if volume {
                            hidden::probe(&root)
                        } else {
                            hidden::Extra::default()
                        };
                        // 存下这次各文件夹的大小，同时和上一次比出「什么变了」。
                        let history =
                            history::record(&root, &space::Snapshot::of(&tree, unix_now()));
                        (tree, extra, history)
                    })
                })
                .unwrap_or_else(|| {
                    Err(space::Error::Io(std::io::Error::other("the scan crashed")))
                });
                let _ = tx.send_blocking(result);
            }
        });
        if !started {
            let error = t!("sa_error_thread").into_owned();
            self.replace_scan(Scan::Failed { target, error });
            cx.notify();
            return;
        }
        let expected = space::volume_of(&target.path).map_or(0, |volume| volume.used);
        let label = target.clone();
        self.replace_scan(Scan::Running(Running {
            target,
            progress,
            cancel,
            started: Instant::now(),
            expected,
        }));

        cx.spawn(async move |this, cx| {
            let Ok(result) = rx.recv().await else { return };
            let _ = this.update(cx, |this, cx| this.finished(epoch, label, result, cx));
        })
        .detach();
        // 扫描期间每隔一会儿重画一次，让计数动起来。
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let running = this.update(cx, |this, cx| {
                    let running = this.epoch == epoch && matches!(this.scan, Scan::Running(_));
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

    fn finished(
        &mut self,
        epoch: u64,
        target: Target,
        result: Result<(Tree, hidden::Extra, Option<history::History>), space::Error>,
        cx: &mut Context<Self>,
    ) {
        if self.epoch != epoch || !matches!(self.scan, Scan::Running(_)) {
            return;
        }
        match result {
            Ok((tree, extra, history)) => {
                self.next_report += 1;
                let report = Report::new(self.next_report, target, tree, extra, history);
                self.replace_scan(Scan::Done(Box::new(report)));
            }
            Err(space::Error::Cancelled) => self.replace_scan(Scan::Idle),
            Err(error) => {
                let error = match error {
                    space::Error::NotADirectory => t!("sa_error_not_dir").into_owned(),
                    other => other.to_string(),
                };
                self.replace_scan(Scan::Failed { target, error });
            }
        }
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Scan::Running(run) = &self.scan {
            run.cancel.store(true, Ordering::Relaxed);
            // 旧任务的结果回来时会因为编号对不上被丢弃。
            self.epoch += 1;
            self.replace_scan(Scan::Idle);
            cx.notify();
        }
    }

    /// 回到「选择要分析的位置」。
    fn close(&mut self, cx: &mut Context<Self>) {
        self.replace_scan(Scan::Idle);
        self.query = search::Query::default();
        self.reset_search = true;
        cx.notify();
    }

    fn rescan(&mut self, cx: &mut Context<Self>) {
        let target = match &self.scan {
            Scan::Done(report) => report.target.clone(),
            Scan::Failed { target, .. } => target.clone(),
            _ => return,
        };
        self.replace_scan(Scan::Idle);
        self.start(target, cx);
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.start(folder_target(path), cx));
        })
        .detach();
    }

    // ---- 浏览 -------------------------------------------------------------

    fn is_dir_at(&self, path: &[u32]) -> bool {
        self.report()
            .and_then(|r| r.tree.root.at(path))
            .is_some_and(Node::is_dir)
    }

    /// 钻进一个目录。
    fn zoom(&mut self, path: Vec<u32>, cx: &mut Context<Self>) {
        if !self.is_dir_at(&path) {
            return;
        }
        if let Some(report) = self.report_mut() {
            report.view = path;
            report.selected = None;
            report.hover = None;
            report.list_limit = LIST_STEP;
            report.tidy_filter();
            cx.notify();
        }
    }

    /// 回到上一级，并选中刚才待的那个目录，方便看出自己是从哪里出来的。
    fn up(&mut self, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && !report.view.is_empty()
        {
            let from = report.view.clone();
            report.view.pop();
            report.selected = Some(from);
            report.hover = None;
            report.list_limit = LIST_STEP;
            report.tidy_filter();
            cx.notify();
        }
    }

    /// 面包屑：回到第 `depth` 级（0 是扫描的起点）。
    fn crumb(&mut self, depth: usize, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && depth < report.view.len()
        {
            report.view.truncate(depth);
            report.selected = None;
            report.hover = None;
            report.list_limit = LIST_STEP;
            report.tidy_filter();
            cx.notify();
        }
    }

    fn select(&mut self, path: Option<Vec<u32>>, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && report.selected != path
        {
            report.selected = path;
            cx.notify();
        }
    }

    /// 选中某一项，并让它出现在树图里：藏在折叠的子目录里的文件，就把视图切到它所在的目录。
    fn focus(&mut self, path: Vec<u32>, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut() {
            let visible = report
                .laid
                .as_ref()
                .is_some_and(|laid| laid.layout.contains(&path));
            if !visible && path.len() > 1 {
                let parent = path[..path.len() - 1].to_vec();
                if report.tree.root.at(&parent).is_some_and(Node::is_dir) {
                    report.view = parent;
                    report.tidy_filter();
                }
            }
            report.selected = Some(path);
            report.hover = None;
            cx.notify();
        }
    }

    fn set_filter(&mut self, filter: Option<Category>, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut() {
            report.filter = if report.filter == filter {
                None
            } else {
                filter
            };
            report.list_limit = LIST_STEP;
            cx.notify();
        }
    }

    /// 搜索条件变了：列表回到第一页。
    fn reset_list(&mut self) {
        if let Some(report) = self.report_mut() {
            report.list_limit = LIST_STEP;
        }
    }

    fn set_min_size(&mut self, bytes: u64, cx: &mut Context<Self>) {
        self.query.min_size = bytes;
        self.reset_list();
        cx.notify();
    }

    fn set_older_than(&mut self, days: u32, cx: &mut Context<Self>) {
        self.query.older_days = days;
        self.reset_list();
        cx.notify();
    }

    /// 清掉所有搜索条件，回到平常的列表。
    fn clear_query(&mut self, cx: &mut Context<Self>) {
        self.query = search::Query::default();
        self.reset_search = true;
        self.reset_list();
        cx.notify();
    }

    fn set_levels(&mut self, levels: u8, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && report.levels != levels
        {
            report.levels = levels;
            cx.notify();
        }
    }

    fn set_map(&mut self, map: MapKind, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && report.map != map
        {
            report.map = map;
            cx.notify();
        }
    }

    fn set_list(&mut self, list: ListKind, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && report.list != list
        {
            report.list = list;
            report.list_limit = LIST_STEP;
            cx.notify();
        }
    }

    fn show_more(&mut self, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut() {
            report.list_limit += LIST_STEP;
            cx.notify();
        }
    }

    // ---- 鼠标 -------------------------------------------------------------

    fn hover_at(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut() {
            let hit = report.path_at(position);
            if report.hover != hit {
                report.hover = hit;
                cx.notify();
            }
        }
    }

    fn leave(&mut self, cx: &mut Context<Self>) {
        if let Some(report) = self.report_mut()
            && report.hover.take().is_some()
        {
            cx.notify();
        }
    }

    /// 单击选中；双击目录钻进去；右键只选中（菜单由 `ContextMenu` 自己弹出，它要的就是当前选中项）。
    fn press(
        &mut self,
        position: Point<Pixels>,
        clicks: usize,
        secondary: bool,
        cx: &mut Context<Self>,
    ) {
        let hit = self.report().and_then(|report| report.path_at(position));
        // 旋转图圆心的空洞：单击回到上一级。
        if hit.is_none()
            && clicks == 1
            && !secondary
            && self.report().is_some_and(|report| report.in_hole(position))
        {
            self.up(cx);
            return;
        }
        if clicks >= 2 && !secondary {
            if let Some(path) = hit {
                self.zoom(path, cx);
            }
            return;
        }
        self.select(hit, cx);
    }

    // ---- 对选中项的操作 ---------------------------------------------------

    fn toast(&self, kind: NotificationType, message: String, cx: &mut Context<Self>) {
        self.app
            .update(cx, |state, cx| state.toast(kind, message, cx));
    }

    fn reveal(&mut self, path: &[u32], cx: &mut Context<Self>) {
        if let Some(item) = self.report().and_then(|r| r.item(path)) {
            cx.reveal_path(&item.full);
        }
    }

    /// 用系统默认的程序打开（文件夹就在访达 / 资源管理器里打开）。
    fn open_item(&mut self, path: &[u32], cx: &mut Context<Self>) {
        if let Some(item) = self.report().and_then(|r| r.item(path)) {
            cx.open_with_system(&item.full);
        }
    }

    /// 快速预览一个文件：macOS 用 Quick Look，别的系统没有同类的东西，就直接打开。
    fn preview(&mut self, path: &[u32], cx: &mut Context<Self>) {
        let Some(item) = self.report().and_then(|r| r.item(path)) else {
            return;
        };
        if item.kind != Kind::File {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            use std::process::{Command, Stdio};
            if let Some(mut old) = self.preview.take() {
                let _ = old.kill();
                let _ = old.wait();
            }
            self.preview = Command::new("qlmanage")
                .arg("-p")
                .arg(&item.full)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok();
        }
        #[cfg(not(target_os = "macos"))]
        cx.open_with_system(&item.full);
        #[cfg(target_os = "macos")]
        let _ = cx;
    }

    /// 键盘：空格预览，回车进入文件夹 / 打开文件，退格回上一级，Delete（或 ⌘⌫）放进收集箱，Esc 取消选中。
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // 只在焦点就在整页上（点过树图或列表）时响应；在搜索框里打字时焦点在输入框上，不受影响。
        if !self.focus.is_focused(window) || self.report().is_none() {
            return;
        }
        let selected = self.report().and_then(|report| report.selected.clone());
        let command = event.keystroke.modifiers.secondary(cx);
        match (event.keystroke.key.as_str(), command) {
            ("space", false) => {
                if let Some(path) = selected {
                    self.preview(&path, cx);
                }
            }
            ("enter", false) => {
                if let Some(path) = selected {
                    if self.is_dir_at(&path) {
                        self.zoom(path, cx);
                    } else {
                        self.open_item(&path, cx);
                    }
                }
            }
            ("backspace", false) => self.up(cx),
            ("backspace", true) | ("delete", _) => {
                if let Some(path) = selected {
                    self.toggle_collect(path, cx);
                }
            }
            ("escape", _) => self.select(None, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn copy_path(&mut self, path: &[u32], cx: &mut Context<Self>) {
        if let Some(item) = self.report().and_then(|r| r.item(path)) {
            let text = item.full.display().to_string();
            self.app
                .update(cx, |state, cx| state.copy_to_clipboard(text, cx));
        }
    }

    /// 先弹确认框，点了确定才真的去移。
    fn ask_trash(&mut self, path: Vec<u32>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.report().and_then(|r| r.item(&path)) else {
            return;
        };
        if item.kind == Kind::Rest || self.trashing {
            return;
        }
        let bin = bin_name();
        if is_protected(&item.full) {
            let message = t!("sa_trash_protected", name = item.name).into_owned();
            self.toast(NotificationType::Warning, message, cx);
            return;
        }
        let unit = self.app.read(cx).settings.size_unit;
        let size = fmt::bytes(item.size, unit);
        let title = t!("sa_trash_title", bin = bin).into_owned();
        let description = if item.kind == Kind::Dir {
            t!(
                "sa_trash_desc_dir",
                name = item.name,
                size = size,
                files = analyzer::count(u64::from(item.files)),
                bin = bin
            )
        } else {
            t!(
                "sa_trash_desc_file",
                name = item.name,
                size = size,
                bin = bin
            )
        }
        .into_owned();
        let ok = t!("sa_trash_ok", bin = bin).into_owned();
        let cancel = t!("sa_cancel").into_owned();
        let page = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (page, path) = (page.clone(), path.clone());
            alert
                .confirm()
                .title(title.clone())
                .description(description.clone())
                .ok_text(ok.clone())
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(cancel.clone())
                .on_ok(move |_, _, cx| {
                    let path = path.clone();
                    page.update(cx, |page, cx| page.trash(path, cx));
                    true
                })
        });
    }

    fn trash(&mut self, path: Vec<u32>, cx: &mut Context<Self>) {
        let Some((id, item)) = self
            .report()
            .and_then(|r| r.item(&path).map(|item| (r.id, item)))
        else {
            return;
        };
        if self.trashing {
            return;
        }
        self.trashing = true;
        let target = item.full.clone();
        let task = cx
            .background_executor()
            .spawn(async move { move_to_trash(&target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| this.trashed(id, path, item, result, cx));
        })
        .detach();
    }

    fn trashed(
        &mut self,
        id: u64,
        path: Vec<u32>,
        item: Item,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        self.trashing = false;
        let bin = bin_name();
        match result {
            Ok(()) => {
                // 等的这段时间里用户可能已经重新扫描过：树不是原来那棵了就不去动它。
                if let Some(report) = self.report_mut()
                    && report.id == id
                {
                    report.remove(&path);
                }
                let unit = self.app.read(cx).settings.size_unit;
                let message = t!(
                    "sa_trash_done",
                    name = item.name,
                    bin = bin,
                    size = fmt::bytes(item.size, unit)
                )
                .into_owned();
                self.toast(NotificationType::Success, message, cx);
            }
            Err(error) => {
                let message = t!(
                    "sa_trash_failed",
                    name = item.name,
                    bin = bin,
                    error = error
                )
                .into_owned();
                self.toast(NotificationType::Error, message, cx);
            }
        }
        cx.notify();
    }
}

/// 放进废纸篓 / 回收站。macOS 上直接调系统的接口：默认的做法是让访达来删，
/// 第一次会弹出「想要控制访达」的授权，换成系统接口就不用问用户要任何权限。
pub(super) fn move_to_trash(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos as _};
        let mut context = trash::TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context.delete(path).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        trash::delete(path).map_err(|e| e.to_string())
    }
}

impl Render for StoragePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.reset_search) {
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        let page = cx.entity();
        let switch = ui::segmented(
            "storage-mode",
            vec![
                (Mode::Overview, t!("sa_mode_overview").into_owned()),
                (Mode::Analyze, t!("sa_mode_analyze").into_owned()),
                (Mode::Clean, t!("sa_mode_clean").into_owned()),
                (Mode::Speed, t!("sa_mode_speed").into_owned()),
            ],
            self.mode,
            move |mode, cx| page.update(cx, |page, cx| page.set_mode(mode, cx)),
        )
        .into_any_element();

        // 绘制是系统回调直接驱动的，里面 panic 会带走整个进程；
        // 这一页画不出来就让正文空着，模式开关还在，切一下就能重画（和其它页面一样）。
        let (subtitle, mut actions, body) = guarded("render storage", || match self.mode {
            Mode::Overview => {
                self.refresh_purgeable(cx);
                let (subtitle, body) = overview::body(self, cx);
                (subtitle, Vec::new(), body)
            }
            Mode::Analyze => {
                let (actions, body) = analyzer::body(self, window, cx);
                (None, actions, body)
            }
            Mode::Clean => {
                let (actions, body) = cleaner::body(self, cx);
                (None, actions, body)
            }
            Mode::Speed => {
                let (actions, body) = speed::body(self, cx);
                (None, actions, body)
            }
        })
        .unwrap_or_default();
        actions.push(switch);
        let page = ui::page(
            "page-storage",
            t!("nav_storage").into_owned(),
            subtitle,
            actions,
            body,
            cx,
        );
        div()
            .size_full()
            .min_h_0()
            .track_focus(&self.focus)
            .on_key_down(
                cx.listener(|this, event: &KeyDownEvent, window, cx| this.key(event, window, cx)),
            )
            .child(page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_an_item_shifts_later_siblings_only() {
        // 摘掉 [2, 1]（目录 2 下的第 1 项）。
        let removed = [2, 1];
        let check = |mut path: Vec<u32>| (shift_after_removal(&mut path, &removed), path);
        assert_eq!(
            check(vec![2, 0]),
            (true, vec![2, 0]),
            "earlier sibling stays"
        );
        assert_eq!(
            check(vec![2, 2]),
            (true, vec![2, 1]),
            "later sibling moves up"
        );
        assert_eq!(
            check(vec![2, 5, 3]),
            (true, vec![2, 4, 3]),
            "so does its subtree"
        );
        assert_eq!(
            check(vec![2, 1]),
            (false, vec![2, 1]),
            "the item itself is gone"
        );
        assert_eq!(
            check(vec![2, 1, 7]),
            (false, vec![2, 1, 7]),
            "and everything inside it"
        );
        assert_eq!(
            check(vec![3, 9]),
            (true, vec![3, 9]),
            "other directories are untouched"
        );
        assert_eq!(check(vec![2]), (true, vec![2]), "an ancestor is untouched");
        assert_eq!(check(vec![]), (true, vec![]), "so is the root");
        // 顶层的一项。
        let mut path = vec![4, 1];
        assert!(shift_after_removal(&mut path, &[1]));
        assert_eq!(path, vec![3, 1]);
    }

    #[test]
    fn top_level_and_home_paths_are_protected() {
        assert!(is_protected(Path::new("/")));
        assert!(is_protected(Path::new("/usr")));
        assert!(is_protected(Path::new("/Users")));
        assert!(!is_protected(Path::new("/Users/someone/Downloads/big.iso")));
        assert!(!is_protected(Path::new("/var/log/system.log")));
        if let Some(home) = dirs::home_dir() {
            assert!(is_protected(&home));
            assert!(!is_protected(&home.join("Downloads")));
        }
    }

    #[test]
    fn recent_folders_keep_the_latest_first_without_repeats() {
        let mut list = Vec::new();
        for name in ["a", "b", "c", "a"] {
            push_recent(&mut list, Path::new(name));
        }
        assert_eq!(list, ["a", "c", "b"]);
        for i in 0..10 {
            push_recent(&mut list, Path::new(&format!("dir{i}")));
        }
        assert_eq!(list.len(), RECENT_MAX);
        assert_eq!(list[0], "dir9");
    }

    #[test]
    fn folder_targets_are_named_after_the_folder() {
        assert_eq!(
            folder_target(PathBuf::from("/a/b/Projects")).label,
            "Projects"
        );
        assert!(!folder_target(PathBuf::from("/a/b/Projects")).volume);
        assert!(volume_target("", "/mnt/data").volume);
        assert_eq!(volume_target("", "/mnt/data").label, "/mnt/data");
        assert_eq!(volume_target("Data", "/mnt/data").label, "Data");
    }
}

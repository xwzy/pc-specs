//! 存储空间分析：把一个目录（或整块磁盘）扫成一棵按大小裁剪过的树，界面把它画成矩形树图。
//!
//! 只读文件的元数据，不碰内容，也不跟随符号链接。几个取舍：
//!
//! - 大小是「实际占用」：Unix 上取 `st_blocks`，Windows 上按 4 KiB 簇取整，
//!   和访达 / 资源管理器里的「占用空间」同口径；稀疏文件、未下载的云端占位文件不会虚胖。
//!   硬链接只算一次；不会跨进别的卷、网络盘（它们的设备号不同）。
//! - 整块磁盘动辄上千万个文件，全存下来要几个 G。所以边扫边裁：每个目录只留最大的若干项，
//!   其余并成一个「小项目」；留下的节点总数超过预算时，再自动抬高「值得保留」的门槛。
//!   被裁掉的部分仍然计入各级目录的大小和类型统计，所以总数一分不少，只是细节少了。

use parking_lot::Mutex;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant, UNIX_EPOCH};

/// 文件类型的个数。
pub const CATEGORIES: usize = 8;

/// 各类型的字节数，下标是 [`Category::index`]。
pub type Cats = [u64; CATEGORIES];

/// 目录最深钻到多少层，防止绑定挂载之类造成的环。
const MAX_DEPTH: usize = 256;

/// 一个目录里的条目不少于这么多，就分成块并行去读元数据（单个目录里有几十万个文件并不少见）。
const PARALLEL_FROM: usize = 2048;
const CHUNK: usize = 1024;

/// 小于这个大小的目录扫完后不再记各类型的明细，省内存；
/// 它们的类型只记「占空间最多的那一种」，往上汇总时仍然用的是精确值。
const DETAIL_FROM: u64 = 1 << 20;

/// 硬链接去重表分成这么多片，扫描线程不会都挤在一把锁上。
#[cfg(unix)]
const LINK_SHARDS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Video,
    Audio,
    Image,
    Document,
    /// 压缩包与磁盘镜像。
    Archive,
    /// 可执行文件、安装包、动态库。
    Program,
    Code,
    Other,
}

impl Category {
    pub const ALL: [Category; CATEGORIES] = [
        Category::Video,
        Category::Audio,
        Category::Image,
        Category::Document,
        Category::Archive,
        Category::Program,
        Category::Code,
        Category::Other,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// 按扩展名归类；没有已知扩展名、但带可执行权限的算程序（Unix 上的二进制通常没有扩展名）。
    pub fn of_file(name: &str, executable: bool) -> Category {
        let ext = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => ext,
            _ => "",
        };
        // 扩展名都很短：在栈上转成小写，免得每个文件分配一次。
        let mut buf = [0u8; 12];
        let lower = if ext.is_ascii() && ext.len() <= buf.len() {
            for (slot, byte) in buf.iter_mut().zip(ext.bytes()) {
                *slot = byte.to_ascii_lowercase();
            }
            std::str::from_utf8(&buf[..ext.len()]).unwrap_or("")
        } else {
            ""
        };
        match lower {
            "mp4" | "m4v" | "mov" | "mkv" | "avi" | "wmv" | "flv" | "webm" | "mpg" | "mpeg"
            | "m2ts" | "mts" | "vob" | "3gp" | "rmvb" | "ogv" => Category::Video,
            "mp3" | "m4a" | "aac" | "flac" | "wav" | "aiff" | "aif" | "ogg" | "oga" | "opus"
            | "wma" | "alac" | "mid" | "midi" | "ape" => Category::Audio,
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "tif" | "tiff" | "webp" | "heic" | "heif"
            | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "psd" | "ai" | "svg" | "ico"
            | "icns" | "xcf" => Category::Image,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp"
            | "pages" | "numbers" | "key" | "txt" | "md" | "rtf" | "epub" | "mobi" | "csv"
            | "tex" => Category::Document,
            "zip" | "rar" | "7z" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "lz4" | "cab"
            | "iso" | "dmg" | "img" | "vmdk" | "vdi" | "vhd" | "vhdx" | "qcow2" => {
                Category::Archive
            }
            "exe" | "dll" | "so" | "dylib" | "app" | "msi" | "pkg" | "deb" | "rpm" | "apk"
            | "ipa" | "appimage" | "bin" | "o" | "a" | "lib" | "sys" | "jar" | "class" | "wasm" => {
                Category::Program
            }
            "rs" | "go" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "java" | "kt"
            | "swift" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "rb" | "php" | "sh" | "bash"
            | "zsh" | "ps1" | "lua" | "pl" | "r" | "scala" | "dart" | "vue" | "svelte" | "html"
            | "css" | "scss" | "less" | "json" | "yaml" | "yml" | "toml" | "xml" | "sql"
            | "proto" | "gradle" => Category::Code,
            _ if executable => Category::Program,
            _ => Category::Other,
        }
    }
}

/// 占空间最多的类型；全是 0 时算「其它」。
fn dominant(cats: &Cats) -> Category {
    let mut best = Category::Other;
    let mut most = 0;
    for category in Category::ALL {
        if cats[category.index()] > most {
            most = cats[category.index()];
            best = category;
        }
    }
    best
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    /// 被裁掉的小项目合并成的一块：没有名字，只有大小、数量和类型分布。
    Rest,
}

#[derive(Debug)]
pub struct Node {
    /// 文件 / 目录名（根节点是扫描时给的路径名）；「小项目」为空。
    pub name: Box<str>,
    pub kind: Kind,
    /// 文件自己的类型；目录和「小项目」是占空间最多的类型。
    pub category: Category,
    /// 实际占用的字节数（目录是整棵子树的总和）。
    pub size: u64,
    /// 里面有多少个文件（文件自己是 1）。
    pub files: u32,
    /// 最后修改时间，Unix 秒；0 表示不知道。目录不记。
    pub modified: u32,
    /// 这棵子树里的节点个数（含自己），用来掌握内存。
    count: u32,
    /// 目录与「小项目」里各类型各占多少字节；文件没有。
    pub cats: Option<Box<Cats>>,
    /// 子节点，扫描出来时按大小从大到小排；之后删掉一项也不会重排，所以按大小排要自己排。
    pub children: Vec<Node>,
}

/// 取「最大的 N 个」时堆里的一项：按分数比，分数相同时下标路径小的算大，结果才稳定。
struct Hit<'a>(u64, Vec<u32>, &'a Node);

impl PartialEq for Hit<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Hit<'_> {}

impl PartialOrd for Hit<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hit<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0).then_with(|| other.1.cmp(&self.1))
    }
}

impl Node {
    fn file(name: &str, size: u64, modified: u32, category: Category) -> Node {
        Node {
            name: name.into(),
            kind: Kind::File,
            category,
            size,
            files: 1,
            modified,
            count: 1,
            cats: None,
            children: Vec::new(),
        }
    }

    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }

    /// 这个节点里属于 `category` 的字节数。
    pub fn category_size(&self, category: Category) -> u64 {
        match &self.cats {
            Some(cats) => cats[category.index()],
            None if self.category == category => self.size,
            None => 0,
        }
    }

    fn add_cats(&self, into: &mut Cats, sign: Sign) {
        let apply = |slot: &mut u64, bytes: u64| match sign {
            Sign::Add => *slot += bytes,
            Sign::Sub => *slot = slot.saturating_sub(bytes),
        };
        match &self.cats {
            Some(cats) => {
                for (slot, bytes) in into.iter_mut().zip(cats.iter()) {
                    apply(slot, *bytes);
                }
            }
            None => apply(&mut into[self.category.index()], self.size),
        }
    }

    /// 按「第几个子项」的下标路径找到后代；空路径就是自己。
    pub fn at(&self, path: &[u32]) -> Option<&Node> {
        let mut node = self;
        for &index in path {
            node = node.children.get(index as usize)?;
        }
        Some(node)
    }

    /// 下标路径一路经过的名字（不含 `self`）。删掉一项之后同级的下标会前移，名字不会，
    /// 所以要隔一阵再回来找同一项的，记名字、用 [`Node::locate`] 找回来。
    /// 路径走不通、或经过没有名字的「小项目」就是 `None`。
    pub fn names(&self, path: &[u32]) -> Option<Vec<Box<str>>> {
        let mut node = self;
        let mut names = Vec::with_capacity(path.len());
        for &index in path {
            node = node.children.get(index as usize)?;
            if node.kind == Kind::Rest {
                return None;
            }
            names.push(node.name.clone());
        }
        Some(names)
    }

    /// [`Node::names`] 的反过来：按名字一级一级找到后代，返回下标路径。同一目录里名字不重复。
    pub fn locate(&self, names: &[Box<str>]) -> Option<Vec<u32>> {
        let mut node = self;
        let mut path = Vec::with_capacity(names.len());
        for name in names {
            let index = node
                .children
                .iter()
                .position(|child| child.kind != Kind::Rest && child.name == *name)?;
            path.push(index as u32);
            node = &node.children[index];
        }
        Some(path)
    }

    /// 子树里最大的 `limit` 个文件（可限定类型），从大到小；带上它们的下标路径（相对 `self`）。
    pub fn top_files(&self, limit: usize, only: Option<Category>) -> Vec<(Vec<u32>, &Node)> {
        fn walk<'a>(
            node: &'a Node,
            path: &mut Vec<u32>,
            limit: usize,
            only: Option<Category>,
            heap: &mut BinaryHeap<Reverse<Hit<'a>>>,
        ) {
            for (index, child) in node.children.iter().enumerate() {
                // 堆满之后，比堆里最小的还小的，连同它整个子树都不用看。
                let floor = if heap.len() >= limit {
                    heap.peek().map(|hit| hit.0.0)
                } else {
                    None
                };
                match child.kind {
                    Kind::File => {
                        if child.size == 0 || only.is_some_and(|c| c != child.category) {
                            continue;
                        }
                        if floor.is_some_and(|floor| child.size <= floor) {
                            continue;
                        }
                        path.push(index as u32);
                        heap.push(Reverse(Hit(child.size, path.clone(), child)));
                        if heap.len() > limit {
                            heap.pop();
                        }
                        path.pop();
                    }
                    Kind::Dir => {
                        let relevant = only.map_or(child.size, |c| child.category_size(c));
                        if relevant == 0 || floor.is_some_and(|floor| relevant <= floor) {
                            continue;
                        }
                        path.push(index as u32);
                        walk(child, path, limit, only, heap);
                        path.pop();
                    }
                    Kind::Rest => {}
                }
            }
        }

        if limit == 0 {
            return Vec::new();
        }
        let mut heap = BinaryHeap::new();
        walk(self, &mut Vec::new(), limit, only, &mut heap);
        // `Reverse` 的升序就是文件大小的降序。
        heap.into_sorted_vec()
            .into_iter()
            .map(|Reverse(Hit(_, path, node))| (path, node))
            .collect()
    }

    /// 子树里（不含 `self`）排名最靠前的 `limit` 个节点，文件和目录都算（「小项目」没有名字，不算）。
    /// `rank` 给每个节点打分，`None` 表示不符合条件；结果按分数从大到小，带上下标路径（相对 `self`）和分数。
    /// 分数不能超过节点自己的 `size`：这样一棵子树整个比堆里最小的分数还小时，连它里面都不用看。
    pub fn search(
        &self,
        limit: usize,
        rank: &dyn Fn(&Node) -> Option<u64>,
    ) -> Vec<(Vec<u32>, &Node, u64)> {
        fn walk<'a>(
            node: &'a Node,
            path: &mut Vec<u32>,
            limit: usize,
            rank: &dyn Fn(&Node) -> Option<u64>,
            heap: &mut BinaryHeap<Reverse<Hit<'a>>>,
        ) {
            for (index, child) in node.children.iter().enumerate() {
                if child.kind == Kind::Rest || child.size == 0 {
                    continue;
                }
                let floor = if heap.len() >= limit {
                    heap.peek().map(|hit| hit.0.0)
                } else {
                    None
                };
                if floor.is_some_and(|floor| child.size <= floor) {
                    continue;
                }
                path.push(index as u32);
                if let Some(score) = rank(child)
                    && score > 0
                    && floor.is_none_or(|floor| score > floor)
                {
                    heap.push(Reverse(Hit(score, path.clone(), child)));
                    if heap.len() > limit {
                        heap.pop();
                    }
                }
                if child.kind == Kind::Dir {
                    walk(child, path, limit, rank, heap);
                }
                path.pop();
            }
        }

        if limit == 0 {
            return Vec::new();
        }
        let mut heap = BinaryHeap::new();
        walk(self, &mut Vec::new(), limit, rank, &mut heap);
        heap.into_sorted_vec()
            .into_iter()
            .map(|Reverse(Hit(score, path, node))| (path, node, score))
            .collect()
    }

    /// 从 `self` 这棵子树里摘掉 `path` 指的那一项，并从沿途每一级的大小、文件数和类型统计里扣掉它。
    fn remove(&mut self, path: &[u32]) -> Option<Node> {
        let (&first, rest) = path.split_first()?;
        let index = first as usize;
        let removed = if rest.is_empty() {
            if index >= self.children.len() {
                return None;
            }
            self.children.remove(index)
        } else {
            self.children.get_mut(index)?.remove(rest)?
        };
        self.size = self.size.saturating_sub(removed.size);
        self.files = self.files.saturating_sub(removed.files);
        // 下面一层已经按自己的 `count` 扣过了，这里只扣被摘掉的那一整棵。
        self.count = self.count.saturating_sub(removed.count);
        if let Some(cats) = self.cats.as_mut() {
            removed.add_cats(cats, Sign::Sub);
            self.category = dominant(cats);
        }
        Some(removed)
    }
}

#[derive(Clone, Copy)]
enum Sign {
    Add,
    Sub,
}

/// 一次扫描的结果。
#[derive(Debug)]
pub struct Tree {
    /// 扫描的起点。
    pub path: PathBuf,
    pub root: Node,
    /// 扫了多少个目录。
    pub dirs: u64,
    /// 有多少个目录没权限读（它们的大小没算进来）。
    pub unreadable: u64,
    pub elapsed: Duration,
    /// 起点所在卷扫描开始时的容量；取不到就是 `None`。
    pub volume: Option<Volume>,
}

impl Tree {
    /// 下标路径对应的完整路径。「小项目」没有自己的路径，停在它上一层。
    pub fn path_of(&self, path: &[u32]) -> PathBuf {
        let mut full = self.path.clone();
        let mut node = &self.root;
        for &index in path {
            let Some(child) = node.children.get(index as usize) else {
                break;
            };
            if child.kind != Kind::Rest {
                full.push(&*child.name);
            }
            node = child;
        }
        full
    }

    /// 把一项从树里摘掉（比如已经被移到废纸篓）。返回被摘掉的那一棵。
    pub fn remove(&mut self, path: &[u32]) -> Option<Node> {
        self.root.remove(path)
    }
}

/// 存档里只记不小于这么大的文件夹：再小的，哪怕变了也不值得在「什么变大了」里占一行。
const SNAPSHOT_MIN: u64 = 16 << 20;
/// 存档最多记多少个文件夹（最大的优先），文件不会无限变大。
const SNAPSHOT_MAX: usize = 3000;
/// 变化不到这么多的文件夹不算「变了」：缓存、日志天天都在涨涨落落。
pub const CHANGE_MIN: u64 = 64 << 20;
/// 「什么变了」最多列多少个。
const CHANGE_MAX: usize = 100;

/// 一次扫描里各个文件夹各占多少。存下来，下次扫同一个位置时拿来对比：这段时间里什么变大了、什么变小了。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// 扫描完成的时间，Unix 秒。
    pub taken: u64,
    /// 整个位置的大小。
    pub total: u64,
    /// 文件夹相对扫描起点的路径（用 `/` 隔开）和大小；只记够大的。
    pub dirs: Vec<(String, u64)>,
}

/// 两次扫描之间，一个文件夹的大小变了多少。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    /// 从扫描起点往下一层层的文件夹名。
    pub names: Vec<Box<str>>,
    pub before: u64,
    pub after: u64,
}

impl Change {
    /// 变大是正数，变小是负数。
    pub fn delta(&self) -> i64 {
        self.after as i64 - self.before as i64
    }
}

impl Snapshot {
    pub fn of(tree: &Tree, taken: u64) -> Snapshot {
        Self::with_floor(tree, taken, SNAPSHOT_MIN)
    }

    fn with_floor(tree: &Tree, taken: u64, floor: u64) -> Snapshot {
        fn walk(node: &Node, prefix: &str, floor: u64, out: &mut Vec<(String, u64)>) {
            for child in &node.children {
                // 文件夹比门槛小，它里面的更小，不用往下看。
                if child.kind != Kind::Dir || child.size < floor {
                    continue;
                }
                let path = if prefix.is_empty() {
                    child.name.to_string()
                } else {
                    format!("{prefix}/{}", child.name)
                };
                walk(child, &path, floor, out);
                out.push((path, child.size));
            }
        }

        let mut dirs = Vec::new();
        walk(&tree.root, "", floor, &mut dirs);
        // 大的优先留：父文件夹一定不比里面的小，被裁掉的总是最里层、最小的。
        dirs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        dirs.truncate(SNAPSHOT_MAX);
        Snapshot {
            taken,
            total: tree.root.size,
            dirs,
        }
    }

    /// 和更早的一次存档比：哪些文件夹变大了、变小了，变得多的在前。
    ///
    /// 一个文件夹变了，它的每一级父文件夹也跟着变。只列到「真正变的那一层」：
    /// 某个子孙文件夹的变化占了它八成以上，就只列那个子孙。
    /// 存档里没有的文件夹按 0 算（要么新出现，要么原来不够大没被记下来）。
    pub fn changes_since(&self, old: &Snapshot) -> Vec<Change> {
        use std::collections::BTreeMap;
        // 按「一级一级的名字」排序：一个文件夹的子孙紧跟在它后面，连成一段。
        // 按整条路径字符串排不行，`a b`、`a-x` 会插到 `a` 和 `a/c` 中间。
        let mut moved: BTreeMap<Vec<&str>, (u64, u64)> = BTreeMap::new();
        for (path, size) in &old.dirs {
            moved.entry(path.split('/').collect()).or_default().0 = *size;
        }
        for (path, size) in &self.dirs {
            moved.entry(path.split('/').collect()).or_default().1 = *size;
        }
        let moved: Vec<(Vec<&str>, i64, u64, u64)> = moved
            .into_iter()
            .filter(|(_, (was, now))| was.abs_diff(*now) >= CHANGE_MIN)
            .map(|(names, (was, now))| (names, now as i64 - was as i64, was, now))
            .collect();

        let mut changes: Vec<Change> = moved
            .iter()
            .enumerate()
            .filter(|&(at, (names, delta, ..))| {
                !moved[at + 1..]
                    .iter()
                    .take_while(|(other, ..)| other.starts_with(names))
                    .any(|(_, inner, ..)| {
                        inner.signum() == delta.signum()
                            && inner.unsigned_abs() * 5 >= delta.unsigned_abs() * 4
                    })
            })
            .map(|(_, (names, _, was, now))| Change {
                names: names.iter().map(|&name| Box::from(name)).collect(),
                before: *was,
                after: *now,
            })
            .collect();
        changes.sort_by(|a, b| {
            b.delta()
                .unsigned_abs()
                .cmp(&a.delta().unsigned_abs())
                .then_with(|| a.names.cmp(&b.names))
        });
        changes.truncate(CHANGE_MAX);
        changes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Volume {
    pub total: u64,
    pub used: u64,
}

#[derive(Debug)]
pub enum Error {
    Cancelled,
    NotADirectory,
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => f.write_str("cancelled"),
            Error::NotADirectory => f.write_str("not a directory"),
            Error::Io(error) => error.fmt(f),
        }
    }
}

/// 扫描进度，界面隔一会儿读一次。
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    /// 没权限读的目录个数。
    pub unreadable: AtomicU64,
    current: Mutex<String>,
}

impl Progress {
    /// 最近扫到的某个目录，只用来让人看出「还在动」。
    pub fn current(&self) -> String {
        self.current.lock().clone()
    }
}

pub struct Options {
    /// 小于它的文件 / 目录不单独保留，并进「小项目」。
    pub base_keep: u64,
    /// 每个目录最多保留多少个子项。
    pub max_children: usize,
    /// 留下的节点总数超过它，就把 `base_keep` 翻倍。
    pub budget: usize,
    pub threads: usize,
}

impl Default for Options {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism().map_or(4, |n| n.get());
        Options {
            base_keep: 16 * 1024,
            max_children: 400,
            budget: 800_000,
            threads: cpus.clamp(2, 8),
        }
    }
}

/// 扫描 `root`。`cancel` 被置位后尽快返回 [`Error::Cancelled`]。
pub fn scan(root: &Path, progress: &Progress, cancel: &AtomicBool) -> Result<Tree, Error> {
    scan_with(root, &Options::default(), progress, cancel)
}

pub fn scan_with(
    root: &Path,
    options: &Options,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<Tree, Error> {
    let started = Instant::now();
    // 起点本身是符号链接（比如 /tmp）时跟过去一次。
    let meta = fs::metadata(root).map_err(Error::Io)?;
    if !meta.is_dir() {
        return Err(Error::NotADirectory);
    }
    // 起点都打不开，那是错误，不是「空目录」。
    fs::read_dir(root).map_err(Error::Io)?;

    let ctx = Ctx {
        cancel,
        progress,
        options,
        #[cfg(unix)]
        device: std::os::unix::fs::MetadataExt::dev(&meta),
        #[cfg(unix)]
        links: std::array::from_fn(|_| Mutex::new(std::collections::HashSet::new())),
        live: AtomicI64::new(0),
        limit: AtomicI64::new(options.budget as i64),
        keep: AtomicU64::new(options.base_keep),
    };
    let volume = volume_of(root);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.threads.max(1))
        // 目录一层套一层地递归，给足栈。
        .stack_size(8 << 20)
        .thread_name(|index| format!("pc-specs-space-{index}"))
        .build()
        .map_err(|error| Error::Io(std::io::Error::other(error)))?;
    let name = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into(),
    );
    let mut node = pool.install(|| scan_dir(&ctx, root, &name, 0));
    if cancel.load(Relaxed) {
        return Err(Error::Cancelled);
    }
    // 扫描途中门槛可能抬高过：最早扫完的那些子树用的是旧门槛，这里统一按最终的门槛再裁一遍。
    prune(&mut node, ctx.keep.load(Relaxed), options.max_children);

    Ok(Tree {
        path: root.to_path_buf(),
        root: node,
        dirs: progress.dirs.load(Relaxed),
        unreadable: progress.unreadable.load(Relaxed),
        elapsed: started.elapsed(),
        volume,
    })
}

struct Ctx<'a> {
    cancel: &'a AtomicBool,
    progress: &'a Progress,
    options: &'a Options,
    /// 起点所在的设备；别的设备（挂载点）不进去。
    #[cfg(unix)]
    device: u64,
    /// 已经算过的硬链接。只有链接数大于 1 的文件才会进来；pnpm 之类的缓存里能有几百万个，
    /// 所以只存一个 64 位的键（设备号和 inode 混在一起），分片加锁。
    #[cfg(unix)]
    links: [Mutex<std::collections::HashSet<u64>>; LINK_SHARDS],
    /// 现在内存里一共有多少个节点。
    live: AtomicI64,
    /// `live` 超过它才会再抬一次门槛；抬完后跟着 `live` 往上挪，避免一口气翻很多倍。
    limit: AtomicI64,
    /// 当前「值得保留」的最小大小。
    keep: AtomicU64,
}

impl Ctx<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Relaxed)
    }

    fn min_keep(&self) -> u64 {
        self.keep.load(Relaxed)
    }

    /// 节点太多了就把门槛翻倍。
    fn relieve(&self) {
        let live = self.live.load(Relaxed);
        let limit = self.limit.load(Relaxed);
        if live > limit
            && self
                .limit
                .compare_exchange(
                    limit,
                    live + self.options.budget as i64 / 2,
                    Relaxed,
                    Relaxed,
                )
                .is_ok()
        {
            // 只有抢到上面那次比较交换的线程会走到这里，所以读出来再写回去不会丢更新。
            self.keep
                .store(self.keep.load(Relaxed).saturating_mul(2), Relaxed);
        }
    }

    #[cfg(unix)]
    fn same_volume(&self, meta: &fs::Metadata) -> bool {
        std::os::unix::fs::MetadataExt::dev(meta) == self.device
    }

    #[cfg(not(unix))]
    fn same_volume(&self, _: &fs::Metadata) -> bool {
        true
    }

    /// 文件实际占的字节数；已经算过的硬链接是 0。
    fn disk_size(&self, meta: &fs::Metadata) -> u64 {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() > 1 {
                let key = meta.ino() ^ meta.dev().rotate_left(48);
                if !self.links[(key % LINK_SHARDS as u64) as usize]
                    .lock()
                    .insert(key)
                {
                    return 0;
                }
            }
        }
        file_usage(meta)
    }
}

/// 一个文件实际占的字节数：Unix 上取 `st_blocks`，Windows 上按 4 KiB 簇取整。
/// 不管硬链接有没有算过——要去重得靠扫描时的记账。
pub fn file_usage(meta: &fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        std::os::unix::fs::MetadataExt::blocks(meta) * 512
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // 离线 / 按需下载的云端文件：数据不在本机，不占空间。
        const OFFLINE: u32 = 0x1000;
        const RECALL_ON_OPEN: u32 = 0x4_0000;
        const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;
        if meta.file_attributes() & (OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0 {
            return 0;
        }
        meta.len().div_ceil(4096) * 4096
    }
    #[cfg(not(any(unix, windows)))]
    {
        meta.len()
    }
}

/// 一个文件或文件夹（连同里面的一切）实际占多少字节、里面有多少文件。
/// 和扫描同口径，但不建树、不去重硬链接——给只要一个总数的地方用（比如清理前后各量一次）。
/// 不跟随符号链接；读不了的部分算 0。
pub fn usage(path: &Path, cancel: &AtomicBool) -> (u64, u64) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !meta.is_dir() {
        return (file_usage(&meta), 1);
    }
    usage_in(path, 0, cancel)
}

/// 前几层的文件夹里，各项并行去量；再往下一个个量。
/// 一个 `target` 或 `node_modules` 动辄上百万个文件，只在最外面一层分头量，剩下的几个大文件夹会拖很久；
/// 但并行也不能随目录深度一起往下套，线程栈会跟着长。
const PARALLEL_DEPTH: usize = 4;

fn usage_in(dir: &Path, depth: usize, cancel: &AtomicBool) -> (u64, u64) {
    if depth >= PARALLEL_DEPTH {
        return usage_sequential(dir, depth, cancel);
    }
    if cancel.load(Relaxed) {
        return (0, 0);
    }
    // 先把这一层读完再往下，这一层的目录句柄就不会一直开着。
    let Ok(entries) = fs::read_dir(dir) else {
        return (0, 0);
    };
    let entries: Vec<fs::DirEntry> = entries.flatten().collect();
    entries
        .par_iter()
        .map(|entry| {
            let Ok(meta) = entry.metadata() else {
                return (0, 0);
            };
            if meta.is_dir() {
                usage_in(&entry.path(), depth + 1, cancel)
            } else {
                (file_usage(&meta), 1)
            }
        })
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
}

fn usage_sequential(dir: &Path, depth: usize, cancel: &AtomicBool) -> (u64, u64) {
    let (mut bytes, mut files) = (0u64, 0u64);
    // 用显式的栈而不是递归：目录套得再深，也不会同时开着一长串目录句柄。
    let mut pending = vec![(dir.to_path_buf(), depth)];
    while let Some((dir, depth)) = pending.pop() {
        if cancel.load(Relaxed) {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if depth < MAX_DEPTH {
                    pending.push((entry.path(), depth + 1));
                }
            } else {
                bytes += file_usage(&meta);
                files += 1;
            }
        }
    }
    (bytes, files)
}

#[cfg(unix)]
fn is_executable(meta: &fs::Metadata) -> bool {
    std::os::unix::fs::MetadataExt::mode(meta) & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &fs::Metadata) -> bool {
    false
}

fn modified_secs(meta: &fs::Metadata) -> u32 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_secs().min(u32::MAX as u64) as u32)
}

/// 被裁掉的那些项的合计。
#[derive(Default)]
struct Folded {
    size: u64,
    files: u32,
    cats: Cats,
}

impl Folded {
    fn file(&mut self, size: u64, category: Category) {
        self.size += size;
        self.files = self.files.saturating_add(1);
        self.cats[category.index()] += size;
    }

    fn node(&mut self, node: &Node) {
        self.size += node.size;
        self.files = self.files.saturating_add(node.files);
        node.add_cats(&mut self.cats, Sign::Add);
    }

    fn merge(&mut self, other: Folded) {
        self.size += other.size;
        self.files = self.files.saturating_add(other.files);
        for (slot, bytes) in self.cats.iter_mut().zip(other.cats) {
            *slot += bytes;
        }
    }

    fn into_node(self) -> Option<Node> {
        (self.size > 0).then(|| Node {
            name: "".into(),
            kind: Kind::Rest,
            category: dominant(&self.cats),
            size: self.size,
            files: self.files,
            modified: 0,
            count: 1,
            cats: Some(Box::new(self.cats)),
            children: Vec::new(),
        })
    }
}

fn by_size(a: &Node, b: &Node) -> Ordering {
    b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name))
}

/// 一个目录的子项在扫描过程中的累加器。
#[derive(Default)]
struct Acc {
    kept: Vec<Node>,
    folded: Folded,
    size: u64,
    files: u32,
    cats: Cats,
    /// 被裁掉的子目录带走了多少个节点（它们在扫完自己时已经记进了 `live`）。
    dropped: i64,
}

impl Acc {
    fn file(&mut self, ctx: &Ctx, name: &str, size: u64, modified: u32, category: Category) {
        self.size += size;
        self.files = self.files.saturating_add(1);
        self.cats[category.index()] += size;
        if size < ctx.min_keep() {
            self.folded.file(size, category);
            return;
        }
        self.kept.push(Node::file(name, size, modified, category));
        // 一个目录里有几十万个文件时别全攒着：够多了就先收一次。
        if self.kept.len() >= ctx.options.max_children * 2 {
            self.settle(ctx);
        }
    }

    fn dir(&mut self, ctx: &Ctx, node: Node) {
        self.size += node.size;
        self.files = self.files.saturating_add(node.files);
        node.add_cats(&mut self.cats, Sign::Add);
        if node.size < ctx.min_keep() {
            self.drop_node(node);
        } else {
            self.kept.push(node);
        }
    }

    /// 并入同一个目录里另一块条目的结果。
    fn merge(&mut self, ctx: &Ctx, other: Acc) {
        self.size += other.size;
        self.files = self.files.saturating_add(other.files);
        for (slot, bytes) in self.cats.iter_mut().zip(other.cats) {
            *slot += bytes;
        }
        self.folded.merge(other.folded);
        self.dropped += other.dropped;
        self.kept.extend(other.kept);
        if self.kept.len() >= ctx.options.max_children * 2 {
            self.settle(ctx);
        }
    }

    fn drop_node(&mut self, node: Node) {
        self.folded.node(&node);
        if node.kind != Kind::File {
            self.dropped += i64::from(node.count);
        }
    }

    /// 只留最大的、而且不小于门槛的那些，其余并进「小项目」。
    fn settle(&mut self, ctx: &Ctx) {
        self.kept.sort_unstable_by(by_size);
        let min = ctx.min_keep();
        let keep = self
            .kept
            .iter()
            .take(ctx.options.max_children)
            .take_while(|node| node.size >= min)
            .count();
        for node in self.kept.split_off(keep) {
            self.drop_node(node);
        }
    }

    fn finish(mut self, ctx: &Ctx, name: &str) -> Node {
        self.settle(ctx);
        let mut children = self.kept;
        children.shrink_to_fit();
        let files_kept = children.iter().filter(|c| c.kind == Kind::File).count();
        let rest = self.folded.into_node();
        let rest_nodes = usize::from(rest.is_some());
        if let Some(rest) = rest {
            children.push(rest);
            children.sort_unstable_by(by_size);
        }
        let count = children
            .iter()
            .fold(1u32, |sum, child| sum.saturating_add(child.count));

        // 新记进内存的：留下的文件、「小项目」和这个目录自己；扔掉的：被裁掉的子目录。
        ctx.live
            .fetch_add((files_kept + rest_nodes + 1) as i64 - self.dropped, Relaxed);
        ctx.relieve();

        Node {
            name: name.into(),
            kind: Kind::Dir,
            category: dominant(&self.cats),
            size: self.size,
            files: self.files,
            modified: 0,
            count,
            cats: Some(Box::new(self.cats)),
            children,
        }
    }
}

/// 一块目录条目的处理结果：留下的文件、合计，和要往下钻的子目录。
struct Part {
    acc: Acc,
    subdirs: Vec<(String, PathBuf)>,
}

fn scan_entries(ctx: &Ctx, entries: &[fs::DirEntry]) -> Part {
    let mut part = Part {
        acc: Acc::default(),
        subdirs: Vec::new(),
    };
    let (mut files, mut bytes) = (0u64, 0u64);
    for (index, entry) in entries.iter().enumerate() {
        if index % 256 == 0 && ctx.cancelled() {
            break;
        }
        // 不跟随符号链接。
        let Ok(meta) = entry.metadata() else { continue };
        let kind = meta.file_type();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if kind.is_dir() {
            // 别的设备（挂载点、网络盘）不进去。
            if ctx.same_volume(&meta) {
                part.subdirs.push((file_name.into_owned(), entry.path()));
            }
        } else if kind.is_file() {
            let size = ctx.disk_size(&meta);
            let category = Category::of_file(&file_name, is_executable(&meta));
            part.acc
                .file(ctx, &file_name, size, modified_secs(&meta), category);
            files += 1;
            bytes += size;
        }
    }
    ctx.progress.files.fetch_add(files, Relaxed);
    ctx.progress.bytes.fetch_add(bytes, Relaxed);
    part
}

fn scan_dir(ctx: &Ctx, path: &Path, name: &str, depth: usize) -> Node {
    if ctx.cancelled() {
        return Acc::default().finish(ctx, name);
    }
    let progress = ctx.progress;
    // 隔几百个目录记一次路径，只为让界面看得出还在动，不值得每个目录都抢一次锁。
    if progress.dirs.fetch_add(1, Relaxed).is_multiple_of(256) {
        *progress.current.lock() = path.display().to_string();
    }
    let entries: Vec<fs::DirEntry> = match fs::read_dir(path) {
        Ok(entries) if depth < MAX_DEPTH => entries.filter_map(Result::ok).collect(),
        _ => {
            progress.unreadable.fetch_add(1, Relaxed);
            return Acc::default().finish(ctx, name);
        }
    };

    // 小目录直接读；条目特别多的目录分块并行读，不然它会让别的线程干等。
    let parts: Vec<Part> = if entries.len() >= PARALLEL_FROM {
        entries
            .par_chunks(CHUNK)
            .map(|chunk| scan_entries(ctx, chunk))
            .collect()
    } else {
        vec![scan_entries(ctx, &entries)]
    };
    drop(entries);

    let mut acc = Acc::default();
    let mut subdirs = Vec::new();
    for part in parts {
        acc.merge(ctx, part.acc);
        subdirs.extend(part.subdirs);
    }
    let nodes: Vec<Node> = subdirs
        .into_par_iter()
        .map(|(name, path)| scan_dir(ctx, &path, &name, depth + 1))
        .collect();
    for node in nodes {
        acc.dir(ctx, node);
    }
    acc.finish(ctx, name)
}

/// 按 `min` 和 `max_children` 把整棵树重新裁一遍：门槛在扫描途中抬高过，先扫完的子树比后扫完的细。
fn prune(node: &mut Node, min: u64, max_children: usize) {
    if node.kind != Kind::Dir {
        return;
    }
    for child in &mut node.children {
        prune(child, min, max_children);
    }
    node.children.sort_unstable_by(by_size);

    let mut folded = Folded::default();
    let mut kept = Vec::with_capacity(node.children.len().min(max_children + 1));
    for child in std::mem::take(&mut node.children) {
        let small = child.size < min || kept.len() >= max_children;
        if child.kind == Kind::Rest || small {
            folded.node(&child);
        } else {
            kept.push(child);
        }
    }
    if let Some(rest) = folded.into_node() {
        kept.push(rest);
        kept.sort_unstable_by(by_size);
    }
    // 子目录的各类型明细在刚才并「小项目」时已经用过了；小的这时可以扔，省下每个目录 80 字节左右。
    for child in &mut kept {
        if child.kind == Kind::Dir && child.size < DETAIL_FROM {
            child.cats = None;
        }
    }
    node.count = kept
        .iter()
        .fold(1u32, |sum, child| sum.saturating_add(child.count));
    node.children = kept;
}

/// 路径所在卷的容量。
#[cfg(unix)]
pub fn volume_of(path: &Path) -> Option<Volume> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: `c_path` 是以 NUL 结尾的有效路径；`stat` 是纯输出缓冲，调用成功后各字段才有意义。
    let stat = unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return None;
        }
        stat
    };
    #[allow(clippy::unnecessary_cast)]
    let (block, blocks, available) = (
        stat.f_frsize as u64,
        stat.f_blocks as u64,
        stat.f_bavail as u64,
    );
    let total = blocks.saturating_mul(block);
    (total > 0).then(|| Volume {
        total,
        used: total.saturating_sub(available.saturating_mul(block)),
    })
}

#[cfg(not(unix))]
pub fn volume_of(path: &Path) -> Option<Volume> {
    use sysinfo::Disks;
    let disks = Disks::new_with_refreshed_list().ok()?;
    // 挂载点最长的那个就是路径所在的卷。
    disks
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| Volume {
            total: disk.total_space(),
            used: disk.total_space().saturating_sub(disk.available_space()),
        })
        .filter(|volume| volume.total > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    /// 测试用的临时目录，用完自己删。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let path = std::env::temp_dir().join(format!("pcs-space-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }

        fn write(&self, relative: &str, bytes: usize) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            // 写非零字节，免得文件系统把它当稀疏文件。
            fs::File::create(path)
                .unwrap()
                .write_all(&vec![1u8; bytes])
                .unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const KIB: usize = 1024;
    const MIB: usize = 1024 * KIB;

    fn run(root: &Path, options: &Options) -> Tree {
        scan_with(root, options, &Progress::default(), &AtomicBool::new(false)).unwrap()
    }

    /// 实际占用比文件长度大不了多少（取整到块）。
    fn assert_about(actual: u64, expected: usize) {
        let expected = expected as u64;
        assert!(
            actual >= expected && actual <= expected + 64 * 1024,
            "{actual} is not about {expected}"
        );
    }

    fn child<'a>(node: &'a Node, name: &str) -> &'a Node {
        node.children
            .iter()
            .find(|c| &*c.name == name)
            .unwrap_or_else(|| panic!("no child named {name}"))
    }

    #[test]
    fn sums_sizes_and_types_up_the_tree() {
        let dir = Scratch::new("sums");
        dir.write("movies/a.mp4", 3 * MIB);
        dir.write("movies/extras/b.MKV", MIB);
        dir.write("docs/c.pdf", 2 * MIB);
        dir.write("notes.txt", 64 * KIB);
        let tree = run(&dir.0, &Options::default());

        assert_about(tree.root.size, 6 * MIB + 64 * KIB);
        assert_eq!(tree.root.files, 4);
        let movies = child(&tree.root, "movies");
        assert_about(movies.size, 4 * MIB);
        assert_eq!(movies.category, Category::Video);
        // 目录按大小从大到小排；扩展名不分大小写。
        assert_eq!(&*tree.root.children[0].name, "movies");
        assert_about(movies.category_size(Category::Video), 4 * MIB);
        assert_about(
            tree.root.category_size(Category::Document),
            2 * MIB + 64 * KIB,
        );
        let total: u64 = tree.root.cats.as_ref().unwrap().iter().sum();
        assert_eq!(total, tree.root.size);
        assert!(tree.dirs >= 4);
        assert_eq!(tree.unreadable, 0);
        // 路径由下标一层层还原：movies 里 3 MiB 的 a.mp4 排第一，extras 排第二。
        let extras = tree.path_of(&[0, 1]);
        assert_eq!(extras, dir.0.join("movies").join("extras"));
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_count_once_and_symlinks_are_not_followed() {
        let dir = Scratch::new("links");
        dir.write("big/file.bin", 2 * MIB);
        fs::hard_link(dir.0.join("big/file.bin"), dir.0.join("big/twin.bin")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("big"), dir.0.join("alias")).unwrap();
        let tree = run(&dir.0, &Options::default());
        assert_about(tree.root.size, 2 * MIB);
        assert!(tree.root.children.iter().all(|c| &*c.name != "alias"));
    }

    #[test]
    fn small_items_are_folded_but_still_counted() {
        let dir = Scratch::new("fold");
        dir.write("big.bin", 2 * MIB);
        for i in 0..40 {
            dir.write(&format!("tiny/{i}.dat"), 8 * KIB);
        }
        let options = Options {
            base_keep: MIB as u64,
            ..Options::default()
        };
        let tree = run(&dir.0, &options);

        // 40 个小文件：本身都不到门槛，整个 tiny 目录（约 320 KiB）也不到，被并进根上的「小项目」。
        let rest = tree
            .root
            .children
            .iter()
            .find(|c| c.kind == Kind::Rest)
            .expect("rest bucket");
        assert_eq!(rest.files, 40);
        assert!(tree.root.children.iter().all(|c| &*c.name != "tiny"));
        // 总数没有因为裁剪少一分。
        assert_eq!(tree.root.files, 41);
        assert!(tree.root.size > 2 * MIB as u64);
        assert_eq!(
            tree.root.children.iter().map(|c| c.size).sum::<u64>(),
            tree.root.size
        );
        // 节点数也跟着变少了：根 + 大文件 + 小项目。
        assert_eq!(tree.root.count, 3);
    }

    #[test]
    fn a_crowded_directory_keeps_only_its_largest_children() {
        let dir = Scratch::new("crowd");
        for i in 0..30 {
            dir.write(&format!("f{i:02}.dat"), (i + 1) * 16 * KIB);
        }
        let options = Options {
            base_keep: 16 * 1024,
            max_children: 5,
            ..Options::default()
        };
        let tree = run(&dir.0, &options);
        let files: Vec<_> = tree
            .root
            .children
            .iter()
            .filter(|c| c.kind == Kind::File)
            .collect();
        assert_eq!(files.len(), 5);
        assert_eq!(&*files[0].name, "f29.dat");
        assert!(files.windows(2).all(|pair| pair[0].size >= pair[1].size));
        assert_eq!(tree.root.files, 30);
        assert_eq!(
            tree.root.children.iter().map(|c| c.size).sum::<u64>(),
            tree.root.size
        );
    }

    #[test]
    fn a_huge_directory_is_read_in_chunks_without_losing_anything() {
        let dir = Scratch::new("huge");
        let count = PARALLEL_FROM + CHUNK + 7;
        for i in 0..count {
            dir.write(&format!("many/f{i:05}.dat"), 16 * KIB + (i % 3) * 4 * KIB);
        }
        let options = Options {
            max_children: 50,
            ..Options::default()
        };
        let tree = run(&dir.0, &options);
        let many = child(&tree.root, "many");
        assert_eq!(many.files as usize, count);
        assert_eq!(tree.root.files as usize, count);
        let expected: usize = (0..count).map(|i| 16 * KIB + (i % 3) * 4 * KIB).sum();
        assert_about(many.size, expected);
        assert!(many.children.len() <= 51);
        assert_eq!(many.children.iter().map(|c| c.size).sum::<u64>(), many.size);
    }

    #[test]
    fn a_tight_budget_raises_the_bar_but_loses_nothing() {
        let dir = Scratch::new("budget");
        for d in 0..20 {
            for f in 0..20 {
                dir.write(&format!("d{d:02}/f{f:02}.dat"), (f + 1) * 16 * KIB);
            }
        }
        let options = Options {
            budget: 40,
            threads: 2,
            ..Options::default()
        };
        let tree = run(&dir.0, &options);
        assert_eq!(tree.root.files, 400);
        let sum: usize = (0..20usize).map(|f| (f + 1) * 16 * KIB).sum::<usize>() * 20;
        assert_about(tree.root.size, sum);
        assert!(tree.root.count < 400, "kept {} nodes", tree.root.count);
        assert_eq!(
            tree.root.children.iter().map(|c| c.size).sum::<u64>(),
            tree.root.size
        );
    }

    #[test]
    fn cancelling_and_bad_roots_are_errors() {
        let dir = Scratch::new("errors");
        dir.write("a/b.bin", MIB);
        let cancelled = AtomicBool::new(true);
        let result = scan_with(
            &dir.0,
            &Options::default(),
            &Progress::default(),
            &cancelled,
        );
        assert!(matches!(result, Err(Error::Cancelled)));

        let file = scan(&dir.0.join("a/b.bin"), &Progress::default(), &cancelled);
        assert!(matches!(file, Err(Error::NotADirectory)));
        let missing = scan(&dir.0.join("nope"), &Progress::default(), &cancelled);
        assert!(matches!(missing, Err(Error::Io(_))));
    }

    #[test]
    fn largest_files_come_out_biggest_first_and_can_be_limited_to_a_type() {
        let dir = Scratch::new("top");
        dir.write("a/one.mp4", 5 * MIB);
        dir.write("a/two.mp3", 3 * MIB);
        dir.write("b/three.mp4", 4 * MIB);
        dir.write("b/deep/four.mp4", MIB);
        dir.write("five.zip", 2 * MIB);
        let tree = run(&dir.0, &Options::default());

        let names = |hits: &[(Vec<u32>, &Node)]| -> Vec<String> {
            hits.iter().map(|(_, n)| n.name.to_string()).collect()
        };
        let all = tree.root.top_files(3, None);
        assert_eq!(names(&all), ["one.mp4", "three.mp4", "two.mp3"]);
        // 返回的路径能原样还原成文件。
        for (path, node) in &all {
            assert_eq!(tree.root.at(path).unwrap().name, node.name);
        }
        let videos = tree.root.top_files(10, Some(Category::Video));
        assert_eq!(names(&videos), ["one.mp4", "three.mp4", "four.mp4"]);
        assert!(tree.root.top_files(0, None).is_empty());
    }

    #[test]
    fn search_ranks_files_and_folders_that_pass_the_test() {
        let dir = Scratch::new("search");
        dir.write("video/holiday.mp4", 5 * MIB);
        dir.write("video/clips/holiday-b.mp4", 2 * MIB);
        dir.write("video/clips/other.mov", 3 * MIB);
        dir.write("docs/holiday.txt", MIB);
        dir.write("notes.txt", 64 * KIB);
        let tree = run(&dir.0, &Options::default());

        let names = |hits: &[(Vec<u32>, &Node, u64)]| -> Vec<String> {
            hits.iter().map(|(_, n, _)| n.name.to_string()).collect()
        };
        // 名字里带 holiday 的，文件和目录都算，大的在前。
        let holiday = |node: &Node| node.name.contains("holiday").then_some(node.size);
        let hits = tree.root.search(10, &holiday);
        assert_eq!(
            names(&hits),
            ["holiday.mp4", "holiday-b.mp4", "holiday.txt"]
        );
        for (path, node, score) in &hits {
            assert_eq!(tree.root.at(path).unwrap().name, node.name);
            assert_eq!(*score, node.size);
        }
        // 限量时留下最大的；目录也能被搜到，且排在它里面的文件前面。
        let top = tree.root.search(2, &holiday);
        assert_eq!(names(&top), ["holiday.mp4", "holiday-b.mp4"]);
        let folders = tree
            .root
            .search(10, &|node: &Node| node.is_dir().then_some(node.size));
        assert_eq!(names(&folders)[0], "video");
        assert!(names(&folders).contains(&"clips".to_string()));
        // 没有符合的、或者限量为 0，就是空的。
        assert!(tree.root.search(10, &|_| None).is_empty());
        assert!(tree.root.search(0, &holiday).is_empty());
    }

    #[test]
    fn usage_agrees_with_a_scan_and_does_not_follow_links() {
        let dir = Scratch::new("usage");
        dir.write("a/one.bin", 300 * KIB);
        dir.write("a/deep/two.bin", 200 * KIB);
        dir.write("three.bin", 100 * KIB);
        let tree = run(&dir.0, &Options::default());
        let never = AtomicBool::new(false);
        assert_eq!(
            usage(&dir.0, &never),
            (tree.root.size, u64::from(tree.root.files))
        );
        // 单个文件、不存在的路径。
        assert_eq!(usage(&dir.0.join("three.bin"), &never).1, 1);
        assert_eq!(usage(&dir.0.join("missing"), &never), (0, 0));
        // 指向别处的链接不会把别处的字节算进来。
        #[cfg(unix)]
        {
            let outside = Scratch::new("usage-outside");
            outside.write("huge.bin", 2 * MIB);
            std::os::unix::fs::symlink(&outside.0, dir.0.join("link")).unwrap();
            // 链接本身占的一点点可以有（有的文件系统给符号链接一整块），别处的 2 MiB 不能被算进来。
            let linked = usage(&dir.0, &never).0;
            assert!(
                (tree.root.size..tree.root.size + (MIB / 2) as u64).contains(&linked),
                "{linked} vs {}",
                tree.root.size
            );
        }
        // 取消了就不再往下读。
        assert_eq!(usage(&dir.0, &AtomicBool::new(true)), (0, 0));
    }

    #[test]
    fn usage_is_the_same_whether_it_walks_in_parallel_or_one_by_one() {
        let dir = Scratch::new("usage-par");
        // 一层里有很多项、也有比并行的层数更深的一支。
        for index in 0..300 {
            dir.write(&format!("flat/f{index}.bin"), 1_000 + index);
        }
        let mut deep = String::from("tree");
        for level in 0..PARALLEL_DEPTH + 3 {
            deep.push_str(&format!("/d{level}"));
            dir.write(&format!("{deep}/file{level}.bin"), 5_000 + level);
        }
        dir.write("tree/side/leaf.bin", 70_000);
        let never = AtomicBool::new(false);
        let walked = usage(&dir.0, &never);
        assert_eq!(walked, usage_sequential(&dir.0, 0, &never));
        assert_eq!(walked.1, 300 + (PARALLEL_DEPTH as u64 + 3) + 1);
        // 取消了，并行的几层也一样不再往下读。
        assert_eq!(usage(&dir.0, &AtomicBool::new(true)), (0, 0));
    }

    #[test]
    fn a_snapshot_keeps_the_big_folders_with_their_paths() {
        let dir = Scratch::new("snapshot");
        dir.write("a/one.bin", 300 * KIB);
        dir.write("a/deep/two.bin", 200 * KIB);
        dir.write("b/tiny.bin", 20 * KIB);
        let tree = run(&dir.0, &Options::default());
        let snapshot = Snapshot::with_floor(&tree, 42, 100 * KIB as u64);
        assert_eq!((snapshot.taken, snapshot.total), (42, tree.root.size));
        let paths: Vec<&str> = snapshot.dirs.iter().map(|(p, _)| p.as_str()).collect();
        // 够大的文件夹连同里面的子文件夹都在，大的在前；`b` 不够大。
        assert_eq!(paths, ["a", "a/deep"]);
        assert_eq!(snapshot.dirs[0].1, child(&tree.root, "a").size);
        // 存下来再读回来不走样。
        let again: Snapshot =
            serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
        assert_eq!(again, snapshot);
    }

    #[test]
    fn changes_are_listed_where_they_happened_and_biggest_first() {
        const MB: u64 = 1 << 20;
        let snap = |dirs: &[(&str, u64)]| Snapshot {
            taken: 0,
            total: 0,
            dirs: dirs.iter().map(|&(p, s)| (p.to_string(), s * MB)).collect(),
        };
        let old = snap(&[
            ("a", 1000),
            ("a/cache", 500),
            ("a b", 300),
            ("keep", 800),
            ("shrink", 900),
        ]);
        let new = snap(&[
            ("a", 1700),
            ("a/cache", 1190),
            ("a b", 400),
            ("keep", 810),
            ("shrink", 300),
            ("fresh", 700),
            ("fresh/inner", 650),
        ]);
        let changes = new.changes_since(&old);
        let listed: Vec<(String, i64)> = changes
            .iter()
            .map(|c| (c.names.join("/"), c.delta() / MB as i64))
            .collect();
        // `a` 涨了 700，其中 690 出在 `a/cache`，只列后者；`fresh` 也一样。
        // `a b` 按整条路径的字符串排会插在 `a` 和 `a/cache` 之间，不能因此把它们隔开。
        // `keep` 只涨了 10，不算。
        let expected = [
            ("a/cache", 690),
            ("fresh/inner", 650),
            ("shrink", -600),
            ("a b", 100),
        ];
        assert_eq!(
            listed,
            expected.map(|(path, delta)| (path.to_string(), delta))
        );
        assert!(new.changes_since(&new).is_empty());
    }

    #[test]
    fn removing_an_item_updates_every_level_above_it() {
        let dir = Scratch::new("remove");
        dir.write("a/x.mp4", 3 * MIB);
        dir.write("a/y.txt", MIB);
        dir.write("b/z.mp4", 2 * MIB);
        let mut tree = run(&dir.0, &Options::default());
        let before = tree.root.size;
        let count = tree.root.count;
        let a_index = tree
            .root
            .children
            .iter()
            .position(|c| &*c.name == "a")
            .unwrap();
        let x_index = tree.root.children[a_index]
            .children
            .iter()
            .position(|c| &*c.name == "x.mp4")
            .unwrap();

        let removed = tree.remove(&[a_index as u32, x_index as u32]).unwrap();
        assert_eq!(&*removed.name, "x.mp4");
        assert_eq!(tree.root.size, before - removed.size);
        assert_eq!(tree.root.files, 2);
        assert_eq!(tree.root.count, count - 1);
        let a = child(&tree.root, "a");
        assert_eq!(a.files, 1);
        assert_eq!(a.category, Category::Document);
        assert_eq!(a.category_size(Category::Video), 0);
        let total: u64 = tree.root.cats.as_ref().unwrap().iter().sum();
        assert_eq!(total, tree.root.size);
        assert!(tree.remove(&[99]).is_none());
        assert!(tree.remove(&[]).is_none());
    }

    #[test]
    fn an_item_can_be_found_again_by_name_after_its_siblings_move() {
        let dir = Scratch::new("locate");
        dir.write("a/first.mp4", 4 * MIB);
        dir.write("a/second.mp4", 2 * MIB);
        dir.write("a/third.mp4", MIB);
        dir.write("b.bin", 3 * MIB);
        let mut tree = run(&dir.0, &Options::default());

        let third = tree.root.locate(&["a".into(), "third.mp4".into()]).unwrap();
        assert_eq!(
            tree.root.names(&third).unwrap(),
            ["a".into(), "third.mp4".into()]
        );
        assert_eq!(&*tree.root.at(&third).unwrap().name, "third.mp4");

        // 同级前面的一项被摘掉以后，旧下标指向别的东西，按名字却仍能找到原来那一项。
        let first = tree.root.locate(&["a".into(), "first.mp4".into()]).unwrap();
        tree.remove(&first).unwrap();
        let moved = tree.root.locate(&["a".into(), "third.mp4".into()]).unwrap();
        assert_ne!(moved, third);
        assert_eq!(&*tree.root.at(&moved).unwrap().name, "third.mp4");

        assert!(
            tree.root
                .locate(&["a".into(), "first.mp4".into()])
                .is_none()
        );
        assert!(tree.root.locate(&["nope".into()]).is_none());
        assert_eq!(tree.root.locate(&[]), Some(Vec::new()));
        assert!(tree.root.names(&[99]).is_none());
    }

    #[test]
    fn files_are_sorted_into_types_by_extension_or_the_executable_bit() {
        assert_eq!(Category::of_file("Movie.MP4", false), Category::Video);
        assert_eq!(Category::of_file("song.flac", false), Category::Audio);
        assert_eq!(Category::of_file("photo.HEIC", false), Category::Image);
        assert_eq!(Category::of_file("report.pdf", false), Category::Document);
        assert_eq!(Category::of_file("backup.tar.gz", false), Category::Archive);
        assert_eq!(Category::of_file("disk.dmg", false), Category::Archive);
        assert_eq!(Category::of_file("setup.exe", false), Category::Program);
        assert_eq!(Category::of_file("main.rs", false), Category::Code);
        assert_eq!(Category::of_file("mystery", false), Category::Other);
        assert_eq!(Category::of_file("mystery", true), Category::Program);
        // 点开头的隐藏文件不是「扩展名」。
        assert_eq!(Category::of_file(".mp4", false), Category::Other);
        assert_eq!(Category::of_file("noext.", false), Category::Other);
        assert_eq!(dominant(&[0; CATEGORIES]), Category::Other);
    }

    #[test]
    fn volume_of_a_real_path_has_a_capacity() {
        let volume = volume_of(&std::env::temp_dir()).expect("volume");
        assert!(volume.total > 0 && volume.used <= volume.total);
    }
}

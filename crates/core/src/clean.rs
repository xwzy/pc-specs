//! 清理：把应用随时能重新生成的东西——缓存、日志、临时文件、开发工具的下载缓存、不碰的项目里的构建产物——一次清掉。
//!
//! 安全靠「白名单」：只认下面写死的这些位置，只动它们的**里面**（位置本身不删）；
//! 不跟随符号链接；名字在保留清单里的、刚刚还在被改动的项目不碰；位置太浅（磁盘根、个人文件夹）一律不认。
//! 清掉的东西是直接删除、不进废纸篓：缓存进废纸篓只会白占空间，而白名单里的东西都是应用下次用到时会自己重建的。
//! 唯一不是写死位置的是项目里的构建产物，它们在 [`projects`] 里另有一套更严的认法。

mod projects;

pub use projects::projects;

use crate::space;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, SystemTime};

/// 可以清理的几类东西。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 应用缓存。
    Caches,
    /// 浏览器缓存。
    Browsers,
    /// 日志与崩溃报告。
    Logs,
    /// 临时文件。
    Temp,
    /// 开发工具的构建产物和下载缓存。
    Developer,
    /// 闲置项目里的构建产物（`node_modules`、`target`、虚拟环境……）。
    Projects,
    /// 废纸篓。
    Trash,
}

/// 白名单里的一个位置。
#[derive(Clone, Debug)]
pub struct Root {
    pub path: PathBuf,
    /// 里面名字符合其中之一的项目不动；以 `*` 结尾的按前缀匹配。
    pub keep: Vec<&'static str>,
    /// 只清理这么久没被改动过的项目；0 表示不限。
    pub idle: Duration,
    /// 给界面看的名字（比如「Chrome」「Cargo」）；空表示直接用里面各项的名字。
    /// 同名的位置会合在一起算。
    pub label: String,
    /// 位置如果是某个项目里的构建产物，这就是那个项目的文件夹：项目最近被动过，位置就整个不碰。
    pub project: Option<PathBuf>,
}

impl Root {
    fn new(path: PathBuf) -> Root {
        Root {
            path,
            keep: Vec::new(),
            idle: Duration::ZERO,
            label: String::new(),
            project: None,
        }
    }

    fn labeled(mut self, label: impl AsRef<str>) -> Root {
        self.label = label.as_ref().to_owned();
        self
    }

    fn in_project(mut self, project: PathBuf) -> Root {
        self.project = Some(project);
        self
    }

    #[cfg(any(not(windows), test))]
    fn keeping(mut self, keep: impl IntoIterator<Item = &'static str>) -> Root {
        self.keep = keep.into_iter().collect();
        self
    }

    #[cfg(any(windows, test))]
    fn idle_for(mut self, idle: Duration) -> Root {
        self.idle = idle;
        self
    }
}

/// 一类东西，以及它在这台机器上的各个位置。
#[derive(Clone, Debug)]
pub struct Spec {
    pub kind: Kind,
    pub roots: Vec<Root>,
}

/// 名字 `name` 在不在保留清单里。
fn kept(name: &str, keep: &[&str]) -> bool {
    keep.iter().any(|rule| match rule.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *rule,
    })
}

/// 位置够深：磁盘根、个人文件夹本身、`/Users` 这样只有一两层的目录，绝不当作清理位置。
/// 万一哪个环境变量没设好、拼出了一个很短的路径，也不会误伤。
fn deep_enough(path: &Path) -> bool {
    let depth = path
        .components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count();
    path.is_absolute() && depth >= 3 && !dirs::home_dir().is_some_and(|home| home.starts_with(path))
}

/// 用户可以改的位置（环境变量指定的缓存、临时文件夹）用它核对一下：最后一级的名字里要带 `words` 之一，
/// 否则宁可不认。清理是直接删除，指错了地方代价太大。
#[cfg(any(target_os = "linux", windows, test))]
fn named_like(path: &Path, words: &[&str]) -> bool {
    path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy().to_lowercase();
        words.iter().any(|word| name.contains(word))
    })
}

#[cfg(any(windows, test))]
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// 各个浏览器在缓存目录（macOS 的 `~/Library/Caches`、Linux 的 `~/.cache`）下的文件夹。
/// 它们单独成一类，所以「应用缓存」里要把它们让出来，两边的数字才不会重复。
#[cfg(target_os = "macos")]
const BROWSER_DIRS: [(&str, &str); 8] = [
    ("Google", "Chrome"),
    ("Firefox", "Firefox"),
    ("Microsoft Edge", "Edge"),
    ("BraveSoftware", "Brave"),
    ("com.apple.Safari", "Safari"),
    ("company.thebrowser.Browser", "Arc"),
    ("com.operasoftware.Opera", "Opera"),
    ("Vivaldi", "Vivaldi"),
];

/// 这台机器上可以清理的位置，只留下真的存在的。
pub fn specs() -> Vec<Spec> {
    let mut specs: Vec<Spec> = candidates()
        .into_iter()
        .map(|mut spec| {
            spec.roots.retain(|root| {
                deep_enough(&root.path)
                    && fs::symlink_metadata(&root.path).is_ok_and(|meta| meta.file_type().is_dir())
            });
            spec
        })
        .filter(|spec| !spec.roots.is_empty())
        .collect();
    specs.sort_by_key(|spec| spec.kind as u8);
    specs
}

#[allow(unused_mut, clippy::vec_init_then_push)]
fn candidates() -> Vec<Spec> {
    let mut specs: Vec<Spec> = Vec::new();
    let Some(home) = dirs::home_dir() else {
        return specs;
    };
    let mut spec = |kind: Kind, roots: Vec<Root>| specs.push(Spec { kind, roots });
    let plain = |paths: Vec<PathBuf>| paths.into_iter().map(Root::new).collect::<Vec<_>>();

    // 各系统都有的开发工具缓存：重新下载就有，只是要等一会儿。
    let mut developer = vec![
        ("Cargo", home.join(".cargo/registry/cache")),
        ("Cargo", home.join(".cargo/registry/src")),
        ("Cargo", home.join(".cargo/git/checkouts")),
        ("Cargo", home.join(".cargo/git/db")),
        ("Gradle", home.join(".gradle/caches")),
        ("Gradle", home.join(".gradle/wrapper/dists")),
        ("Go modules", home.join("go/pkg/mod/cache")),
        ("Bun", home.join(".bun/install/cache")),
    ];

    #[cfg(target_os = "macos")]
    {
        let library = home.join("Library");
        let caches = library.join("Caches");
        // 苹果自己的缓存不动：是系统服务在用，清了也腾不出多少。浏览器的另算。
        let keep = ["com.apple.*", "CloudKit"]
            .into_iter()
            .chain(BROWSER_DIRS.iter().map(|(dir, _)| *dir));
        spec(Kind::Caches, vec![Root::new(caches.clone()).keeping(keep)]);
        spec(
            Kind::Browsers,
            BROWSER_DIRS
                .iter()
                .map(|(dir, label)| Root::new(caches.join(dir)).labeled(label))
                .collect(),
        );
        spec(Kind::Logs, plain(vec![library.join("Logs")]));
        developer.extend([
            (
                "Xcode DerivedData",
                library.join("Developer/Xcode/DerivedData"),
            ),
            (
                "Xcode device support",
                library.join("Developer/Xcode/iOS DeviceSupport"),
            ),
            (
                "Xcode device support",
                library.join("Developer/Xcode/watchOS DeviceSupport"),
            ),
            (
                "Simulator caches",
                library.join("Developer/CoreSimulator/Caches"),
            ),
            ("Xcode caches", caches.join("com.apple.dt.Xcode")),
            ("npm", home.join(".npm/_cacache")),
            // uv 在 macOS 上也用 XDG 的缓存位置，不在 `Library/Caches` 里。
            ("uv", home.join(".cache/uv")),
        ]);
        spec(Kind::Trash, plain(vec![home.join(".Trash")]));
    }

    #[cfg(target_os = "linux")]
    {
        // `XDG_CACHE_HOME` 是用户自己设的：只有名字里带 cache 才信它，免得指到文档之类的地方去。
        let cache = dirs::cache_dir()
            .filter(|dir| named_like(dir, &["cache"]))
            .unwrap_or_else(|| home.join(".cache"));
        const BROWSERS: [(&str, &str); 7] = [
            ("google-chrome", "Chrome"),
            ("chromium", "Chromium"),
            ("mozilla", "Firefox"),
            ("BraveSoftware", "Brave"),
            ("microsoft-edge", "Edge"),
            ("vivaldi", "Vivaldi"),
            ("opera", "Opera"),
        ];
        spec(
            Kind::Caches,
            vec![Root::new(cache.clone()).keeping(BROWSERS.iter().map(|(dir, _)| *dir))],
        );
        spec(
            Kind::Browsers,
            BROWSERS
                .iter()
                .map(|(dir, label)| Root::new(cache.join(dir)).labeled(label))
                .collect(),
        );
        developer.push(("npm", home.join(".npm/_cacache")));
        if let Some(data) = dirs::data_local_dir() {
            spec(
                Kind::Trash,
                plain(vec![data.join("Trash/files"), data.join("Trash/info")]),
            );
        }
    }

    #[cfg(windows)]
    {
        if let Some(local) = dirs::data_local_dir() {
            spec(
                Kind::Caches,
                plain(vec![
                    local.join("Microsoft/Windows/INetCache"),
                    local.join("D3DSCache"),
                ]),
            );
            spec(
                Kind::Logs,
                plain(vec![
                    local.join("CrashDumps"),
                    local.join("Microsoft/Windows/WER"),
                ]),
            );
            // 浏览器的每个用户配置里，只有这几个文件夹是缓存。
            let mut browsers = Vec::new();
            for (label, user_data) in [
                ("Chrome", local.join("Google/Chrome/User Data")),
                ("Edge", local.join("Microsoft/Edge/User Data")),
            ] {
                let Ok(entries) = fs::read_dir(&user_data) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name == "Default" || name.starts_with("Profile ") {
                        for cache in ["Cache", "Code Cache", "GPUCache"] {
                            browsers.push(Root::new(entry.path().join(cache)).labeled(label));
                        }
                    }
                }
            }
            if let Ok(profiles) = fs::read_dir(local.join("Mozilla/Firefox/Profiles")) {
                for profile in profiles.flatten() {
                    browsers.push(Root::new(profile.path().join("cache2")).labeled("Firefox"));
                }
            }
            spec(Kind::Browsers, browsers);
            developer.extend([
                ("npm", local.join("npm-cache")),
                ("pip", local.join("pip/Cache")),
                ("NuGet", local.join("NuGet/v3-cache")),
                ("Go build cache", local.join("go-build")),
                ("uv", local.join("uv/cache")),
            ]);
        }
        // 用户的临时文件夹；刚刚还在用的（一天之内改过的）不碰。
        // `TEMP` 也是可以被改的，文件夹名字不像临时文件夹就不认。
        let temp = std::env::temp_dir();
        if named_like(&temp, &["temp", "tmp"]) {
            spec(Kind::Temp, vec![Root::new(temp).idle_for(DAY)]);
        }
    }

    spec(
        Kind::Developer,
        developer
            .into_iter()
            .map(|(label, path)| Root::new(path).labeled(label))
            .collect(),
    );
    specs
}

/// 位置里要处理的一项：它在位置的第一层。
struct Item {
    path: PathBuf,
    name: String,
}

/// 位置第一层里该处理的项目（去掉保留的、太新的）。
/// 位置本身必须是真正的文件夹；符号链接不算，免得顺着它清到别处去。
fn items(root: &Root) -> io::Result<Vec<Item>> {
    if !fs::symlink_metadata(&root.path)?.file_type().is_dir() {
        return Ok(Vec::new());
    }
    // 项目里的产物：项目最近动过就一概不碰。量和清都从这里过，所以它们看到的永远是同一批。
    if let Some(project) = &root.project
        && projects::recently_touched(project, &root.path)
    {
        return Ok(Vec::new());
    }
    let now = SystemTime::now();
    let mut found = Vec::new();
    for entry in fs::read_dir(&root.path)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if kept(&name, &root.keep) {
            continue;
        }
        if !root.idle.is_zero() {
            // 取不到修改时间、或者时间在「未来」的，一律当作刚改过。
            let recent = entry
                .metadata()
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| now.duration_since(time).ok())
                .is_none_or(|age| age < root.idle);
            if recent {
                continue;
            }
        }
        found.push(Item {
            path: entry.path(),
            name,
        });
    }
    Ok(found)
}

/// 量出来的结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    /// 能清出多少字节、涉及多少个文件。
    pub bytes: u64,
    pub files: u64,
    /// 有没有读不了的位置（没有权限）。
    pub blocked: bool,
    /// 占得最多的几项（名字，字节数），大的在前。位置有名字的按名字合起来算，否则是位置里各项自己的名字。
    pub biggest: Vec<(String, u64)>,
    /// 占得最多的那个位置。
    pub main: Option<PathBuf>,
}

/// 一类最多列几个「最大的项」。
const BIGGEST: usize = 3;

/// 量一类东西能清出多少。
pub fn measure(spec: &Spec, cancel: &AtomicBool) -> Measured {
    let mut out = Measured::default();
    let mut best = 0;
    let mut named: BTreeMap<String, u64> = BTreeMap::new();
    for root in &spec.roots {
        let items = match items(root) {
            Ok(items) => items,
            Err(error) => {
                out.blocked |= error.kind() == io::ErrorKind::PermissionDenied;
                continue;
            }
        };
        let sizes: Vec<(String, u64, u64)> = items
            .par_iter()
            .map(|item| {
                let (bytes, files) = space::usage(&item.path, cancel);
                (item.name.clone(), bytes, files)
            })
            .collect();
        let root_bytes: u64 = sizes.iter().map(|(_, bytes, _)| bytes).sum();
        if out.main.is_none() || root_bytes > best {
            best = root_bytes;
            out.main = Some(root.path.clone());
        }
        out.bytes += root_bytes;
        out.files += sizes.iter().map(|(_, _, files)| files).sum::<u64>();
        if root.label.is_empty() {
            for (name, bytes, _) in sizes {
                *named.entry(name).or_default() += bytes;
            }
        } else {
            *named.entry(root.label.clone()).or_default() += root_bytes;
        }
    }
    out.biggest = named.into_iter().filter(|(_, bytes)| *bytes > 0).collect();
    out.biggest
        .sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out.biggest.truncate(BIGGEST);
    out
}

/// 清理的进度，界面隔一会儿读一次。
#[derive(Default)]
pub struct Progress {
    /// 到目前为止释放了多少字节。
    pub freed: AtomicU64,
}

/// 清理的结果（按位置第一层的项目数）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub removed: u64,
    /// 删不掉的：被别的程序用着、没有权限、或者被系统保护。
    pub failed: u64,
}

/// 清理一类东西：删掉各个位置第一层里该处理的项目。删不掉的跳过，不中断。
/// 真正释放了多少，由调用方前后各量一次得出。
pub fn clean(spec: &Spec, progress: &Progress, cancel: &AtomicBool) -> Cleaned {
    let mut cleaned = Cleaned::default();
    for root in &spec.roots {
        let Ok(items) = items(root) else { continue };
        let results: Vec<Option<bool>> = items
            .par_iter()
            .map(|item| {
                if cancel.load(Relaxed) {
                    return None;
                }
                let (before, _) = space::usage(&item.path, cancel);
                let removed = remove(&item.path);
                let after = if removed {
                    0
                } else {
                    space::usage(&item.path, cancel).0
                };
                progress
                    .freed
                    .fetch_add(before.saturating_sub(after), Relaxed);
                Some(removed)
            })
            .collect();
        for result in results.into_iter().flatten() {
            if result {
                cleaned.removed += 1;
            } else {
                cleaned.failed += 1;
            }
        }
    }
    cleaned
}

/// 删一项：文件夹连同里面的一切；符号链接只删链接本身，不碰它指向的东西。
/// 已经不在了也算删掉了。
fn remove(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() => fs::remove_dir_all(path).is_ok(),
        Ok(_) => fs::remove_file(path).is_ok(),
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let path = std::env::temp_dir().join(format!("pcs-clean-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }

        fn write(&self, relative: &str, bytes: usize) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![1u8; bytes]).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn spec_of(root: Root) -> Spec {
        Spec {
            kind: Kind::Caches,
            roots: vec![root],
        }
    }

    fn never() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn only_the_inside_of_a_location_is_cleaned_and_kept_names_survive() {
        let dir = Scratch::new("inside");
        let cache = dir.0.join("cache");
        dir.write("cache/app-a/data.bin", 100_000);
        dir.write("cache/app-a/deep/more.bin", 50_000);
        dir.write("cache/loose.tmp", 20_000);
        dir.write("cache/com.apple.thing/keep.bin", 70_000);
        dir.write("cache/Google/keep.bin", 70_000);
        let spec = spec_of(Root::new(cache.clone()).keeping(["com.apple.*", "Google"]));

        let measured = measure(&spec, &never());
        assert!(measured.bytes >= 170_000 && measured.bytes < 170_000 + 64 * 1024);
        assert_eq!(measured.files, 3);
        assert_eq!(measured.biggest[0].0, "app-a");
        assert_eq!(measured.main.as_deref(), Some(cache.as_path()));

        let progress = Progress::default();
        let cleaned = clean(&spec, &progress, &never());
        assert_eq!(
            cleaned,
            Cleaned {
                removed: 2,
                failed: 0
            }
        );
        assert!(progress.freed.load(Relaxed) >= 170_000);
        // 位置本身还在，保留的东西原封不动。
        assert!(cache.is_dir());
        assert!(!cache.join("app-a").exists() && !cache.join("loose.tmp").exists());
        assert!(cache.join("com.apple.thing/keep.bin").is_file());
        assert!(cache.join("Google/keep.bin").is_file());
        assert_eq!(measure(&spec, &never()).bytes, 0);
    }

    #[test]
    fn labelled_locations_are_summed_under_their_label() {
        let dir = Scratch::new("labels");
        dir.write("a/cache/one.bin", 100_000);
        dir.write("a/code/two.bin", 100_000);
        dir.write("b/cache/three.bin", 20_000);
        let spec = Spec {
            kind: Kind::Browsers,
            roots: vec![
                Root::new(dir.0.join("a/cache")).labeled("Chrome"),
                Root::new(dir.0.join("a/code")).labeled("Chrome"),
                Root::new(dir.0.join("b/cache")).labeled("Firefox"),
            ],
        };
        let measured = measure(&spec, &never());
        let names: Vec<&str> = measured
            .biggest
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["Chrome", "Firefox"]);
        // 两个 Chrome 位置合成一项：大小是它们的和，并且比 Firefox 大。
        assert!(measured.biggest[0].1 >= 200_000 && measured.biggest[0].1 > measured.biggest[1].1);
        assert_eq!(
            measured.bytes,
            measured.biggest.iter().map(|(_, bytes)| bytes).sum::<u64>()
        );
        assert_eq!(measured.files, 3);
    }

    #[test]
    fn recently_changed_items_are_left_alone() {
        let dir = Scratch::new("idle");
        let old = dir.write("temp/old.bin", 10_000);
        dir.write("temp/new.bin", 10_000);
        // 把 old.bin 的修改时间拨到三天前。
        let three_days_ago = SystemTime::now() - 3 * DAY;
        File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(three_days_ago)
            .unwrap();
        let spec = spec_of(Root::new(dir.0.join("temp")).idle_for(DAY));
        assert_eq!(
            clean(&spec, &Progress::default(), &never()),
            Cleaned {
                removed: 1,
                failed: 0
            }
        );
        assert!(!dir.0.join("temp/old.bin").exists());
        assert!(dir.0.join("temp/new.bin").exists());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_removed_but_never_followed() {
        let dir = Scratch::new("links");
        let outside = dir.write("outside/precious.bin", 30_000);
        dir.write("cache/real.bin", 10_000);
        std::os::unix::fs::symlink(dir.0.join("outside"), dir.0.join("cache/link-to-dir")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.0.join("cache/link-to-file")).unwrap();
        let spec = spec_of(Root::new(dir.0.join("cache")));
        // 量的时候也不跟着链接走：链接指向的 30 000 字节不算。
        assert!(measure(&spec, &never()).bytes < 30_000);
        clean(&spec, &Progress::default(), &never());
        assert!(outside.is_file(), "what a link points to is untouched");
        assert!(fs::read_dir(dir.0.join("cache")).unwrap().next().is_none());

        // 位置本身是链接：什么也不清。
        std::os::unix::fs::symlink(dir.0.join("outside"), dir.0.join("alias")).unwrap();
        let spec = spec_of(Root::new(dir.0.join("alias")));
        assert_eq!(
            clean(&spec, &Progress::default(), &never()),
            Cleaned::default()
        );
        assert!(outside.is_file());
    }

    #[test]
    fn a_missing_or_cancelled_location_cleans_nothing() {
        let dir = Scratch::new("cancel");
        dir.write("cache/a.bin", 10_000);
        let missing = spec_of(Root::new(dir.0.join("nope")));
        assert_eq!(measure(&missing, &never()), Measured::default());
        assert_eq!(
            clean(&missing, &Progress::default(), &never()),
            Cleaned::default()
        );

        let spec = spec_of(Root::new(dir.0.join("cache")));
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            clean(&spec, &Progress::default(), &cancelled),
            Cleaned::default()
        );
        assert!(dir.0.join("cache/a.bin").exists());
    }

    #[test]
    fn locations_must_be_deep_enough_and_names_match_by_prefix_or_exactly() {
        let home = dirs::home_dir().unwrap();
        assert!(!deep_enough(Path::new("/")));
        assert!(!deep_enough(Path::new("/Users")));
        assert!(!deep_enough(&home));
        assert!(!deep_enough(Path::new("relative/path/to/cache")));
        assert!(deep_enough(&home.join("Library/Caches")));
        // 不拿真实的家目录拼：以 root 身份跑时它是 /root，本来就只有两层。
        assert!(deep_enough(Path::new("/home/someone/.cache")));

        assert!(kept("com.apple.Safari", &["com.apple.*"]));
        assert!(!kept("com.example", &["com.apple.*"]));
        assert!(kept("Google", &["Google"]));
        assert!(!kept("GoogleDrive", &["Google"]));

        // 环境变量指定的位置，名字不像缓存 / 临时文件夹就不认。
        assert!(named_like(Path::new("/home/me/.cache"), &["cache"]));
        assert!(named_like(
            Path::new("C:/Users/me/AppData/Local/Temp"),
            &["temp", "tmp"]
        ));
        assert!(!named_like(Path::new("/home/me/Documents"), &["cache"]));
        assert!(!named_like(Path::new("/"), &["cache"]));
    }

    #[test]
    fn the_real_whitelist_is_safe_and_counts_nothing_twice() {
        let specs = specs();
        let roots: Vec<&Root> = specs.iter().flat_map(|spec| &spec.roots).collect();
        for root in &roots {
            assert!(
                deep_enough(&root.path),
                "{} is too shallow",
                root.path.display()
            );
        }
        // 一个位置如果在另一个位置里面，外面那个必须把它让出来，否则同一批文件会算两遍、清两遍。
        for outer in &roots {
            for inner in &roots {
                if inner.path != outer.path
                    && let Ok(rest) = inner.path.strip_prefix(&outer.path)
                {
                    let first = rest
                        .components()
                        .next()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .unwrap();
                    assert!(
                        kept(&first, &outer.keep),
                        "{} is inside {} but not excluded",
                        inner.path.display(),
                        outer.path.display()
                    );
                }
            }
        }
        // 同一个位置不会出现在两类里。
        let mut seen = std::collections::HashSet::new();
        for root in &roots {
            assert!(
                seen.insert(&root.path),
                "{} listed twice",
                root.path.display()
            );
        }
    }
}

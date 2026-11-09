//! 已安装的应用：列出来，找出卸载之后会留在硬盘上的东西。
//!
//! - macOS：扫 `/Applications` 和 `~/Applications` 里的 `.app`，用 `Info.plist` 里的 Bundle ID 和名字，
//!   到 `~/Library` 里找属于它的数据、缓存、偏好设置……（`/Library` 里的只列出来，不动。）
//!   卸载就是把 `.app` 和勾选的残留移到废纸篓：移动本身由界面层做，这里只负责找。
//! - Windows：读注册表里的「卸载」项；卸载是启动应用自己登记的卸载程序，残留文件它自己会处理。
//! - 其他系统没有统一的「应用」概念（要看各家包管理器），列表是空的。
//!
//! 找残留宁缺毋滥，误删比漏删糟得多：只认「名字等于 Bundle ID（或以它加一个点开头）」和
//! 「名字和应用同名」，并且已经归属另一个已安装应用的不算。

#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(any(windows, test))]
mod registry;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(windows)]
use registry as platform;
#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use super::{App, Leftover};
    use std::sync::atomic::AtomicBool;

    pub fn list() -> Vec<App> {
        Vec::new()
    }

    pub fn size_of(_: &App, _: &AtomicBool) -> Option<u64> {
        None
    }

    pub fn leftovers(_: &App, _: &[App], _: &AtomicBool) -> Vec<Leftover> {
        Vec::new()
    }

    pub fn start_uninstaller(_: &App) -> Result<(), String> {
        Err("not supported on this system".into())
    }
}

/// 这个系统上有没有「应用」可管。没有的话界面不该把这一页摆出来。
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", windows));

/// 卸载是不是「把 .app 移到废纸篓」（macOS）。不是的话（Windows）就是启动应用自带的卸载程序。
pub const MOVES_TO_TRASH: bool = cfg!(target_os = "macos");

/// 一个已安装的应用。
#[derive(Clone, Debug)]
pub struct App {
    pub name: String,
    /// macOS：Bundle ID（有的应用没有，是空的）；Windows：注册表里那一项的名字。
    pub id: String,
    pub version: String,
    /// 发布者。macOS 的应用包里没有这一项。
    pub publisher: String,
    /// macOS：`.app` 的位置；Windows：安装目录（应用登记了才有）。
    pub path: Option<PathBuf>,
    /// 最近一次打开的时间（Unix 秒）。macOS 来自 Spotlight，查不到或 Windows 上是 `None`。
    pub last_used: Option<u64>,
    /// 系统登记的占用字节数（Windows 注册表里的估算值）。macOS 上没有，要用 [`size_of`] 量。
    #[cfg_attr(not(windows), allow(dead_code))]
    registered_size: Option<u64>,
    /// 找残留时除了文件名之外也认的名字（`CFBundleName` 等）。
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    names: Vec<String>,
    /// Windows：登记的卸载命令。
    #[cfg_attr(not(windows), allow(dead_code))]
    uninstall: String,
}

/// 卸载之后留在硬盘上的一个文件或文件夹。
#[derive(Clone, Debug)]
pub struct Leftover {
    pub path: PathBuf,
    /// 实际占用的字节数。
    pub bytes: u64,
    /// 在所有用户共用的位置（`/Library`）：删它要管理员权限，这里只列出来。
    pub system: bool,
}

/// 读出已安装的应用，按名字排好。
pub fn list() -> Vec<App> {
    let mut apps = platform::list();
    apps.sort_by_cached_key(|app| app.name.to_lowercase());
    apps
}

/// 一个应用占多少字节。量不出来（Windows 上应用没登记）就是 `None`。
pub fn size_of(app: &App, cancel: &AtomicBool) -> Option<u64> {
    platform::size_of(app, cancel)
}

/// 找出卸载这个应用之后会留下的东西。`installed` 是全部已安装的应用，用来排除属于别人的。
/// 先是用户自己目录里的，再是系统共用位置的；各自按大小从大到小。
pub fn leftovers(app: &App, installed: &[App], cancel: &AtomicBool) -> Vec<Leftover> {
    platform::leftovers(app, installed, cancel)
}

/// 应用此刻是不是正开着：它的任何一个进程都跑在应用自己的目录里。
pub fn is_running(app: &App) -> bool {
    let Some(root) = app.path.as_deref().filter(|path| is_deep_enough(path)) else {
        return false;
    };
    let Ok(mut system) = System::new() else {
        return false;
    };
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    system
        .processes()
        .values()
        .any(|process| process.exe().is_some_and(|exe| exe.starts_with(root)))
}

/// 启动应用登记的卸载程序（Windows）。不等它结束：它有自己的界面，可能要用户点好几步。
pub fn start_uninstaller(app: &App) -> Result<(), String> {
    platform::start_uninstaller(app)
}

/// 路径够深才拿来比较进程：安装目录登记成 `C:\` 或 `C:\Program Files` 的话，
/// 那就成了「机器上所有程序」。
fn is_deep_enough(path: &Path) -> bool {
    path.is_absolute() && path.components().count() >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broad_directories_are_not_used_to_tell_whether_an_app_runs() {
        assert!(!is_deep_enough(Path::new("/")));
        assert!(!is_deep_enough(Path::new("/Applications")));
        assert!(is_deep_enough(Path::new("/Applications/Foo.app")));
        assert!(!is_deep_enough(Path::new("relative/Foo.app")));
    }

    #[test]
    fn an_app_without_a_location_is_never_running() {
        let app = App {
            name: "Ghost".into(),
            id: String::new(),
            version: String::new(),
            publisher: String::new(),
            path: None,
            last_used: None,
            registered_size: None,
            names: Vec::new(),
            uninstall: String::new(),
        };
        assert!(!is_running(&app));
    }
}

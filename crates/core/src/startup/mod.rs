//! 启动项：开机 / 登录时会自己跑起来的东西。
//!
//! 只做可以随时反悔的开关：用各系统自己的「停用」机制（launchd 的 disable、注册表的 StartupApproved、
//! autostart 的 Hidden），不删配置文件、不删注册表项。各系统的细节在子模块里：
//! macOS 的登录项和 launchd 代理 / 守护进程、Windows 的 Run 注册表项和启动文件夹、Linux 的 autostart。

#[cfg(any(target_os = "linux", test))]
mod xdg;

#[cfg(any(target_os = "macos", test))]
mod launchd;

#[cfg(any(windows, test))]
mod registry;

use std::path::PathBuf;

// 各系统的实现统一成 `platform::list` / `platform::set_enabled`。
#[cfg(target_os = "macos")]
use launchd as platform;
#[cfg(windows)]
use registry as platform;
#[cfg(target_os = "linux")]
use xdg as platform;
#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
mod platform {
    pub fn list() -> super::Listing {
        super::Listing::default()
    }

    pub fn set_enabled(_: &super::Entry, _: bool) -> Result<(), String> {
        Err("not supported on this system".into())
    }
}

/// 启动项从哪里来。顺序就是界面上分组的顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Source {
    /// macOS：系统设置 → 通用 → 登录项。
    LoginItem,
    /// macOS：launchd 用户代理，登录后跑。
    LaunchAgent,
    /// macOS：launchd 守护进程，开机就跑，改它要管理员权限。
    LaunchDaemon,
    /// Windows：注册表的 Run 项。
    Registry,
    /// Windows：「启动」文件夹。
    StartupFolder,
    /// Linux：autostart 目录里的 .desktop 文件。
    Autostart,
}

/// 它什么时候跑。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// 登录（或开机）时启动。
    Login,
    /// 一直保持运行，退出了系统会再拉起来。
    KeepAlive,
    /// 按固定间隔或日程运行。
    Schedule,
    /// 有程序来找它、或者某个事件发生时才启动。
    OnDemand,
}

/// 一个启动项。
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    /// 技术上的标识（launchd 的 Label、.desktop 的文件名）：名字重复时靠它分辨；没有就是空的。
    pub id: String,
    /// 它跑的命令（或应用的路径）。
    pub command: String,
    pub source: Source,
    pub trigger: Trigger,
    pub enabled: bool,
    /// 此刻正在运行（只有 launchd 的用户代理查得到）。
    pub running: bool,
    /// 放在所有用户共用的位置：在这里关掉，只对当前用户生效。
    pub system: bool,
    /// 能不能在这里开关（macOS 的守护进程要管理员权限，这里不碰）。
    pub editable: bool,
    /// 配置文件（或它跑的程序）的位置，可以在文件管理器里显示。
    pub path: Option<PathBuf>,
    /// 开关它要用到的标识：launchd 的 Label、注册表的值名、登录项的应用路径、.desktop 的文件名……
    key: String,
}

/// 读出来的全部启动项。
#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// macOS：没有「系统事件」的自动化授权，读不到登录项。
    pub login_items_denied: bool,
}

/// 读出这台电脑上的启动项：先按来源分组，组内按名字排。
pub fn list() -> Listing {
    let mut listing = platform::list();
    listing
        .entries
        .sort_by_cached_key(|entry| (entry.source, entry.name.to_lowercase()));
    listing
}

/// 打开或关掉一个启动项。改动从下一次登录起生效，正在运行的不会被停掉。
pub fn set_enabled(entry: &Entry, enabled: bool) -> Result<(), String> {
    if !entry.editable {
        return Err("this item can only be changed by an administrator".into());
    }
    platform::set_enabled(entry, enabled)
}

/// 从一条命令行里取出可执行文件的路径：带引号的取引号里的；不带的取到第一个 `.exe` 为止
/// （Windows 的路径常常有空格却不加引号），没有 `.exe` 就取第一个空格之前。
#[cfg(any(windows, test))]
fn executable_of(command: &str) -> Option<PathBuf> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(PathBuf::from(&rest[..end]));
    }
    let lower = command.to_ascii_lowercase();
    let end = lower
        .find(".exe")
        .map(|at| at + 4)
        .or_else(|| command.find(' '))
        .unwrap_or(command.len());
    let path = command[..end].trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_program_is_found_in_a_command_line() {
        let path = |text: &str| executable_of(text).unwrap();
        assert_eq!(
            path(r#""C:\Program Files\App\app.exe" --background"#),
            Path::new(r"C:\Program Files\App\app.exe")
        );
        assert_eq!(
            path(r"C:\Program Files\App\app.exe --background"),
            Path::new(r"C:\Program Files\App\app.exe")
        );
        assert_eq!(path("/usr/bin/thing --flag"), Path::new("/usr/bin/thing"));
        assert_eq!(path("thing"), Path::new("thing"));
        assert!(executable_of("   ").is_none());
        assert!(executable_of("\"unterminated").is_none());
    }

    #[test]
    fn groups_come_in_the_order_the_page_shows_them() {
        assert!(Source::LoginItem < Source::LaunchAgent);
        assert!(Source::LaunchAgent < Source::LaunchDaemon);
        assert!(Source::LaunchDaemon < Source::Registry);
        assert!(Source::Registry < Source::StartupFolder);
        assert!(Source::StartupFolder < Source::Autostart);
    }
}

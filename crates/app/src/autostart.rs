//! 开机自启：macOS 写 LaunchAgent（沙盒里的商店版写不了它，改用系统的登录项 `SMAppService`），
//! Windows 写当前用户的注册表 Run 键，Linux 写 XDG autostart。
//!
//! 是否已开启以系统里的登记为准（用户可能在系统设置里手动关掉过），所以不存进设置文件。
//! 自启一律由用户在设置里主动打开，默认关着。

#[cfg(target_os = "macos")]
use auto_launch::MacOSLaunchMode;
use auto_launch::{AutoLaunch, AutoLaunchBuilder, WindowsEnableMode};
#[cfg(target_os = "macos")]
use pc_specs_core::edition;

/// 开机自启时附带的参数：有托盘就只留在托盘里，不弹主窗口。
pub const BACKGROUND_ARG: &str = "--background";

const APP_ID: &str = "com.xwzy.pc-specs";

/// macOS 的 plist 里每个参数各占一项，不能加引号；Windows 的 Run 键和 Linux 的 desktop 文件里
/// 命令行是一整段文本，路径带空格就必须加引号，否则启动不起来。
fn command_path(exe: &str) -> String {
    if cfg!(target_os = "macos") || !exe.contains(char::is_whitespace) {
        exe.to_string()
    } else {
        format!("\"{exe}\"")
    }
}

fn handle() -> Option<AutoLaunch> {
    let exe = std::env::current_exe().ok()?;
    let exe = command_path(exe.to_str()?);
    // macOS 的名字会成为 LaunchAgent 的 Label 和文件名；Windows / Linux 的名字会显示在系统的「启动项」里。
    let name = if cfg!(target_os = "macos") {
        APP_ID
    } else {
        "PC Specs"
    };
    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(name)
        .set_app_path(&exe)
        .set_args(&[BACKGROUND_ARG])
        .set_bundle_identifiers(&[APP_ID])
        .set_windows_enable_mode(WindowsEnableMode::CurrentUser);
    // 沙盒里写不进 `~/Library/LaunchAgents`（写进去的是应用自己的容器，launchd 不会去读）。
    // 登录项登记的是正在运行的这个应用，不带命令行参数，是不是登录时启动的靠 `launched_at_login` 判断。
    #[cfg(target_os = "macos")]
    if edition::SANDBOXED {
        builder.set_macos_launch_mode(MacOSLaunchMode::SMAppService);
    }
    builder.build().ok()
}

/// 这一次是不是系统在用户登录时拉起来的。要在应用刚启动、还在处理启动事件的时候问才有答案。
///
/// 带 `--background` 参数的是 LaunchAgent / 注册表 / XDG 的自启；macOS 的登录项（`SMAppService`
/// 登记的，或者用户自己拖进「登录项」的）带不了参数，要看系统的启动事件里有没有「作为登录项启动」的标记。
pub fn launched_at_login() -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::NSAppleEventManager;
        // AppleEvents.h 里的四字符代码：`kAEOpenApplication`、`keyAEPropData`、`keyAELaunchedAsLogInItem`。
        const OPEN_APPLICATION: u32 = u32::from_be_bytes(*b"oapp");
        const PROP_DATA: u32 = u32::from_be_bytes(*b"prdt");
        const AS_LOGIN_ITEM: u32 = u32::from_be_bytes(*b"lgit");
        let Some(event) = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent() else {
            return false;
        };
        event.eventID() == OPEN_APPLICATION
            && event
                .paramDescriptorForKeyword(PROP_DATA)
                .is_some_and(|data| data.enumCodeValue() == AS_LOGIN_ITEM)
    }
    #[cfg(not(target_os = "macos"))]
    false
}

pub fn is_enabled() -> bool {
    handle().is_some_and(|auto| auto.is_enabled().unwrap_or(false))
}

pub fn set_enabled(enabled: bool) -> Result<(), String> {
    let auto = handle().ok_or_else(|| "cannot locate the executable".to_string())?;
    let result = if enabled {
        auto.enable()
    } else {
        auto.disable()
    };
    result.map_err(|error| error.to_string())
}

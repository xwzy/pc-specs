//! 开机自启：macOS 写 LaunchAgent，Windows 写当前用户的注册表 Run 键，Linux 写 XDG autostart。
//!
//! 是否已开启以系统里的登记为准（用户可能在系统设置里手动关掉过），所以不存进设置文件。

use auto_launch::{AutoLaunch, AutoLaunchBuilder, WindowsEnableMode};

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
    builder.build().ok()
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

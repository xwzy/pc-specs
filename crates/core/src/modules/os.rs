use crate::model::OsInfo;
use sysinfo::System;

pub fn collect() -> OsInfo {
    // os_info 在 Linux / macOS 上会拉起 lsb_release / sw_vers，仅在 sysinfo 取不到时才用。
    let name = System::long_os_version()
        .or_else(|_| System::name())
        .unwrap_or_else(|_| os_info::get().os_type().to_string());
    let version = System::os_version().unwrap_or_else(|_| os_info::get().version().to_string());

    OsInfo {
        family: std::env::consts::OS.to_string(),
        name,
        version,
        kernel: System::kernel_version().unwrap_or_else(|_| "unknown".to_string()),
        arch: std::env::consts::ARCH.to_string(),
        locale: locale(),
        shell: std::env::var("SHELL")
            .ok()
            .or_else(|| std::env::var("ComSpec").ok()),
        desktop: crate::platform::desktop_env(),
    }
}

/// 系统界面语言。从 Dock / 开始菜单启动的 GUI 进程通常没有 LANG 环境变量，
/// 所以优先询问操作系统（NSLocale / GetUserPreferredUILanguages），环境变量垫底。
fn locale() -> String {
    sys_locale::get_locale()
        .or_else(|| {
            ["LC_ALL", "LANG"]
                .iter()
                .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        })
        .unwrap_or_else(|| "en-US".to_string())
}

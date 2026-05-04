use crate::model::OsInfo;
use sysinfo::System;

pub fn collect() -> OsInfo {
    let info = os_info::get();
    let family = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();

    let kernel = System::kernel_version().unwrap_or_else(|| "unknown".to_string());
    let name = System::long_os_version().unwrap_or_else(|| info.os_type().to_string());
    let version = System::os_version().unwrap_or_else(|| info.version().to_string());

    let locale = sys_locale_or_default();
    let shell = std::env::var("SHELL").ok().or_else(|| std::env::var("ComSpec").ok());
    let desktop = crate::platform::desktop_env();

    OsInfo {
        family,
        name,
        version,
        kernel,
        arch,
        locale,
        shell,
        desktop,
    }
}

fn sys_locale_or_default() -> String {
    if let Ok(v) = std::env::var("LC_ALL") {
        if !v.is_empty() {
            return v;
        }
    }
    if let Ok(v) = std::env::var("LANG") {
        if !v.is_empty() {
            return v;
        }
    }
    // whoami::langs() 在 Windows 通过 GetUserPreferredUILanguages 工作，
    // 在 macOS 通过 NSLocale，在 Linux 仍读 LC_*。
    // 注意 langs() 返回 Result<Iterator>，我们取首个 BCP-47 标签
    if let Ok(mut it) = whoami::langs() {
        if let Some(lang) = it.next() {
            return lang.to_string();
        }
    }
    "en-US".to_string()
}

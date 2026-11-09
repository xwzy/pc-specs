//! Windows：从注册表的「卸载」项读出已安装的程序。卸载就是启动它自己登记的卸载程序——
//! 安装目录、注册表、快捷方式这些，只有它自己清得干净，所以这里不找残留。

use super::App;
#[cfg(windows)]
use super::Leftover;
#[cfg(windows)]
use std::sync::atomic::AtomicBool;

/// 一项卸载登记里和我们有关的部分。
#[derive(Default)]
struct Record {
    key: String,
    name: String,
    version: String,
    publisher: String,
    location: String,
    uninstall: String,
    /// 登记的占用估算，单位 KB；没登记是 0。
    size_kb: u64,
    /// 系统组件、某个程序的补丁、不让卸载的：设置里的「应用」不列，这里也不列。
    hidden: bool,
}

/// 设置里的「应用」会列出的才算：有名字、有卸载命令，不是系统组件，也不是补丁。
fn usable(record: &Record) -> bool {
    !record.hidden && !record.name.trim().is_empty() && !record.uninstall.trim().is_empty()
}

fn into_app(record: Record) -> App {
    let location = record.location.trim().trim_matches('"');
    App {
        names: vec![record.name.clone()],
        name: record.name.trim().to_string(),
        id: record.key,
        version: record.version.trim().to_string(),
        publisher: record.publisher.trim().to_string(),
        path: (!location.is_empty()).then(|| location.into()),
        last_used: None,
        registered_size: (record.size_kb > 0).then(|| record.size_kb.saturating_mul(1024)),
        uninstall: record.uninstall.trim().to_string(),
    }
}

/// 同一个程序常常在 32 位和 64 位两处都登记了一遍：名字和版本一样的只留第一个。
fn dedupe(apps: &mut Vec<App>) {
    let mut seen = std::collections::HashSet::new();
    apps.retain(|app| seen.insert((app.name.to_lowercase(), app.version.clone())));
}

#[cfg(windows)]
pub fn list() -> Vec<App> {
    let mut apps: Vec<App> = sys::read()
        .into_iter()
        .filter(usable)
        .map(into_app)
        .collect();
    dedupe(&mut apps);
    apps
}

#[cfg(windows)]
pub fn size_of(app: &App, _: &AtomicBool) -> Option<u64> {
    app.registered_size
}

#[cfg(windows)]
pub fn leftovers(_: &App, _: &[App], _: &AtomicBool) -> Vec<Leftover> {
    Vec::new()
}

/// 用 `start` 去打开卸载命令：它走系统的「打开」，卸载程序要管理员权限时会弹出授权框；
/// 直接创建进程的话会因为需要提升权限而失败。`start` 马上就返回，卸载程序自己在界面里跑。
#[cfg(windows)]
pub fn start_uninstaller(app: &App) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("cmd")
        .arg("/C")
        // 命令行原样交给 cmd，不让 Rust 再按 CreateProcess 的规则转义一遍。
        .raw_arg(format!("start \"\" {}", app.uninstall))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
mod sys {
    use super::Record;
    use winreg::enums::KEY_READ;
    use winreg::{HKCU, HKLM, RegKey};

    const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    const UNINSTALL_32: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";

    /// 补丁、更新之类的发行类型：设置里的「应用」不列它们。
    const UPDATES: [&str; 4] = ["Hotfix", "Security Update", "Update Rollup", "ServicePack"];

    pub(super) fn read() -> Vec<Record> {
        let mut records = Vec::new();
        for (hive, path) in [(HKLM, UNINSTALL), (HKLM, UNINSTALL_32), (HKCU, UNINSTALL)] {
            let Ok(root) = hive.open_subkey_with_flags(path, KEY_READ) else {
                continue;
            };
            for name in root.enum_keys().flatten() {
                if let Ok(key) = root.open_subkey_with_flags(&name, KEY_READ) {
                    records.push(record(&name, &key));
                }
            }
        }
        records
    }

    fn record(name: &str, key: &RegKey) -> Record {
        let text = |value: &str| key.get_value::<String, _>(value).unwrap_or_default();
        let number = |value: &str| key.get_value::<u32, _>(value).unwrap_or(0);
        let release = text("ReleaseType");
        Record {
            key: name.to_string(),
            name: text("DisplayName"),
            version: text("DisplayVersion"),
            publisher: text("Publisher"),
            location: text("InstallLocation"),
            uninstall: text("UninstallString"),
            size_kb: u64::from(number("EstimatedSize")),
            hidden: number("SystemComponent") == 1
                || number("NoRemove") == 1
                || !text("ParentKeyName").is_empty()
                || UPDATES.contains(&release.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, uninstall: &str) -> Record {
        Record {
            key: "{GUID}".into(),
            name: name.into(),
            version: "1.0".into(),
            uninstall: uninstall.into(),
            ..Record::default()
        }
    }

    #[test]
    fn only_programs_the_settings_app_would_list_are_kept() {
        assert!(usable(&record("Foo", "C:\\Foo\\unins000.exe")));
        assert!(!usable(&record("", "C:\\Foo\\unins000.exe")));
        assert!(!usable(&record("  ", "C:\\Foo\\unins000.exe")));
        // 没有卸载命令就没法卸载。
        assert!(!usable(&record("Foo", "")));
        let mut component = record("Foo", "x");
        component.hidden = true;
        assert!(!usable(&component));
    }

    #[test]
    fn a_registration_becomes_an_app() {
        let mut registration = record(" Foo ", "  MsiExec.exe /X{GUID} ");
        registration.publisher = " Acme ".into();
        registration.location = "\"C:\\Program Files\\Foo\"".into();
        registration.size_kb = 2048;
        let app = into_app(registration);
        assert_eq!(app.name, "Foo");
        assert_eq!(app.publisher, "Acme");
        assert_eq!(app.uninstall, "MsiExec.exe /X{GUID}");
        assert_eq!(app.registered_size, Some(2 * 1024 * 1024));
        assert_eq!(
            app.path.as_deref(),
            Some(std::path::Path::new("C:\\Program Files\\Foo"))
        );

        // 没登记安装目录和大小的，就是没有。
        let bare = into_app(record("Bar", "x"));
        assert!(bare.path.is_none());
        assert!(bare.registered_size.is_none());
    }

    #[test]
    fn the_same_program_registered_twice_is_listed_once() {
        let mut apps = vec![
            into_app(record("Foo", "a")),
            into_app(record("FOO", "b")),
            into_app(record("Bar", "c")),
        ];
        apps[2].version = "2.0".into();
        dedupe(&mut apps);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].uninstall, "a");
    }
}

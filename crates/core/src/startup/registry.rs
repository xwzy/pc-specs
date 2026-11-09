//! Windows：注册表的 Run 项和「启动」文件夹。
//!
//! 开关用的是任务管理器「启动」页同一套机制：`StartupApproved` 下每一项有个 12 字节的值，
//! 第一个字节的最低位是 1 就是停用。Run 项和启动文件夹里的文件本身一个字不动，所以随时可以再打开。

#![cfg_attr(not(windows), allow(dead_code))]

/// `StartupApproved` 里的值：前 4 个字节是状态，后 8 个字节是停用的时间（可以全 0）。
const ENABLED: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const DISABLED: [u8; 12] = [3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

/// 这个值表示的是不是「停用」：状态的最低位为 1（任务管理器停用时写 3，启用时是 2 或 6）。
fn disabled(data: &[u8]) -> bool {
    data.first().is_some_and(|state| state & 1 == 1)
}

#[cfg(windows)]
pub(super) use sys::{list, set_enabled};

#[cfg(windows)]
mod sys {
    use super::{DISABLED, ENABLED, disabled};
    use crate::startup::{Entry, Listing, Source, Trigger, executable_of};
    use std::borrow::Cow;
    use std::path::PathBuf;
    use winreg::enums::{KEY_READ, KEY_SET_VALUE, REG_BINARY};
    use winreg::types::FromRegValue as _;
    use winreg::{HKCU, HKLM, RegKey, RegValue};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_32: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

    pub fn list() -> Listing {
        let mut entries = Vec::new();
        // （根键，Run 项的位置，StartupApproved 下对应的子键，是不是所有用户共用的）
        let places: [(&RegKey, &str, &str, bool); 3] = [
            (HKCU, RUN, "Run", false),
            (HKLM, RUN, "Run", true),
            (HKLM, RUN_32, "Run32", true),
        ];
        for (root, path, approved, system) in places {
            let Ok(key) = root.open_subkey_with_flags(path, KEY_READ) else {
                continue;
            };
            for (name, value) in key.enum_values().flatten() {
                let command = String::from_reg_value(&value).unwrap_or_default();
                entries.push(Entry {
                    name: name.clone(),
                    id: String::new(),
                    path: executable_of(&command).filter(|path| path.exists()),
                    command,
                    source: Source::Registry,
                    trigger: Trigger::Login,
                    enabled: !is_disabled(approved, &name),
                    running: false,
                    system,
                    editable: true,
                    key: format!(r"{approved}\{name}"),
                });
            }
        }

        // 「启动」文件夹：当前用户的，和所有用户共用的。
        let mut folders: Vec<(PathBuf, bool)> = Vec::new();
        if let Some(dir) = dirs::data_dir() {
            folders.push((
                dir.join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
                false,
            ));
        }
        if let Some(dir) = std::env::var_os("ProgramData") {
            folders.push((
                PathBuf::from(dir).join(r"Microsoft\Windows\Start Menu\Programs\StartUp"),
                true,
            ));
        }
        for (dir, system) in folders {
            for item in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let file = item.file_name().to_string_lossy().into_owned();
                if file.eq_ignore_ascii_case("desktop.ini") {
                    continue;
                }
                let path = item.path();
                entries.push(Entry {
                    id: String::new(),
                    name: path
                        .file_stem()
                        .map_or_else(|| file.clone(), |stem| stem.to_string_lossy().into_owned()),
                    command: path.display().to_string(),
                    source: Source::StartupFolder,
                    trigger: Trigger::Login,
                    enabled: !is_disabled("StartupFolder", &file),
                    running: false,
                    system,
                    editable: true,
                    path: Some(path),
                    key: format!(r"StartupFolder\{file}"),
                });
            }
        }
        Listing {
            entries,
            login_items_denied: false,
        }
    }

    /// 当前用户的设置优先，没有再看所有用户共用的；都没有就是启用。
    fn is_disabled(approved: &str, name: &str) -> bool {
        let path = format!(r"{APPROVED}\{approved}");
        for root in [HKCU, HKLM] {
            if let Ok(key) = root.open_subkey_with_flags(&path, KEY_READ)
                && let Ok(value) = key.get_raw_value(name)
            {
                return disabled(&value.bytes);
            }
        }
        false
    }

    /// 写到当前用户下：改所有用户共用的项要管理员权限，这里只对当前用户生效。
    pub fn set_enabled(entry: &Entry, enabled: bool) -> Result<(), String> {
        let (approved, name) = entry
            .key
            .split_once('\\')
            .ok_or_else(|| "unrecognised startup item".to_string())?;
        let path = format!(r"{APPROVED}\{approved}");
        let (key, _) = HKCU
            .create_subkey_with_flags(path, KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        let data = if enabled { ENABLED } else { DISABLED };
        key.set_raw_value(
            name,
            &RegValue {
                bytes: Cow::Borrowed(&data),
                vtype: REG_BINARY,
            },
        )
        .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_byte_decides_whether_an_item_is_off() {
        assert!(disabled(&DISABLED));
        assert!(!disabled(&ENABLED));
        // 任务管理器用过的各种值：2 和 6 是启用，3 是停用。
        assert!(!disabled(&[6, 0, 0, 0]));
        assert!(disabled(&[3, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8]));
        // 空的值当作没设置。
        assert!(!disabled(&[]));
        assert_eq!(ENABLED.len(), 12);
        assert_eq!(DISABLED.len(), 12);
    }
}

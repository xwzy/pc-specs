//! Linux 的 autostart：用户目录（`~/.config/autostart`）和系统目录（`/etc/xdg/autostart`）里的 .desktop 文件。
//!
//! 同名文件用户目录的盖过系统目录的。停用靠 `Hidden=true`：系统的那份不能改，
//! 就在用户目录里放一份带 `Hidden=true` 的副本；重新启用时把这份副本删掉，系统的那份就又生效了。

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::{Entry, Listing, Source, Trigger};
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// .desktop 文件里 `[Desktop Entry]` 一组的内容。
#[derive(Default)]
struct Desktop {
    name: Option<String>,
    exec: Option<String>,
    hidden: bool,
}

/// 解析一个 .desktop 文件。不是应用（`Type` 不是 `Application`）就是 `None`。
/// 只给「盖住系统那份」用的文件没有 `Exec`，也照样解析出来。
fn parse(text: &str) -> Option<Desktop> {
    let mut in_entry = false;
    let mut found = Desktop::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Type" if value != "Application" => return None,
            "Name" => found.name = Some(value.to_string()),
            "Exec" => found.exec = Some(value.to_string()),
            "Hidden" => found.hidden |= value.eq_ignore_ascii_case("true"),
            "X-GNOME-Autostart-enabled" => found.hidden |= value.eq_ignore_ascii_case("false"),
            _ => {}
        }
    }
    Some(found)
}

/// 把文件改成停用（`Hidden=true`）或启用（去掉 `Hidden` 和 GNOME 的 `X-GNOME-Autostart-enabled`），其余内容原样保留。
fn with_hidden(text: &str, hidden: bool) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut in_entry = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_entry = trimmed == "[Desktop Entry]";
            out.push(line);
            if in_entry && hidden {
                out.push("Hidden=true");
            }
            continue;
        }
        let setting = in_entry
            && trimmed.split_once('=').is_some_and(|(key, _)| {
                matches!(key.trim(), "Hidden" | "X-GNOME-Autostart-enabled")
            });
        if !setting {
            out.push(line);
        }
    }
    let mut result = out.join("\n");
    result.push('\n');
    result
}

/// 用户目录里的这份是不是只为盖住系统的那份而存在：没有 `Exec`，或者除了启用状态外和系统的一模一样。
fn is_override(user: &str, system: &str) -> bool {
    parse(user).is_some_and(|desktop| desktop.exec.is_none())
        || with_hidden(user, false).trim() == with_hidden(system, false).trim()
}

fn user_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("autostart"))
}

/// 系统目录：`XDG_CONFIG_DIRS`（默认 `/etc/xdg`）下的 `autostart`。
fn system_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_CONFIG_DIRS")
        .ok()
        .filter(|dirs| !dirs.trim().is_empty())
        .unwrap_or_else(|| "/etc/xdg".into());
    dirs.split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join("autostart"))
        .collect()
}

pub(super) fn list() -> Listing {
    Listing {
        entries: entries(user_dir().as_deref(), &system_dirs()),
        login_items_denied: false,
    }
}

/// 两类目录里所有的 autostart 项，同名的以用户目录的为准。
fn entries(user_dir: Option<&Path>, system_dirs: &[PathBuf]) -> Vec<Entry> {
    let desktop_files = |dir: &Path| -> Vec<String> {
        fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".desktop"))
            .collect()
    };
    let mut names: BTreeSet<String> = BTreeSet::new();
    names.extend(user_dir.into_iter().flat_map(desktop_files));
    for dir in system_dirs {
        names.extend(desktop_files(dir));
    }

    let mut entries = Vec::new();
    for file in names {
        let user_path = user_dir.map(|dir| dir.join(&file)).filter(|p| p.is_file());
        let system_path = system_dirs
            .iter()
            .map(|dir| dir.join(&file))
            .find(|p| p.is_file());
        let read = |path: &Option<PathBuf>| {
            path.as_ref()
                .and_then(|path| fs::read_to_string(path).ok())
                .and_then(|text| parse(&text))
        };
        let (top, base) = (read(&user_path), read(&system_path));
        // 用户目录的盖住系统的：没写的项（比如只有 Hidden 的副本）用系统那份补。
        let name = top
            .as_ref()
            .and_then(|d| d.name.clone())
            .or_else(|| base.as_ref().and_then(|d| d.name.clone()));
        let exec = top
            .as_ref()
            .and_then(|d| d.exec.clone())
            .or_else(|| base.as_ref().and_then(|d| d.exec.clone()));
        let hidden = top.as_ref().or(base.as_ref()).is_some_and(|d| d.hidden);
        let Some(exec) = exec else { continue };
        entries.push(Entry {
            name: name.unwrap_or_else(|| file.trim_end_matches(".desktop").to_string()),
            id: file.clone(),
            command: exec,
            source: Source::Autostart,
            trigger: Trigger::Login,
            enabled: !hidden,
            running: false,
            system: user_path.is_none(),
            editable: true,
            path: user_path.clone().or(system_path),
            key: file,
        });
    }
    entries
}

pub(super) fn set_enabled(entry: &Entry, enabled: bool) -> Result<(), String> {
    let user_dir = user_dir().ok_or_else(|| "there is no user configuration folder".to_string())?;
    apply(&user_dir, &system_dirs(), &entry.key, enabled).map_err(|e| e.to_string())
}

/// 对名叫 `file` 的那一项做启用 / 停用。
fn apply(user_dir: &Path, system_dirs: &[PathBuf], file: &str, enabled: bool) -> io::Result<()> {
    let user_path = user_dir.join(file);
    let system_text = system_dirs
        .iter()
        .map(|dir| dir.join(file))
        .find_map(|path| fs::read_to_string(path).ok());
    let user_text = fs::read_to_string(&user_path).ok();

    match (user_text, system_text, enabled) {
        // 用户目录里有这一份：改它的状态；它只是盖住系统那份的，启用时直接删掉。
        (Some(user), system, true) => {
            if system
                .as_deref()
                .is_some_and(|system| is_override(&user, system))
            {
                fs::remove_file(&user_path)
            } else {
                fs::write(&user_path, with_hidden(&user, false))
            }
        }
        (Some(user), _, false) => fs::write(&user_path, with_hidden(&user, true)),
        // 只有系统有这一份：启用就是本来的样子；停用要在用户目录放一份副本。
        (None, Some(_), true) => Ok(()),
        (None, Some(system), false) => {
            fs::create_dir_all(user_dir)?;
            fs::write(&user_path, with_hidden(&system, true))
        }
        (None, None, _) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{file} no longer exists"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEM: &str = "[Desktop Entry]\nType=Application\nName=Sync\nExec=/usr/bin/sync-daemon --quiet\nIcon=sync\n";

    struct Dirs(PathBuf);

    impl Dirs {
        fn new(tag: &str) -> Dirs {
            let root = std::env::temp_dir().join(format!("pcs-xdg-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("user")).unwrap();
            fs::create_dir_all(root.join("system")).unwrap();
            Dirs(root)
        }

        fn user(&self) -> PathBuf {
            self.0.join("user")
        }

        fn system(&self) -> Vec<PathBuf> {
            vec![self.0.join("system")]
        }

        fn put(&self, side: &str, file: &str, text: &str) {
            fs::write(self.0.join(side).join(file), text).unwrap();
        }

        fn list(&self) -> Vec<Entry> {
            entries(Some(&self.user()), &self.system())
        }
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_desktop_entry_group_is_read_and_other_groups_are_not() {
        let text = "[Desktop Entry]\nType=Application\nName=App\nName[zh_CN]=应用\nExec=app %U\nX-GNOME-Autostart-enabled=false\n\n[Desktop Action new]\nName=Other\nExec=other\nHidden=true\n";
        let desktop = parse(text).unwrap();
        assert_eq!(desktop.name.as_deref(), Some("App"));
        assert_eq!(desktop.exec.as_deref(), Some("app %U"));
        assert!(desktop.hidden, "GNOME's switch counts as disabled");

        assert!(
            parse("[Desktop Entry]\nType=Application\nHidden=TRUE\n")
                .unwrap()
                .hidden
        );
        assert!(
            !parse("[Desktop Entry]\nType=Application\nHidden=false\n")
                .unwrap()
                .hidden
        );
        assert!(parse("[Desktop Entry]\nType=Link\nURL=https://example.com\n").is_none());
    }

    #[test]
    fn switching_only_touches_the_hidden_keys() {
        let off = with_hidden(SYSTEM, true);
        assert_eq!(off.matches("Hidden=true").count(), 1);
        assert!(off.contains("Exec=/usr/bin/sync-daemon --quiet\n") && off.contains("Icon=sync\n"));
        // 反复停用不会堆出多行；启用后回到原样。
        assert_eq!(with_hidden(&off, true), off);
        assert_eq!(with_hidden(&off, false), SYSTEM);
        let gnome = "[Desktop Entry]\nName=A\nExec=a\nX-GNOME-Autostart-enabled=false\nHidden=false\n[Other]\nHidden=keep\n";
        assert_eq!(
            with_hidden(gnome, false),
            "[Desktop Entry]\nName=A\nExec=a\n[Other]\nHidden=keep\n"
        );
    }

    #[test]
    fn a_users_file_covers_the_system_one_and_a_bare_copy_borrows_its_details() {
        let dirs = Dirs::new("list");
        dirs.put("system", "sync.desktop", SYSTEM);
        dirs.put(
            "system",
            "agent.desktop",
            "[Desktop Entry]\nType=Application\nName=Agent\nExec=agent\n",
        );
        // 只有 Hidden 的副本：名字和命令用系统的，状态用自己的。
        dirs.put(
            "user",
            "sync.desktop",
            "[Desktop Entry]\nType=Application\nHidden=true\n",
        );
        dirs.put(
            "user",
            "mine.desktop",
            "[Desktop Entry]\nType=Application\nName=Mine\nExec=mine\n",
        );
        dirs.put("user", "ignored.txt", "not a desktop file");
        // 没有 Exec、系统里也没有同名的：什么也不是。
        dirs.put(
            "user",
            "empty.desktop",
            "[Desktop Entry]\nType=Application\nHidden=true\n",
        );

        let mut found = dirs.list();
        found.sort_by(|a, b| a.key.cmp(&b.key));
        let summary: Vec<_> = found
            .iter()
            .map(|e| {
                (
                    e.key.as_str(),
                    e.name.as_str(),
                    e.command.as_str(),
                    e.enabled,
                    e.system,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("agent.desktop", "Agent", "agent", true, true),
                ("mine.desktop", "Mine", "mine", true, false),
                (
                    "sync.desktop",
                    "Sync",
                    "/usr/bin/sync-daemon --quiet",
                    false,
                    false
                ),
            ]
        );
    }

    #[test]
    fn turning_a_system_entry_off_and_on_leaves_no_trace() {
        let dirs = Dirs::new("system");
        dirs.put("system", "sync.desktop", SYSTEM);
        let (user, system) = (dirs.user(), dirs.system());

        apply(&user, &system, "sync.desktop", false).unwrap();
        assert!(!dirs.list()[0].enabled);
        let copy = fs::read_to_string(user.join("sync.desktop")).unwrap();
        assert!(copy.contains("Hidden=true") && copy.contains("Exec=/usr/bin/sync-daemon"));
        // 系统目录里的原文件一个字没动。
        assert_eq!(
            fs::read_to_string(system[0].join("sync.desktop")).unwrap(),
            SYSTEM
        );

        apply(&user, &system, "sync.desktop", true).unwrap();
        assert!(!user.join("sync.desktop").exists(), "the cover is removed");
        let back = dirs.list();
        assert!(back[0].enabled && back[0].system);
        // 已经是启用的再启用：什么也不发生。
        apply(&user, &system, "sync.desktop", true).unwrap();
        assert!(!user.join("sync.desktop").exists());
    }

    #[test]
    fn a_users_own_entry_keeps_its_file_and_a_customised_copy_is_not_deleted() {
        let dirs = Dirs::new("user");
        dirs.put(
            "user",
            "mine.desktop",
            "[Desktop Entry]\nType=Application\nName=Mine\nExec=mine\n",
        );
        let (user, system) = (dirs.user(), dirs.system());
        apply(&user, &system, "mine.desktop", false).unwrap();
        assert!(!dirs.list()[0].enabled);
        apply(&user, &system, "mine.desktop", true).unwrap();
        assert!(dirs.list()[0].enabled);
        assert!(user.join("mine.desktop").exists());

        // 用户改过命令的副本：启用时不能当成「只是盖住」删掉。
        dirs.put("system", "sync.desktop", SYSTEM);
        dirs.put(
            "user",
            "sync.desktop",
            "[Desktop Entry]\nType=Application\nName=Sync\nExec=/opt/my-sync\nHidden=true\n",
        );
        apply(&user, &system, "sync.desktop", true).unwrap();
        let mine = fs::read_to_string(user.join("sync.desktop")).unwrap();
        assert!(mine.contains("Exec=/opt/my-sync") && !mine.contains("Hidden"));

        assert_eq!(
            apply(&user, &system, "gone.desktop", false)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }
}

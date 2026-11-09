//! macOS：`.app` 应用包，和它留在 `~/Library`（以及 `/Library`）里的东西。

// 别的系统上只有测试会编进来（这里的逻辑是纯函数，到处都能测）。
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::{App, Leftover};
use crate::{space, util};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 往下找几层文件夹：`/Applications/Utilities/Foo.app`、`~/Applications/Edge Apps.localized/Foo.app`。
const DEPTH: usize = 2;

/// 找残留时，目录里的条目靠什么认出「这是它的」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    /// 条目名是 Bundle ID，或者以 Bundle ID 加一个点开头。
    Id,
    /// 同上，或者条目名和应用的名字一样（`Application Support/Spotify`）。
    IdOrName,
    /// 同 `Id`，只是条目名前面可能带着 `group.` 或者十位的团队前缀。
    GroupId,
}

/// `~/Library` 里各个目录，以及怎么认。
const USER_PLACES: &[(&str, Rule)] = &[
    ("Application Support", Rule::IdOrName),
    ("Caches", Rule::IdOrName),
    ("Containers", Rule::Id),
    ("Group Containers", Rule::GroupId),
    ("Preferences", Rule::Id),
    ("Preferences/ByHost", Rule::Id),
    ("Saved Application State", Rule::Id),
    ("Logs", Rule::IdOrName),
    ("WebKit", Rule::Id),
    ("HTTPStorages", Rule::Id),
    ("Cookies", Rule::Id),
    ("Application Scripts", Rule::Id),
    ("LaunchAgents", Rule::Id),
];

/// `/Library` 里的：所有用户共用，删要管理员权限，所以只列出来。
const SYSTEM_PLACES: &[(&str, Rule)] = &[
    ("Application Support", Rule::IdOrName),
    ("Caches", Rule::Id),
    ("Preferences", Rule::Id),
    ("Logs", Rule::IdOrName),
    ("LaunchAgents", Rule::Id),
    ("LaunchDaemons", Rule::Id),
    ("PrivilegedHelperTools", Rule::Id),
];

pub fn list() -> Vec<App> {
    let mut roots = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join("Applications"));
    }
    let mut bundles = Vec::new();
    for root in &roots {
        find_bundles(root, DEPTH, &mut bundles);
    }
    bundles.sort();
    bundles.dedup();

    let mut apps: Vec<App> = bundles.iter().map(|path| read_bundle(path)).collect();
    for (app, date) in apps.iter_mut().zip(last_used(&bundles)) {
        app.last_used = date;
    }
    apps
}

pub fn size_of(app: &App, cancel: &AtomicBool) -> Option<u64> {
    app.path.as_deref().map(|path| space::usage(path, cancel).0)
}

pub fn leftovers(app: &App, installed: &[App], cancel: &AtomicBool) -> Vec<Leftover> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    find(
        app,
        installed,
        &home.join("Library"),
        Path::new("/Library"),
        cancel,
    )
}

/// macOS 的卸载是把应用移到废纸篓，没有「应用自带的卸载程序」。
pub fn start_uninstaller(_: &App) -> Result<(), String> {
    Err("not supported on this system".into())
}

/// 在 `dir` 里找 `.app`，最多再往下 `depth` 层文件夹。
/// 符号链接和隐藏项不算：移走一个链接并没有卸载什么。
fn find_bundles(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if has_app_extension(&name) {
            out.push(entry.path());
        } else if depth > 1 {
            find_bundles(&entry.path(), depth - 1, out);
        }
    }
}

fn has_app_extension(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() > 4 && bytes[bytes.len() - 4..].eq_ignore_ascii_case(b".app")
}

/// 读一个应用包：名字取文件名（访达里看到的就是这个），其余来自 `Info.plist`。
/// 读不到 `Info.plist`（比如装来的 iOS 应用）也算一个应用，只是没有 Bundle ID。
fn read_bundle(path: &Path) -> App {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let info = plist::Value::from_file(path.join("Contents/Info.plist"))
        .ok()
        .and_then(|value| value.into_dictionary());
    // 没替换掉的构建变量（`$(PRODUCT_NAME)`）不是名字。
    let text = |key: &str| {
        info.as_ref()
            .and_then(|info| info.get(key))
            .and_then(|value| value.as_string())
            .map(str::trim)
            .filter(|text| !text.is_empty() && !text.contains("$("))
            .map(str::to_string)
    };
    let mut names = vec![stem.clone()];
    for key in ["CFBundleName", "CFBundleDisplayName"] {
        if let Some(name) = text(key)
            && !names.iter().any(|known| known.eq_ignore_ascii_case(&name))
        {
            names.push(name);
        }
    }
    App {
        name: stem,
        id: text("CFBundleIdentifier").unwrap_or_default(),
        version: text("CFBundleShortVersionString")
            .or_else(|| text("CFBundleVersion"))
            .unwrap_or_default(),
        publisher: String::new(),
        path: Some(path.to_path_buf()),
        last_used: None,
        registered_size: None,
        names,
        uninstall: String::new(),
    }
}

/// 向 Spotlight 要这些应用最近一次打开的时间；一次问完，不然每个应用起一个进程。
fn last_used(bundles: &[PathBuf]) -> Vec<Option<u64>> {
    if bundles.is_empty() {
        return Vec::new();
    }
    let paths: Vec<String> = bundles
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let mut args = vec!["-name", "kMDItemLastUsedDate", "-raw"];
    args.extend(paths.iter().map(String::as_str));
    let output =
        util::run_stdout_timeout("mdls", &args, Duration::from_secs(20)).unwrap_or_default();
    parse_dates(&output, bundles.len())
}

/// `mdls -raw` 把每个文件的值用 NUL 隔开；没有记录的是 `(null)`。
fn parse_dates(output: &str, count: usize) -> Vec<Option<u64>> {
    let mut dates: Vec<Option<u64>> = output.split('\0').map(parse_date).collect();
    dates.resize(count, None);
    dates
}

fn parse_date(text: &str) -> Option<u64> {
    let time = chrono::DateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M:%S %z").ok()?;
    u64::try_from(time.timestamp()).ok()
}

/// 在 `user_library`（`~/Library`）和 `system_library`（`/Library`）里找属于 `app` 的东西。
fn find(
    app: &App,
    installed: &[App],
    user_library: &Path,
    system_library: &Path,
    cancel: &AtomicBool,
) -> Vec<Leftover> {
    // 同一个应用装了两份（`Foo.app` 和 `Foo 2.app`）共用一份数据：删掉其中一份，不能把数据带走。
    if !app.id.is_empty()
        && installed
            .iter()
            .any(|other| other.path != app.path && other.id == app.id)
    {
        return Vec::new();
    }
    let mut found = Vec::new();
    for (system, library, places) in [
        (false, user_library, USER_PLACES),
        (true, system_library, SYSTEM_PLACES),
    ] {
        for &(dir, rule) in places {
            let Ok(entries) = fs::read_dir(library.join(dir)) else {
                continue;
            };
            for entry in entries.flatten() {
                if cancel.load(Ordering::Relaxed) {
                    return found;
                }
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if belongs_to(app, rule, &name) && !claimed_by_other(app, installed, rule, &name) {
                    let path = entry.path();
                    let bytes = space::usage(&path, cancel).0;
                    found.push(Leftover {
                        path,
                        bytes,
                        system,
                    });
                }
            }
        }
    }
    found.sort_by(|a, b| {
        (a.system, std::cmp::Reverse(a.bytes), &a.path).cmp(&(
            b.system,
            std::cmp::Reverse(b.bytes),
            &b.path,
        ))
    });
    found
}

/// 条目名认得出是 `app` 的吗。
fn belongs_to(app: &App, rule: Rule, entry: &str) -> bool {
    by_id(app, rule, entry) || by_name(app, rule, entry)
}

fn by_id(app: &App, rule: Rule, entry: &str) -> bool {
    match rule {
        Rule::GroupId => is_named_after(without_group_prefix(entry), &app.id),
        Rule::Id | Rule::IdOrName => is_named_after(entry, &app.id),
    }
}

/// 按名字认只在少数目录里，而且不认苹果自己的应用：`Application Support/Notes` 不该因为
/// 一个也叫 Notes 的第三方应用就被当成残留。
fn by_name(app: &App, rule: Rule, entry: &str) -> bool {
    rule == Rule::IdOrName
        && !app.id.starts_with("com.apple.")
        && app
            .names
            .iter()
            .any(|name| name.len() >= 3 && entry.eq_ignore_ascii_case(name))
}

/// `entry` 就是 `id`，或者以 `id` 加一个点开头（`id.plist`、`id.savedState`、`id.helper`……）。
/// 没有这个点的话 `com.foo.barbaz` 会被当成 `com.foo.bar` 的。
fn is_named_after(entry: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let Some(head) = entry.get(..id.len()) else {
        return false;
    };
    head.eq_ignore_ascii_case(id) && matches!(entry.as_bytes().get(id.len()), None | Some(b'.'))
}

/// Group Containers 里的名字常常是 `group.<id>` 或 `<十位团队 ID>.<id>`。
fn without_group_prefix(entry: &str) -> &str {
    if let Some(rest) = entry.strip_prefix("group.") {
        return rest;
    }
    match entry.split_once('.') {
        Some((team, rest))
            if team.len() == 10
                && team
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()) =>
        {
            rest
        }
        _ => entry,
    }
}

/// 这个条目其实更像是另一个已安装应用的：它的 Bundle ID 更长而且也认得这个条目
/// （`com.foo.bar.pro` 之于 `com.foo.bar`），或者我们只是靠名字认的、别人也认。
fn claimed_by_other(app: &App, installed: &[App], rule: Rule, entry: &str) -> bool {
    let ours_by_id = by_id(app, rule, entry);
    installed.iter().any(|other| {
        other.path != app.path
            && belongs_to(other, rule, entry)
            && (!ours_by_id || other.id.len() > app.id.len())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    /// 每个测试一个独立的临时目录，用完删掉。
    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Dir {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "pc-specs-apps-{tag}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Dir(path)
        }

        fn file(&self, relative: &str, len: usize) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, vec![b'x'; len]).unwrap();
            path
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn app(name: &str, id: &str) -> App {
        App {
            name: name.into(),
            id: id.into(),
            version: String::new(),
            publisher: String::new(),
            path: Some(PathBuf::from(format!("/Applications/{name}.app"))),
            last_used: None,
            registered_size: None,
            names: vec![name.into()],
            uninstall: String::new(),
        }
    }

    fn names_of(found: &[Leftover], root: &Path) -> Vec<String> {
        found
            .iter()
            .map(|item| {
                item.path
                    .strip_prefix(root)
                    .unwrap_or(&item.path)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    #[test]
    fn a_name_belongs_to_a_bundle_id_only_at_a_dot_boundary() {
        assert!(is_named_after("com.foo.bar", "com.foo.bar"));
        assert!(is_named_after("com.foo.bar.plist", "com.foo.bar"));
        assert!(is_named_after("com.foo.bar.savedState", "com.foo.bar"));
        assert!(is_named_after("COM.FOO.BAR.plist", "com.foo.bar"));
        assert!(!is_named_after("com.foo.barbaz", "com.foo.bar"));
        assert!(!is_named_after("com.foo", "com.foo.bar"));
        assert!(!is_named_after("anything", ""));
        // 不在字符边界上切：不能 panic，也不能认错。
        assert!(!is_named_after("com.fo中.bar", "com.foo.bar"));
        assert!(!is_named_after("com.f中中", "com.foo"));
        assert!(!is_named_after("é", "ab"));
    }

    #[test]
    fn group_container_names_may_carry_a_prefix() {
        assert_eq!(without_group_prefix("group.com.foo.bar"), "com.foo.bar");
        assert_eq!(
            without_group_prefix("UBF8T346G9.com.foo.bar"),
            "com.foo.bar"
        );
        // 不是十位大写字母数字，就不是团队前缀。
        assert_eq!(without_group_prefix("com.foo.bar"), "com.foo.bar");
        assert_eq!(without_group_prefix("ubf8t346g9.x"), "ubf8t346g9.x");
        assert_eq!(without_group_prefix("SHORT.x"), "SHORT.x");
    }

    #[test]
    fn what_belongs_to_an_app_is_found_and_the_rest_is_left_alone() {
        let dir = Dir::new("find");
        let (user, system) = (dir.0.join("user"), dir.0.join("system"));
        let layout = [
            ("user/Application Support/Spotify/db", 30_000),
            ("user/Application Support/com.spotify.client/db", 10_000),
            ("user/Application Support/Other/db", 50_000),
            ("user/Caches/com.spotify.client/blob", 90_000),
            ("user/Preferences/com.spotify.client.plist", 5_000),
            ("user/Preferences/com.spotify.clientx.plist", 5_000),
            ("user/Containers/com.spotify.client/data", 8_000),
            ("user/Group Containers/group.com.spotify.client/data", 8_000),
            (
                "user/Group Containers/UBF8T346G9.com.spotify.client/data",
                8_000,
            ),
            ("user/Group Containers/group.com.other/data", 8_000),
            (
                "user/Saved Application State/com.spotify.client.savedState/w",
                4_000,
            ),
            ("user/Logs/Spotify/log", 2_000),
            (
                "system/LaunchDaemons/com.spotify.client.helper.plist",
                1_000,
            ),
            ("system/Caches/com.other/blob", 1_000),
        ];
        for (path, len) in layout {
            dir.file(path, len);
        }
        let spotify = app("Spotify", "com.spotify.client");
        let found = find(
            &spotify,
            std::slice::from_ref(&spotify),
            &user,
            &system,
            &AtomicBool::new(false),
        );
        let names = names_of(&found, &dir.0);
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            [
                "system/LaunchDaemons/com.spotify.client.helper.plist",
                "user/Application Support/Spotify",
                "user/Application Support/com.spotify.client",
                "user/Caches/com.spotify.client",
                "user/Containers/com.spotify.client",
                "user/Group Containers/UBF8T346G9.com.spotify.client",
                "user/Group Containers/group.com.spotify.client",
                "user/Logs/Spotify",
                "user/Preferences/com.spotify.client.plist",
                "user/Saved Application State/com.spotify.client.savedState",
            ]
        );
        // 用户自己的在前、系统共用的在后；各自按大小排。
        assert_eq!(
            found.iter().map(|item| item.system).collect::<Vec<_>>(),
            [vec![false; 9], vec![true]].concat()
        );
        assert!(
            found[..9]
                .windows(2)
                .all(|pair| pair[0].bytes >= pair[1].bytes)
        );
        assert!(found[0].bytes >= 90_000);
    }

    #[test]
    fn files_of_a_longer_named_app_are_not_taken() {
        let dir = Dir::new("longer");
        let (user, system) = (dir.0.join("user"), dir.0.join("system"));
        dir.file("user/Caches/com.foo.bar/blob", 1_000);
        dir.file("user/Caches/com.foo.bar.pro/blob", 1_000);
        dir.file("user/Preferences/com.foo.bar.pro.plist", 1_000);
        dir.file("user/Preferences/com.foo.bar.plist", 1_000);
        let base = app("Foo", "com.foo.bar");
        let pro = app("Foo Pro", "com.foo.bar.pro");
        let installed = [base.clone(), pro.clone()];
        let cancel = AtomicBool::new(false);

        let base_found = names_of(&find(&base, &installed, &user, &system, &cancel), &dir.0);
        assert_eq!(
            base_found
                .iter()
                .filter(|name| name.contains("pro"))
                .count(),
            0,
            "{base_found:?}"
        );
        assert_eq!(base_found.len(), 2, "{base_found:?}");

        // 反过来，更长的那个仍然拿得到自己的。
        let pro_found = names_of(&find(&pro, &installed, &user, &system, &cancel), &dir.0);
        assert_eq!(pro_found.len(), 2, "{pro_found:?}");
        assert!(pro_found.iter().all(|name| name.contains("pro")));

        // 只装了短的那个：长 ID 的文件仍然像是「别人的」，但没人认领时它就是个残留。
        let alone = names_of(
            &find(&base, std::slice::from_ref(&base), &user, &system, &cancel),
            &dir.0,
        );
        assert_eq!(alone.len(), 4, "{alone:?}");
    }

    #[test]
    fn a_second_copy_of_an_app_keeps_the_data_in_place() {
        let dir = Dir::new("copies");
        let (user, system) = (dir.0.join("user"), dir.0.join("system"));
        dir.file("user/Caches/com.foo.bar/blob", 1_000);
        let first = app("Foo", "com.foo.bar");
        let second = app("Foo 2", "com.foo.bar");
        let installed = [first.clone(), second];
        let found = find(&first, &installed, &user, &system, &AtomicBool::new(false));
        assert!(found.is_empty());
    }

    #[test]
    fn names_alone_do_not_match_apple_apps_or_shared_names() {
        let dir = Dir::new("names");
        let (user, system) = (dir.0.join("user"), dir.0.join("system"));
        dir.file("user/Application Support/Notes/db", 1_000);
        dir.file("user/Application Support/Shared/db", 1_000);
        let cancel = AtomicBool::new(false);

        // 苹果的应用只认 Bundle ID。
        let apple = app("Notes", "com.apple.Notes");
        assert!(
            find(
                &apple,
                std::slice::from_ref(&apple),
                &user,
                &system,
                &cancel
            )
            .is_empty()
        );

        // 同名的第三方应用认得到。
        let third_party = app("Notes", "org.example.notes");
        assert_eq!(
            find(
                &third_party,
                std::slice::from_ref(&third_party),
                &user,
                &system,
                &cancel
            )
            .len(),
            1
        );

        // 两个不同的应用都叫 Shared：谁也不能认领。
        let one = app("Shared", "org.example.one");
        let mut two = app("Shared", "org.example.two");
        two.path = Some(PathBuf::from("/Applications/Shared Two.app"));
        let installed = [one.clone(), two];
        assert!(find(&one, &installed, &user, &system, &cancel).is_empty());
    }

    #[test]
    fn finding_stops_when_asked_to() {
        let dir = Dir::new("cancel");
        let (user, system) = (dir.0.join("user"), dir.0.join("system"));
        dir.file("user/Caches/com.foo.bar/blob", 1_000);
        let foo = app("Foo", "com.foo.bar");
        let found = find(
            &foo,
            std::slice::from_ref(&foo),
            &user,
            &system,
            &AtomicBool::new(true),
        );
        assert!(found.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn applications_are_found_one_folder_down_but_not_inside_other_apps() {
        let dir = Dir::new("bundles");
        for path in [
            "A.app/Contents/Info.plist",
            "Folder/B.app/Contents/Info.plist",
            "Folder/Sub/C.app/Contents/Info.plist",
            ".Hidden/D.app/Contents/Info.plist",
            "A.app/Contents/Inner.app/Contents/Info.plist",
            "Plain/readme.txt",
            "notanapp.application/Contents/Info.plist",
        ] {
            dir.file(path, 10);
        }
        std::os::unix::fs::symlink(dir.0.join("A.app"), dir.0.join("Link.app")).unwrap();
        let mut found = Vec::new();
        find_bundles(&dir.0, DEPTH, &mut found);
        let mut names: Vec<String> = found
            .iter()
            .map(|path| {
                path.strip_prefix(&dir.0)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        assert_eq!(names, ["A.app", "Folder/B.app"]);
    }

    #[test]
    fn an_application_is_described_by_its_info_plist() {
        let dir = Dir::new("plist");
        let info = dir.file("Foo Bar.app/Contents/Info.plist", 0);
        fs::write(
            &info,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleIdentifier</key><string>com.foo.bar</string>
  <key>CFBundleName</key><string>FooBar</string>
  <key>CFBundleDisplayName</key><string>$(PRODUCT_NAME)</string>
  <key>CFBundleVersion</key><string>42</string>
</dict></plist>"#,
        )
        .unwrap();
        let read = read_bundle(&dir.0.join("Foo Bar.app"));
        assert_eq!(read.name, "Foo Bar");
        assert_eq!(read.id, "com.foo.bar");
        // 没有 ShortVersionString 就退而求其次；没替换的变量不当名字。
        assert_eq!(read.version, "42");
        assert_eq!(read.names, ["Foo Bar", "FooBar"]);

        // 没有 Info.plist 也算一个应用。
        let bare = read_bundle(&dir.0.join("Missing.app"));
        assert_eq!(bare.name, "Missing");
        assert!(bare.id.is_empty());
    }

    #[test]
    fn spotlight_dates_are_matched_back_to_the_applications() {
        let output = "2026-10-31 06:05:48 +0000\0(null)\0garbage";
        let dates = parse_dates(output, 4);
        assert_eq!(dates.len(), 4);
        assert_eq!(dates[0], Some(1_793_426_748));
        assert_eq!(dates[1], None);
        assert_eq!(dates[2], None);
        assert_eq!(dates[3], None);
        // 没有输出（Spotlight 关了、命令失败）：每个应用都是「不知道」。
        assert_eq!(parse_dates("", 2), vec![None, None]);
    }
}

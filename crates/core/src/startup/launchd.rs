//! macOS：launchd 的用户代理和守护进程，加上「登录项」。
//!
//! launchd 的项用 `launchctl disable / enable` 开关：它记在 launchd 自己的数据库里，plist 一个字不改。
//! 登录项没有这种机制，只能通过「系统事件」删掉再加回来，所以停用的登录项由我们记着，好让用户随时打开。
//! 守护进程在系统域里，改它要管理员权限，这里只列出来。

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::{Entry, Listing, Source, Trigger};
use plist::{Dictionary, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// 一个 launchd 任务里和启动有关的部分。
struct Job {
    label: String,
    /// 它跑的命令行，只用来显示。
    command: String,
    trigger: Trigger,
    /// plist 里写着 `Disabled`（launchd 自己的开关记录优先）。
    disabled: bool,
}

/// 解析一个 launchd 的 plist。没有 `Label` 的（空的 plist）不是任务。
fn parse_job(value: Value) -> Option<Job> {
    let dict = value.into_dictionary()?;
    let label = dict.get("Label")?.as_string()?.to_string();
    let command = match dict.get("ProgramArguments").and_then(Value::as_array) {
        Some(args) => args
            .iter()
            .filter_map(Value::as_string)
            .map(quote)
            .collect::<Vec<_>>()
            .join(" "),
        None => dict
            .get("Program")
            .and_then(Value::as_string)
            .unwrap_or_default()
            .to_string(),
    };
    Some(Job {
        label,
        command,
        trigger: trigger_of(&dict),
        disabled: truthy(&dict, "Disabled"),
    })
}

/// 值是 `true`，或者是个不空的字典（`KeepAlive` 可以写成一组条件）。
fn truthy(dict: &Dictionary, key: &str) -> bool {
    match dict.get(key) {
        Some(Value::Boolean(on)) => *on,
        Some(Value::Dictionary(conditions)) => !conditions.is_empty(),
        _ => false,
    }
}

fn trigger_of(dict: &Dictionary) -> Trigger {
    if truthy(dict, "KeepAlive") {
        Trigger::KeepAlive
    } else if truthy(dict, "RunAtLoad") {
        Trigger::Login
    } else if dict.contains_key("StartInterval") || dict.contains_key("StartCalendarInterval") {
        Trigger::Schedule
    } else {
        Trigger::OnDemand
    }
}

/// 显示命令行时，带空格的参数加上引号。
fn quote(arg: &str) -> String {
    if arg.contains(char::is_whitespace) {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    }
}

/// 给任务起个好认的名字：跑的是某个应用里的东西，就用应用的名字，否则用 Label。
fn friendly_name(label: &str, command: &str) -> String {
    command
        .split('/')
        .find_map(|part| part.strip_suffix(".app"))
        .filter(|name| !name.is_empty())
        .map_or_else(|| label.to_string(), str::to_string)
}

/// `launchctl print-disabled` 的输出：每行 `"label" => disabled` 或 `=> enabled`（老系统写 `true` / `false`）。
/// 返回「标签 → 是否被停用」，没有出现的就是没有记录。
fn parse_disabled(output: &str) -> HashMap<String, bool> {
    output
        .lines()
        .filter_map(|line| {
            let (label, state) = line.split_once("=>")?;
            let label = label.trim().strip_prefix('"')?.strip_suffix('"')?;
            let off = matches!(state.trim(), "disabled" | "true");
            Some((label.to_string(), off))
        })
        .collect()
}

/// `launchctl list` 的输出：`PID  Status  Label`，没在跑的 PID 是 `-`。返回正在跑的标签。
fn parse_running(output: &str) -> HashSet<String> {
    output
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let pid = columns.next()?;
            let label = columns.nth(1)?;
            pid.parse::<u32>().ok().map(|_| label.to_string())
        })
        .collect()
}

fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: `getuid` 没有参数、不会失败，也不碰任何内存。
        unsafe { libc::getuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn launchctl(args: &[&str]) -> Result<String, String> {
    let output = Command::new("/bin/launchctl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if error.is_empty() {
            format!("launchctl exited with {}", output.status)
        } else {
            error
        })
    }
}

/// 目录里的 .plist 文件，按文件名排好。
fn plist_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "plist"))
        .collect();
    files.sort();
    files
}

pub(super) fn list() -> Listing {
    let uid = current_uid();
    let user_off = launchctl(&["print-disabled", &format!("gui/{uid}")])
        .map(|text| parse_disabled(&text))
        .unwrap_or_default();
    let system_off = launchctl(&["print-disabled", "system"])
        .map(|text| parse_disabled(&text))
        .unwrap_or_default();
    let running = launchctl(&["list"])
        .map(|text| parse_running(&text))
        .unwrap_or_default();

    let mut places = Vec::new();
    if let Some(home) = dirs::home_dir() {
        places.push((
            home.join("Library/LaunchAgents"),
            Source::LaunchAgent,
            false,
        ));
    }
    places.push(("/Library/LaunchAgents".into(), Source::LaunchAgent, true));
    places.push(("/Library/LaunchDaemons".into(), Source::LaunchDaemon, true));

    let mut entries = Vec::new();
    for (dir, source, system) in places {
        let daemon = source == Source::LaunchDaemon;
        let overrides = if daemon { &system_off } else { &user_off };
        for path in plist_files(&dir) {
            let Some(job) = Value::from_file(&path).ok().and_then(parse_job) else {
                continue;
            };
            let off = overrides.get(&job.label).copied().unwrap_or(job.disabled);
            entries.push(Entry {
                name: friendly_name(&job.label, &job.command),
                id: job.label.clone(),
                command: job.command,
                source,
                trigger: job.trigger,
                enabled: !off,
                running: !daemon && running.contains(&job.label),
                system,
                editable: !daemon,
                path: Some(path),
                key: job.label,
            });
        }
    }

    let mut listing = Listing {
        entries,
        login_items_denied: false,
    };
    match login_items() {
        Ok(items) => listing.entries.extend(login_item_entries(items)),
        Err(Failure::Denied) => listing.login_items_denied = true,
        Err(Failure::Other(error)) => tracing::warn!("couldn't read the login items: {error}"),
    }
    listing
}

pub(super) fn set_enabled(entry: &Entry, enabled: bool) -> Result<(), String> {
    match entry.source {
        Source::LaunchAgent => {
            let target = format!("gui/{}/{}", current_uid(), entry.key);
            launchctl(&[if enabled { "enable" } else { "disable" }, &target]).map(|_| ())
        }
        Source::LoginItem => {
            let item: LoginItem = serde_json::from_str(&entry.key).map_err(|e| e.to_string())?;
            if enabled {
                add_login_item(&item)
            } else {
                remove_login_item(&item)
            }
        }
        _ => Err("this item can only be changed by an administrator".into()),
    }
}

// ---- 登录项 --------------------------------------------------------------------

/// 一个登录项。停用的登录项存成这个样子，重新启用时照着加回去。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct LoginItem {
    name: String,
    path: String,
    hidden: bool,
}

/// 「系统事件」里读不到登录项的两种原因。
enum Failure {
    /// 用户没有授权（或拒绝了）自动化控制「系统事件」。
    Denied,
    Other(String),
}

const LIST_SCRIPT: &str = r#"tell application "System Events"
    set out to ""
    repeat with login_item in every login item
        set out to out & (name of login_item) & tab & (path of login_item) & tab & (hidden of login_item) & linefeed
    end repeat
    return out
end tell"#;

/// 解析 `LIST_SCRIPT` 的输出：每行 `名字⇥路径⇥是否隐藏`。
fn parse_login_items(output: &str) -> Vec<LoginItem> {
    output
        .lines()
        .filter_map(|line| {
            let mut columns = line.split('\t');
            let (name, path, hidden) = (columns.next()?, columns.next()?, columns.next()?);
            (!path.is_empty()).then(|| LoginItem {
                name: name.to_string(),
                path: path.to_string(),
                hidden: hidden.trim() == "true",
            })
        })
        .collect()
}

/// 跑一段 AppleScript。第一次会弹出授权框，等用户点完才有结果，所以给个上限，免得一直卡着。
fn osascript(lines: &[&str], args: &[&str]) -> Result<String, Failure> {
    const LIMIT: Duration = Duration::from_secs(60);
    let mut command = Command::new("/usr/bin/osascript");
    for line in lines {
        command.arg("-e").arg(line);
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Failure::Other(e.to_string()))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < LIMIT => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Other("timed out waiting for System Events".into()));
            }
            Err(e) => return Err(Failure::Other(e.to_string())),
        }
    };
    // 进程已经退出了，管道里剩下的读完就是结尾。
    let (mut stdout, mut stderr) = (String::new(), String::new());
    if let Some(pipe) = child.stdout.as_mut() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(pipe) = child.stderr.as_mut() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    if status.success() {
        Ok(stdout)
    } else if stderr.contains("-1743") {
        Err(Failure::Denied)
    } else {
        Err(Failure::Other(stderr.trim().to_string()))
    }
}

fn login_items() -> Result<Vec<LoginItem>, Failure> {
    osascript(&[LIST_SCRIPT], &[]).map(|text| parse_login_items(&text))
}

fn add_login_item(item: &LoginItem) -> Result<(), String> {
    osascript(
        &[
            "on run argv",
            "tell application \"System Events\" to make login item at end with properties {path:(item 1 of argv), hidden:((item 2 of argv) is \"true\")}",
            "end run",
        ],
        &[&item.path, if item.hidden { "true" } else { "false" }],
    )
    .map_err(failure_text)?;
    forget(item);
    Ok(())
}

fn remove_login_item(item: &LoginItem) -> Result<(), String> {
    // 先记下来再删：删成功了用户才能从列表里把它找回来。
    remember(item)?;
    let removed = osascript(
        &[
            "on run argv",
            "tell application \"System Events\" to delete (every login item whose path is (item 1 of argv))",
            "end run",
        ],
        &[&item.path],
    );
    if let Err(failure) = removed {
        forget(item);
        return Err(failure_text(failure));
    }
    Ok(())
}

fn failure_text(failure: Failure) -> String {
    match failure {
        Failure::Denied => "PC Specs isn't allowed to control System Events".into(),
        Failure::Other(error) => error,
    }
}

/// 现在有的登录项，加上我们记着的、被停用的那些。
fn login_item_entries(live: Vec<LoginItem>) -> Vec<Entry> {
    let live_paths: HashSet<String> = live.iter().map(|item| item.path.clone()).collect();
    // 应用已经删掉了的，不必再留着。
    let off = load_off()
        .into_iter()
        .filter(|item| !live_paths.contains(&item.path) && Path::new(&item.path).exists());
    live.into_iter()
        .map(|item| (item, true))
        .chain(off.map(|item| (item, false)))
        .map(|(item, enabled)| Entry {
            name: item.name.clone(),
            id: String::new(),
            command: item.path.clone(),
            source: Source::LoginItem,
            trigger: Trigger::Login,
            enabled,
            running: false,
            system: false,
            editable: true,
            path: Some(PathBuf::from(&item.path)),
            key: serde_json::to_string(&item).unwrap_or_default(),
        })
        .collect()
}

fn off_file() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("pc-specs").join("login_items_off.json"))
}

fn load_off() -> Vec<LoginItem> {
    off_file()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_off(items: &[LoginItem]) -> Result<(), String> {
    let path = off_file().ok_or_else(|| "there is no configuration folder".to_string())?;
    let json = serde_json::to_vec_pretty(items).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // 先写临时文件再改名，中途断掉也不会留下半个文件。
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json).map_err(|e| e.to_string())?;
    fs::rename(&temp, &path).map_err(|e| e.to_string())
}

fn remember(item: &LoginItem) -> Result<(), String> {
    let mut items = load_off();
    items.retain(|other| other.path != item.path);
    items.push(item.clone());
    save_off(&items)
}

fn forget(item: &LoginItem) {
    let mut items = load_off();
    let before = items.len();
    items.retain(|other| other.path != item.path);
    if items.len() != before {
        let _ = save_off(&items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(xml: &str) -> Option<Job> {
        let text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>{xml}</dict></plist>"
        );
        Value::from_reader_xml(text.as_bytes())
            .ok()
            .and_then(parse_job)
    }

    #[test]
    fn a_job_is_read_with_its_command_and_what_starts_it() {
        let found = job(
            "<key>Label</key><string>io.example.helper</string>\
             <key>ProgramArguments</key><array><string>/Applications/Clash Verge.app/Contents/MacOS/clash-verge</string><string>--flag</string></array>\
             <key>RunAtLoad</key><true/>",
        )
        .unwrap();
        assert_eq!(found.label, "io.example.helper");
        assert_eq!(
            found.command,
            "\"/Applications/Clash Verge.app/Contents/MacOS/clash-verge\" --flag"
        );
        assert_eq!(found.trigger, Trigger::Login);
        assert!(!found.disabled);
        assert_eq!(friendly_name(&found.label, &found.command), "Clash Verge");

        let daemon = job(
            "<key>Label</key><string>a.b</string><key>Program</key><string>/usr/libexec/thing</string>\
             <key>MachServices</key><dict><key>a.b</key><true/></dict>",
        )
        .unwrap();
        assert_eq!(daemon.command, "/usr/libexec/thing");
        assert_eq!(daemon.trigger, Trigger::OnDemand);
        assert_eq!(friendly_name(&daemon.label, &daemon.command), "a.b");
    }

    #[test]
    fn the_trigger_follows_keep_alive_then_run_at_load_then_schedules() {
        let trigger = |extra: &str| {
            job(&format!("<key>Label</key><string>x</string>{extra}"))
                .unwrap()
                .trigger
        };
        assert_eq!(
            trigger("<key>RunAtLoad</key><true/><key>KeepAlive</key><true/>"),
            Trigger::KeepAlive
        );
        // 条件写成字典的 KeepAlive 也算；空字典不算。
        assert_eq!(
            trigger("<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>"),
            Trigger::KeepAlive
        );
        assert_eq!(
            trigger("<key>KeepAlive</key><dict/><key>RunAtLoad</key><false/>"),
            Trigger::OnDemand
        );
        assert_eq!(
            trigger("<key>StartInterval</key><integer>300</integer>"),
            Trigger::Schedule
        );
        assert_eq!(trigger(""), Trigger::OnDemand);
        assert!(
            job("<key>Label</key><string>x</string><key>Disabled</key><true/>")
                .unwrap()
                .disabled
        );
        // 空的 plist（没有 Label）不是任务。
        assert!(job("").is_none());
        assert!(
            Value::from_reader_xml("<plist version=\"1.0\"><array/></plist>".as_bytes())
                .ok()
                .and_then(parse_job)
                .is_none()
        );
    }

    #[test]
    fn launchctl_output_is_understood() {
        let disabled = parse_disabled(
            "disabled services = {\n\t\"com.apple.Siri.agent\" => enabled\n\t\"com.apple.ftpd\" => disabled\n\t\"old.style\" => true\n\t\"old.on\" => false\n}\n",
        );
        assert_eq!(disabled.len(), 4);
        assert!(disabled["com.apple.ftpd"]);
        assert!(!disabled["com.apple.Siri.agent"]);
        assert!(disabled["old.style"]);
        assert!(!disabled["old.on"]);

        let running = parse_running(
            "PID\tStatus\tLabel\n-\t0\tcom.apple.idle\n749\t0\tcom.apple.Finder\n36651\t-9\tcom.apple.progressd\n",
        );
        assert!(running.contains("com.apple.Finder") && running.contains("com.apple.progressd"));
        assert!(!running.contains("com.apple.idle") && running.len() == 2);
    }

    #[test]
    fn login_items_are_read_from_the_script_output() {
        let items = parse_login_items(
            "Slack\t/Applications/Slack.app\tfalse\nDropbox\t/Applications/Dropbox.app\ttrue\n\nbroken line\n",
        );
        assert_eq!(
            items,
            [
                LoginItem {
                    name: "Slack".into(),
                    path: "/Applications/Slack.app".into(),
                    hidden: false
                },
                LoginItem {
                    name: "Dropbox".into(),
                    path: "/Applications/Dropbox.app".into(),
                    hidden: true
                },
            ]
        );
        assert!(parse_login_items("").is_empty());
        // 停用的登录项靠一段 JSON 往返，名字里有引号、制表符也不丢。
        let odd = LoginItem {
            name: "A \"quoted\"\tname".into(),
            path: "/Applications/A.app".into(),
            hidden: true,
        };
        let key = serde_json::to_string(&odd).unwrap();
        assert_eq!(serde_json::from_str::<LoginItem>(&key).unwrap(), odd);
    }

    #[test]
    fn plists_are_found_by_extension_and_sorted() {
        let dir = std::env::temp_dir().join(format!("pcs-launchd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in ["b.plist", "a.plist", "notes.txt"] {
            fs::write(dir.join(name), "").unwrap();
        }
        let names: Vec<_> = plist_files(&dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.plist", "b.plist"]);
        assert!(plist_files(&dir.join("missing")).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

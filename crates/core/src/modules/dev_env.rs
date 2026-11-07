use crate::model::{DevEnvInfo, RuntimeInfo};
use crate::util::{dev_path, run_with, which};
use std::ffi::OsString;
use std::time::Duration;

type Spec = (&'static str, &'static [&'static str]);

const LANGUAGES: &[Spec] = &[
    ("node", &["--version"]),
    ("python3", &["--version"]),
    ("python", &["--version"]),
    ("rustc", &["--version"]),
    ("go", &["version"]),
    ("java", &["-version"]),
    ("dotnet", &["--version"]),
    ("ruby", &["--version"]),
    ("php", &["--version"]),
    ("swift", &["--version"]),
    ("deno", &["--version"]),
    ("bun", &["--version"]),
    ("gcc", &["--version"]),
    ("clang", &["--version"]),
];
const PACKAGE_MANAGERS: &[Spec] = &[
    ("npm", &["--version"]),
    ("pnpm", &["--version"]),
    ("yarn", &["--version"]),
    ("pip", &["--version"]),
    ("pip3", &["--version"]),
    ("uv", &["--version"]),
    ("cargo", &["--version"]),
    ("gem", &["--version"]),
    ("brew", &["--version"]),
    ("apt", &["--version"]),
    ("dnf", &["--version"]),
    ("pacman", &["--version"]),
    ("zypper", &["--version"]),
    ("apk", &["--version"]),
    ("snap", &["--version"]),
    ("flatpak", &["--version"]),
    ("nix", &["--version"]),
    ("winget", &["--version"]),
    ("choco", &["--version"]),
    ("scoop", &["--version"]),
];
const VCS: &[Spec] = &[
    ("git", &["--version"]),
    ("hg", &["--version"]),
    ("svn", &["--version"]),
];
const EDITORS: &[Spec] = &[
    ("code", &["--version"]),
    ("cursor", &["--version"]),
    ("zed", &["--version"]),
    ("vim", &["--version"]),
    ("nvim", &["--version"]),
    ("emacs", &["--version"]),
];
const CONTAINERS: &[Spec] = &[
    ("docker", &["--version"]),
    ("podman", &["--version"]),
    // kubectl 1.28 起 `--short` 已废弃，`--client` 本身输出就很简洁。
    ("kubectl", &["version", "--client"]),
    ("colima", &["version"]),
    ("nerdctl", &["--version"]),
    ("helm", &["version", "--short"]),
];
const SHELLS: &[Spec] = &[
    ("bash", &["--version"]),
    ("zsh", &["--version"]),
    ("fish", &["--version"]),
    ("pwsh", &["--version"]),
    // Windows PowerShell 5.1 不认 --version。
    (
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "$PSVersionTable.PSVersion.ToString()",
        ],
    ),
];

const ENV_KEYS_OF_INTEREST: &[&str] = &[
    "PATH",
    "SHELL",
    "LANG",
    "LC_ALL",
    "EDITOR",
    "TERM",
    "TERM_PROGRAM",
    "GOPATH",
    "GOROOT",
    "JAVA_HOME",
    "ANDROID_HOME",
    "NODE_VERSION",
    "PYTHONPATH",
    "RUSTUP_HOME",
    "CARGO_HOME",
    "VIRTUAL_ENV",
    "CONDA_DEFAULT_ENV",
    "PNPM_HOME",
    "DOCKER_HOST",
];

/// 开发环境探测。
///
/// 从 Finder / 开始菜单 / 桌面启动的 GUI 程序拿到的 PATH 往往只有系统目录，
/// 因此用 `dev_path()`（进程 PATH + 登录 shell PATH）查找和执行；
/// 先在进程内按 PATH 解析可执行文件，找不到的工具连进程都不会启动。
pub fn collect() -> DevEnvInfo {
    let path = dev_path();
    std::thread::scope(|s| {
        let group = |specs: &'static [Spec]| s.spawn(move || probe_group(specs, path));
        let languages = group(LANGUAGES);
        let package_managers = group(PACKAGE_MANAGERS);
        let vcs = group(VCS);
        let editors = group(EDITORS);
        let containers = group(CONTAINERS);
        let shells = group(SHELLS);

        DevEnvInfo {
            languages: languages.join().unwrap_or_default(),
            package_managers: package_managers.join().unwrap_or_default(),
            vcs: vcs.join().unwrap_or_default(),
            editors: editors.join().unwrap_or_default(),
            containers: containers.join().unwrap_or_default(),
            shells: shells.join().unwrap_or_default(),
            env_keys: ENV_KEYS_OF_INTEREST
                .iter()
                .filter(|k| std::env::var_os(k).is_some())
                .map(|k| k.to_string())
                .collect(),
        }
    })
}

fn probe_group(specs: &'static [Spec], path: &'static OsString) -> Vec<RuntimeInfo> {
    std::thread::scope(|s| {
        let handles: Vec<_> = specs
            .iter()
            .map(|(name, args)| s.spawn(move || probe(name, args, path)))
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok().flatten())
            .collect()
    })
}

fn probe(name: &str, args: &[&str], path: &OsString) -> Option<RuntimeInfo> {
    let exe = which(name, Some(path))?;
    if is_installer_stub(name, &exe) {
        return None;
    }
    let out = run_with(exe.to_str()?, args, Duration::from_secs(5), Some(path))?;
    if !out.ok {
        return None;
    }
    // `java -version` 之类把版本写到 stderr。
    let text = if out.stdout.trim().is_empty() {
        &out.stderr
    } else {
        &out.stdout
    };
    let version = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?
        .to_string();
    Some(RuntimeInfo {
        name: name.to_string(),
        version: Some(version),
        path: Some(exe.display().to_string()),
    })
}

/// 系统自带的"安装引导桩"：执行它会弹出安装对话框，而不是输出版本，必须跳过。
///
/// - macOS `/usr/bin/{git,python3,svn,...}`：未装 Xcode 命令行工具时会弹"安装开发者工具"。
/// - macOS `/usr/bin/java`：没有 JDK 时会弹"需要安装 Java"。
/// - Windows `WindowsApps\python*.exe`：Microsoft Store 的别名桩。
fn is_installer_stub(name: &str, exe: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        if exe.starts_with("/usr/bin") {
            match name {
                "java" => {
                    return !run_with("/usr/libexec/java_home", &[], Duration::from_secs(2), None)
                        .is_some_and(|o| o.ok);
                }
                "git" | "python3" | "python" | "svn" | "hg" | "clang" | "gcc" | "swift" => {
                    return !command_line_tools_installed();
                }
                _ => {}
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        if name.starts_with("python")
            && exe
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains("\\microsoft\\windowsapps\\")
        {
            return true;
        }
    }
    let _ = (name, exe);
    false
}

#[cfg(target_os = "macos")]
fn command_line_tools_installed() -> bool {
    use std::sync::OnceLock;
    static INSTALLED: OnceLock<bool> = OnceLock::new();
    *INSTALLED.get_or_init(|| {
        run_with(
            "/usr/bin/xcode-select",
            &["-p"],
            Duration::from_secs(2),
            None,
        )
        .is_some_and(|o| o.ok)
    })
}

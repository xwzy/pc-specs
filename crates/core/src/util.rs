//! 跨模块共用的小工具：时间戳、外部命令（带超时）、PATH 查找、DMI 占位值清洗、macOS sysctl。

use parking_lot::{Condvar, Mutex};
use std::any::Any;
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// 当前 UTC 毫秒时间戳。
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 跑一段不能把进程带走的代码：里面 panic 了只记一条日志并返回 `None`。
///
/// 常驻进程里，界面线程的回调大多夹在系统事件循环的 C 帧之间，panic 沿着这些帧传出去
/// 就是整个进程 abort；后台线程里 panic 则会让这条线程悄悄消失、功能静默失效。
/// 所以每个回调 / 线程的入口都用它兜一层，出了问题丢掉这一轮、下一轮照常。
pub fn guarded<R>(what: &str, f: impl FnOnce() -> R) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(value) => Some(value),
        Err(payload) => {
            tracing::error!(
                "{what} panicked, skipped: {}",
                panic_message(payload.as_ref())
            );
            None
        }
    }
}

/// 取出 panic 携带的文字（`panic!("..")` 给的是 `&str` 或 `String`）。
pub fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}

/// 把「最多能同时打开多少文件」提到系统允许的上限。
///
/// macOS 图形应用默认只有 256 个：每个外部命令占好几个管道、本地 HTTP 服务每个连接占一个，
/// 启动时一轮采集加上几个连接就可能撞线，之后 `spawn` / `accept` 都会失败。
pub fn raise_fd_limit() {
    #[cfg(unix)]
    {
        // SAFETY: 只读写本进程的资源限制结构体。
        unsafe {
            let mut limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) != 0 {
                return;
            }
            // macOS 内核的上限是 OPEN_MAX（10240），`rlim_max` 常常是「无限」，设得更大会被拒绝。
            let target = limit.rlim_max.min(10_240);
            if target > limit.rlim_cur {
                limit.rlim_cur = target;
                let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
            }
        }
    }
}

/// 固件 / DMI 里常见的"厂商没填"占位值统一视为空。
pub fn clean_dmi(s: impl AsRef<str>) -> Option<String> {
    let v = s.as_ref().trim();
    if v.is_empty() {
        return None;
    }
    const PLACEHOLDERS: &[&str] = &[
        "to be filled by o.e.m.",
        "to be filled by oem",
        "default string",
        "system manufacturer",
        "system product name",
        "system version",
        "system serial number",
        "not specified",
        "not applicable",
        "n/a",
        "none",
        "unknown",
        "0x00",
        "0",
    ];
    let lower = v.to_ascii_lowercase();
    if PLACEHOLDERS.contains(&lower.as_str()) {
        None
    } else {
        Some(v.to_string())
    }
}

/// 外部命令的执行结果。
pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// 同一时刻最多有几个外部命令在跑。启动 / 刷新时十几个模块一起采集，每个又要起好几个子进程，
/// 不限制的话会同时冒出几十个进程和上百个线程，把一台机器瞬间压住。
const MAX_COMMANDS: usize = 6;
/// 单个命令的输出上限；超出的部分读掉丢弃，不让一个失控的命令把内存吃光。
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
/// 命令结束（或被杀）之后，再等它的输出读完的最长时间。
const OUTPUT_GRACE: Duration = Duration::from_secs(1);

static RUNNING_COMMANDS: Mutex<usize> = Mutex::new(0);
static COMMAND_FINISHED: Condvar = Condvar::new();

/// 一个外部命令的运行名额。用完（包括中途 panic）自动归还。
struct CommandSlot;

impl CommandSlot {
    fn acquire() -> Self {
        let mut running = RUNNING_COMMANDS.lock();
        while *running >= MAX_COMMANDS {
            COMMAND_FINISHED.wait(&mut running);
        }
        *running += 1;
        Self
    }
}

impl Drop for CommandSlot {
    fn drop(&mut self) {
        *RUNNING_COMMANDS.lock() -= 1;
        COMMAND_FINISHED.notify_one();
    }
}

/// 在独立线程里把管道读到头。线程起不来时返回一个已断开的通道，调用方当作没有输出。
fn read_pipe(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("pc-specs-pipe".into())
        .spawn(move || {
            let mut buf = Vec::new();
            let _ = (&mut pipe).take(OUTPUT_LIMIT).read_to_end(&mut buf);
            // 超出上限的部分读掉扔了，免得子进程写满管道卡死在那里。
            let _ = std::io::copy(&mut pipe, &mut std::io::sink());
            let _ = tx.send(buf);
        });
    rx
}

/// 运行外部命令并在 `timeout` 后强制结束。Windows 下不弹出控制台窗口。
/// 命令不存在、超时、无法启动都返回 `None`。
///
/// 同时最多跑 [`MAX_COMMANDS`] 个命令，多的排队；排队的时间不算进 `timeout`。
pub fn run_with(
    cmd: &str,
    args: &[&str],
    timeout: Duration,
    path_env: Option<&OsString>,
) -> Option<Output> {
    let _slot = CommandSlot::acquire();
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(p) = path_env {
        command.env("PATH", p);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // 自成一个进程组：超时的时候连子孙进程一起杀掉。不然 shell 起的后代还攥着管道，
    // 读输出的线程就一直等下去，整轮采集（以及依赖它的「正在刷新」状态）跟着卡死。
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().ok()?;
    let (Some(out), Some(err)) = (child.stdout.take(), child.stderr.take()) else {
        kill_tree(&mut child, false);
        return None;
    };
    let out_rx = read_pipe(out);
    let err_rx = read_pipe(err);

    let deadline = Instant::now() + timeout;
    // 快命令几毫秒就结束，慢命令（system_profiler 之类）要跑好几秒：等待间隔由短到长，
    // 既不让快命令白等，也不让慢命令期间每秒醒上百次。
    let mut nap = Duration::from_millis(2);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => {
                kill_tree(&mut child, true);
                break None;
            }
            Ok(None) => {
                std::thread::sleep(nap);
                nap = (nap * 2).min(Duration::from_millis(50));
            }
            Err(_) => {
                kill_tree(&mut child, false);
                break None;
            }
        }
    };
    let stdout = out_rx.recv_timeout(OUTPUT_GRACE).unwrap_or_default();
    let stderr = err_rx.recv_timeout(OUTPUT_GRACE).unwrap_or_default();
    let status = status?;
    Some(Output {
        ok: status.success(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// 结束子进程并收尸。`group` 为真时整个进程组一起杀（只在子进程确定还没被回收时才安全，
/// 否则这个组号可能已经被别的进程用上了）。
fn kill_tree(child: &mut Child, group: bool) {
    #[cfg(unix)]
    if group {
        // SAFETY: 只是给一个进程组发信号；组号就是我们刚启动、尚未回收的那个子进程的 pid。
        unsafe {
            libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = group;
    let _ = child.kill();
    let _ = child.wait();
}

/// 运行命令，仅当退出码为 0 时返回 stdout。默认 8 秒超时。
pub fn run_stdout(cmd: &str, args: &[&str]) -> Option<String> {
    run_stdout_timeout(cmd, args, Duration::from_secs(8))
}

pub fn run_stdout_timeout(cmd: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let o = run_with(cmd, args, timeout, None)?;
    o.ok.then_some(o.stdout)
}

/// 在 `path_env`（缺省为进程 PATH）里查找可执行文件。
pub fn which(name: &str, path_env: Option<&OsString>) -> Option<PathBuf> {
    let path = match path_env {
        Some(p) => p.clone(),
        None => std::env::var_os("PATH")?,
    };
    #[cfg(target_os = "windows")]
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect();
    for dir in std::env::split_paths(&path) {
        #[cfg(target_os = "windows")]
        {
            let has_ext = std::path::Path::new(name).extension().is_some();
            if has_ext {
                let p = dir.join(name);
                if p.is_file() {
                    return Some(p);
                }
            }
            for e in &exts {
                let p = dir.join(format!("{name}{e}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = dir.join(name);
            if let Ok(meta) = std::fs::metadata(&p)
                && meta.is_file()
                && meta.permissions().mode() & 0o111 != 0
            {
                return Some(p);
            }
        }
    }
    None
}

/// 面向"从 Dock / 桌面图标启动"的 PATH：
/// macOS / Linux 的图形会话只有最精简的 PATH（`/usr/bin:/bin:...`），找不到 Homebrew、
/// nvm、cargo 等开发工具；这里向用户的登录 shell 询问真实 PATH 并与当前进程 PATH 合并。
/// 结果进程内缓存；询问失败（超时 / 无 shell）时退回常见目录。
pub fn dev_path() -> &'static OsString {
    static PATH: OnceLock<OsString> = OnceLock::new();
    PATH.get_or_init(|| {
        #[allow(unused_mut)] // Windows 上没有登录 shell 可问，不会追加目录。
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect())
            .unwrap_or_default();
        #[cfg(unix)]
        {
            let mut extra = login_shell_path();
            if extra.is_empty() {
                extra = common_unix_dirs();
            }
            for d in extra {
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
        std::env::join_paths(dirs).unwrap_or_default()
    })
}

#[cfg(unix)]
fn login_shell_path() -> Vec<PathBuf> {
    const MARK: &str = "__PCSPECS_PATH__";
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".to_string()
            } else {
                "/bin/sh".to_string()
            }
        });
    let script = format!("printf '%s%s%s' '{MARK}' \"$PATH\" '{MARK}'");
    let Some(o) = run_with(&shell, &["-ilc", &script], Duration::from_secs(4), None) else {
        return Vec::new();
    };
    let mut parts = o.stdout.split(MARK);
    parts.next();
    let Some(path) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    path.split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(unix)]
fn common_unix_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/snap/bin",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for rel in [
            ".cargo/bin",
            ".local/bin",
            "go/bin",
            ".bun/bin",
            ".deno/bin",
        ] {
            v.push(home.join(rel));
        }
    }
    v
}

/// macOS `sysctlbyname` 的薄封装，替代每次 spawn 一个 `sysctl` 进程。
#[cfg(target_os = "macos")]
pub mod sysctl {
    use std::ffi::CString;

    fn raw(name: &str) -> Option<Vec<u8>> {
        let cname = CString::new(name).ok()?;
        let mut size: libc::size_t = 0;
        // SAFETY: 两次调用都只传入有效的 NUL 结尾名字与本地缓冲区；第一次仅查询长度。
        unsafe {
            if libc::sysctlbyname(
                cname.as_ptr(),
                std::ptr::null_mut(),
                &mut size,
                std::ptr::null_mut(),
                0,
            ) != 0
                || size == 0
            {
                return None;
            }
            let mut buf = vec![0u8; size];
            if libc::sysctlbyname(
                cname.as_ptr(),
                buf.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            ) != 0
            {
                return None;
            }
            buf.truncate(size);
            Some(buf)
        }
    }

    pub fn string(name: &str) -> Option<String> {
        let b = raw(name)?;
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        let s = String::from_utf8_lossy(&b[..end]).trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    pub fn uint(name: &str) -> Option<u64> {
        let b = raw(name)?;
        match b.len() {
            1 => Some(b[0] as u64),
            2 => Some(u16::from_ne_bytes(b[..2].try_into().ok()?) as u64),
            4 => Some(u32::from_ne_bytes(b[..4].try_into().ok()?) as u64),
            8 => Some(u64::from_ne_bytes(b[..8].try_into().ok()?)),
            _ => None,
        }
    }
}

/// 字节数 → "12.3 GiB"（二进制前缀），供导出与托盘共用。
pub fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} {}", UNITS[0])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

//! 大屏开着的时候不让屏幕熄灭、不让系统休眠。
//!
//! 持有 `Awake` 就保持唤醒，丢弃（窗口关闭）时自动恢复。
//! macOS 和 Linux 借助系统自带的命令（`caffeinate` / `systemd-inhibit`），它们跟着本进程的生死走：
//! 本进程退出后自己也会结束。Windows 用 `SetThreadExecutionState`，这个状态属于调用它的线程，
//! 所以由一条专门的线程替它保持住。找不到命令之类的失败不影响大屏本身，只是退化成「不常亮」。

pub struct Awake {
    _guard: imp::Guard,
}

impl Awake {
    pub fn start() -> Option<Self> {
        imp::Guard::start().map(|guard| Self { _guard: guard })
    }
}

#[cfg(unix)]
mod imp {
    use std::os::unix::process::CommandExt as _;
    use std::process::{Child, Command, Stdio};

    pub struct Guard(Child);

    impl Guard {
        pub fn start() -> Option<Self> {
            let pid = std::process::id();
            let mut command = if cfg!(target_os = "macos") {
                // -d 不让显示器睡，-i 不让系统空闲睡，-w 等本进程退出。
                let mut command = Command::new("caffeinate");
                command.args(["-d", "-i", "-w", &pid.to_string()]);
                command
            } else {
                let mut command = Command::new("systemd-inhibit");
                command.args([
                    "--what=idle:sleep",
                    "--who=PC Specs",
                    "--why=Monitoring wall",
                    "--mode=block",
                    "sh",
                    "-c",
                    &format!("while kill -0 {pid} 2>/dev/null; do sleep 2; done"),
                ]);
                command
            };
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                // 自成一组，收尾时整组一起结束，不留孤儿进程。
                .process_group(0);
            command.spawn().ok().map(Self)
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            if let Ok(group) = i32::try_from(self.0.id()) {
                // SAFETY: 只给自己刚启动的那个进程组发 SIGTERM。
                unsafe {
                    libc::killpg(group, libc::SIGTERM);
                }
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::mpsc;
    use std::thread::JoinHandle;

    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
    const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }

    pub struct Guard {
        release: Option<mpsc::Sender<()>>,
        thread: Option<JoinHandle<()>>,
    }

    impl Guard {
        pub fn start() -> Option<Self> {
            let (release, wait) = mpsc::channel::<()>();
            let thread = std::thread::Builder::new()
                .name("pc-specs-awake".into())
                .spawn(move || {
                    // SAFETY: 纯 Win32 调用，不涉及任何指针；状态属于本线程，下面同一线程里再清掉。
                    unsafe {
                        SetThreadExecutionState(
                            ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED,
                        );
                    }
                    // 守卫被丢弃时发送端断开，这里就返回。
                    let _ = wait.recv();
                    // SAFETY: 同上。
                    unsafe {
                        SetThreadExecutionState(ES_CONTINUOUS);
                    }
                })
                .ok()?;
            Some(Self {
                release: Some(release),
                thread: Some(thread),
            })
        }
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            self.release.take();
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

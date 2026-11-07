//! 单实例：每个用户只留一个进程。后启动的那个只负责把先启动的叫醒，然后自己退出。
//!
//! 先启动的实例锁住 `instance.lock`（进程退出时系统自动释放，崩溃也不会留下过期的锁），
//! 同时在回环地址上随机监听一个端口，把端口号写进 `instance.port`；后启动的读到端口连上去，
//! 对方收到连接就是「请显示窗口」。端口单独存一个文件，因为 Windows 上被锁住的文件别的进程读不了。
//!
//! 单实例是尽力而为的：锁文件打不开、文件系统不支持锁、叫不醒对方，都照常启动，不拦着用户。

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 持有它就持有锁，进程退出时一并释放。
pub struct Instance {
    _lock: Option<File>,
}

impl Instance {
    fn untracked() -> Option<Instance> {
        Some(Instance { _lock: None })
    }
}

fn state_dir() -> Option<PathBuf> {
    // 都是本机、临时的状态，不该跟着 Windows 的漫游配置走；Linux 优先放在登出即清空的 XDG_RUNTIME_DIR。
    dirs::runtime_dir()
        .or_else(dirs::data_local_dir)
        .map(|dir| dir.join("pc-specs"))
}

/// 抢到锁就返回 `Some`（并开始监听「请显示窗口」）；已经有实例在跑就返回 `None`，
/// 调用方应当直接退出。`wake_existing` 为真时，退出前先叫醒那个实例。
pub fn acquire(wake: async_channel::Sender<()>, wake_existing: bool) -> Option<Instance> {
    let Some(dir) = state_dir() else {
        return Instance::untracked();
    };
    let _ = std::fs::create_dir_all(&dir);
    let Ok(lock) = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("instance.lock"))
    else {
        return Instance::untracked();
    };

    match lock.try_lock() {
        Ok(()) => {
            listen(&dir, wake);
            Some(Instance { _lock: Some(lock) })
        }
        Err(TryLockError::WouldBlock) => {
            if wake_existing && !wake_running(&dir) {
                tracing::warn!(
                    "another instance holds the lock but didn't answer, starting anyway"
                );
                return Instance::untracked();
            }
            None
        }
        Err(TryLockError::Error(error)) => {
            tracing::warn!("single-instance lock unavailable: {error}");
            Instance::untracked()
        }
    }
}

fn listen(dir: &Path, wake: async_channel::Sender<()>) {
    let port_file = dir.join("instance.port");
    // 先清掉上一个实例留下的端口号，免得后来者连到别的程序上。
    let _ = std::fs::remove_file(&port_file);
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)) else {
        return;
    };
    let Ok(address) = listener.local_addr() else {
        return;
    };
    if std::fs::write(&port_file, address.port().to_string()).is_err() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("pc-specs-instance".into())
        .spawn(move || {
            for connection in listener.incoming() {
                if connection.is_ok() && wake.send_blocking(()).is_err() {
                    break;
                }
            }
        });
}

/// 叫醒已经在运行的实例。它刚启动时端口号可能还没写好，所以多试几次。
fn wake_running(dir: &Path) -> bool {
    for _ in 0..15 {
        let port = std::fs::read_to_string(dir.join("instance.port"))
            .ok()
            .and_then(|text| text.trim().parse::<u16>().ok());
        if let Some(port) = port {
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            if let Ok(mut stream) = TcpStream::connect_timeout(&address, Duration::from_millis(300))
            {
                return stream.write_all(b"show\n").is_ok();
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

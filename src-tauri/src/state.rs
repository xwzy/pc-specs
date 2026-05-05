use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;
use sysinfo::{Disks, Networks, System};
use tokio::sync::Notify;

/// 用于"前端按需 query"的共享 sysinfo 状态。
///
/// 重要：`networks` / `disks` **只能由前端 query 路径**（get_storages / get_network /
/// collect_full_snapshot）调用 `refresh()`。Monitor 后台 tick 必须使用 `MonitorSys`
/// 中的独立 networks / disks 实例，否则两边互相 refresh 会让 `received() / written_bytes`
/// 返回的"自上次 refresh 以来"的累积值被另一方"偷走"，导致速率忽高忽低。
pub struct SharedSys {
    pub system: Mutex<System>,
    pub networks: Mutex<Networks>,
    pub disks: Mutex<Disks>,
    /// 历史字段：旧版用于前端 Storage 页 IO 速率；现在由 monitor 单独维护，保留是为了
    /// 旧 collect_aggregate_io stub 不破坏外部签名。
    #[allow(dead_code)]
    pub disks_last_refresh: Mutex<Option<Instant>>,
    #[allow(dead_code)]
    pub disks_io_prev: Mutex<HashMap<String, (u64, u64)>>,
}

impl SharedSys {
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_all();
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        Self {
            system: Mutex::new(sys),
            networks: Mutex::new(networks),
            disks: Mutex::new(disks),
            disks_last_refresh: Mutex::new(None),
            disks_io_prev: Mutex::new(HashMap::new()),
        }
    }
}

/// Monitor 后台 tick 专用的状态。包含独立的 sysinfo Networks，
/// 以及上次采样时的 Instant，用来计算实际 elapsed 而非依赖前端 ticker 时间。
pub struct MonitorSys {
    pub networks: Mutex<Networks>,
    /// 占位：保留以便未来若要用 sysinfo Disks::usage（需 0.33+）兜底，避免再调整 struct。
    #[allow(dead_code)]
    pub disks: Mutex<Disks>,
    /// 上一次 net/disk 采样的时间戳，用于计算真实 elapsed 秒数。
    pub last_sample_at: Mutex<Option<Instant>>,
    /// 各平台累计 IO 字节的"上一次值"快照：
    ///   - Linux：来自 /proc/diskstats（设备名 → (read, written)）
    ///   - Windows：来自 WMI Win32_PerfRawData_PerfDisk_PhysicalDisk
    ///   - macOS：来自 iostat -I 累计 MB 后转换的字节
    pub diskstats_prev: Mutex<HashMap<String, (u64, u64)>>,
}

impl MonitorSys {
    pub fn new() -> Self {
        let networks = Networks::new_with_refreshed_list();
        let disks = Disks::new_with_refreshed_list();
        Self {
            networks: Mutex::new(networks),
            disks: Mutex::new(disks),
            last_sample_at: Mutex::new(None),
            diskstats_prev: Mutex::new(HashMap::new()),
        }
    }
}

pub struct AppState {
    pub sys: Arc<SharedSys>,
    pub monitor: Arc<MonitorSys>,
    pub monitor_stop: Mutex<Option<Arc<Notify>>>,
    /// 当前正在运行的 monitor 任务的 interval。用于 `start_monitor` 幂等：
    /// 多个窗口（主窗口 / 悬浮窗）都可能调 `start_monitor`，相同 interval 不应当
    /// 反复杀线程重启 —— 否则每次都会丢 elapsed 基线，BPS 显示首次为 0。
    pub monitor_interval_ms: Mutex<Option<u64>>,
    /// monitor task 的"还活着"标志。由 task 内部一个 Drop guard 维护：task 正常
    /// 退出 / panic unwind 时都会被 set false，让下次 `start_monitor` 能识别"slot
    /// 残留但 task 实际已死"的情况，重启监控而不是被幂等卡住。
    pub monitor_alive: Mutex<Option<Arc<AtomicBool>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            sys: Arc::new(SharedSys::new()),
            monitor: Arc::new(MonitorSys::new()),
            monitor_stop: Mutex::new(None),
            monitor_interval_ms: Mutex::new(None),
            monitor_alive: Mutex::new(None),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

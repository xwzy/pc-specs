pub mod battery;
pub mod cpu;
pub mod dev_env;
pub mod display;
pub mod gpu;
pub mod host;
pub mod memory;
pub mod network;
pub mod os;
pub mod peripherals;
pub mod sensors;
pub mod storage;

use crate::model::SystemSnapshot;
use crate::state::SharedSys;
use std::sync::Arc;
use tauri::AppHandle;

pub fn collect_full_snapshot(shared: &Arc<SharedSys>, app: &AppHandle) -> SystemSnapshot {
    {
        let mut sys = shared.system.lock();
        sys.refresh_all();
        let mut nets = shared.networks.lock();
        nets.refresh();
    }

    // sysinfo 计算 cpu_usage 需要两次刷新之间至少 ~200ms，否则会得到 0%。
    // 当 monitor tick 已经在 1Hz 跑时这不是问题；但用户立即触发 export 的场景
    // 仍可能命中 0%。这里加一个 250ms 间隔再 refresh 一次确保数据合理。
    std::thread::sleep(std::time::Duration::from_millis(250));
    {
        let mut sys = shared.system.lock();
        sys.refresh_cpu_specifics(sysinfo::CpuRefreshKind::everything());
        let mut nets = shared.networks.lock();
        nets.refresh();
    }

    let host = host::collect();
    let os = os::collect();
    let mut cpu = cpu::collect(shared);
    let gpus = gpu::collect();
    let memory = memory::collect(shared);
    let storages = storage::collect(shared);
    let motherboard = crate::platform::motherboard();
    let network = network::collect(shared);
    let displays = display::collect(app);
    let sensors = sensors::collect(shared);
    cpu::enrich_with_sensors(&mut cpu, &sensors);
    let battery = battery::collect();
    let peripherals = peripherals::collect();
    let dev_env = dev_env::collect();

    SystemSnapshot {
        timestamp: now_ms(),
        host,
        os,
        cpu,
        gpus,
        memory,
        storages,
        motherboard,
        network,
        displays,
        sensors,
        battery,
        peripherals,
        dev_env,
    }
}

pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

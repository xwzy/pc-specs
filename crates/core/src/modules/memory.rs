use crate::model::MemoryInfo;
use crate::state::SharedSys;
use std::sync::Arc;
use sysinfo::MemoryRefreshKind;

pub fn collect(shared: &Arc<SharedSys>) -> MemoryInfo {
    let mut sys = shared.system.lock();
    sys.refresh_memory_specifics(MemoryRefreshKind::everything());

    let total_bytes = sys.total_memory();
    let used_bytes = sys.used_memory();
    let available_bytes = sys.available_memory();
    let swap_total_bytes = sys.total_swap();
    let swap_used_bytes = sys.used_swap();

    drop(sys);

    let modules = crate::platform::memory_modules();
    MemoryInfo {
        total_bytes,
        used_bytes,
        available_bytes,
        swap_total_bytes,
        swap_used_bytes,
        modules,
    }
}

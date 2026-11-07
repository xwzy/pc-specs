use crate::model::MemoryInfo;
use sysinfo::{MemoryRefreshKind, System};

pub fn collect() -> MemoryInfo {
    let mut info = MemoryInfo::default();
    if let Ok(mut sys) = System::new() {
        sys.refresh_memory_specifics(MemoryRefreshKind::everything());
        info.total_bytes = sys.total_memory();
        info.used_bytes = sys.used_memory();
        info.available_bytes = sys.available_memory();
        info.swap_total_bytes = sys.total_swap();
        info.swap_used_bytes = sys.used_swap();
    }
    info.modules = crate::platform::memory_modules();
    info
}

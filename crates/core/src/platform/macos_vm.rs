//! macOS 的内存构成与压力：虚拟内存统计（`host_statistics64`）和内核给出的压力等级。
//! 口径与「活动监视器」一致：App 内存 + 联动内存 + 被压缩 = 已使用。

use crate::model::MemoryBreakdown;
use libc::{
    _SC_PAGESIZE, HOST_VM_INFO64, HOST_VM_INFO64_COUNT, KERN_SUCCESS, c_char, c_int, c_void,
    host_statistics64, sysconf, sysctlbyname, vm_statistics64,
};
use std::mem;
use std::sync::OnceLock;

unsafe extern "C" {
    /// 本机的 host 端口。`libc` 里的同名函数已标为弃用，这里直接声明。
    fn mach_host_self() -> u32;
}

/// 每调用一次 `mach_host_self` 都会多占一个端口引用，所以只取一次。
pub(super) fn host() -> u32 {
    static HOST: OnceLock<u32> = OnceLock::new();
    // SAFETY: 无参数的系统调用，返回当前进程可用的 host 端口。
    *HOST.get_or_init(|| unsafe { mach_host_self() })
}

fn sysctl_int(name: &std::ffi::CStr) -> Option<i32> {
    let mut value: c_int = 0;
    let mut size = mem::size_of::<c_int>();
    // SAFETY: 输出缓冲区是一个 `c_int`，长度如实传入；名字以 NUL 结尾。
    let status = unsafe {
        sysctlbyname(
            name.as_ptr().cast::<c_char>(),
            (&mut value as *mut c_int).cast::<c_void>(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (status == 0).then_some(value)
}

pub fn breakdown() -> Option<MemoryBreakdown> {
    // SAFETY: `vm_statistics64` 是纯数字结构，全零是合法的初值。
    let mut stat: vm_statistics64 = unsafe { mem::zeroed() };
    let mut count = HOST_VM_INFO64_COUNT;
    // SAFETY: 缓冲区大小就是 `HOST_VM_INFO64_COUNT` 个 32 位整数，和内核约定一致。
    let status = unsafe {
        host_statistics64(
            host(),
            HOST_VM_INFO64,
            (&mut stat as *mut vm_statistics64).cast(),
            &mut count,
        )
    };
    if status != KERN_SUCCESS {
        return None;
    }
    // SAFETY: 只读系统配置。
    let page = u64::try_from(unsafe { sysconf(_SC_PAGESIZE) })
        .ok()
        .filter(|p| *p > 0)?;
    let bytes = |pages: u64| pages.saturating_mul(page);

    let purgeable = u64::from(stat.purgeable_count);
    // 内核报告的是「还剩多少内存可用」的百分比，压力就是它的反面；等级则直接取内核的判断。
    let pressure_percent = sysctl_int(c"kern.memorystatus_level")
        .filter(|level| (0..=100).contains(level))
        .map(|level| 100.0 - level as f32);
    let pressure_level = sysctl_int(c"kern.memorystatus_vm_pressure_level").and_then(|v| match v {
        1 => Some(0),
        2 => Some(1),
        4 => Some(2),
        _ => None,
    });
    Some(MemoryBreakdown {
        app_bytes: bytes(u64::from(stat.internal_page_count).saturating_sub(purgeable)),
        wired_bytes: bytes(u64::from(stat.wire_count)),
        compressed_bytes: bytes(u64::from(stat.compressor_page_count)),
        cached_bytes: bytes(u64::from(stat.external_page_count) + purgeable),
        free_bytes: bytes(
            u64::from(stat.free_count).saturating_sub(u64::from(stat.speculative_count)),
        ),
        pressure_percent,
        pressure_level,
        swapped_in_bytes: bytes(stat.swapins),
        swapped_out_bytes: bytes(stat.swapouts),
        ..Default::default()
    })
}

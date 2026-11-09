//! 各操作系统独有的采集实现。上层 `modules` 只依赖这里的统一入口。

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "macos")]
pub(crate) mod macos_cpu;
#[cfg(target_os = "macos")]
pub(crate) mod macos_net;
#[cfg(target_os = "macos")]
pub(crate) mod macos_smc;
#[cfg(target_os = "macos")]
pub(crate) mod macos_vm;
#[cfg(any(target_os = "linux", test))]
mod proc_mem;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

use crate::model::{MemoryBreakdown, MemoryModule, MotherboardInfo, SensorReading};

/// sysinfo 给不了、需要读平台接口补充的 CPU 信息。全部可选，缺失由上层兜底。
#[derive(Debug, Default, Clone)]
pub struct CpuDetails {
    pub brand: Option<String>,
    pub features: Vec<String>,
    /// 每核 L1 数据缓存、L2、L3（字节）。
    pub l1: Option<u64>,
    pub l2: Option<u64>,
    pub l3: Option<u64>,
    pub sockets: Option<u32>,
    pub p_cores: Option<u32>,
    pub e_cores: Option<u32>,
    pub numa_nodes: Option<u32>,
    /// 额定基础频率 / 最高睿频（Hz）。
    pub base_hz: Option<u64>,
    pub max_hz: Option<u64>,
    /// sysinfo 报告的频率是否为"当前实时频率"。macOS 上它是固定值（Apple 芯片是最高频率，Intel 是额定频率）。
    pub live_frequency: bool,
    /// 硬件虚拟化是否可用 / 已在固件中启用。
    pub virtualization: Option<bool>,
}

pub fn cpu_details() -> CpuDetails {
    imp::cpu_details()
}

pub fn motherboard() -> Option<MotherboardInfo> {
    imp::motherboard()
}

pub fn memory_modules() -> Vec<MemoryModule> {
    imp::memory_modules()
}

/// sysinfo `Components` 之外的传感器（hwmon 的风扇 / 电压 / 功率、Windows 热区等）。
pub fn extra_sensors() -> Vec<SensorReading> {
    imp::extra_sensors()
}

/// 进程的内存占用（字节），口径和系统自带的任务管理器一致：macOS 是「活动监视器」的「内存」
/// （`phys_footprint`，含被压缩的部分，不含共享的系统库），Windows 是任务管理器的「内存」
/// （专用工作集，不含和别的进程共享的 DLL）。Linux 上 sysinfo 给的 RSS 就是 top / htop 的口径，
/// 不另外读。读不到（别的用户的进程、受保护的进程）时为 `None`，调用方沿用 sysinfo 的值。
pub fn process_memory(pid: u32) -> Option<u64> {
    imp::process_memory(pid)
}

/// 系统的热压力等级：0 正常、1 中等、2 较重、3 严重、4 休眠。数字大了说明系统已经在压低性能来降温，
/// 和温度读数互为印证。只有 macOS 提供，其他平台为 `None`。
pub fn thermal_pressure() -> Option<u8> {
    #[cfg(target_os = "macos")]
    return macos::thermal_pressure();
    #[cfg(not(target_os = "macos"))]
    return None;
}

/// 内存构成、压力和累计换页量；平台读不到时为 `None`。
pub fn memory_breakdown() -> Option<MemoryBreakdown> {
    imp::memory_breakdown()
}

/// 实时采样里随时间变化的读数：macOS 的 CPU / GPU 逐核温度，以及风扇转速、功率、电压、电流。
/// 读取要足够便宜才能每隔几秒来一次，所以 Windows 的 WMI 查询不在其列：那边只在整份快照里给一次。
pub fn live_sensors() -> Vec<SensorReading> {
    #[cfg(target_os = "macos")]
    return macos_smc::readings();
    #[cfg(target_os = "linux")]
    return imp::extra_sensors();
    #[cfg(target_os = "windows")]
    return Vec::new();
}

pub fn desktop_env() -> Option<String> {
    imp::desktop_env()
}

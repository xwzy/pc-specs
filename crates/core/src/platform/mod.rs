//! 各操作系统独有的采集实现。上层 `modules` 只依赖这里的统一入口。

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "macos")]
pub(crate) mod macos_net;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

use crate::model::{MemoryModule, MotherboardInfo, SensorReading};

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
    /// sysinfo 报告的频率是否为"当前实时频率"。Apple Silicon 上它是固定的最高频率。
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

pub fn desktop_env() -> Option<String> {
    imp::desktop_env()
}

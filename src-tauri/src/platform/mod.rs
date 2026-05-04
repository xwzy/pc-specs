// 各 cfg 分发函数使用 explicit return 避免空块错误，clippy 的 needless_return 在此场景误报。
#![allow(clippy::needless_return)]

pub mod common;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

use crate::model::{MemoryModule, MotherboardInfo, SensorReading};

pub fn motherboard() -> Option<MotherboardInfo> {
    #[cfg(target_os = "windows")]
    {
        return windows::motherboard().or_else(common::motherboard);
    }
    #[cfg(target_os = "macos")]
    {
        return macos::motherboard().or_else(common::motherboard);
    }
    #[cfg(target_os = "linux")]
    {
        return linux::motherboard().or_else(common::motherboard);
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        common::motherboard()
    }
}

pub fn memory_modules() -> Vec<MemoryModule> {
    #[cfg(target_os = "windows")]
    {
        return windows::memory_modules();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::memory_modules();
    }
    #[cfg(target_os = "linux")]
    {
        return linux::memory_modules();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

pub fn extra_sensors() -> Vec<SensorReading> {
    #[cfg(target_os = "linux")]
    {
        return linux::sensors();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::sensors();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::sensors();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

pub fn cpu_sockets() -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        return linux::cpu_sockets();
    }
    #[cfg(target_os = "macos")]
    {
        return macos::cpu_sockets();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::cpu_sockets();
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

pub fn cpu_brand_fallback() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        return macos::cpu_brand();
    }
    #[cfg(target_os = "linux")]
    {
        return linux::cpu_brand();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

pub fn desktop_env() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        return std::env::var("XDG_CURRENT_DESKTOP").ok();
    }
    #[cfg(target_os = "macos")]
    {
        return Some("Aqua".to_string());
    }
    #[cfg(target_os = "windows")]
    {
        return Some("Explorer".to_string());
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

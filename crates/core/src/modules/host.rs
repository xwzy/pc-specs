use crate::model::HostInfo;
use sysinfo::System;

pub fn collect() -> HostInfo {
    HostInfo {
        hostname: System::host_name().unwrap_or_else(|_| "unknown".to_string()),
        username: username(),
        uptime_secs: System::uptime().unwrap_or(0),
        boot_time: System::boot_time().unwrap_or(0),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn username() -> String {
    whoami::username()
        .ok()
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("USERNAME").ok())
        .unwrap_or_else(|| "unknown".to_string())
}

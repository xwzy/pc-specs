use crate::model::HostInfo;
use sysinfo::System;

pub fn collect() -> HostInfo {
    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());
    let username = whoami::username();
    let uptime_secs = System::uptime();
    let boot_time = System::boot_time();
    HostInfo {
        hostname,
        username,
        uptime_secs,
        boot_time,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

use crate::model::{MemoryModule, MotherboardInfo, SensorReading};
use std::process::Command;

fn sysctl(key: &str) -> Option<String> {
    let out = Command::new("sysctl").arg("-n").arg(key).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// 把 hw.model 或 hw.machine 解析成可读的 Apple Silicon 名称，
/// 比如 "Mac14,5" → "Apple Silicon (Mac14,5)"，"j274" → "Apple Silicon (j274)"。
/// 这只是一个 fallback，machdep.cpu.brand_string 一般在 Intel Mac 上能给出完整型号；
/// Apple Silicon 上某些 macOS 版本会返回 "Apple M1"/"Apple M2"/"Apple M3" 等准确字符串。
fn humanize_apple_silicon(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return "Apple Silicon".to_string();
    }
    let lower = trimmed.to_lowercase();
    // 已经是 brand_string 形式："Apple M1 Pro" / "Apple M3 Max"
    if lower.starts_with("apple m") {
        return trimmed.to_string();
    }
    format!("Apple Silicon ({trimmed})")
}

pub fn motherboard() -> Option<MotherboardInfo> {
    let model = sysctl("hw.model");
    let vendor = Some("Apple".to_string());
    Some(MotherboardInfo {
        vendor,
        model,
        version: None,
        serial: None,
        bios_vendor: None,
        bios_version: None,
        bios_date: None,
        chassis: Some(detect_chassis()),
    })
}

fn detect_chassis() -> String {
    // hw.model 实际形如 "MacBookPro18,2" / "Macmini9,1" / "iMac20,1" / "MacPro7,1" / "Mac14,3"
    sysctl("hw.model")
        .map(|m| {
            let lower = m.to_lowercase();
            if lower.starts_with("macbook") || lower.contains("book") {
                "Laptop".to_string()
            } else if lower.starts_with("macmini") || lower.contains("mini") {
                "Mini-PC".to_string()
            } else if lower.starts_with("imac") {
                "All-in-One".to_string()
            } else if lower.starts_with("macpro") || lower.contains("rackmac") {
                "Workstation".to_string()
            } else if lower.starts_with("macstudio") || lower.contains("studio") {
                "Studio".to_string()
            } else {
                "Desktop".to_string()
            }
        })
        .unwrap_or_else(|| "Unknown".to_string())
}

pub fn memory_modules() -> Vec<MemoryModule> {
    // Apple Silicon 内存焊死在 SoC 中，没有 DIMM 槽；但 system_profiler 仍能给出
    // 总览信息（manufacturer / type / speed），适合作为单条"虚拟 DIMM"展示。
    // Intel Mac 上则会列出每根条。
    let out = match Command::new("system_profiler")
        .args(["-json", "SPMemoryDataType"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let v: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let arr = match v.get("SPMemoryDataType").and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };

    let mut modules = Vec::new();
    for entry in arr {
        // Intel Mac 路径：entry 内有 _items 数组，每项是一根 DIMM
        if let Some(items) = entry.get("_items").and_then(|x| x.as_array()) {
            for item in items {
                if let Some(m) = parse_macos_memory_item(item) {
                    modules.push(m);
                }
            }
            continue;
        }
        // Apple Silicon 路径：直接展示总览
        if let Some(m) = parse_macos_memory_summary(entry) {
            modules.push(m);
        }
    }
    modules
}

fn parse_macos_memory_item(item: &serde_json::Value) -> Option<MemoryModule> {
    let slot = item
        .get("_name")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "DIMM".to_string());
    let manufacturer = item
        .get("dimm_manufacturer")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let part_number = item
        .get("dimm_part_number")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let capacity_bytes = item
        .get("dimm_size")
        .and_then(|s| s.as_str())
        .and_then(parse_macos_size)
        .unwrap_or(0);
    let speed_mt_s = item
        .get("dimm_speed")
        .and_then(|s| s.as_str())
        .and_then(parse_macos_speed);
    let kind = item
        .get("dimm_type")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if capacity_bytes == 0 && manufacturer.is_none() && part_number.is_none() {
        return None;
    }
    Some(MemoryModule {
        slot,
        manufacturer,
        part_number,
        capacity_bytes,
        speed_mt_s,
        kind,
        form_factor: Some("DIMM".to_string()),
    })
}

fn parse_macos_memory_summary(entry: &serde_json::Value) -> Option<MemoryModule> {
    let capacity = entry
        .get("SPMemoryDataType")
        .or_else(|| entry.get("dimm_size"))
        .or_else(|| entry.get("physical_memory"))
        .and_then(|s| s.as_str())
        .and_then(parse_macos_size)
        .unwrap_or(0);
    let kind = entry
        .get("dimm_type")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let manufacturer = entry
        .get("dimm_manufacturer")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| Some("Apple".to_string()));
    if capacity == 0 && kind.is_none() {
        return None;
    }
    Some(MemoryModule {
        slot: "Onboard".to_string(),
        manufacturer,
        part_number: None,
        capacity_bytes: capacity,
        speed_mt_s: None,
        kind,
        form_factor: Some("Soldered".to_string()),
    })
}

/// 解析 "8 GB" / "16 GB" / "32 GB" → 字节
fn parse_macos_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (num_part, unit_part) = s.split_once(' ')?;
    let n: u64 = num_part.parse().ok()?;
    let mul: u64 = match unit_part.to_uppercase().as_str() {
        "TB" => 1024 * 1024 * 1024 * 1024,
        "GB" => 1024 * 1024 * 1024,
        "MB" => 1024 * 1024,
        _ => return None,
    };
    Some(n * mul)
}

/// 解析 "3200 MHz" / "DDR4 3200 MHz" → MT/s（约等）
fn parse_macos_speed(s: &str) -> Option<u32> {
    s.split_whitespace()
        .filter_map(|w| w.parse::<u32>().ok())
        .next()
}

pub fn cpu_brand() -> Option<String> {
    if let Some(s) = sysctl("machdep.cpu.brand_string") {
        if !s.is_empty() {
            return Some(s);
        }
    }
    // Apple Silicon 上 brand_string 为空，回退到 hw.model 并 humanize
    sysctl("hw.model")
        .or_else(|| sysctl("hw.machine"))
        .or_else(|| sysctl("hw.targettype"))
        .map(|s| humanize_apple_silicon(&s))
}

pub fn cpu_sockets() -> Option<u32> {
    sysctl("hw.packages").and_then(|s| s.parse::<u32>().ok())
}

pub fn sensors() -> Vec<SensorReading> {
    Vec::new()
}

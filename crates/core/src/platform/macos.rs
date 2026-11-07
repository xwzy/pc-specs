use super::CpuDetails;
use crate::model::{MemoryModule, MotherboardInfo, SensorReading};
use crate::util::{clean_dmi, run_stdout_timeout, sysctl};
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

type ProfilerSlot = Arc<Mutex<Option<(Instant, Option<Value>)>>>;

/// `system_profiler -json <type>` 的带缓存封装。
///
/// 快照各模块并行采集时，GPU / 显示器共用 `SPDisplaysDataType`（约 2 秒），
/// 同一类型只会真正执行一次，其余调用方等待并复用结果。
pub(crate) fn system_profiler(data_type: &'static str) -> Option<Value> {
    static CACHE: LazyLock<Mutex<HashMap<&'static str, ProfilerSlot>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    const TTL: Duration = Duration::from_secs(60);

    let slot = CACHE.lock().entry(data_type).or_default().clone();
    let mut guard = slot.lock();
    if let Some((at, value)) = guard.as_ref()
        && at.elapsed() < TTL
    {
        return value.clone();
    }
    let value = run_stdout_timeout(
        "/usr/sbin/system_profiler",
        &["-json", data_type],
        Duration::from_secs(25),
    )
    .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    *guard = Some((Instant::now(), value.clone()));
    value
}

/// 取 `SP*DataType` 数组的第一项。
pub(crate) fn profiler_first(data_type: &'static str) -> Option<Value> {
    system_profiler(data_type)?
        .get(data_type)?
        .as_array()?
        .first()
        .cloned()
}

const ARM_FEATURES: &[&str] = &[
    "AES", "PMULL", "SHA1", "SHA256", "SHA512", "SHA3", "CRC32", "LSE", "LSE2", "RDM", "FP16",
    "FHM", "DotProd", "BF16", "I8MM", "FCMA", "JSCVT", "FlagM", "FlagM2", "FRINTTS", "SB", "SSBS",
    "PAuth", "FPAC", "BTI", "DPB", "DPB2", "LRCPC", "LRCPC2", "ECV", "AFP", "EBF16", "CSV2",
    "CSV3", "DIT", "SME", "SME2",
];

pub fn cpu_details() -> CpuDetails {
    let is_arm = cfg!(target_arch = "aarch64");

    let mut features: Vec<String> = Vec::new();
    if is_arm {
        if sysctl::uint("hw.optional.neon") == Some(1) {
            features.push("neon".to_string());
        }
        for f in ARM_FEATURES {
            if sysctl::uint(&format!("hw.optional.arm.FEAT_{f}")) == Some(1) {
                features.push(f.to_ascii_lowercase());
            }
        }
    } else {
        for key in [
            "machdep.cpu.features",
            "machdep.cpu.leaf7_features",
            "machdep.cpu.extfeatures",
        ] {
            if let Some(s) = sysctl::string(key) {
                features.extend(s.split_whitespace().map(|t| t.to_ascii_lowercase()));
            }
        }
    }
    features.sort();
    features.dedup();

    // Apple Silicon 的 perflevel0 是性能核，缓存取它才符合 "每核" 语义。
    let cache = |level: &str, plain: &str| {
        sysctl::uint(&format!("hw.perflevel0.{level}")).or_else(|| sysctl::uint(plain))
    };
    let p = sysctl::uint("hw.perflevel0.physicalcpu").map(|v| v as u32);
    let e = sysctl::uint("hw.perflevel1.physicalcpu").map(|v| v as u32);

    CpuDetails {
        brand: sysctl::string("machdep.cpu.brand_string"),
        features,
        l1: cache("l1dcachesize", "hw.l1dcachesize").filter(|v| *v > 0),
        l2: cache("l2cachesize", "hw.l2cachesize").filter(|v| *v > 0),
        l3: sysctl::uint("hw.l3cachesize").filter(|v| *v > 0),
        sockets: sysctl::uint("hw.packages").map(|v| v as u32),
        // 非混合架构（Intel Mac）没有 perflevel1，不展示 P/E 拆分。
        p_cores: e.is_some().then_some(p).flatten(),
        e_cores: e,
        numa_nodes: Some(1),
        base_hz: sysctl::uint("hw.cpufrequency").filter(|v| *v > 0),
        max_hz: sysctl::uint("hw.cpufrequency_max").filter(|v| *v > 0),
        live_frequency: !is_arm,
        virtualization: sysctl::uint("kern.hv_support").map(|v| v == 1),
    }
}

pub fn motherboard() -> Option<MotherboardInfo> {
    let hw = profiler_first("SPHardwareDataType");
    let get = |k: &str| -> Option<String> { hw.as_ref()?.get(k)?.as_str().and_then(clean_dmi) };
    let identifier = sysctl::string("hw.model").or_else(|| get("machine_model"));
    let name = get("machine_name");
    let chassis = detect_chassis(name.as_deref(), identifier.as_deref());
    let model = match (name, identifier.clone()) {
        (Some(n), Some(i)) => Some(format!("{n} ({i})")),
        (None, Some(i)) => Some(i),
        (Some(n), None) => Some(n),
        _ => None,
    };
    let (serial, board_vendor) = match sysinfo::Motherboard::new() {
        Ok(m) => (m.serial_number(), m.vendor_name()),
        Err(_) => (None, None),
    };
    Some(MotherboardInfo {
        vendor: Some(board_vendor.unwrap_or_else(|| "Apple".to_string())),
        model,
        version: get("model_number"),
        serial: serial.or_else(|| get("serial_number")),
        bios_vendor: Some("Apple".to_string()),
        bios_version: get("os_loader_version").or_else(|| get("boot_rom_version")),
        bios_date: None,
        chassis,
    })
}

/// Intel Mac 的 `hw.model` 形如 "MacBookPro16,1" / "Macmini8,1"，但 Apple Silicon 只给 "Mac14,5"，
/// 所以优先看营销名（"MacBook Pro" / "Mac mini" / "Mac Studio"…），标识符只作后备。
fn detect_chassis(marketing_name: Option<&str>, identifier: Option<&str>) -> Option<String> {
    let text = [marketing_name, identifier]
        .into_iter()
        .flatten()
        .map(|s| s.to_ascii_lowercase().replace(' ', ""))
        .collect::<Vec<_>>();
    let has = |prefix: &str| text.iter().any(|t| t.starts_with(prefix));
    let label = if has("macbook") {
        "Laptop"
    } else if has("macmini") {
        "Mini-PC"
    } else if has("imac") {
        "All-in-One"
    } else if has("macpro") {
        "Workstation"
    } else if has("macstudio") {
        "Studio"
    } else if text.is_empty() {
        return None;
    } else {
        "Desktop"
    };
    Some(label.to_string())
}

pub fn memory_modules() -> Vec<MemoryModule> {
    // Apple Silicon 内存封装在 SoC 上，system_profiler 只给一条总览；
    // Intel Mac 会按 DIMM 逐条列出。
    let Some(v) = system_profiler("SPMemoryDataType") else {
        return Vec::new();
    };
    let Some(arr) = v.get("SPMemoryDataType").and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    let mut modules = Vec::new();
    for entry in arr {
        if let Some(items) = entry.get("_items").and_then(|x| x.as_array()) {
            modules.extend(items.iter().filter_map(parse_dimm));
        } else if let Some(m) = parse_onboard(entry) {
            modules.push(m);
        }
    }
    modules
}

fn json_str(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_dimm(item: &Value) -> Option<MemoryModule> {
    let manufacturer = json_str(item, "dimm_manufacturer").filter(|s| s != "0x0000");
    let part_number = json_str(item, "dimm_part_number").filter(|s| s != "-");
    let capacity_bytes = json_str(item, "dimm_size")
        .and_then(|s| parse_size(&s))
        .unwrap_or(0);
    if capacity_bytes == 0 && manufacturer.is_none() && part_number.is_none() {
        return None;
    }
    Some(MemoryModule {
        slot: json_str(item, "_name").unwrap_or_else(|| "DIMM".to_string()),
        manufacturer,
        part_number,
        capacity_bytes,
        speed_mt_s: json_str(item, "dimm_speed").and_then(|s| parse_speed(&s)),
        kind: json_str(item, "dimm_type"),
        form_factor: Some("DIMM".to_string()),
    })
}

fn parse_onboard(entry: &Value) -> Option<MemoryModule> {
    let capacity_bytes = json_str(entry, "SPMemoryDataType")
        .or_else(|| json_str(entry, "dimm_size"))
        .and_then(|s| parse_size(&s))
        .unwrap_or(0);
    let kind = json_str(entry, "dimm_type");
    if capacity_bytes == 0 && kind.is_none() {
        return None;
    }
    Some(MemoryModule {
        slot: "Onboard".to_string(),
        manufacturer: json_str(entry, "dimm_manufacturer").or_else(|| Some("Apple".to_string())),
        part_number: None,
        capacity_bytes,
        speed_mt_s: None,
        kind,
        form_factor: Some("Soldered".to_string()),
    })
}

/// "8 GB" / "16 GB" → 字节
fn parse_size(s: &str) -> Option<u64> {
    let (num, unit) = s.trim().split_once(' ')?;
    let n: u64 = num.parse().ok()?;
    let mul: u64 = match unit.to_ascii_uppercase().as_str() {
        "TB" => 1 << 40,
        "GB" => 1 << 30,
        "MB" => 1 << 20,
        _ => return None,
    };
    Some(n * mul)
}

/// "3200 MHz" / "DDR4 3200 MHz" → 3200
fn parse_speed(s: &str) -> Option<u32> {
    s.split_whitespace().find_map(|w| w.parse::<u32>().ok())
}

pub fn extra_sensors() -> Vec<SensorReading> {
    Vec::new()
}

pub fn desktop_env() -> Option<String> {
    Some("Aqua".to_string())
}

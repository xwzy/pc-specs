use super::CpuDetails;
use crate::model::{MemoryBreakdown, MemoryModule, MotherboardInfo, SensorReading};
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

/// 「系统设置 → 电池」里的「最大容量」（%），取自 `SPPowerDataType`。台式机没有电池，读不到时为 `None`。
pub(crate) fn battery_health_percent() -> Option<f32> {
    maximum_capacity(&system_profiler("SPPowerDataType")?)
}

/// `SPPowerDataType` 里电池那一项的 `sppower_battery_health_maximum_capacity`（形如 `"93%"`）。
fn maximum_capacity(profile: &Value) -> Option<f32> {
    profile
        .get("SPPowerDataType")?
        .as_array()?
        .iter()
        .find_map(|item| {
            item.get("sppower_battery_health_info")?
                .get("sppower_battery_health_maximum_capacity")?
                .as_str()
        })?
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|percent| *percent > 0.0 && *percent <= 200.0)
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
        // sysinfo 在 macOS 上给的是固定值（Apple 芯片是最高频率，Intel 是额定频率），不是实时频率。
        live_frequency: false,
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

/// 风扇、功率、电压、电流，以及 CPU / GPU 逐核温度（其余温度由 sysinfo 通过 HID 读取）。
pub fn extra_sensors() -> Vec<SensorReading> {
    super::macos_smc::readings()
}

pub fn memory_breakdown() -> Option<MemoryBreakdown> {
    super::macos_vm::breakdown()
}

unsafe extern "C" {
    fn notify_register_check(name: *const std::ffi::c_char, out_token: *mut std::ffi::c_int)
    -> u32;
    fn notify_get_state(token: std::ffi::c_int, state: *mut u64) -> u32;
}

/// 系统的热压力等级，来自 `notify` 里的 `com.apple.system.thermalpressurelevel`
/// （`OSThermalNotification.h` 的 `kOSThermalNotificationPressureLevelName`）：
/// 0 正常、1 中等、2 较重、3 严重（已经在强制降频）、4 休眠。
pub fn thermal_pressure() -> Option<u8> {
    static TOKEN: std::sync::OnceLock<Option<std::ffi::c_int>> = std::sync::OnceLock::new();
    let token = (*TOKEN.get_or_init(|| {
        let mut token = 0;
        // SAFETY: 名字以 NUL 结尾，输出参数是本地变量。
        let status = unsafe {
            notify_register_check(
                c"com.apple.system.thermalpressurelevel".as_ptr(),
                &mut token,
            )
        };
        (status == 0).then_some(token)
    }))?;
    let mut state = 0_u64;
    // SAFETY: `token` 是上面注册得到的，输出参数是本地变量。
    let status = unsafe { notify_get_state(token, &mut state) };
    (status == 0)
        .then(|| u8::try_from(state).ok())
        .flatten()
        .filter(|level| *level <= 4)
}

/// 进程的 `phys_footprint`：「活动监视器」「内存」一列的口径，进程自己占着的物理内存，
/// 含被压缩的部分，不含和别的进程共享的系统库。sysinfo 给的是驻留集（RSS），
/// 语言服务器、浏览器这类进程会差出几十倍。别的用户的进程（root 的守护进程等）读不到。
pub fn process_memory(pid: u32) -> Option<u64> {
    // SAFETY: `rusage_info_v0` 是纯数字结构，全零是合法的初值。
    let mut info: libc::rusage_info_v0 = unsafe { std::mem::zeroed() };
    // SAFETY: 缓冲区是一个 `rusage_info_v0`，和 `RUSAGE_INFO_V0` 约定的版本一致。
    let status = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V0,
            (&mut info as *mut libc::rusage_info_v0).cast(),
        )
    };
    (status == 0).then_some(info.ri_phys_footprint)
}

pub fn desktop_env() -> Option<String> {
    Some("Aqua".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_maximum_capacity_is_read_from_the_battery_item() {
        let profile = json!({"SPPowerDataType": [
            {"_name": "spcharger_information"},
            {
                "_name": "spbattery_information",
                "sppower_battery_health_info": {
                    "sppower_battery_cycle_count": 64,
                    "sppower_battery_health": "Good",
                    "sppower_battery_health_maximum_capacity": "93%"
                }
            }
        ]});
        assert_eq!(maximum_capacity(&profile), Some(93.0));
    }

    #[test]
    fn the_thermal_pressure_is_one_of_the_documented_levels() {
        // 机器不同读出来的等级不同，能读到的话必须落在 0–4 里。
        assert!(thermal_pressure().is_none_or(|level| level <= 4));
    }

    #[test]
    fn a_desktop_or_an_unexpected_answer_gives_nothing() {
        assert_eq!(maximum_capacity(&json!({"SPPowerDataType": []})), None);
        assert_eq!(
            maximum_capacity(&json!({"SPPowerDataType": [{"_name": "spcharger_information"}]})),
            None
        );
        assert_eq!(maximum_capacity(&json!({})), None);
        for text in ["", "n/a", "0%", "-5%", "900%"] {
            let profile = json!({"SPPowerDataType": [{
                "sppower_battery_health_info": {
                    "sppower_battery_health_maximum_capacity": text
                }
            }]});
            assert_eq!(maximum_capacity(&profile), None, "{text:?}");
        }
    }
}

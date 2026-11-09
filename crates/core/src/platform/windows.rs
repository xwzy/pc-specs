use super::CpuDetails;
use crate::model::{MemoryBreakdown, MemoryModule, MotherboardInfo, SensorReading};
use crate::util::clean_dmi;
use serde::Deserialize;
use wmi::WMIConnection;

/// 当前线程的 CIMV2 连接。每个采集线程各建一个即可，同一线程内的多条查询复用它。
pub(crate) fn wmi_cimv2() -> Option<WMIConnection> {
    WMIConnection::new().ok()
}

pub(crate) fn wmi_root_wmi() -> Option<WMIConnection> {
    WMIConnection::with_namespace_path("ROOT\\WMI").ok()
}

// ---------------------------------------------------------------- CPU

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Processor {
    name: Option<String>,
    max_clock_speed: Option<u32>,
    virtualization_firmware_enabled: Option<bool>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ComputerSystem {
    number_of_processors: Option<u32>,
}

pub fn cpu_details() -> CpuDetails {
    let mut d = CpuDetails {
        features: detect_features(),
        numa_nodes: Some(1),
        live_frequency: true,
        ..Default::default()
    };

    if let Some(conn) = wmi_cimv2() {
        let procs: Vec<Processor> = conn
            .raw_query(
                "SELECT Name, MaxClockSpeed, VirtualizationFirmwareEnabled FROM Win32_Processor",
            )
            .unwrap_or_default();
        if let Some(p) = procs.first() {
            d.brand = p.name.as_deref().map(|s| s.trim().to_string());
            // MaxClockSpeed 是额定基础频率（MHz）。
            d.base_hz = p
                .max_clock_speed
                .filter(|v| *v > 0)
                .map(|v| v as u64 * 1_000_000);
            d.virtualization = p.virtualization_firmware_enabled;
        }
        let systems: Vec<ComputerSystem> = conn
            .raw_query("SELECT NumberOfProcessors FROM Win32_ComputerSystem")
            .unwrap_or_default();
        d.sockets = systems.first().and_then(|s| s.number_of_processors);
    }

    let (l1, l2, l3) = processor_caches();
    d.l1 = l1;
    d.l2 = l2;
    d.l3 = l3;
    let (p, e) = hybrid_cores();
    d.p_cores = p;
    d.e_cores = e;
    d
}

/// 读取 `GetLogicalProcessorInformationEx` 的变长结果缓冲区。
fn logical_processor_info(
    relationship: windows::Win32::System::SystemInformation::LOGICAL_PROCESSOR_RELATIONSHIP,
) -> Option<Vec<u8>> {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };
    let mut len = 0u32;
    // SAFETY: 第一次调用只查询所需长度（预期以 ERROR_INSUFFICIENT_BUFFER 失败）。
    unsafe {
        let _ = GetLogicalProcessorInformationEx(relationship, None, &mut len);
    }
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    // SAFETY: 缓冲区长度与 `len` 一致；结构体是按 8 字节对齐写入的变长记录。
    unsafe {
        GetLogicalProcessorInformationEx(
            relationship,
            Some(
                buf.as_mut_ptr()
                    .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(),
            ),
            &mut len,
        )
        .ok()?;
    }
    buf.truncate(len as usize);
    Some(buf)
}

/// 遍历变长记录：返回每条记录的指针引用。
fn for_each_record(
    buf: &[u8],
    mut f: impl FnMut(
        &windows::Win32::System::SystemInformation::SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    ),
) {
    use windows::Win32::System::SystemInformation::SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;
    let header = std::mem::size_of::<u32>() * 2;
    let mut off = 0usize;
    while off + header <= buf.len() {
        // SAFETY: 记录由系统写入，`Size` 给出整条记录长度；读取前已确认至少有头部字节。
        let rec = unsafe {
            &*buf
                .as_ptr()
                .add(off)
                .cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>()
        };
        let size = rec.Size as usize;
        if size == 0 || off + size > buf.len() {
            break;
        }
        f(rec);
        off += size;
    }
}

fn processor_caches() -> (Option<u64>, Option<u64>, Option<u64>) {
    use windows::Win32::System::SystemInformation::{CacheData, CacheUnified, RelationCache};
    let (mut l1, mut l2, mut l3) = (None, None, None);
    let Some(buf) = logical_processor_info(RelationCache) else {
        return (l1, l2, l3);
    };
    for_each_record(&buf, |rec| {
        // SAFETY: 请求的是 RelationCache，联合体当前成员即 Cache。
        let c = unsafe { rec.Anonymous.Cache };
        let size = c.CacheSize as u64;
        if size == 0 {
            return;
        }
        match c.Level {
            1 if c.Type == CacheData || c.Type == CacheUnified => l1 = l1.or(Some(size)),
            2 => l2 = l2.or(Some(size)),
            3 => l3 = l3.or(Some(size)),
            _ => {}
        }
    });
    (l1, l2, l3)
}

/// 用 EfficiencyClass 识别混合架构（Intel 12 代+ / Windows on ARM）。
fn hybrid_cores() -> (Option<u32>, Option<u32>) {
    use windows::Win32::System::SystemInformation::RelationProcessorCore;
    let Some(buf) = logical_processor_info(RelationProcessorCore) else {
        return (None, None);
    };
    let mut classes: Vec<u8> = Vec::new();
    for_each_record(&buf, |rec| {
        // SAFETY: 请求的是 RelationProcessorCore，联合体当前成员即 Processor。
        classes.push(unsafe { rec.Anonymous.Processor }.EfficiencyClass);
    });
    let (Some(&max), Some(&min)) = (classes.iter().max(), classes.iter().min()) else {
        return (None, None);
    };
    if max == min {
        return (None, None);
    }
    let p = classes.iter().filter(|c| **c == max).count() as u32;
    (Some(p), Some(classes.len() as u32 - p))
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detect_features() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    macro_rules! probe {
        ($($f:tt),+ $(,)?) => { $( if std::is_x86_feature_detected!($f) { v.push($f.to_string()); } )+ };
    }
    probe!(
        "sse",
        "sse2",
        "sse3",
        "ssse3",
        "sse4.1",
        "sse4.2",
        "popcnt",
        "avx",
        "avx2",
        "fma",
        "f16c",
        "bmi1",
        "bmi2",
        "lzcnt",
        "movbe",
        "aes",
        "pclmulqdq",
        "sha",
        "rdrand",
        "rdseed",
        "adx",
        "fxsr",
        "xsave",
        "avx512f",
        "avx512bw",
        "avx512cd",
        "avx512dq",
        "avx512vl",
    );
    v
}

#[cfg(target_arch = "aarch64")]
fn detect_features() -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    macro_rules! probe {
        ($($f:tt),+ $(,)?) => { $( if std::arch::is_aarch64_feature_detected!($f) { v.push($f.to_string()); } )+ };
    }
    probe!(
        "neon", "aes", "pmull", "sha2", "sha3", "crc", "lse", "fp16", "dotprod", "rdm", "sve",
        "sve2", "bf16", "i8mm", "fhm", "jsconv", "fcma", "bti", "dpb", "rcpc", "flagm", "sb",
        "ssbs", "dit",
    );
    v
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64", target_arch = "aarch64")))]
fn detect_features() -> Vec<String> {
    Vec::new()
}

// ------------------------------------------------------- 主板 / 固件

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct BaseBoard {
    manufacturer: Option<String>,
    product: Option<String>,
    version: Option<String>,
    serial_number: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Bios {
    manufacturer: Option<String>,
    #[serde(rename = "SMBIOSBIOSVersion")]
    smbios_bios_version: Option<String>,
    version: Option<String>,
    release_date: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Enclosure {
    chassis_types: Option<Vec<u16>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct SystemProduct {
    vendor: Option<String>,
    name: Option<String>,
}

pub fn motherboard() -> Option<MotherboardInfo> {
    let conn = wmi_cimv2()?;
    let board: Option<BaseBoard> = conn
        .raw_query::<BaseBoard>(
            "SELECT Manufacturer, Product, Version, SerialNumber FROM Win32_BaseBoard",
        )
        .ok()
        .and_then(|v| v.into_iter().next());
    let bios: Option<Bios> = conn
        .raw_query::<Bios>(
            "SELECT Manufacturer, SMBIOSBIOSVersion, Version, ReleaseDate FROM Win32_BIOS",
        )
        .ok()
        .and_then(|v| v.into_iter().next());
    let chassis = conn
        .raw_query::<Enclosure>("SELECT ChassisTypes FROM Win32_SystemEnclosure")
        .ok()
        .and_then(|v| v.into_iter().next())
        .and_then(|c| c.chassis_types)
        .and_then(|v| v.into_iter().next())
        .map(decode_chassis);
    // 品牌机 / 笔记本的 BaseBoard 往往是通用占位，整机厂商与型号在 ComputerSystemProduct。
    let product: Option<SystemProduct> = conn
        .raw_query::<SystemProduct>("SELECT Vendor, Name FROM Win32_ComputerSystemProduct")
        .ok()
        .and_then(|v| v.into_iter().next());

    let nz = |s: Option<String>| s.and_then(clean_dmi);
    let board_vendor = nz(board.as_ref().and_then(|b| b.manufacturer.clone()));
    let board_model = nz(board.as_ref().and_then(|b| b.product.clone()));
    Some(MotherboardInfo {
        vendor: board_vendor.or_else(|| nz(product.as_ref().and_then(|p| p.vendor.clone()))),
        model: board_model.or_else(|| nz(product.as_ref().and_then(|p| p.name.clone()))),
        version: nz(board.as_ref().and_then(|b| b.version.clone())),
        serial: nz(board.and_then(|b| b.serial_number)),
        bios_vendor: nz(bios.as_ref().and_then(|b| b.manufacturer.clone())),
        bios_version: nz(bios
            .as_ref()
            .and_then(|b| b.smbios_bios_version.clone().or_else(|| b.version.clone()))),
        bios_date: nz(bios.and_then(|b| b.release_date)).map(|s| normalize_wmi_date(&s)),
        chassis,
    })
}

/// WMI ReleaseDate 形如 "20231005000000.000000+000" → "2023-10-05"
fn normalize_wmi_date(raw: &str) -> String {
    let t = raw.trim();
    if t.len() >= 8 && t[..8].chars().all(|c| c.is_ascii_digit()) {
        return format!("{}-{}-{}", &t[..4], &t[4..6], &t[6..8]);
    }
    t.to_string()
}

fn decode_chassis(t: u16) -> String {
    match t {
        1 | 2 => "Other".to_string(),
        3..=7 | 15 | 16 => "Desktop".to_string(),
        8..=11 | 14 => "Laptop".to_string(),
        30..=32 => "Tablet".to_string(),
        12 | 21 => "Mini-PC".to_string(),
        13 => "All-in-One".to_string(),
        17..=20 | 23 | 24 => "Server".to_string(),
        _ => format!("Type-{t}"),
    }
}

// ---------------------------------------------------------------- 内存

pub fn memory_modules() -> Vec<MemoryModule> {
    // Win32_PhysicalMemory.MemoryType 在 DDR4/DDR5 机器上常返回 0（Unknown），
    // SMBIOSMemoryType 才是 SMBIOS 标准值。
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct PhysicalMemory {
        device_locator: Option<String>,
        manufacturer: Option<String>,
        part_number: Option<String>,
        capacity: Option<String>,
        speed: Option<u32>,
        configured_clock_speed: Option<u32>,
        memory_type: Option<u16>,
        #[serde(rename = "SMBIOSMemoryType")]
        smbios_memory_type: Option<u32>,
        form_factor: Option<u16>,
    }

    let Some(conn) = wmi_cimv2() else {
        return Vec::new();
    };
    let rows: Vec<PhysicalMemory> = conn
        .raw_query(
            "SELECT DeviceLocator, Manufacturer, PartNumber, Capacity, Speed, ConfiguredClockSpeed, MemoryType, SMBIOSMemoryType, FormFactor FROM Win32_PhysicalMemory",
        )
        .unwrap_or_default();
    rows.into_iter()
        .map(|m| {
            let kind = match m.memory_type {
                Some(t) if t != 0 => Some(decode_memory_type(t)),
                _ => m
                    .smbios_memory_type
                    .filter(|t| *t != 0)
                    .map(decode_smbios_memory_type),
            };
            // ConfiguredClockSpeed 是实际运行频率（XMP/EXPO 启用后比 Speed 更接近真实值）。
            let speed_mt_s = m
                .configured_clock_speed
                .filter(|n| *n > 0)
                .or(m.speed.filter(|n| *n > 0));
            MemoryModule {
                slot: m
                    .device_locator
                    .and_then(clean_dmi)
                    .unwrap_or_else(|| "DIMM".to_string()),
                manufacturer: m.manufacturer.and_then(clean_dmi),
                part_number: m.part_number.and_then(clean_dmi),
                capacity_bytes: m
                    .capacity
                    .as_deref()
                    .and_then(|c| c.parse::<u64>().ok())
                    .unwrap_or(0),
                speed_mt_s,
                kind,
                form_factor: m.form_factor.map(decode_form_factor),
            }
        })
        .collect()
}

fn decode_memory_type(t: u16) -> String {
    match t {
        20 => "DDR".to_string(),
        21 => "DDR2".to_string(),
        24 => "DDR3".to_string(),
        26 => "DDR4".to_string(),
        34 => "DDR5".to_string(),
        _ => format!("Type-{t}"),
    }
}

/// SMBIOS Memory Device Type（DSP0134 Table 17），仅列常见值。
fn decode_smbios_memory_type(t: u32) -> String {
    match t {
        18 => "DDR".to_string(),
        19 | 22 => "DDR2".to_string(),
        24 => "DDR3".to_string(),
        26 => "DDR4".to_string(),
        27 | 28 => "LPDDR3".to_string(),
        29 => "LPDDR4".to_string(),
        34 => "DDR5".to_string(),
        35 => "LPDDR5".to_string(),
        _ => format!("Type-{t}"),
    }
}

fn decode_form_factor(f: u16) -> String {
    match f {
        8 => "DIMM".to_string(),
        12 => "SO-DIMM".to_string(),
        13 => "SRIMM".to_string(),
        _ => format!("FF-{f}"),
    }
}

// ------------------------------------------------------------ 传感器

/// ACPI 热区温度（需要管理员权限时可能为空）。
pub fn extra_sensors() -> Vec<SensorReading> {
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct ThermalZone {
        instance_name: Option<String>,
        current_temperature: Option<u32>,
    }

    let Some(conn) = wmi_root_wmi() else {
        return Vec::new();
    };
    let rows: Vec<ThermalZone> = conn
        .raw_query("SELECT InstanceName, CurrentTemperature FROM MSAcpi_ThermalZoneTemperature")
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|t| {
            let raw = t.current_temperature?;
            // 单位 1/10 开尔文
            let c = (raw as f32 / 10.0) - 273.15;
            (c > -50.0 && c < 200.0).then(|| SensorReading {
                source: "acpi".to_string(),
                label: t.instance_name.unwrap_or_else(|| "ThermalZone".to_string()),
                kind: "temperature".to_string(),
                value: c,
                unit: "C".to_string(),
            })
        })
        .collect()
}

/// 物理内存的构成。Windows 能分得清的只有内核的非分页池和「可用」，其余都算应用占用。
pub fn memory_breakdown() -> Option<MemoryBreakdown> {
    use windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};

    let mut info = PERFORMANCE_INFORMATION::default();
    let size = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
    // SAFETY: 结构体和长度都由我们提供，函数只往里面写数字。
    unsafe { GetPerformanceInfo(&mut info, size) }.ok()?;
    let page = info.PageSize as u64;
    let total = (info.PhysicalTotal as u64).saturating_mul(page);
    let free = (info.PhysicalAvailable as u64)
        .saturating_mul(page)
        .min(total);
    let used = total - free;
    let wired = (info.KernelNonpaged as u64).saturating_mul(page).min(used);
    Some(MemoryBreakdown {
        app_bytes: used - wired,
        wired_bytes: wired,
        free_bytes: free,
        ..Default::default()
    })
}

/// 进程的专用工作集：任务管理器「内存」一列的口径。sysinfo 给的是整个工作集，
/// 连和别的进程共享的 DLL 页一起算，浏览器这类多进程应用会被高估很多。
/// 受保护的系统进程、权限不够的进程读不到。
pub fn process_memory(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX2 {
        cb: size,
        ..Default::default()
    };
    // SAFETY: 句柄是 `OpenProcess` 刚给的，用完就关；缓冲区是一个 `PROCESS_MEMORY_COUNTERS_EX2`，
    // 长度如实传入（它以 `PROCESS_MEMORY_COUNTERS` 开头，接口按 `cb` 决定写多少）。
    let result = unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let result = GetProcessMemoryInfo(
            process,
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(),
            size,
        );
        let _ = CloseHandle(process);
        result
    };
    result.ok()?;
    Some(counters.PrivateWorkingSetSize as u64)
}

pub fn desktop_env() -> Option<String> {
    Some("Explorer".to_string())
}

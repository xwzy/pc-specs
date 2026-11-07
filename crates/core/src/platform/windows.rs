use crate::model::{MemoryModule, MotherboardInfo, SensorReading};

fn nz(s: Option<String>) -> Option<String> {
    s.and_then(|v| {
        let trimmed = v.trim().to_string();
        if trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("To be filled by O.E.M.")
            || trimmed.eq_ignore_ascii_case("Default string")
            || trimmed.eq_ignore_ascii_case("System manufacturer")
            || trimmed.eq_ignore_ascii_case("System Product Name")
            || trimmed.eq_ignore_ascii_case("None")
            || trimmed.eq_ignore_ascii_case("Not Specified")
        {
            None
        } else {
            Some(trimmed)
        }
    })
}

pub fn motherboard() -> Option<MotherboardInfo> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

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
        version: Option<String>,
        release_date: Option<String>,
    }

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Chassis {
        chassis_types: Option<Vec<u16>>,
    }

    let com = match COMLibrary::new() {
        Ok(c) => c,
        Err(_) => return None,
    };
    let conn = match WMIConnection::new(com) {
        Ok(c) => c,
        Err(_) => return None,
    };

    let board: Vec<BaseBoard> = conn
        .raw_query("SELECT Manufacturer, Product, Version, SerialNumber FROM Win32_BaseBoard")
        .unwrap_or_default();
    let bios: Vec<Bios> = conn
        .raw_query("SELECT Manufacturer, Version, ReleaseDate FROM Win32_BIOS")
        .unwrap_or_default();
    let chassis: Vec<Chassis> = conn
        .raw_query("SELECT ChassisTypes FROM Win32_SystemEnclosure")
        .unwrap_or_default();

    let board0 = board.into_iter().next();
    let bios0 = bios.into_iter().next();

    let chassis_str = chassis
        .into_iter()
        .next()
        .and_then(|c| c.chassis_types)
        .and_then(|v| v.into_iter().next())
        .map(decode_chassis);

    Some(MotherboardInfo {
        vendor: nz(board0.as_ref().and_then(|b| b.manufacturer.clone())),
        model: nz(board0.as_ref().and_then(|b| b.product.clone())),
        version: nz(board0.as_ref().and_then(|b| b.version.clone())),
        serial: nz(board0.and_then(|b| b.serial_number)),
        bios_vendor: nz(bios0.as_ref().and_then(|b| b.manufacturer.clone())),
        bios_version: nz(bios0.as_ref().and_then(|b| b.version.clone())),
        bios_date: nz(bios0.and_then(|b| b.release_date)).map(|s| normalize_wmi_date(&s)),
        chassis: chassis_str,
    })
}

/// WMI ReleaseDate 形如 "20231005000000.000000+000" → "2023-10-05"
fn normalize_wmi_date(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 8 && trimmed[..8].chars().all(|c| c.is_ascii_digit()) {
        return format!("{}-{}-{}", &trimmed[..4], &trimmed[4..6], &trimmed[6..8]);
    }
    trimmed.to_string()
}

fn decode_chassis(t: u16) -> String {
    match t {
        1 | 2 => "Other".to_string(),
        3 | 4 | 5 | 6 | 7 | 15 | 16 => "Desktop".to_string(),
        8 | 9 | 10 | 11 | 14 => "Laptop".to_string(),
        12 | 21 => "Mini-PC".to_string(),
        13 => "All-in-One".to_string(),
        17..=20 | 23 | 24 => "Server".to_string(),
        _ => format!("Type-{t}"),
    }
}

pub fn memory_modules() -> Vec<MemoryModule> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    // Win32_PhysicalMemory.MemoryType 在 Windows 10/11 + DDR4/DDR5 系统上经常返回 0
    // (Unknown)，因为该字段是 Windows XP 时代定义、未跟进 SMBIOS 新值。
    // SMBIOSMemoryType 直接使用 SMBIOS 标准值，更准确。
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
        smbios_memory_type: Option<u32>,
        form_factor: Option<u16>,
    }

    let com = match COMLibrary::new() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let conn = match WMIConnection::new(com) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let rows: Vec<PhysicalMemory> = conn
        .raw_query(
            "SELECT DeviceLocator, Manufacturer, PartNumber, Capacity, Speed, ConfiguredClockSpeed, MemoryType, SMBIOSMemoryType, FormFactor FROM Win32_PhysicalMemory",
        )
        .unwrap_or_default();
    rows.into_iter()
        .map(|m| {
            // 优先 MemoryType（旧字段，仍然准确的 DDR/DDR2/DDR3 系统）；
            // 为 0 时 fallback 到 SMBIOSMemoryType（DDR4/DDR5 必备）。
            let kind = match m.memory_type {
                Some(t) if t != 0 => Some(decode_memory_type(t)),
                _ => m.smbios_memory_type.and_then(|t| {
                    if t == 0 {
                        None
                    } else {
                        Some(decode_smbios_memory_type(t))
                    }
                }),
            };
            // ConfiguredClockSpeed 是实际运行频率（XMP 启用后比 Speed 更接近真实值）。
            let speed_mt_s = m
                .configured_clock_speed
                .filter(|n| *n > 0)
                .or(m.speed.filter(|n| *n > 0));
            MemoryModule {
                slot: nz(m.device_locator).unwrap_or_else(|| "DIMM".to_string()),
                manufacturer: nz(m.manufacturer),
                part_number: nz(m.part_number),
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

/// SMBIOS Memory Device Type (SMBIOS spec table 17 + DSP0134)。
/// 仅列出常见值，未列出的 fallback 到 "Type-N"。
fn decode_smbios_memory_type(t: u32) -> String {
    match t {
        18 => "DDR".to_string(),
        19 | 22 => "DDR2".to_string(),
        24 => "DDR3".to_string(),
        26 => "DDR4".to_string(),
        27 | 28 => "LPDDR3".to_string(),
        29 => "LPDDR4".to_string(),
        30 => "Logical".to_string(),
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

pub fn sensors() -> Vec<SensorReading> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct ThermalZone {
        instance_name: Option<String>,
        current_temperature: Option<u32>,
    }

    let com = match COMLibrary::new() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let conn = match WMIConnection::with_namespace_path("ROOT\\WMI", com) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let rows: Vec<ThermalZone> = conn
        .raw_query("SELECT InstanceName, CurrentTemperature FROM MSAcpi_ThermalZoneTemperature")
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|t| {
            let raw = t.current_temperature?;
            let c = (raw as f32 / 10.0) - 273.15;
            Some(SensorReading {
                source: "wmi".to_string(),
                label: t.instance_name.unwrap_or_else(|| "ThermalZone".to_string()),
                kind: "temperature".to_string(),
                value: c,
                unit: "C".to_string(),
            })
        })
        .collect()
}

pub fn cpu_sockets() -> Option<u32> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Cs {
        number_of_processors: Option<u32>,
    }
    let com = COMLibrary::new().ok()?;
    let conn = WMIConnection::new(com).ok()?;
    let rows: Vec<Cs> = conn
        .raw_query("SELECT NumberOfProcessors FROM Win32_ComputerSystem")
        .ok()?;
    rows.into_iter().next().and_then(|r| r.number_of_processors)
}

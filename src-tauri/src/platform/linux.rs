use crate::model::{MemoryModule, MotherboardInfo, SensorReading};
use std::fs;
use std::path::Path;

fn read_trim(path: &str) -> Option<String> {
    let v = fs::read_to_string(path).ok()?.trim().to_string();
    if v.is_empty()
        || v.eq_ignore_ascii_case("To be filled by O.E.M.")
        || v.eq_ignore_ascii_case("Default string")
        || v.eq_ignore_ascii_case("System manufacturer")
        || v.eq_ignore_ascii_case("System Product Name")
        || v.eq_ignore_ascii_case("Not Specified")
        || v.eq_ignore_ascii_case("None")
        || v == "0x00"
    {
        None
    } else {
        Some(v)
    }
}

pub fn motherboard() -> Option<MotherboardInfo> {
    let dmi = "/sys/class/dmi/id";
    if !Path::new(dmi).exists() {
        return None;
    }
    let chassis = read_trim(&format!("{dmi}/chassis_type")).map(|t| {
        match t.parse::<u16>().unwrap_or(0) {
            3 | 4 | 5 | 6 | 7 | 15 | 16 => "Desktop".to_string(),
            8 | 9 | 10 | 11 | 14 => "Laptop".to_string(),
            12 | 21 => "Mini-PC".to_string(),
            13 => "All-in-One".to_string(),
            17..=20 | 23 | 24 => "Server".to_string(),
            _ => format!("Type-{t}"),
        }
    });
    Some(MotherboardInfo {
        vendor: read_trim(&format!("{dmi}/board_vendor"))
            .or_else(|| read_trim(&format!("{dmi}/sys_vendor"))),
        model: read_trim(&format!("{dmi}/board_name"))
            .or_else(|| read_trim(&format!("{dmi}/product_name"))),
        version: read_trim(&format!("{dmi}/board_version")),
        serial: read_trim(&format!("{dmi}/board_serial")),
        bios_vendor: read_trim(&format!("{dmi}/bios_vendor")),
        bios_version: read_trim(&format!("{dmi}/bios_version")),
        bios_date: read_trim(&format!("{dmi}/bios_date")),
        chassis,
    })
}

pub fn cpu_sockets() -> Option<u32> {
    let content = fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut ids = std::collections::HashSet::new();
    for line in content.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim() == "physical id" {
                ids.insert(v.trim().to_string());
            }
        }
    }
    if ids.is_empty() {
        None
    } else {
        Some(ids.len() as u32)
    }
}

pub fn memory_modules() -> Vec<MemoryModule> {
    Vec::new()
}

pub fn cpu_brand() -> Option<String> {
    let content = fs::read_to_string("/proc/cpuinfo").ok()?;
    for line in content.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim() == "model name" {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

pub fn sensors() -> Vec<SensorReading> {
    // hwmon 子文件名规则（Linux Kernel docs / Documentation/hwmon/sysfs-interface）：
    //   tempN_input    单位 1/1000 °C
    //   fanN_input     单位 RPM
    //   inN_input      单位 mV（电压）—— 注意 in0_input 通常是 VID 不是 vcore
    //   currN_input    单位 mA（电流）
    //   powerN_input   单位 µW（功率，注意是微瓦不是毫瓦）
    // 每个 channel 都可能有 *_label 文件给可读名字。
    let mut out = Vec::new();
    let hwmon_root = Path::new("/sys/class/hwmon");
    let entries = match fs::read_dir(hwmon_root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let source = read_trim(dir.join("name").to_string_lossy().as_ref())
            .unwrap_or_else(|| entry.file_name().to_string_lossy().to_string());
        let inner = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for f in inner.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            let path = f.path();
            if !name.ends_with("_input") {
                continue;
            }
            let raw: f32 = match read_trim(path.to_string_lossy().as_ref())
                .and_then(|s| s.parse::<f32>().ok())
            {
                Some(v) => v,
                None => continue,
            };
            let label_path = path.with_file_name(name.replace("_input", "_label"));
            let label = read_trim(label_path.to_string_lossy().as_ref())
                .unwrap_or_else(|| name.replace("_input", ""));

            let (kind, value, unit) = if name.starts_with("temp") {
                let v = raw / 1000.0;
                if !(-50.0..200.0).contains(&v) {
                    continue;
                }
                ("temperature".to_string(), v, "C".to_string())
            } else if name.starts_with("fan") {
                if !(0.0..50000.0).contains(&raw) {
                    continue;
                }
                ("fan".to_string(), raw, "RPM".to_string())
            } else if name.starts_with("in") && name.chars().nth(2).map(|c| c.is_ascii_digit()).unwrap_or(false) {
                // mV → V
                let v = raw / 1000.0;
                if !(-30.0..30.0).contains(&v) || raw == 0.0 {
                    continue;
                }
                ("voltage".to_string(), v, "V".to_string())
            } else if name.starts_with("curr") {
                // mA → A
                let v = raw / 1000.0;
                if !(-200.0..200.0).contains(&v) {
                    continue;
                }
                ("current".to_string(), v, "A".to_string())
            } else if name.starts_with("power") {
                // µW → W
                let v = raw / 1_000_000.0;
                if !(-2000.0..2000.0).contains(&v) || raw == 0.0 {
                    continue;
                }
                ("power".to_string(), v, "W".to_string())
            } else {
                continue;
            };
            out.push(SensorReading {
                source: source.clone(),
                label,
                kind,
                value,
                unit,
            });
        }
    }
    out
}

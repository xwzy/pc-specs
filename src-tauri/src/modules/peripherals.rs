use crate::model::PeripheralInfo;

pub fn collect() -> Vec<PeripheralInfo> {
    let mut out = collect_platform();
    out.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    out.dedup_by(|a, b| {
        a.kind == b.kind
            && a.name == b.name
            && a.vendor_id == b.vendor_id
            && a.product_id == b.product_id
    });
    out
}

#[cfg(target_os = "macos")]
fn collect_platform() -> Vec<PeripheralInfo> {
    use std::process::Command;
    let out = match Command::new("system_profiler")
        .args(["-json", "SPUSBDataType"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let v: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut acc = Vec::new();
    if let Some(arr) = v.get("SPUSBDataType").and_then(|x| x.as_array()) {
        for node in arr {
            walk_macos_usb(node, &mut acc);
        }
    }
    acc
}

#[cfg(target_os = "macos")]
fn walk_macos_usb(node: &serde_json::Value, acc: &mut Vec<PeripheralInfo>) {
    let name = node
        .get("_name")
        .and_then(|s| s.as_str())
        .unwrap_or("USB Device")
        .to_string();
    let vendor_id = node.get("vendor_id").and_then(|s| s.as_str()).map(|s| {
        // macOS 返回类似 "0x05ac (Apple Inc.)"，截首段
        s.split_whitespace().next().unwrap_or(s).to_string()
    });
    let product_id = node
        .get("product_id")
        .and_then(|s| s.as_str())
        .map(|s| s.split_whitespace().next().unwrap_or(s).to_string());
    let bus = node
        .get("location_id")
        .and_then(|s| s.as_str())
        .map(|s| s.split_whitespace().next().unwrap_or(s).to_string());

    let is_hub = name.to_lowercase().contains("hub")
        || node
            .get("_items")
            .map(|x| x.is_array())
            .unwrap_or(false);
    let is_root = name.to_lowercase().contains("usb bus");

    if !is_root && (vendor_id.is_some() || !is_hub) {
        acc.push(PeripheralInfo {
            kind: "usb".to_string(),
            name,
            vendor_id,
            product_id,
            bus,
        });
    }
    if let Some(arr) = node.get("_items").and_then(|x| x.as_array()) {
        for child in arr {
            walk_macos_usb(child, acc);
        }
    }
}

#[cfg(target_os = "linux")]
fn collect_platform() -> Vec<PeripheralInfo> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir("/sys/bus/usb/devices") {
        Ok(e) => e,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let dir = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        // 跳过 usb 控制器（1-0:1.0 这种）
        if name.contains(':') {
            continue;
        }
        let vid = read_hex_id(&dir.join("idVendor"));
        let pid = read_hex_id(&dir.join("idProduct"));
        if vid.is_none() && pid.is_none() {
            continue;
        }
        // 过滤 Linux Foundation root hub（vendor 0x1d6b）—— 它们是内核虚拟设备，不是真实外设
        if vid.as_deref() == Some("0x1d6b") {
            continue;
        }
        let product = std::fs::read_to_string(dir.join("product"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let manuf = std::fs::read_to_string(dir.join("manufacturer"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let display_name = match (manuf.as_deref(), product.as_deref()) {
            (Some(m), Some(p)) => format!("{m} {p}"),
            (None, Some(p)) => p.to_string(),
            (Some(m), None) => m.to_string(),
            _ => match (vid.as_deref(), pid.as_deref()) {
                (Some(v), Some(p)) => format!("USB Device {v}:{p}"),
                _ => format!("USB Device {name}"),
            },
        };
        out.push(PeripheralInfo {
            kind: "usb".to_string(),
            name: display_name,
            vendor_id: vid,
            product_id: pid,
            bus: Some(name),
        });
    }
    out
}

#[cfg(target_os = "linux")]
fn read_hex_id(path: &std::path::Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(format!("0x{s}"))
    }
}

#[cfg(target_os = "windows")]
fn collect_platform() -> Vec<PeripheralInfo> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct PnP {
        name: Option<String>,
        device_id: Option<String>,
        manufacturer: Option<String>,
        pnp_class: Option<String>,
    }

    let com = match COMLibrary::new() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let conn = match WMIConnection::new(com) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    // 只取我们关心的几类，避免上百条键鼠 / 系统设备噪音
    let interesting: &[&str] = &[
        "USB", "Bluetooth", "Camera", "Image", "Media", "AudioEndpoint", "HIDClass", "Printer",
    ];
    let where_clause = interesting
        .iter()
        .map(|k| format!("PNPClass='{k}'"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let q = format!(
        "SELECT Name, DeviceID, Manufacturer, PNPClass FROM Win32_PnPEntity WHERE {where_clause}"
    );
    let rows: Vec<PnP> = conn.raw_query(&q).unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            let name = r.name?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            let (vid, pid) = parse_usb_id(r.device_id.as_deref().unwrap_or(""));
            let kind = match r.pnp_class.as_deref().unwrap_or("USB") {
                "Bluetooth" => "bluetooth",
                "Camera" | "Image" => "camera",
                "Media" | "AudioEndpoint" => "audio",
                "Printer" => "printer",
                _ => "usb",
            }
            .to_string();
            Some(PeripheralInfo {
                kind,
                name,
                vendor_id: vid,
                product_id: pid,
                bus: None,
            })
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn parse_usb_id(device_id: &str) -> (Option<String>, Option<String>) {
    // 形如: USB\VID_046D&PID_C52B\...
    let mut vid = None;
    let mut pid = None;
    for tok in device_id.split(|c: char| c == '\\' || c == '&') {
        if let Some(rest) = tok.strip_prefix("VID_") {
            vid = Some(format!("0x{}", rest.to_lowercase()));
        } else if let Some(rest) = tok.strip_prefix("PID_") {
            pid = Some(format!("0x{}", rest.to_lowercase()));
        }
    }
    (vid, pid)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn collect_platform() -> Vec<PeripheralInfo> {
    Vec::new()
}

use crate::model::PeripheralInfo;

/// 外设清单（USB / 蓝牙 / 摄像头 / 音频）。`kind` 取值：usb、bluetooth、camera、audio、printer。
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
fn first_word(s: &str) -> String {
    s.split_whitespace().next().unwrap_or(s).to_string()
}

/// `system_profiler` 给的电量是 `"85%"` 这样的字符串。
#[cfg(any(target_os = "macos", test))]
fn percent(text: &str) -> Option<u8> {
    text.trim()
        .trim_end_matches('%')
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|percent| *percent <= 100)
}

/// 蓝牙设备的电量。耳机左右耳、充电盒各有各的：取设备本身最低的那个，
/// 充电盒的电量说明不了耳机要不要充电。
#[cfg(any(target_os = "macos", test))]
fn bluetooth_battery(props: &serde_json::Value) -> Option<u8> {
    [
        "device_batteryLevelMain",
        "device_batteryLevelLeft",
        "device_batteryLevelRight",
    ]
    .iter()
    .filter_map(|key| props.get(*key).and_then(|v| v.as_str()).and_then(percent))
    .min()
}

// ------------------------------------------------------------------ macOS

#[cfg(target_os = "macos")]
fn collect_platform() -> Vec<PeripheralInfo> {
    use crate::platform::macos::system_profiler;

    // 四个 system_profiler 互相独立，并行跑，总耗时取最慢的一个。
    let (usb, cam, audio, bt) = std::thread::scope(|s| {
        let usb = s.spawn(|| system_profiler("SPUSBDataType"));
        let cam = s.spawn(|| system_profiler("SPCameraDataType"));
        let audio = s.spawn(|| system_profiler("SPAudioDataType"));
        let bt = s.spawn(|| system_profiler("SPBluetoothDataType"));
        (
            usb.join().ok().flatten(),
            cam.join().ok().flatten(),
            audio.join().ok().flatten(),
            bt.join().ok().flatten(),
        )
    });

    let mut acc = Vec::new();
    let items = |root: &Option<serde_json::Value>, key: &str| -> Vec<serde_json::Value> {
        root.as_ref()
            .and_then(|r| r.get(key))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };

    for node in items(&usb, "SPUSBDataType") {
        walk_usb(&node, &mut acc);
    }
    for c in items(&cam, "SPCameraDataType") {
        if let Some(name) = c.get("_name").and_then(|v| v.as_str()) {
            acc.push(PeripheralInfo {
                kind: "camera".to_string(),
                name: name.to_string(),
                ..Default::default()
            });
        }
    }
    for group in items(&audio, "SPAudioDataType") {
        for d in group
            .get("_items")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(name) = d.get("_name").and_then(|v| v.as_str()) else {
                continue;
            };
            let transport = d
                .get("coreaudio_device_transport")
                .and_then(|v| v.as_str())
                .map(|t| t.trim_start_matches("coreaudio_device_type_").to_string());
            acc.push(PeripheralInfo {
                kind: "audio".to_string(),
                name: name.to_string(),
                vendor_id: d
                    .get("coreaudio_device_manufacturer")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                bus: transport,
                ..Default::default()
            });
        }
    }
    for ctrl in items(&bt, "SPBluetoothDataType") {
        // 只列当前已连接的设备；历史配对记录属于隐私噪音。
        for entry in ctrl
            .get("device_connected")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(map) = entry.as_object() else {
                continue;
            };
            for (name, props) in map {
                let text = |k: &str| props.get(k).and_then(|v| v.as_str());
                acc.push(PeripheralInfo {
                    kind: "bluetooth".to_string(),
                    name: name.clone(),
                    vendor_id: text("device_vendorID").map(first_word),
                    product_id: text("device_productID").map(first_word),
                    bus: text("device_minorType").map(str::to_string),
                    battery_percent: bluetooth_battery(props),
                });
            }
        }
    }
    acc
}

#[cfg(target_os = "macos")]
fn walk_usb(node: &serde_json::Value, acc: &mut Vec<PeripheralInfo>) {
    let text = |k: &str| node.get(k).and_then(|s| s.as_str());
    let name = text("_name").unwrap_or("USB Device").to_string();
    // "0x05ac (Apple Inc.)" → "0x05ac"
    let vendor_id = text("vendor_id").map(first_word);
    let product_id = text("product_id").map(first_word);
    let bus = text("location_id").map(first_word);
    let has_children = node.get("_items").is_some_and(|x| x.is_array());
    let is_root =
        name.to_lowercase().contains("usb bus") || name.to_lowercase().contains("host controller");
    let is_hub = name.to_lowercase().contains("hub") || has_children;

    if !is_root && (vendor_id.is_some() || !is_hub) {
        acc.push(PeripheralInfo {
            kind: "usb".to_string(),
            name,
            vendor_id,
            product_id,
            bus,
            ..Default::default()
        });
    }
    for child in node
        .get("_items")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
    {
        walk_usb(child, acc);
    }
}

// ------------------------------------------------------------------ Linux

#[cfg(target_os = "linux")]
fn collect_platform() -> Vec<PeripheralInfo> {
    let mut out = usb_devices();
    out.extend(cameras());
    out.extend(audio_cards());
    out.extend(bluetooth_connected());
    out
}

#[cfg(target_os = "linux")]
fn read_trim(path: &std::path::Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

#[cfg(target_os = "linux")]
fn usb_devices() -> Vec<PeripheralInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else {
        return out;
    };
    for e in entries.flatten() {
        let dir = e.path();
        let bus = e.file_name().to_string_lossy().to_string();
        // "1-0:1.0" 这类是接口而非设备。
        if bus.contains(':') {
            continue;
        }
        let vid = read_trim(&dir.join("idVendor")).map(|s| format!("0x{s}"));
        let pid = read_trim(&dir.join("idProduct")).map(|s| format!("0x{s}"));
        if vid.is_none() && pid.is_none() {
            continue;
        }
        // Linux Foundation 的 root hub 是内核虚拟设备，不是真实外设。
        if vid.as_deref() == Some("0x1d6b") {
            continue;
        }
        let product = read_trim(&dir.join("product"));
        let manuf = read_trim(&dir.join("manufacturer"));
        let name = match (manuf.as_deref(), product.as_deref()) {
            (Some(m), Some(p)) if p.starts_with(m) => p.to_string(),
            (Some(m), Some(p)) => format!("{m} {p}"),
            (None, Some(p)) => p.to_string(),
            (Some(m), None) => m.to_string(),
            _ => format!(
                "USB device {}:{}",
                vid.as_deref().unwrap_or("?"),
                pid.as_deref().unwrap_or("?")
            ),
        };
        out.push(PeripheralInfo {
            kind: "usb".to_string(),
            name,
            vendor_id: vid,
            product_id: pid,
            bus: Some(bus),
            ..Default::default()
        });
    }
    out
}

/// video4linux：同一个摄像头会暴露采集 + 元数据两个节点，只取 index 0。
#[cfg(target_os = "linux")]
fn cameras() -> Vec<PeripheralInfo> {
    let Ok(entries) = std::fs::read_dir("/sys/class/video4linux") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            if read_trim(&dir.join("index"))? != "0" {
                return None;
            }
            Some(PeripheralInfo {
                kind: "camera".to_string(),
                name: read_trim(&dir.join("name"))?,
                ..Default::default()
            })
        })
        .collect()
}

/// ALSA 声卡：" 0 [PCH            ]: HDA-Intel - HDA Intel PCH"
#[cfg(target_os = "linux")]
fn audio_cards() -> Vec<PeripheralInfo> {
    let Ok(text) = std::fs::read_to_string("/proc/asound/cards") else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let (head, rest) = l.split_once("]:")?;
            head.trim_start()
                .chars()
                .next()
                .filter(char::is_ascii_digit)?;
            let (driver, name) = rest.split_once(" - ")?;
            Some(PeripheralInfo {
                kind: "audio".to_string(),
                name: name.trim().to_string(),
                bus: Some(driver.trim().to_string()),
                ..Default::default()
            })
        })
        .collect()
}

/// BlueZ：`bluetoothctl devices Connected` → "Device AA:BB:CC:DD:EE:FF Name"
#[cfg(target_os = "linux")]
fn bluetooth_connected() -> Vec<PeripheralInfo> {
    let Some(text) = crate::util::run_stdout_timeout(
        "bluetoothctl",
        &["devices", "Connected"],
        std::time::Duration::from_secs(2),
    ) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Device ")?;
            let (addr, name) = rest.split_once(' ')?;
            // 电量要另外问：`bluetoothctl info <地址>`。设备没报电量（或者系统没开这个功能）就是没有。
            let battery_percent = crate::util::run_stdout_timeout(
                "bluetoothctl",
                &["info", addr],
                std::time::Duration::from_secs(2),
            )
            .and_then(|info| bluez_battery(&info));
            Some(PeripheralInfo {
                kind: "bluetooth".to_string(),
                name: name.trim().to_string(),
                battery_percent,
                ..Default::default()
            })
        })
        .collect()
}

/// `bluetoothctl info` 里的一行：`Battery Percentage: 0x5a (90)`。括号里是十进制的值。
#[cfg(any(target_os = "linux", test))]
fn bluez_battery(info: &str) -> Option<u8> {
    let line = info
        .lines()
        .find_map(|l| l.trim().strip_prefix("Battery Percentage:"))?;
    let decimal = line
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .and_then(|(value, _)| value.trim().parse::<u8>().ok());
    // 没有括号的话，退一步读十六进制的那个。
    let hex = line
        .split_whitespace()
        .next()
        .and_then(|value| value.strip_prefix("0x"))
        .and_then(|value| u8::from_str_radix(value, 16).ok());
    decimal.or(hex).filter(|percent| *percent <= 100)
}

// ---------------------------------------------------------------- Windows

#[cfg(target_os = "windows")]
fn collect_platform() -> Vec<PeripheralInfo> {
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct PnP {
        name: Option<String>,
        #[serde(rename = "DeviceID")]
        device_id: Option<String>,
        #[serde(rename = "PNPClass")]
        pnp_class: Option<String>,
    }

    let Some(conn) = crate::platform::windows::wmi_cimv2() else {
        return Vec::new();
    };
    // 只取关心的几类，避免上百条系统设备噪音。
    let classes = [
        "USB",
        "Bluetooth",
        "Camera",
        "Image",
        "Media",
        "AudioEndpoint",
        "HIDClass",
        "Printer",
    ];
    let filter = classes
        .iter()
        .map(|k| format!("PNPClass='{k}'"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let rows: Vec<PnP> = conn
        .raw_query(format!(
            "SELECT Name, DeviceID, PNPClass FROM Win32_PnPEntity WHERE {filter}"
        ))
        .unwrap_or_default();

    rows.into_iter()
        .filter_map(|r| {
            let name = r.name?.trim().to_string();
            let lower = name.to_lowercase();
            // 控制器 / 集线器 / 通用 HID 子节点没有信息量。
            const NOISE: [&str; 6] = [
                "root hub",
                "composite device",
                "host controller",
                "generic usb hub",
                "usb input device",
                "hid-compliant",
            ];
            if name.is_empty() || NOISE.iter().any(|n| lower.contains(n)) {
                return None;
            }
            let (vendor_id, product_id) = parse_usb_id(r.device_id.as_deref().unwrap_or(""));
            let kind = match r.pnp_class.as_deref().unwrap_or("USB") {
                "Bluetooth" => "bluetooth",
                "Camera" | "Image" => "camera",
                "Media" | "AudioEndpoint" => "audio",
                "Printer" => "printer",
                _ => "usb",
            };
            Some(PeripheralInfo {
                kind: kind.to_string(),
                name,
                vendor_id,
                product_id,
                ..Default::default()
            })
        })
        .collect()
}

/// `USB\VID_046D&PID_C52B\...` → ("0x046d", "0xc52b")
#[cfg(target_os = "windows")]
fn parse_usb_id(device_id: &str) -> (Option<String>, Option<String>) {
    let mut vid = None;
    let mut pid = None;
    for tok in device_id.split(['\\', '&']) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_bluetooth_device_reports_its_lowest_own_battery() {
        assert_eq!(percent("85%"), Some(85));
        assert_eq!(percent(" 100 % "), Some(100));
        assert_eq!(percent("n/a"), None);
        assert_eq!(percent("250%"), None);

        // 键盘、鼠标只有一个电量。
        assert_eq!(
            bluetooth_battery(&json!({ "device_batteryLevelMain": "72%" })),
            Some(72)
        );
        // 耳机：左右耳取低的，充电盒不算。
        assert_eq!(
            bluetooth_battery(&json!({
                "device_batteryLevelLeft": "60%",
                "device_batteryLevelRight": "45%",
                "device_batteryLevelCase": "10%",
            })),
            Some(45)
        );
        assert_eq!(
            bluetooth_battery(&json!({ "device_minorType": "Mouse" })),
            None
        );
    }

    #[test]
    fn bluez_prints_the_battery_in_hex_and_decimal() {
        let info = "Device AA:BB:CC:DD:EE:FF (public)\n\tName: MX Master\n\tConnected: yes\n\tBattery Percentage: 0x5a (90)\n";
        assert_eq!(bluez_battery(info), Some(90));
        assert_eq!(bluez_battery("\tBattery Percentage: 0x32\n"), Some(50));
        assert_eq!(bluez_battery("\tConnected: yes\n"), None);
        assert_eq!(bluez_battery("\tBattery Percentage: 0xff (255)\n"), None);
    }
}

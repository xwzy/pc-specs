use crate::model::GpuInfo;
use wgpu::{Backends, DeviceType, Instance, InstanceDescriptor};

pub fn collect() -> Vec<GpuInfo> {
    let instance = Instance::new(InstanceDescriptor {
        backends: Backends::all(),
        ..Default::default()
    });

    let adapters: Vec<_> = instance.enumerate_adapters(Backends::all());
    let drm = drm_extras();
    let mut wmi_info: Vec<WmiVideo> = wmi_extras();

    let mut raw: Vec<GpuInfo> = adapters
        .into_iter()
        .enumerate()
        .map(|(i, adapter)| {
            let info = adapter.get_info();
            let is_software = is_software_adapter(&info.name, info.device_type);
            let drm_match = if !is_software {
                drm.iter().find(|d| name_matches(&info.name, &d.label)).cloned()
            } else {
                None
            };
            let wmi_match_idx = if !is_software {
                wmi_info.iter().position(|w| name_matches(&info.name, &w.name))
            } else {
                None
            };
            let wmi_match = wmi_match_idx.map(|idx| wmi_info.swap_remove(idx));

            let vram_total = drm_match
                .as_ref()
                .and_then(|d| d.vram_bytes)
                .or_else(|| wmi_match.as_ref().and_then(|w| w.vram_bytes));
            let pcie_link = drm_match.as_ref().and_then(|d| d.pcie_link.clone());
            let driver = if info.driver.is_empty() {
                wmi_match.as_ref().and_then(|w| w.driver.clone())
            } else {
                Some(info.driver.clone())
            };

            GpuInfo {
                index: i as u32,
                vendor: if is_software {
                    "Software".to_string()
                } else {
                    vendor_name(info.vendor)
                },
                name: info.name,
                backend: if is_software {
                    "Software".to_string()
                } else {
                    format!("{:?}", info.backend)
                },
                driver,
                vram_total_bytes: vram_total,
                vram_used_bytes: None,
                utilization: None,
                temperature_c: None,
                power_w: None,
                pcie_link,
                is_discrete: !is_software && matches!(info.device_type, DeviceType::DiscreteGpu),
            }
        })
        .collect();

    dedup_by_physical(&mut raw);
    // 重排 index 使其连续
    for (i, g) in raw.iter_mut().enumerate() {
        g.index = i as u32;
    }
    raw
}

/// 同一物理 GPU 在 Linux 上可能被 wgpu 通过 Vulkan + OpenGL + GLES 各报一次。
/// 按 (vendor, name) 去重，优先保留 Vulkan / Metal / Dx12，再退到 GL / Software。
fn dedup_by_physical(items: &mut Vec<GpuInfo>) {
    fn backend_priority(b: &str) -> u8 {
        match b {
            "Vulkan" | "Metal" | "Dx12" => 0,
            "Dx11" => 1,
            "Gl" | "GL" | "OpenGL" => 2,
            _ => 3,
        }
    }
    items.sort_by(|a, b| {
        a.vendor
            .cmp(&b.vendor)
            .then(a.name.cmp(&b.name))
            .then(backend_priority(&a.backend).cmp(&backend_priority(&b.backend)))
    });
    let mut seen = std::collections::HashSet::new();
    items.retain(|g| seen.insert((g.vendor.clone(), g.name.clone())));
}

fn name_matches(wgpu_name: &str, other: &str) -> bool {
    let a = wgpu_name.to_lowercase();
    let b = other.to_lowercase();
    if a == b {
        return true;
    }
    // 取第一个空格前的关键词比较，wgpu 名字会带 "(Discrete GPU)" 后缀，WMI/DRM 通常更短
    let key_a = a.split('(').next().unwrap_or(&a).trim();
    let key_b = b.split('(').next().unwrap_or(&b).trim();
    if key_a.contains(key_b) || key_b.contains(key_a) {
        return true;
    }
    false
}

#[derive(Clone)]
struct DrmInfo {
    label: String,
    vram_bytes: Option<u64>,
    pcie_link: Option<String>,
}

#[cfg(target_os = "linux")]
fn drm_extras() -> Vec<DrmInfo> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir("/sys/class/drm") {
        Ok(e) => e,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.starts_with("card") || name.contains('-') {
            // 跳过 card0-eDP-1 这种 connector 目录
            continue;
        }
        let dev_dir = e.path().join("device");
        if !dev_dir.exists() {
            continue;
        }
        let vendor = read_trim(&dev_dir.join("vendor"));
        let device = read_trim(&dev_dir.join("device"));
        let label = format!("{} {}", vendor.unwrap_or_default(), device.unwrap_or_default());

        let vram_bytes = read_trim(&dev_dir.join("mem_info_vram_total"))
            .and_then(|s| s.parse::<u64>().ok());
        let speed = read_trim(&dev_dir.join("current_link_speed"));
        let width = read_trim(&dev_dir.join("current_link_width"));
        let pcie_link = match (speed, width) {
            (Some(s), Some(w)) => Some(format_pcie_link(&s, &w)),
            _ => None,
        };
        out.push(DrmInfo {
            label,
            vram_bytes,
            pcie_link,
        });
    }
    out
}

#[cfg(target_os = "linux")]
fn read_trim(p: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 把 sysfs 给出的 "8.0 GT/s PCIe" + "16" 格式化成 "PCIe 3.0 ×16"。
/// GT/s → PCIe Gen 映射：2.5→1.0, 5.0→2.0, 8.0→3.0, 16.0→4.0, 32.0→5.0, 64.0→6.0
#[cfg(target_os = "linux")]
fn format_pcie_link(speed_raw: &str, width: &str) -> String {
    let gts: f32 = speed_raw
        .split_whitespace()
        .find_map(|tok| tok.parse::<f32>().ok())
        .unwrap_or(0.0);
    let gen = match gts as i32 {
        x if (2..=3).contains(&x) => "1.0",
        5 => "2.0",
        8 => "3.0",
        16 => "4.0",
        32 => "5.0",
        64 => "6.0",
        _ => return format!("{} ×{}", speed_raw.trim(), width.trim()),
    };
    format!("PCIe {gen} ×{}", width.trim())
}

#[cfg(not(target_os = "linux"))]
fn drm_extras() -> Vec<DrmInfo> {
    Vec::new()
}

struct WmiVideo {
    name: String,
    vram_bytes: Option<u64>,
    driver: Option<String>,
}

#[cfg(target_os = "windows")]
fn wmi_extras() -> Vec<WmiVideo> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Vc {
        name: Option<String>,
        adapter_ram: Option<u32>,
        driver_version: Option<String>,
    }
    let Ok(com) = COMLibrary::new() else { return Vec::new() };
    let Ok(conn) = WMIConnection::new(com) else { return Vec::new() };
    let rows: Vec<Vc> = conn
        .raw_query("SELECT Name, AdapterRAM, DriverVersion FROM Win32_VideoController")
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|v| {
            let name = v.name?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            // AdapterRAM 是 32-bit，对 > 4GB 的现代显卡会截断；这里只在 < 4GB 时使用
            let vram = v
                .adapter_ram
                .map(|n| n as u64)
                .filter(|n| *n > 0 && *n < u32::MAX as u64);
            Some(WmiVideo {
                name,
                vram_bytes: vram,
                driver: v.driver_version,
            })
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn wmi_extras() -> Vec<WmiVideo> {
    Vec::new()
}

fn is_software_adapter(name: &str, ty: DeviceType) -> bool {
    if matches!(ty, DeviceType::Cpu) {
        return true;
    }
    let n = name.to_lowercase();
    // 软渲染 + 常见虚拟机虚拟显卡。这些都不是真实物理 GPU，应该归到 "Software"
    // 类别避免在 Dashboard 报"独立显卡"误导用户。
    [
        // 软渲染
        "llvmpipe",
        "softpipe",
        "swiftshader",
        "microsoft basic render",
        "warp",
        "cpu",
        // Hyper-V / Windows Sandbox
        "hyper-v video",
        "microsoft hyper-v",
        // VMware
        "vmware svga",
        "vmware vmsvga",
        // VirtualBox
        "virtualbox graphics",
        "vbox",
        // Parallels
        "parallels display",
        "parallels graphics",
        // QEMU / KVM
        "qxl",
        "virtio gpu",
        "virtio-gpu",
        "bochs",
        "stdvga",
        "cirrus",
        "qemu standard vga",
        // Citrix / Xen
        "xen virtual",
        "citrix",
        // 远程桌面
        "remote desktop session",
        "rdp encoder",
        "indirectdisplay",
    ]
    .iter()
    .any(|m| n.contains(m))
}

fn vendor_name(vid: u32) -> String {
    match vid {
        0x10DE => "NVIDIA".to_string(),
        0x1002 | 0x1022 => "AMD".to_string(),
        0x8086 => "Intel".to_string(),
        0x106B => "Apple".to_string(),
        0x1010 => "ImgTec".to_string(),
        0x13B5 => "ARM".to_string(),
        0x5143 => "Qualcomm".to_string(),
        0 => "Unknown".to_string(),
        v => format!("0x{v:04X}"),
    }
}

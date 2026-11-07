use crate::model::GpuInfo;
use sysinfo::Gpus;

/// 平台侧补充信息（驱动 / PCIe 链路 / 核心数 / 显存兜底）。
#[derive(Default, Clone)]
struct Extra {
    cores: Option<u32>,
    driver: Option<String>,
    pcie_link: Option<String>,
    vram_total: Option<u64>,
}

struct GpuKey {
    /// macOS / Windows 靠名字把平台侧信息对应到 GPU。
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    name: String,
    /// 形如 `0000:01:00.0`；Apple Silicon 为占位地址。仅 Linux 用它去 sysfs 查询驱动与链路。
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pci: String,
}

pub fn collect() -> Vec<GpuInfo> {
    let Ok(gpus) = Gpus::new_with_refreshed_list() else {
        return Vec::new();
    };

    let keys: Vec<GpuKey> = gpus
        .iter()
        .map(|g| GpuKey {
            name: g.model().unwrap_or("GPU").trim().to_string(),
            pci: g.pci().to_string(),
        })
        .collect();
    let extras = extras(&keys);

    let mut out: Vec<GpuInfo> = gpus
        .iter()
        .zip(extras)
        .map(|(g, extra)| {
            let name = g.model().unwrap_or("GPU").trim().to_string();
            let vendor = normalize_vendor(g.vendor().unwrap_or(""));
            let vram_total = g.total_memory().filter(|v| *v > 0).or(extra.vram_total);
            let virtual_adapter = is_virtual_adapter(&name);
            GpuInfo {
                index: 0,
                is_discrete: !virtual_adapter && is_discrete(&vendor, &name, vram_total),
                backend: if virtual_adapter {
                    "Virtual".to_string()
                } else {
                    native_api().to_string()
                },
                vendor,
                name,
                driver: extra.driver,
                vram_total_bytes: vram_total,
                vram_used_bytes: g.used_memory(),
                utilization: g.usage(),
                temperature_c: None,
                power_w: None,
                pcie_link: extra.pcie_link,
                cores: extra.cores,
            }
        })
        .collect();

    // 独显优先，其次按显存从大到小，保证 index 0 是"主力 GPU"。
    out.sort_by(|a, b| {
        b.is_discrete
            .cmp(&a.is_discrete)
            .then(b.vram_total_bytes.cmp(&a.vram_total_bytes))
            .then(a.name.cmp(&b.name))
    });
    for (i, g) in out.iter_mut().enumerate() {
        g.index = i as u32;
    }
    out
}

fn native_api() -> &'static str {
    if cfg!(target_os = "macos") {
        "Metal"
    } else if cfg!(target_os = "windows") {
        "DirectX"
    } else {
        "Vulkan"
    }
}

fn normalize_vendor(raw: &str) -> String {
    let l = raw.to_ascii_lowercase();
    if l.contains("nvidia") {
        "NVIDIA".to_string()
    } else if l.contains("advanced micro") || l.contains("amd") || l.contains("ati") {
        "AMD".to_string()
    } else if l.contains("intel") {
        "Intel".to_string()
    } else if l.contains("apple") {
        "Apple".to_string()
    } else if l.contains("qualcomm") {
        "Qualcomm".to_string()
    } else if l.contains("arm") {
        "ARM".to_string()
    } else if raw.trim().is_empty() {
        "Unknown".to_string()
    } else {
        raw.trim().to_string()
    }
}

/// 独立显卡的启发式判断：sysinfo 不直接给出集显 / 独显标记。
fn is_discrete(vendor: &str, name: &str, vram: Option<u64>) -> bool {
    let n = name.to_ascii_lowercase();
    match vendor {
        "Apple" => false,
        "NVIDIA" => !n.contains("tegra"),
        "AMD" => {
            // 核显：Radeon Graphics / Vega 8 / 680M / 780M / 890M ……
            let integrated = n.ends_with("graphics")
                || n.contains("radeon(tm) graphics")
                || n.split_whitespace().any(|t| {
                    t.len() >= 4
                        && t.ends_with('m')
                        && t[..t.len() - 1].bytes().all(|b| b.is_ascii_digit())
                })
                || (n.contains("vega")
                    && !n.contains("frontier")
                    && !n.contains("56")
                    && !n.contains("64")
                    && !n.contains("vii"));
            !integrated || (vram.unwrap_or(0) >= 4 << 30 && n.contains("rx"))
        }
        "Intel" => {
            // 独立 Arc 型号形如 A770 / B580；Arc Graphics（Meteor Lake 起）是核显。
            n.split_whitespace().any(|t| {
                let t = t.trim_matches(|c: char| !c.is_ascii_alphanumeric());
                let t = t.strip_suffix('m').unwrap_or(t);
                t.len() == 4
                    && matches!(t.as_bytes()[0], b'a' | b'b')
                    && t[1..].bytes().all(|b| b.is_ascii_digit())
            })
        }
        _ => vram.unwrap_or(0) >= 1 << 30,
    }
}

/// 虚拟机里的显示适配器 / 远程桌面伪设备，不是真实物理 GPU。
pub(crate) fn is_virtual_adapter(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "llvmpipe",
        "swiftshader",
        "microsoft basic render",
        "hyper-v video",
        "microsoft hyper-v",
        "vmware svga",
        "vmware vmsvga",
        "virtualbox graphics",
        "vbox",
        "parallels display",
        "parallels graphics",
        "qxl",
        "virtio",
        "bochs",
        "stdvga",
        "cirrus",
        "qemu standard vga",
        "xen virtual",
        "citrix",
        "remote desktop",
        "indirectdisplay",
    ]
    .iter()
    .any(|m| n.contains(m))
}

// ------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
fn extras(keys: &[GpuKey]) -> Vec<Extra> {
    use crate::platform::macos::system_profiler;
    let entries: Vec<serde_json::Value> = system_profiler("SPDisplaysDataType")
        .and_then(|v| {
            v.get("SPDisplaysDataType")
                .and_then(|a| a.as_array().cloned())
        })
        .unwrap_or_default();

    keys.iter()
        .map(|k| {
            let entry = entries
                .iter()
                .find(|e| {
                    e.get("sppci_model")
                        .and_then(|m| m.as_str())
                        .is_some_and(|m| m.eq_ignore_ascii_case(&k.name))
                })
                .or(if keys.len() == 1 {
                    entries.first()
                } else {
                    None
                });
            let Some(e) = entry else {
                return Extra::default();
            };
            let text = |key: &str| e.get(key).and_then(|v| v.as_str());
            Extra {
                cores: text("sppci_cores").and_then(|s| s.trim().parse().ok()),
                driver: text("spdisplays_mtlgpufamilysupport").map(metal_label),
                pcie_link: None,
                vram_total: text("spdisplays_vram")
                    .or_else(|| text("spdisplays_vram_shared"))
                    .and_then(parse_gib),
            }
        })
        .collect()
}

/// "spdisplays_metal4" → "Metal 4"
#[cfg(target_os = "macos")]
fn metal_label(s: &str) -> String {
    let rest = s.trim_start_matches("spdisplays_metal");
    if rest.is_empty() || rest.len() == s.len() {
        s.to_string()
    } else {
        format!("Metal {rest}")
    }
}

/// "8 GB" → 字节
#[cfg(target_os = "macos")]
fn parse_gib(s: &str) -> Option<u64> {
    let (num, unit) = s.trim().split_once(' ')?;
    let n: u64 = num.parse().ok()?;
    match unit.to_ascii_uppercase().as_str() {
        "GB" => Some(n << 30),
        "MB" => Some(n << 20),
        _ => None,
    }
}

// ------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
fn extras(keys: &[GpuKey]) -> Vec<Extra> {
    keys.iter().map(|k| linux_extra(&k.pci)).collect()
}

#[cfg(target_os = "linux")]
fn linux_extra(pci: &str) -> Extra {
    let dir = std::path::Path::new("/sys/bus/pci/devices").join(pci);
    let read = |name: &str| {
        std::fs::read_to_string(dir.join(name))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let module = std::fs::read_link(dir.join("driver"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let version = module.as_deref().and_then(|m| match m {
        "nvidia" => nvidia_driver_version(),
        _ => std::fs::read_to_string(format!("/sys/module/{m}/version"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    });
    let driver = match (module, version) {
        (Some(m), Some(v)) => Some(format!("{m} {v}")),
        (Some(m), None) => Some(m),
        _ => None,
    };

    let pcie_link = match (read("current_link_speed"), read("current_link_width")) {
        (Some(s), Some(w)) => Some(format_pcie_link(&s, &w)),
        _ => None,
    };
    let vram_total = read("mem_info_vram_total").and_then(|s| s.parse::<u64>().ok());
    Extra {
        cores: None,
        driver,
        pcie_link,
        vram_total,
    }
}

/// `/proc/driver/nvidia/version`：`NVRM version: NVIDIA UNIX x86_64 Kernel Module  550.54.14 ...`
#[cfg(target_os = "linux")]
fn nvidia_driver_version() -> Option<String> {
    let s = std::fs::read_to_string("/proc/driver/nvidia/version").ok()?;
    s.lines()
        .next()?
        .split_whitespace()
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))
        .map(str::to_string)
}

/// "8.0 GT/s PCIe" + "16" → "PCIe 3.0 ×16"
#[cfg(target_os = "linux")]
fn format_pcie_link(speed: &str, width: &str) -> String {
    let gts: f32 = speed
        .split_whitespace()
        .find_map(|t| t.parse::<f32>().ok())
        .unwrap_or(0.0);
    let generation = match gts as u32 {
        2 | 3 => "1.0",
        5 => "2.0",
        8 => "3.0",
        16 => "4.0",
        32 => "5.0",
        64 => "6.0",
        _ => return format!("{} ×{}", speed.trim(), width.trim()),
    };
    format!("PCIe {generation} ×{}", width.trim())
}

// ----------------------------------------------------------- Windows

#[cfg(target_os = "windows")]
fn extras(keys: &[GpuKey]) -> Vec<Extra> {
    use serde::Deserialize;

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct VideoController {
        name: Option<String>,
        adapter_ram: Option<u32>,
        driver_version: Option<String>,
    }

    let rows: Vec<VideoController> = crate::platform::windows::wmi_cimv2()
        .and_then(|c| {
            c.raw_query("SELECT Name, AdapterRAM, DriverVersion FROM Win32_VideoController")
                .ok()
        })
        .unwrap_or_default();

    keys.iter()
        .map(|k| {
            let n = k.name.to_ascii_lowercase();
            let row = rows.iter().find(|r| {
                r.name
                    .as_deref()
                    .map(|rn| rn.trim().to_ascii_lowercase())
                    .is_some_and(|rn| rn == n || rn.contains(&n) || n.contains(&rn))
            });
            Extra {
                cores: None,
                driver: row.and_then(|r| r.driver_version.clone()),
                pcie_link: None,
                // AdapterRAM 是 32 位字段，超过 4 GiB 的显卡会被截断，只在未截断时使用。
                vram_total: row
                    .and_then(|r| r.adapter_ram)
                    .filter(|v| *v > 0 && *v < u32::MAX)
                    .map(u64::from),
            }
        })
        .collect()
}

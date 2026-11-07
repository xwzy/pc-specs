use crate::model::DisplayInfo;
use tauri::{AppHandle, Manager};

pub fn collect(app: &AppHandle) -> Vec<DisplayInfo> {
    let win = match app.get_webview_window("main") {
        Some(w) => w,
        None => return Vec::new(),
    };
    let monitors = win.available_monitors().unwrap_or_default();
    let primary = win.primary_monitor().ok().flatten();
    let primary_key: Option<(String, i32, i32, u32, u32)> = primary.as_ref().map(|p| {
        let pos = p.position();
        let size = p.size();
        let name = p.name().cloned().unwrap_or_default();
        (name, pos.x, pos.y, size.width, size.height)
    });
    let mut primary_assigned = false;

    let extras = collect_extras();

    monitors
        .into_iter()
        .filter_map(|m| {
            let size = m.size();
            if size.width == 0 || size.height == 0 {
                return None;
            }
            let scale = m.scale_factor() as f32;
            let pos = m.position();
            let name = m.name().cloned().unwrap_or_else(|| "Display".to_string());
            let key = (name.clone(), pos.x, pos.y, size.width, size.height);
            let is_primary = if !primary_assigned {
                let matches_primary = primary_key
                    .as_ref()
                    .map(|p| *p == key)
                    .unwrap_or(false);
                if matches_primary {
                    primary_assigned = true;
                }
                matches_primary
            } else {
                false
            };

            // 通过分辨率/名字模糊匹配寻找平台层补充信息（refresh_hz 等）
            let extra = extras.iter().find(|e| {
                (e.width.is_none() || e.width == Some(size.width))
                    && (e.height.is_none() || e.height == Some(size.height))
                    && (e.name.is_empty() || e.name.eq_ignore_ascii_case(&name) || name.contains(&e.name))
            });

            Some(DisplayInfo {
                name,
                width_px: size.width,
                height_px: size.height,
                refresh_hz: extra.and_then(|e| e.refresh_hz),
                scale_factor: Some(scale),
                is_primary,
                physical_width_mm: extra.and_then(|e| e.physical_width_mm),
                physical_height_mm: extra.and_then(|e| e.physical_height_mm),
                color_depth: extra.and_then(|e| e.color_depth),
            })
        })
        .collect()
}

#[derive(Default)]
struct DisplayExtra {
    name: String,
    width: Option<u32>,
    height: Option<u32>,
    refresh_hz: Option<u32>,
    physical_width_mm: Option<u32>,
    physical_height_mm: Option<u32>,
    color_depth: Option<u8>,
}

#[cfg(target_os = "macos")]
fn collect_extras() -> Vec<DisplayExtra> {
    use std::process::Command;
    let out = match Command::new("system_profiler")
        .args(["-json", "SPDisplaysDataType"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let v: serde_json::Value = match serde_json::from_slice(&out.stdout) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut out_vec = Vec::new();
    if let Some(arr) = v.get("SPDisplaysDataType").and_then(|x| x.as_array()) {
        for gpu in arr {
            // 每个 GPU 节点包含 spdisplays_ndrvs 数组（每个连接的显示器）
            if let Some(displays) = gpu.get("spdisplays_ndrvs").and_then(|x| x.as_array()) {
                for d in displays {
                    let mut e = DisplayExtra::default();
                    if let Some(n) = d.get("_name").and_then(|s| s.as_str()) {
                        e.name = n.to_string();
                    }
                    // "spdisplays_resolution": "3024 x 1964 @ 60.00Hz"
                    if let Some(res) = d.get("_spdisplays_resolution").and_then(|s| s.as_str()) {
                        parse_macos_resolution(res, &mut e);
                    }
                    if let Some(d_str) = d
                        .get("_spdisplays_pixeldepth")
                        .and_then(|s| s.as_str())
                    {
                        // "CGSThirtyBitColor" → 30, "CGSThirtytwoBitColor" → 32
                        if let Some(n) = parse_macos_pixel_depth(d_str) {
                            e.color_depth = Some(n);
                        }
                    }
                    out_vec.push(e);
                }
            }
        }
    }
    out_vec
}

#[cfg(target_os = "macos")]
fn parse_macos_resolution(s: &str, e: &mut DisplayExtra) {
    // 形如 "3024 x 1964 @ 60.00Hz" 或 "1920 x 1080"
    let parts: Vec<&str> = s.splitn(2, '@').collect();
    let dims = parts[0].trim();
    let mut nums = dims.split('x');
    if let (Some(w), Some(h)) = (
        nums.next().and_then(|x| x.trim().parse::<u32>().ok()),
        nums.next().and_then(|x| x.trim().parse::<u32>().ok()),
    ) {
        e.width = Some(w);
        e.height = Some(h);
    }
    if parts.len() > 1 {
        let hz_part = parts[1].trim();
        let hz: f32 = hz_part
            .trim_end_matches("Hz")
            .trim_end_matches("hz")
            .trim()
            .parse()
            .unwrap_or(0.0);
        if hz > 0.0 {
            e.refresh_hz = Some(hz.round() as u32);
        }
    }
}

#[cfg(target_os = "macos")]
fn parse_macos_pixel_depth(s: &str) -> Option<u8> {
    let lower = s.to_lowercase();
    if lower.contains("thirty") {
        Some(30)
    } else if lower.contains("twentyeight") {
        Some(28)
    } else if lower.contains("twentyfour") || lower.contains("thirtytwo") {
        Some(32)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn collect_extras() -> Vec<DisplayExtra> {
    use serde::Deserialize;
    use wmi::{COMLibrary, WMIConnection};

    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Vc {
        name: Option<String>,
        current_refresh_rate: Option<u32>,
        current_horizontal_resolution: Option<u32>,
        current_vertical_resolution: Option<u32>,
        current_bits_per_pixel: Option<u32>,
    }

    let Ok(com) = COMLibrary::new() else { return Vec::new() };
    let Ok(conn) = WMIConnection::new(com) else { return Vec::new() };
    let rows: Vec<Vc> = conn
        .raw_query(
            "SELECT Name, CurrentRefreshRate, CurrentHorizontalResolution, CurrentVerticalResolution, CurrentBitsPerPixel FROM Win32_VideoController",
        )
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|v| {
            let name = v.name.unwrap_or_default();
            // 过滤掉无分辨率的虚拟设备
            if v.current_horizontal_resolution.unwrap_or(0) == 0
                || v.current_vertical_resolution.unwrap_or(0) == 0
            {
                return None;
            }
            Some(DisplayExtra {
                name,
                width: v.current_horizontal_resolution,
                height: v.current_vertical_resolution,
                refresh_hz: v.current_refresh_rate.filter(|n| *n > 0),
                physical_width_mm: None,
                physical_height_mm: None,
                color_depth: v.current_bits_per_pixel.and_then(|n| u8::try_from(n).ok()),
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn collect_extras() -> Vec<DisplayExtra> {
    // Wayland 下 xrandr 通常缺失或只显示 XWayland。先尝试 xrandr，再尝试 wlr-randr。
    if let Some(v) = xrandr_extras() {
        if !v.is_empty() {
            return v;
        }
    }
    wlr_randr_extras().unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn xrandr_extras() -> Option<Vec<DisplayExtra>> {
    use std::process::Command;
    let out = Command::new("xrandr").arg("--query").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let mut result = Vec::new();
    let mut current: Option<DisplayExtra> = None;
    for line in s.lines() {
        // 输出名行: "HDMI-A-1 connected primary 3840x2160+0+0 (...) 597mm x 336mm"
        if !line.starts_with(' ') && !line.starts_with('\t') && line.contains("connected") {
            if let Some(prev) = current.take() {
                result.push(prev);
            }
            let mut e = DisplayExtra::default();
            let mut tokens = line.split_whitespace();
            if let Some(name) = tokens.next() {
                e.name = name.to_string();
            }
            // 末尾 "597mm x 336mm" 段
            if let Some(mm_idx) = line.rfind("mm") {
                let prefix = &line[..mm_idx];
                if let Some(start) = prefix.rfind(|c: char| c.is_whitespace()) {
                    let segment = &line[start..mm_idx];
                    let parts: Vec<&str> = segment.split('x').map(|s| s.trim()).collect();
                    if parts.len() == 2 {
                        if let (Ok(w), Some(h_str)) = (
                            parts[0].trim_end_matches("mm").trim().parse::<u32>(),
                            parts[1].split_whitespace().next(),
                        ) {
                            if let Ok(h) = h_str.trim_end_matches("mm").trim().parse::<u32>() {
                                e.physical_width_mm = Some(w);
                                e.physical_height_mm = Some(h);
                            }
                        }
                    }
                }
            }
            current = Some(e);
            continue;
        }
        // 模式行: "   3840x2160     60.00*+ 30.00    24.00"
        // 带 * 的为当前模式，带 + 的为首选；取 * 后的刷新率。
        if let Some(e) = current.as_mut() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let mut tokens = trimmed.split_whitespace();
            let res = match tokens.next() {
                Some(r) => r,
                None => continue,
            };
            // 仅处理形如 "WxH"
            let dims: Vec<&str> = res.split('x').collect();
            if dims.len() != 2 {
                continue;
            }
            let (Ok(w), Ok(h)) = (dims[0].parse::<u32>(), dims[1].parse::<u32>()) else { continue };
            for tok in tokens {
                if tok.contains('*') {
                    let hz: f32 = tok
                        .trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.')
                        .parse()
                        .unwrap_or(0.0);
                    if hz > 0.0 {
                        e.width = Some(w);
                        e.height = Some(h);
                        e.refresh_hz = Some(hz.round() as u32);
                    }
                    break;
                }
            }
        }
    }
    if let Some(prev) = current.take() {
        result.push(prev);
    }
    Some(result)
}

#[cfg(target_os = "linux")]
fn wlr_randr_extras() -> Option<Vec<DisplayExtra>> {
    use std::process::Command;
    // wlr-randr 输出例：
    //   HDMI-A-1 "Samsung..."
    //     ...
    //     3840x2160 px, 59.997 Hz (current)
    let out = Command::new("wlr-randr").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).to_string();
    let mut result = Vec::new();
    let mut current: Option<DisplayExtra> = None;
    for line in s.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') && !line.is_empty() {
            if let Some(prev) = current.take() {
                result.push(prev);
            }
            let mut e = DisplayExtra::default();
            if let Some(name) = line.split_whitespace().next() {
                e.name = name.to_string();
            }
            current = Some(e);
            continue;
        }
        let trimmed = line.trim();
        if trimmed.contains("(current)") && trimmed.contains("Hz") {
            if let Some(e) = current.as_mut() {
                let mut parts = trimmed.split_whitespace();
                if let Some(res) = parts.next() {
                    let dims: Vec<&str> = res.split('x').collect();
                    if dims.len() == 2 {
                        e.width = dims[0].parse().ok();
                        e.height = dims[1].trim_end_matches("px,").parse().ok();
                    }
                }
                for tok in trimmed.split_whitespace() {
                    if let Ok(hz) = tok.parse::<f32>() {
                        if hz > 0.0 && hz < 1000.0 {
                            e.refresh_hz = Some(hz.round() as u32);
                        }
                    }
                }
            }
        }
    }
    if let Some(prev) = current.take() {
        result.push(prev);
    }
    Some(result)
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn collect_extras() -> Vec<DisplayExtra> {
    Vec::new()
}

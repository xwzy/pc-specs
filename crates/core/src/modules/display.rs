use crate::model::DisplayInfo;

/// 当前已连接显示器的物理参数。不依赖任何 UI 框架，直接问操作系统。
pub fn collect() -> Vec<DisplayInfo> {
    let mut v = collect_platform();
    v.retain(|d| d.width_px > 0 && d.height_px > 0);
    // 主显示器排最前，其余保持系统给出的顺序。
    v.sort_by_key(|d| !d.is_primary);
    v
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn parse_wh(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

// ------------------------------------------------------------------ macOS

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct CgSize {
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDisplayScreenSize(display: u32) -> CgSize;
}

#[cfg(target_os = "macos")]
fn collect_platform() -> Vec<DisplayInfo> {
    use crate::platform::macos::system_profiler;

    let Some(root) = system_profiler("SPDisplaysDataType") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for gpu in root
        .get("SPDisplaysDataType")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        for d in gpu
            .get("spdisplays_ndrvs")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let text = |k: &str| d.get(k).and_then(|v| v.as_str());
            // 原生像素 "3024 x 1964"；界面分辨率 "1512 x 982 @ 120.00Hz"（HiDPI 时是缩放后的"看起来"尺寸）。
            let Some((w, h)) = text("_spdisplays_pixels").and_then(parse_wh) else {
                continue;
            };
            let (looks, hz) = match text("_spdisplays_resolution").or(text("spdisplays_resolution"))
            {
                Some(r) => match r.split_once('@') {
                    Some((dims, rate)) => (
                        parse_wh(dims),
                        rate.trim()
                            .trim_end_matches(|c: char| c.is_ascii_alphabetic())
                            .trim()
                            .parse::<f32>()
                            .ok()
                            .filter(|v| *v > 0.0),
                    ),
                    None => (parse_wh(r), None),
                },
                None => (None, None),
            };
            let scale = looks.map(|(lw, _)| w as f32 / lw.max(1) as f32);

            // `_spdisplays_displayID` 与 CoreGraphics 的 CGDirectDisplayID 一致，可取物理尺寸。
            let (pw, ph) = text("_spdisplays_displayID")
                .and_then(|s| s.trim().parse::<u32>().ok())
                .map(|id| {
                    // SAFETY: 纯查询调用，无指针参数；未知 ID 返回 0。
                    let s = unsafe { CGDisplayScreenSize(id) };
                    (s.width.round() as u32, s.height.round() as u32)
                })
                .filter(|(a, b)| *a > 0 && *b > 0)
                .unzip();

            out.push(DisplayInfo {
                name: text("_name").unwrap_or("Display").to_string(),
                width_px: w,
                height_px: h,
                refresh_hz: hz.map(|v| v.round() as u32),
                scale_factor: scale,
                is_primary: text("spdisplays_main") == Some("spdisplays_yes"),
                physical_width_mm: pw,
                physical_height_mm: ph,
                color_depth: None,
            });
        }
    }
    out
}

// ---------------------------------------------------------------- Windows

#[cfg(target_os = "windows")]
fn collect_platform() -> Vec<DisplayInfo> {
    use serde::Deserialize;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        DEVMODEW, DISPLAY_DEVICEW, ENUM_CURRENT_SETTINGS, EnumDisplayDevicesW, EnumDisplayMonitors,
        EnumDisplaySettingsW, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::core::{BOOL, PCWSTR};

    fn wide(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    unsafe extern "system" fn collect_monitor(
        monitor: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` 是下面传入的 `&mut Vec<HMONITOR>`，回调期间始终有效。
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        BOOL(1)
    }

    let mut monitors: Vec<HMONITOR> = Vec::new();
    // SAFETY: 回调只向传入的 Vec 追加元素。
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM(&mut monitors as *mut Vec<HMONITOR> as isize),
        );
    }

    // 物理尺寸与型号名：WMI 的 EDID 视图，按 PnP 编号（如 DEL41BA）与 GDI 设备配对。
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct Basic {
        instance_name: Option<String>,
        max_horizontal_image_size: Option<u32>,
        max_vertical_image_size: Option<u32>,
    }
    #[derive(Deserialize, Debug)]
    #[serde(rename_all = "PascalCase")]
    struct MonitorId {
        instance_name: Option<String>,
        user_friendly_name: Option<Vec<u16>>,
    }
    let (basic, ids): (Vec<Basic>, Vec<MonitorId>) = match crate::platform::windows::wmi_root_wmi() {
        Some(c) => (
            c.raw_query(
                "SELECT InstanceName, MaxHorizontalImageSize, MaxVerticalImageSize FROM WmiMonitorBasicDisplayParams",
            )
            .unwrap_or_default(),
            c.raw_query("SELECT InstanceName, UserFriendlyName FROM WmiMonitorID")
                .unwrap_or_default(),
        ),
        None => (Vec::new(), Vec::new()),
    };
    let pnp_of = |instance: &str| {
        instance
            .split('\\')
            .nth(1)
            .unwrap_or("")
            .to_ascii_uppercase()
    };

    let mut out = Vec::new();
    for hmon in monitors {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` 的首字段就是 MONITORINFO，cbSize 已按扩展结构设置。
        let ok =
            unsafe { GetMonitorInfoW(hmon, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) };
        if !ok.as_bool() {
            continue;
        }
        let device = PCWSTR(info.szDevice.as_ptr());

        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: device 指向以 NUL 结尾的缓冲区；mode.dmSize 已设置。
        if !unsafe { EnumDisplaySettingsW(device, ENUM_CURRENT_SETTINGS, &mut mode) }.as_bool() {
            continue;
        }

        let (mut dx, mut dy) = (0u32, 0u32);
        // SAFETY: 输出指针指向本地变量。
        let scale = unsafe { GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy) }
            .ok()
            .map(|_| dx as f32 / 96.0);

        let mut dev = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        // SAFETY: 取该显示输出上的第一个监视器设备。
        let has_dev = unsafe { EnumDisplayDevicesW(device, 0, &mut dev, 0) }.as_bool();
        let pnp = if has_dev {
            pnp_of(&wide(&dev.DeviceID))
        } else {
            String::new()
        };
        let gdi_name = if has_dev {
            wide(&dev.DeviceString)
        } else {
            String::new()
        };

        let friendly = ids
            .iter()
            .find(|m| m.instance_name.as_deref().map(pnp_of).as_deref() == Some(pnp.as_str()))
            .and_then(|m| m.user_friendly_name.as_ref())
            .map(|u| {
                let end = u.iter().position(|&c| c == 0).unwrap_or(u.len());
                String::from_utf16_lossy(&u[..end]).trim().to_string()
            })
            .filter(|s| !s.is_empty());
        let size_mm = basic
            .iter()
            .find(|b| b.instance_name.as_deref().map(pnp_of).as_deref() == Some(pnp.as_str()))
            .and_then(|b| {
                Some((
                    b.max_horizontal_image_size? * 10,
                    b.max_vertical_image_size? * 10,
                ))
            })
            .filter(|(w, h)| *w > 0 && *h > 0);

        out.push(DisplayInfo {
            name: friendly
                .or(Some(gdi_name).filter(|s| !s.is_empty()))
                .unwrap_or_else(|| wide(&info.szDevice)),
            width_px: mode.dmPelsWidth,
            height_px: mode.dmPelsHeight,
            refresh_hz: Some(mode.dmDisplayFrequency).filter(|v| *v > 1),
            scale_factor: scale,
            is_primary: info.monitorInfo.dwFlags & 1 != 0, // MONITORINFOF_PRIMARY
            physical_width_mm: size_mm.map(|s| s.0),
            physical_height_mm: size_mm.map(|s| s.1),
            color_depth: u8::try_from(mode.dmBitsPerPel).ok().filter(|v| *v > 0),
        });
    }
    out
}

// ------------------------------------------------------------------ Linux

#[cfg(target_os = "linux")]
#[derive(Default, Clone, Debug)]
struct Output {
    name: String,
    primary: bool,
    width: Option<u32>,
    height: Option<u32>,
    hz: Option<f32>,
    mm: Option<(u32, u32)>,
    scale: Option<f32>,
    /// 来自 EDID 的型号名
    model: Option<String>,
}

#[cfg(target_os = "linux")]
fn collect_platform() -> Vec<DisplayInfo> {
    use crate::util::run_stdout_timeout;
    use std::time::Duration;

    let drm = drm_outputs();

    // X11（含 XWayland）：xrandr 给出当前模式、刷新率与主显示器标记。
    let mut live = run_stdout_timeout("xrandr", &["--query"], Duration::from_secs(3))
        .map(|t| parse_xrandr(&t))
        .unwrap_or_default();
    // wlroots 系合成器：wlr-randr 还有精确缩放系数。
    if live.is_empty() {
        live = run_stdout_timeout("wlr-randr", &[], Duration::from_secs(3))
            .map(|t| parse_wlr_randr(&t))
            .unwrap_or_default();
    }

    // 有实时来源就以它为准，再从 DRM 补型号名；纯 DRM 兜底（GNOME/KDE Wayland 无 xrandr 输出时）。
    let mut outputs: Vec<Output> = Vec::new();
    if live.is_empty() {
        outputs = drm;
    } else {
        let mut pool = drm;
        for mut l in live {
            // 先按连接器名配对；NVIDIA 专有驱动的命名与内核不同，退而用物理尺寸配对。
            let key = norm_connector(&l.name);
            let pos = pool
                .iter()
                .position(|o| norm_connector(&o.name) == key)
                .or_else(|| {
                    l.mm.and_then(|mm| pool.iter().position(|o| o.mm == Some(mm)))
                });
            if let Some(p) = pos {
                let d = pool.remove(p);
                l.model = d.model;
                l.mm = l.mm.or(d.mm);
            }
            outputs.push(l);
        }
    }
    if !outputs.iter().any(|o| o.primary)
        && let Some(o) = outputs.first_mut()
    {
        o.primary = true;
    }

    outputs
        .into_iter()
        .map(|o| DisplayInfo {
            name: o.model.clone().unwrap_or_else(|| o.name.clone()),
            width_px: o.width.unwrap_or(0),
            height_px: o.height.unwrap_or(0),
            refresh_hz: o.hz.map(|v| v.round() as u32).filter(|v| *v > 0),
            scale_factor: o.scale,
            is_primary: o.primary,
            physical_width_mm: o.mm.map(|m| m.0),
            physical_height_mm: o.mm.map(|m| m.1),
            color_depth: None,
        })
        .collect()
}

/// "card0-HDMI-A-1" / "HDMI-A-1" / "HDMI-1" 归一成同一个键。
#[cfg(target_os = "linux")]
fn norm_connector(name: &str) -> String {
    let n = name
        .split_once('-')
        .filter(|(p, _)| p.starts_with("card"))
        .map_or(name, |(_, r)| r);
    n.replace("HDMI-A-", "HDMI-").to_ascii_lowercase()
}

/// 内核 DRM 的已连接输出：EDID 里有型号名与物理尺寸，Wayland 下 xrandr 缺失时也能用。
#[cfg(target_os = "linux")]
fn drm_outputs() -> Vec<Output> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut names: Vec<_> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("card") && n.contains('-'))
        .collect();
    names.sort();

    let mut out = Vec::new();
    for n in names {
        let dir = std::path::Path::new("/sys/class/drm").join(&n);
        let status = std::fs::read_to_string(dir.join("status")).unwrap_or_default();
        if status.trim() != "connected" {
            continue;
        }
        let mut o = Output {
            name: n
                .split_once('-')
                .map(|(_, r)| r.to_string())
                .unwrap_or(n.clone()),
            ..Default::default()
        };
        // modes 的第一行是首选模式；没有实时来源时作为兜底。
        if let Some((w, h)) = std::fs::read_to_string(dir.join("modes"))
            .ok()
            .and_then(|s| s.lines().next().and_then(parse_wh))
        {
            o.width = Some(w);
            o.height = Some(h);
        }
        if let Ok(edid) = std::fs::read(dir.join("edid"))
            && let Some((model, mm)) = parse_edid(&edid)
        {
            o.model = model;
            o.mm = mm;
        }
        out.push(o);
    }
    out
}

/// EDID 里读出来的（型号名, 物理尺寸 mm）。
#[cfg(target_os = "linux")]
type Edid = (Option<String>, Option<(u32, u32)>);

/// 解析 EDID 基础块。
#[cfg(target_os = "linux")]
fn parse_edid(e: &[u8]) -> Option<Edid> {
    if e.len() < 128 || e[..8] != [0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0] {
        return None;
    }
    // 四个 18 字节描述符；标签 0xFC 是显示器名称。
    let mut model = None;
    for i in 0..4 {
        let d = &e[54 + 18 * i..54 + 18 * (i + 1)];
        if d[0] == 0 && d[1] == 0 && d[3] == 0xFC {
            let name: String = d[5..18]
                .iter()
                .take_while(|&&b| b != 0x0A)
                .map(|&b| b as char)
                .collect();
            let name = name.trim().to_string();
            if !name.is_empty() {
                model = Some(name);
            }
        }
    }
    // 首个详细时序描述符里的图像尺寸精确到 mm；否则退回头部的厘米值。
    let dtd = &e[54..72];
    let (mut w, mut h) = (0u32, 0u32);
    if dtd[0] != 0 || dtd[1] != 0 {
        w = dtd[12] as u32 | ((dtd[14] as u32 & 0xF0) << 4);
        h = dtd[13] as u32 | ((dtd[14] as u32 & 0x0F) << 8);
    }
    if w == 0 || h == 0 {
        w = e[21] as u32 * 10;
        h = e[22] as u32 * 10;
    }
    Some((model, (w > 0 && h > 0).then_some((w, h))))
}

/// ```text
/// eDP-1 connected primary 1920x1080+0+0 (normal left ...) 309mm x 173mm
///    1920x1080     60.01*+  59.97
/// ```
#[cfg(target_os = "linux")]
fn parse_xrandr(text: &str) -> Vec<Output> {
    let mut out: Vec<Output> = Vec::new();
    let mut current: Option<usize> = None;
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            current = None;
            let toks: Vec<&str> = line.split_whitespace().collect();
            if toks.len() < 2 || toks[1] != "connected" {
                continue;
            }
            let mut o = Output {
                name: toks[0].to_string(),
                ..Default::default()
            };
            for (i, t) in toks.iter().enumerate().skip(2) {
                if *t == "primary" {
                    o.primary = true;
                } else if let Some((w, h)) = t.split('+').next().and_then(parse_wh) {
                    if t.contains('+') {
                        o.width = Some(w);
                        o.height = Some(h);
                    }
                } else if let (Some(a), Some("x"), Some(b)) = (
                    t.strip_suffix("mm"),
                    toks.get(i + 1).copied(),
                    toks.get(i + 2).and_then(|b| b.strip_suffix("mm")),
                ) && let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>())
                    && a > 0
                    && b > 0
                {
                    o.mm = Some((a, b));
                }
            }
            out.push(o);
            current = Some(out.len() - 1);
            continue;
        }
        let Some(idx) = current else { continue };
        let mut toks = line.split_whitespace();
        let Some(mode) = toks.next() else { continue };
        if parse_wh(mode).is_none() {
            continue;
        }
        if let Some(rate) = toks.find(|t| t.contains('*')) {
            let hz = rate.trim_end_matches(['*', '+']).parse::<f32>().ok();
            out[idx].hz = hz.filter(|v| *v > 0.0);
            // 无几何信息（未启用输出）时才用模式分辨率。
            if out[idx].width.is_none()
                && let Some((w, h)) = parse_wh(mode)
            {
                out[idx].width = Some(w);
                out[idx].height = Some(h);
            }
        }
    }
    out
}

/// ```text
/// HDMI-A-1 "Samsung ..."
///   Physical size: 600x340 mm
///   Modes:
///     3840x2160 px, 59.997002 Hz (preferred, current)
///   Scale: 1.500000
/// ```
#[cfg(target_os = "linux")]
fn parse_wlr_randr(text: &str) -> Vec<Output> {
    let mut out: Vec<Output> = Vec::new();
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            if let Some(name) = line.split_whitespace().next() {
                out.push(Output {
                    name: name.to_string(),
                    ..Default::default()
                });
            }
            continue;
        }
        let Some(o) = out.last_mut() else { continue };
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Physical size:") {
            if let Some((w, h)) = rest.trim().trim_end_matches("mm").trim().split_once('x')
                && let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>())
            {
                o.mm = Some((w, h)).filter(|m| m.0 > 0 && m.1 > 0);
            }
        } else if let Some(rest) = t.strip_prefix("Scale:") {
            o.scale = rest.trim().parse::<f32>().ok();
        } else if t.contains("current") && t.contains("Hz") {
            let mut toks = t.split_whitespace();
            if let Some((w, h)) = toks.next().and_then(parse_wh) {
                o.width = Some(w);
                o.height = Some(h);
            }
            let all: Vec<&str> = t.split_whitespace().collect();
            if let Some(i) = all.iter().position(|x| *x == "Hz") {
                o.hz = i.checked_sub(1).and_then(|j| all[j].parse::<f32>().ok());
            }
        }
    }
    out
}

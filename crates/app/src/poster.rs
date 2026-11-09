//! PNG 长图：把快照排成一张固定 1200px 宽的深色海报，便于发到群 / 论坛。
//!
//! 做法：拼一段 SVG，再用 resvg 栅格化（2 倍像素密度，Retina 下也清晰）。
//! 海报只包含硬件规格，不含 MAC / 序列号 / 公网 IP；主机名只出现在页脚之外的 OS 一行里。

use crate::fmt;
use crate::settings::Settings;
use pc_specs_core::model::SystemSnapshot;
use resvg::{tiny_skia, usvg};
use rust_i18n::t;
use std::fmt::Write as _;

const WIDTH: f32 = 1200.0;
const MARGIN: f32 = 56.0;
const LABEL_X: f32 = MARGIN;
const CONTENT_X: f32 = 230.0;
const SCALE: f32 = 2.0;

const BG0: &str = "#0a0d12";
const BG1: &str = "#11151c";
const BORDER: &str = "#222a36";
const TEXT: &str = "#e7ecf3";
const TEXT2: &str = "#9aa6b6";
const TEXT3: &str = "#5b6678";
const ACCENT: &str = "#22d3ee";
const ACCENT2: &str = "#a78bfa";

/// 优先的无衬线字体（按平台常见程度排列）；CJK 文字由 usvg 的逐字回退兜底。
const SANS: &str = "Inter, 'Segoe UI', 'Helvetica Neue', 'PingFang SC', 'Microsoft YaHei', \
'Noto Sans CJK SC', 'Noto Sans', 'DejaVu Sans', Arial, sans-serif";

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// 单行文字过长时截断，避免溢出海报右侧。
fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let head: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

struct Row {
    label: String,
    big: String,
    subs: Vec<String>,
}

fn rows(snap: &SystemSnapshot, settings: &Settings) -> Vec<Row> {
    let mut rows = Vec::new();
    let bytes = |n: u64| settings.bytes(n);

    let cpu = &snap.cpu;
    if !cpu.brand.is_empty() {
        let mut sub = format!(
            "{}C / {}T · {}",
            cpu.physical_cores, cpu.logical_cores, cpu.arch
        );
        if cpu.max_frequency_hz > 0 {
            sub += &format!(" · {}", fmt::freq(cpu.max_frequency_hz));
        }
        if let Some(l3) = cpu.cache_l3_bytes {
            sub += &format!(" · L3 {}", bytes(l3));
        }
        rows.push(Row {
            label: t!("dash_summary_cpu").into_owned(),
            big: cpu.brand.clone(),
            subs: vec![sub],
        });
    }

    let mem = &snap.memory;
    if mem.total_bytes > 0 {
        let detail = mem
            .modules
            .iter()
            .find(|m| m.kind.is_some() || m.speed_mt_s.is_some())
            .map(|m| {
                [m.kind.clone(), m.speed_mt_s.map(|v| format!("{v} MT/s"))]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ")
            })
            .unwrap_or_default();
        let used = format!(
            "{} {} ({})",
            t!("spec_used"),
            bytes(mem.used_bytes),
            fmt::percent(fmt::ratio(mem.used_bytes, mem.total_bytes))
        );
        let sub = if detail.is_empty() {
            used
        } else {
            format!("{detail} · {used}")
        };
        rows.push(Row {
            label: t!("dash_summary_memory").into_owned(),
            big: bytes(mem.total_bytes),
            subs: vec![sub],
        });
    }

    for gpu in snap.gpus.iter().take(3) {
        let mut sub = vec![
            if gpu.is_discrete {
                t!("gpu_discrete").into_owned()
            } else {
                t!("gpu_integrated").into_owned()
            },
            gpu.backend.clone(),
        ];
        if let Some(v) = gpu.vram_total_bytes {
            sub.push(format!("VRAM {}", bytes(v)));
        }
        if let Some(d) = &gpu.driver {
            sub.push(format!("{} {d}", t!("spec_driver")));
        }
        sub.retain(|s| !s.is_empty());
        rows.push(Row {
            label: t!("dash_summary_gpu").into_owned(),
            big: gpu.name.clone(),
            subs: vec![sub.join(" · ")],
        });
    }

    for disk in snap.storages.iter().take(4) {
        let pct = fmt::percent(fmt::ratio(disk.used_bytes, disk.total_bytes));
        let mut sub = format!(
            "{} · {} · {} {} ({pct})",
            disk.kind,
            bytes(disk.total_bytes),
            t!("spec_used"),
            bytes(disk.used_bytes)
        );
        if let Some(smart) = &disk.smart_health {
            sub += &format!(" · SMART {smart}");
        }
        rows.push(Row {
            label: t!("dash_summary_storage").into_owned(),
            big: disk.name.clone(),
            subs: vec![sub],
        });
    }

    if let Some(mb) = &snap.motherboard {
        let big = [mb.vendor.clone(), mb.model.clone()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        if !big.is_empty() {
            let bios = [
                mb.bios_vendor.clone(),
                mb.bios_version.clone(),
                mb.bios_date.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
            rows.push(Row {
                label: t!("dash_summary_motherboard").into_owned(),
                big,
                subs: if bios.is_empty() {
                    vec![]
                } else {
                    vec![format!("BIOS {bios}")]
                },
            });
        }
    }

    if let Some(d) = snap
        .displays
        .iter()
        .find(|d| d.is_primary)
        .or(snap.displays.first())
    {
        let hz = d
            .refresh_hz
            .map(|v| format!(" @ {v} Hz"))
            .unwrap_or_default();
        let mut sub = Vec::new();
        if let Some(f) = d.scale_factor {
            sub.push(format!("{} {f:.1}×", t!("disp_scale")));
        }
        if snap.displays.len() > 1 {
            sub.push(t!("count_found", n = snap.displays.len()).into_owned());
        }
        rows.push(Row {
            label: t!("dash_summary_display").into_owned(),
            big: format!("{} × {}{}", d.width_px, d.height_px, hz),
            subs: if sub.is_empty() {
                vec![]
            } else {
                vec![sub.join(" · ")]
            },
        });
    }

    let os = &snap.os;
    if !os.name.is_empty() {
        let sub = [
            os.kernel.clone(),
            os.arch.clone(),
            snap.host.hostname.clone(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        rows.push(Row {
            label: t!("dash_summary_os").into_owned(),
            big: fmt::os_title(os),
            subs: vec![sub],
        });
    }

    if let Some(n) = snap
        .network
        .interfaces
        .iter()
        .find(|n| n.is_up && !n.is_loopback && !n.ipv4.is_empty())
    {
        let mut sub = Vec::new();
        if let Some(gw) = &snap.network.default_gateway {
            sub.push(format!("{} {gw}", t!("net_gw_short")));
        }
        if let Some(speed) = n.link_speed_mbps {
            sub.push(format!("{speed} Mbps"));
        }
        rows.push(Row {
            label: t!("dash_summary_network").into_owned(),
            big: format!("{} · {}", n.name, n.ipv4.join(", ")),
            subs: if sub.is_empty() {
                vec![]
            } else {
                vec![sub.join(" · ")]
            },
        });
    }

    rows
}

/// 生成海报 SVG（尚未栅格化）。
pub fn build_svg(snap: &SystemSnapshot, settings: &Settings) -> String {
    let rows = rows(snap, settings);

    const HEADER_H: f32 = 150.0;
    const FOOTER_H: f32 = 84.0;
    let row_h = |r: &Row| 26.0 + 34.0 + 24.0 * r.subs.len() as f32 + 18.0;
    let height = HEADER_H + rows.iter().map(row_h).sum::<f32>() + FOOTER_H;

    let mut svg = String::new();
    let _ = write!(
        svg,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" font-family="{SANS}">
<defs>
<linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{BG0}"/><stop offset="0.6" stop-color="{BG1}"/><stop offset="1" stop-color="{BG0}"/></linearGradient>
<linearGradient id="bar" x1="0" y1="0" x2="1" y2="0"><stop offset="0" stop-color="{ACCENT}"/><stop offset="1" stop-color="{ACCENT2}"/></linearGradient>
</defs>
<rect width="{WIDTH}" height="{height}" fill="url(#bg)"/>
<rect x="{MARGIN}" y="52" width="6" height="46" rx="3" fill="url(#bar)"/>
<text x="{tx}" y="78" font-size="34" font-weight="700" fill="{TEXT}">{title}</text>
<text x="{tx}" y="104" font-size="16" fill="{TEXT2}">{subtitle}</text>
"##,
        tx = MARGIN + 22.0,
        title = esc(&t!("app_name")),
        subtitle = esc(&clip(&t!("app_tagline"), 80)),
    );

    let mut y = HEADER_H;
    for row in &rows {
        let h = row_h(row);
        let _ = write!(
            svg,
            r##"<line x1="{MARGIN}" y1="{y}" x2="{x2}" y2="{y}" stroke="{BORDER}" stroke-width="1"/>
<text x="{LABEL_X}" y="{ly}" font-size="14" font-weight="700" letter-spacing="1.2" fill="{ACCENT}">{label}</text>
<text x="{CONTENT_X}" y="{by}" font-size="24" font-weight="600" fill="{TEXT}">{big}</text>
"##,
            x2 = WIDTH - MARGIN,
            ly = y + 40.0,
            by = y + 42.0,
            label = esc(&clip(&row.label.to_uppercase(), 16)),
            big = esc(&clip(&row.big, 58)),
        );
        for (i, sub) in row.subs.iter().enumerate() {
            let _ = writeln!(
                svg,
                r##"<text x="{CONTENT_X}" y="{sy}" font-size="15" fill="{TEXT2}">{sub}</text>"##,
                sy = y + 68.0 + 24.0 * i as f32,
                sub = esc(&clip(sub, 110)),
            );
        }
        y += h;
    }

    let _ = write!(
        svg,
        r##"<line x1="{MARGIN}" y1="{y}" x2="{x2}" y2="{y}" stroke="{BORDER}" stroke-width="1"/>
<text x="{MARGIN}" y="{fy}" font-size="14" fill="{TEXT3}">PC Specs v{version} · {time}</text>
</svg>
"##,
        x2 = WIDTH - MARGIN,
        fy = y + 44.0,
        version = env!("CARGO_PKG_VERSION"),
        time = esc(&fmt::datetime(snap.timestamp / 1000)),
    );
    svg
}

/// 把 SVG 栅格化成 PNG 字节。较慢（要加载系统字体），请放在后台线程调用。
pub fn render_png(svg: &str) -> Result<Vec<u8>, String> {
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &options).map_err(|e| e.to_string())?;
    let size = tree.size();
    let (w, h) = (
        (size.width() * SCALE).ceil() as u32,
        (size.height() * SCALE).ceil() as u32,
    );
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| "empty poster".to_string())?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|e| e.to_string())
}

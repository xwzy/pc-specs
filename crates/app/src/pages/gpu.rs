//! 显卡：每块 GPU 一张卡片，含实时占用、显存与规格。

use super::chart;
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, IntoElement as _, ParentElement as _, SharedString, Styled as _, div, rems,
};
use pc_specs_core::model::GpuInfo;
use rust_i18n::t;

pub fn render(s: &AppState, cx: &App) -> AnyElement {
    let gpus = &s.snapshot.gpus;
    let (subtitle, body) = if gpus.is_empty() {
        (
            None,
            vec![ui::empty_state(
                IconName::Gpu,
                t!("gpu_no_devices").into_owned(),
                t!("gpu_no_devices_hint").into_owned(),
            )],
        )
    } else {
        (
            // 只有一块 GPU 时「共 1 项」没有信息量。
            (gpus.len() > 1)
                .then(|| SharedString::from(t!("count_found", n = gpus.len()).into_owned())),
            gpus.iter()
                .enumerate()
                .map(|(i, g)| card(i, g, gpus.len() > 1, s, cx))
                .collect(),
        )
    };
    ui::page(
        "page-gpu",
        t!("nav_gpu").into_owned(),
        subtitle,
        vec![],
        body,
        cx,
    )
}

fn card(index: usize, g: &GpuInfo, numbered: bool, s: &AppState, cx: &App) -> AnyElement {
    let settings = &s.settings;
    let palette = chart::palette(cx);
    let kind = if g.is_discrete {
        t!("gpu_discrete")
    } else {
        t!("gpu_integrated")
    }
    .into_owned();

    let mut content = v_flex().gap_4().child(
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(ui::chip(g.vendor.clone()))
            .child(ui::chip(g.backend.clone()))
            .child(ui::chip(kind.clone())),
    );

    if let Some(util) = g.utilization {
        let name = g.name.clone();
        let points = chart::series(s, Some(120), move |tick| {
            tick.gpus
                .iter()
                .find(|x| x.name == name)
                .and_then(|x| x.utilization)
                .unwrap_or(0.0) as f64
        });
        content = content
            .child(ui::meter(
                SharedString::from(format!("gpu-util-{index}")),
                t!("gpu_utilization").into_owned(),
                fmt::percent(util),
                util,
                cx,
            ))
            .child(div().h(rems(7.)).w_full().child(chart::area(
                &format!("gpu-util-chart-{index}"),
                points,
                Some((0.0, 100.0)),
                palette[3],
                |v| format!("{v:.0}%"),
                "%",
            )));
    }

    if let (Some(used), Some(total)) = (g.vram_used_bytes, g.vram_total_bytes)
        && total > 0
    {
        content = content.child(ui::meter(
            SharedString::from(format!("gpu-vram-{index}")),
            t!("gpu_vram_used").into_owned(),
            format!("{} / {}", settings.bytes(used), settings.bytes(total)),
            fmt::ratio(used, total),
            cx,
        ));
    }

    // 厂商 / 图形 API / 类型已经是上面的标签，型号是卡片标题，占用和显存是上面的进度条，
    // 这里只列剩下的，不重复。
    let mut items = vec![
        (
            t!("spec_driver").into_owned(),
            g.driver.clone().unwrap_or_default(),
        ),
        (
            t!("spec_temp").into_owned(),
            s.gpu_temperature(g)
                .map_or(String::new(), |v| settings.temp(v)),
        ),
        (
            t!("gpu_power").into_owned(),
            g.power_w.map_or(String::new(), |w| format!("{w:.1} W")),
        ),
        (
            t!("spec_pcie").into_owned(),
            g.pcie_link.clone().unwrap_or_default(),
        ),
        (
            t!("spec_cores").into_owned(),
            g.cores.map_or(String::new(), |c| c.to_string()),
        ),
    ];
    // 只知道显存总量、不知道已用量时，上面没有进度条，总量得在这里补上。
    if g.vram_used_bytes.is_none()
        && let Some(total) = g.vram_total_bytes.filter(|total| *total > 0)
    {
        items.insert(
            0,
            (t!("gpu_vram_total").into_owned(), settings.bytes(total)),
        );
    }
    content = content.child(ui::spec_list(items, cx));

    // 极客面板只放上面没有的原始值（驱动、PCIe 已经在规格里）。
    if settings.geek_mode {
        content = content.child(
            v_flex()
                .gap_2()
                .child(ui::muted(t!("gpu_geek_panel").into_owned(), cx))
                .child(ui::spec_list(
                    vec![
                        (t!("gpu_geek_index").into_owned(), g.index.to_string()),
                        (t!("gpu_geek_vendor_raw").into_owned(), g.vendor.clone()),
                        (
                            t!("gpu_geek_device_type").into_owned(),
                            if g.is_discrete {
                                "DiscreteGpu"
                            } else {
                                "IntegratedGpu"
                            }
                            .to_string(),
                        ),
                    ],
                    cx,
                )),
        );
    }

    // 多块 GPU 时用序号区分；只有一块就直接写型号。
    let title = if numbered {
        format!("[{}] {}", g.index, g.name)
    } else {
        g.name.clone()
    };
    ui::section(SharedString::from(format!("gpu-{index}")), title)
        .child(content)
        .into_any_element()
}

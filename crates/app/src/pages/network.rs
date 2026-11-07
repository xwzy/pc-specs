//! 网络：实时吞吐、网关 / DNS / 公网 IP 与各网卡明细。

use super::chart;
use crate::state::{AppState, LookupState};
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::{
    AnyElement, App, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _, div,
    px, rems,
};
use rust_i18n::t;

pub fn render(s: &AppState, state: &Entity<AppState>, cx: &App) -> AnyElement {
    let net = &s.snapshot.network;
    let settings = &s.settings;
    let palette = chart::palette(cx);
    let unit = settings.net_unit;
    let scale = fmt::RateScale::new(
        s.ticks
            .iter()
            .rev()
            .take(180)
            .map(|t| t.net_rx_bps.max(t.net_tx_bps))
            .max()
            .unwrap_or(0),
        unit,
    );

    // ---- 副标题：数量 · 网关 · DNS ----
    let mut parts = vec![t!("count_found", n = net.interfaces.len()).into_owned()];
    if let Some(gw) = &net.default_gateway {
        parts.push(format!("{} {}", t!("net_gw_short"), gw));
    }
    if !net.dns_servers.is_empty() {
        let dns = net
            .dns_servers
            .iter()
            .take(2)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        parts.push(format!("{} {}", t!("net_dns_short"), dns));
    }

    // ---- 公网 IP：只在用户点击时才发起查询 ----
    let loading = s.public_ip == LookupState::Loading;
    let known_ip = match &s.public_ip {
        LookupState::Done(ip) => ip.clone(),
        _ => net.public_ip.clone(),
    };
    let label = if loading {
        t!("net_query_loading").into_owned()
    } else if let Some(ip) = known_ip {
        format!("{} {}", t!("net_public_prefix"), ip)
    } else {
        t!("net_query_public").into_owned()
    };
    let fetch_state = state.clone();
    let ip_button = Button::new("net-public-ip")
        .small()
        .outline()
        .icon(IconName::Globe)
        .label(label)
        .loading(loading)
        .on_click(move |_, _, cx| {
            fetch_state.update(cx, |s, cx| s.fetch_public_ip(cx));
        })
        .into_any_element();

    // ---- 吞吐图 ----
    let points = chart::series2(s, Some(180), move |t| {
        (scale.value(t.net_rx_bps), scale.value(t.net_tx_bps))
    });
    let (rx, tx) = s
        .latest
        .as_ref()
        .map(|t| (t.net_rx_bps, t.net_tx_bps))
        .unwrap_or_default();
    let throughput = ui::section(
        "net-throughput",
        format!("{} ({})", t!("net_throughput"), scale.name),
    )
    .child(
        gpui_kit::component::v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_6()
                    .text_lg()
                    .child(readout("↓", palette[2], settings.rate(rx)))
                    .child(readout("↑", palette[3], settings.rate(tx))),
            )
            .child(div().h(rems(12.)).w_full().child(chart::area2(
                "net-throughput-chart",
                points,
                (palette[2], palette[3]),
                (
                    t!("monitor_down").into_owned(),
                    t!("monitor_up").into_owned(),
                ),
                fmt::axis_label,
                cx,
            ))),
    );

    // ---- 网卡表 ----
    let interfaces = if net.interfaces.is_empty() {
        ui::empty_state(IconName::Network, t!("net_no_interfaces").into_owned(), "")
    } else {
        // 状态用名字前的小圆点表示（绿 = 在线），省一列宽度给实时速率和累计流量。
        let mut cols = vec![
            ui::Col::new(t!("net_interfaces").into_owned()).width(7.),
            ui::Col::new(t!("spec_kind").into_owned()).width(5.5),
            ui::Col::new("IPv4"),
            ui::Col::new(t!("spec_mac").into_owned()),
            ui::Col::new(t!("spec_link_speed").into_owned())
                .width(6.5)
                .right(),
            ui::Col::new(t!("net_iface_live").into_owned()).right(),
            ui::Col::new(format!("{} / {}", t!("net_rx_total"), t!("net_tx_total"))).right(),
        ];
        if settings.geek_mode {
            cols.insert(3, ui::Col::new("IPv6"));
        }
        // 在线、有 IPv4 的物理网卡排最前；离线 / 虚拟的往后放并调暗。
        let mut sorted: Vec<_> = net.interfaces.iter().collect();
        sorted.sort_by(|a, b| {
            let key = |n: &pc_specs_core::model::NetworkInterface| {
                (
                    !n.is_up,
                    n.ipv4.is_empty(),
                    n.is_loopback,
                    n.kind == "virtual",
                )
            };
            key(a)
                .cmp(&key(b))
                .then_with(|| fmt::natural_cmp(&a.name, &b.name))
        });
        let rows = sorted
            .into_iter()
            .map(|n| {
                let cell = |text: String| {
                    if n.is_up {
                        ui::txt(text)
                    } else {
                        ui::dim(text, cx).truncate().into_any_element()
                    }
                };
                let dot = if n.is_up {
                    cx.theme().green
                } else {
                    cx.theme().muted_foreground
                };
                let name = h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(div().flex_none().size(px(7.)).rounded_full().bg(dot))
                    .child(cell(n.name.clone()))
                    .into_any_element();
                let live = if n.is_up {
                    format!(
                        "↓ {} ↑ {}",
                        fmt::rate_compact(n.rx_bytes_per_sec, unit),
                        fmt::rate_compact(n.tx_bytes_per_sec, unit)
                    )
                } else {
                    "—".to_string()
                };
                let mut row = vec![
                    name,
                    cell(if n.is_loopback {
                        "loopback".to_string()
                    } else {
                        n.kind.clone()
                    }),
                    cell(fmt::or_dash(&n.ipv4.join(", "))),
                    cell(fmt::or_dash(n.mac.as_deref().unwrap_or(""))),
                    cell(
                        n.link_speed_mbps
                            .map_or("—".to_string(), |v| format!("{v} Mbps")),
                    ),
                    cell(live),
                    cell(format!(
                        "{} / {}",
                        settings.bytes(n.rx_total_bytes),
                        settings.bytes(n.tx_total_bytes)
                    )),
                ];
                if settings.geek_mode {
                    row.insert(3, cell(fmt::or_dash(&n.ipv6.join(", "))));
                }
                row
            })
            .collect();
        ui::table(cols, rows).into_any_element()
    };

    ui::page(
        "page-network",
        t!("nav_network").into_owned(),
        Some(SharedString::from(parts.join(" · "))),
        vec![ip_button],
        vec![
            throughput.into_any_element(),
            ui::plain_section("net-interfaces", t!("net_interfaces").into_owned())
                .child(interfaces)
                .into_any_element(),
        ],
        cx,
    )
}

/// 带方向箭头的数值，箭头颜色和图线一致，兼作图例。
fn readout(arrow: &'static str, color: gpui_kit::Hsla, value: String) -> AnyElement {
    h_flex()
        .gap_1p5()
        .child(div().text_color(color).child(arrow))
        .child(div().child(value))
        .into_any_element()
}

//! 网络：实时吞吐、连通性（到路由器 / DNS / 公网的延迟）、Wi-Fi 详情、流量统计，
//! 以及网关 / DNS / 公网 IP 与各网卡明细。

use super::chart;
use crate::health::{self, Level};
use crate::settings::NetUnit;
use crate::state::{AppState, LookupState};
use crate::usage::{self, Traffic};
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Entity, IntoElement as _, ParentElement as _, SharedString, Styled as _, div,
    prelude::FluentBuilder as _, px, relative, rems,
};
use pc_specs_core::model::NetworkInterface;
use pc_specs_core::netprobe::{self, TargetKind, WifiInfo};
use rust_i18n::t;

/// 延迟统计和走势图取最近多少个采样（两秒一个，两分钟）。
const WINDOW: usize = 60;

/// 在线、有 IPv4 的物理网卡：用户真正在用的那几个。
fn is_active(n: &NetworkInterface) -> bool {
    n.is_up && !n.is_loopback && !n.ipv4.is_empty()
}

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

    // ---- 副标题：网关 · DNS ----
    let mut parts = Vec::new();
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
    let subtitle = (!parts.is_empty()).then(|| SharedString::from(parts.join(" · ")));

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
        v_flex()
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
                scale.name,
                cx,
            ))),
    );

    // ---- 网卡表 ----
    let total = net.interfaces.len();
    let interfaces = if net.interfaces.is_empty() {
        ui::empty_state(IconName::Network, t!("net_no_interfaces").into_owned(), "")
    } else {
        // 默认只列在线的；一块在线的都没有（断网）、或者全都在线时没有可筛的，直接全列。
        let active = net.interfaces.iter().filter(|n| is_active(n)).count();
        let can_filter = active > 0 && active < total;
        let filtered = can_filter && !s.net_show_all;

        // 状态用名字前的小圆点表示（绿 = 在线），省一列宽度给实时速率和累计流量。
        let rate_name = match unit {
            NetUnit::BytesPerSec => "B/s",
            NetUnit::BitsPerSec => "bps",
        };
        let mut cols = vec![
            ui::Col::new(t!("net_interfaces").into_owned()).width(7.),
            ui::Col::new(t!("spec_kind").into_owned()).width(7.),
            ui::Col::new("IPv4"),
            ui::Col::new(t!("spec_mac").into_owned()),
            ui::Col::new(t!("spec_link_speed").into_owned())
                .width(6.5)
                .right(),
            // 表里的速率是 `97.3K` 这样的缩写，单位写在表头。
            ui::Col::new(format!("{} ({rate_name})", t!("net_iface_live"))).right(),
            ui::Col::new(format!("{} / {}", t!("net_rx_total"), t!("net_tx_total"))).right(),
        ];
        if settings.geek_mode {
            cols.insert(3, ui::Col::new("IPv6"));
        }
        // 在线、有 IPv4 的物理网卡排最前；离线 / 虚拟的往后放并调暗。
        let mut sorted: Vec<_> = net
            .interfaces
            .iter()
            .filter(|n| !filtered || is_active(n))
            .collect();
        let shown = sorted.len();
        sorted.sort_by(|a, b| {
            let key = |n: &NetworkInterface| {
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
                    cell(fmt::net_kind(if n.is_loopback {
                        "loopback"
                    } else {
                        &n.kind
                    })),
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

        // 表上方一行：当前显示了几个，以及显示全部 / 只看在线的开关。
        let toggle_state = state.clone();
        let show_all = s.net_show_all;
        let header = h_flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(ui::muted(
                if can_filter {
                    t!("net_showing", shown = shown, total = total).into_owned()
                } else {
                    t!("count_found", n = total).into_owned()
                },
                cx,
            ))
            .when(can_filter, |this| {
                this.child(
                    Button::new("net-toggle-all")
                        .small()
                        .ghost()
                        .label(if show_all {
                            t!("net_show_active").into_owned()
                        } else {
                            t!("net_show_all").into_owned()
                        })
                        .on_click(move |_, _, cx| {
                            toggle_state.update(cx, |s, cx| {
                                s.net_show_all = !s.net_show_all;
                                cx.notify();
                            });
                        }),
                )
            });
        v_flex()
            .gap_2()
            .child(header)
            .child(ui::table(cols, rows))
            .into_any_element()
    };

    let mut body = vec![throughput.into_any_element(), connection(s, cx)];
    if let Some(info) = &s.wifi {
        body.push(wifi(info, cx));
    }
    body.push(data_usage(s, state, cx));
    body.push(
        ui::plain_section("net-interfaces", t!("net_interfaces").into_owned())
            .child(interfaces)
            .into_any_element(),
    );
    ui::page(
        "page-network",
        t!("nav_network").into_owned(),
        subtitle,
        vec![ip_button],
        body,
        cx,
    )
}

fn target_name(kind: TargetKind) -> String {
    match kind {
        TargetKind::Gateway => t!("net_target_gateway"),
        TargetKind::Dns => t!("net_target_dns"),
        TargetKind::Internet => t!("net_target_internet"),
    }
    .into_owned()
}

/// 连通性：到路由器、DNS 服务器和公网的延迟、抖动、丢包与走势。
fn connection(s: &AppState, cx: &App) -> AnyElement {
    let palette = chart::palette(cx);
    let latest = s.latency.back();
    let status = match latest.and_then(|sample| sample.online()) {
        Some(true) => ui::level_tag(Level::Healthy, t!("net_online").into_owned()),
        Some(false) => ui::level_tag(Level::Critical, t!("net_offline").into_owned()),
        None => ui::chip(t!("net_measuring").into_owned()),
    };

    let table = match latest {
        None => ui::muted(t!("net_measuring").into_owned(), cx).into_any_element(),
        Some(latest) => {
            let rows = latest
                .readings
                .iter()
                .map(|reading| {
                    let kind = reading.kind;
                    // 只取量过这个对象的采样（比如中途换了网络，一开始没有路由器）。
                    let recent: Vec<_> = s
                        .latency
                        .iter()
                        .rev()
                        .take(WINDOW)
                        .rev()
                        .filter_map(|sample| {
                            Some((sample.timestamp_ms, sample.reading(kind)?.rtt_ms))
                        })
                        .collect();
                    let stats = netprobe::quality(recent.iter().map(|(_, rtt)| *rtt));
                    let points: Vec<chart::Point> = recent
                        .iter()
                        .filter_map(|(at, rtt)| {
                            Some(chart::Point {
                                label: fmt::clock(*at).into(),
                                value: f64::from((*rtt)?),
                            })
                        })
                        .collect();
                    let peak = points.iter().map(|point| point.value).fold(5.0, f64::max);
                    let now = match reading.rtt_ms {
                        Some(ms) => div()
                            .text_color(ui::level_color(health::latency_level(kind, ms), cx))
                            .child(fmt::latency(ms)),
                        None => div()
                            .text_color(cx.theme().danger)
                            .child(t!("net_timeout").into_owned()),
                    };
                    let ms = |value: Option<f32>| value.map_or("—".to_string(), fmt::latency);
                    vec![
                        ui::txt(target_name(kind)),
                        ui::txt(reading.address.clone()),
                        now.into_any_element(),
                        ui::txt(ms(stats.average_ms)),
                        ui::txt(ms(stats.jitter_ms)),
                        ui::txt(format!("{:.0}%", stats.loss_percent)),
                        div()
                            .w_full()
                            .h(rems(1.75))
                            .child(chart::sparkline(
                                &format!("net-trend-{kind:?}"),
                                points,
                                Some((0.0, peak * 1.25)),
                                palette[0],
                            ))
                            .into_any_element(),
                    ]
                })
                .collect();
            let cols = vec![
                ui::Col::new(t!("net_target").into_owned()).width(8.),
                ui::Col::new(t!("net_address").into_owned()),
                ui::Col::new(t!("net_latency").into_owned())
                    .right()
                    .width(6.),
                ui::Col::new(t!("net_latency_avg").into_owned())
                    .right()
                    .width(5.5),
                ui::Col::new(t!("net_jitter").into_owned())
                    .right()
                    .width(5.5),
                ui::Col::new(t!("net_loss").into_owned()).right().width(4.5),
                ui::Col::new(t!("net_trend").into_owned()).width(9.),
            ];
            ui::table(cols, rows).into_any_element()
        }
    };

    ui::plain_section("net-connection", t!("net_connection").into_owned())
        .child(
            v_flex()
                .gap_2()
                .child(h_flex().child(status))
                .child(table)
                .child(ui::muted(t!("net_probe_hint").into_owned(), cx).text_xs()),
        )
        .into_any_element()
}

fn quality_text(quality: u8) -> String {
    match quality {
        75.. => t!("net_wifi_excellent"),
        50..=74 => t!("net_wifi_good"),
        25..=49 => t!("net_wifi_fair"),
        _ => t!("net_wifi_weak"),
    }
    .into_owned()
}

/// 当前 Wi-Fi 连接：信号质量一眼看懂，细节列在下面。系统没给的项不列。
fn wifi(info: &WifiInfo, cx: &App) -> AnyElement {
    let mut summary = h_flex().items_center().gap_2().flex_wrap();
    if let Some(quality) = info.quality() {
        summary = summary.child(ui::level_tag(
            health::signal_level(quality),
            quality_text(quality),
        ));
    }
    if let Some(generation) = info.generation() {
        summary = summary.child(ui::chip(generation));
    }
    if let Some(dbm) = info.signal_dbm {
        summary = summary.child(div().text_lg().child(format!("{dbm} dBm")));
    }

    let join = |parts: Vec<Option<String>>| {
        let text = parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
        (!text.is_empty()).then_some(text)
    };
    let items: Vec<(String, Option<String>)> = vec![
        (t!("net_wifi_network").into_owned(), info.ssid.clone()),
        (
            t!("net_wifi_signal").into_owned(),
            info.signal_dbm
                .map(|dbm| format!("{dbm} dBm"))
                .or_else(|| info.signal_percent.map(|percent| format!("{percent}%"))),
        ),
        (
            t!("net_wifi_noise").into_owned(),
            info.noise_dbm.map(|dbm| format!("{dbm} dBm")),
        ),
        (
            t!("net_wifi_snr").into_owned(),
            info.snr_db().map(|db| format!("{db} dB")),
        ),
        (
            t!("net_wifi_channel").into_owned(),
            join(vec![
                info.channel.map(|channel| channel.to_string()),
                info.band.clone(),
                info.width_mhz.map(|width| format!("{width} MHz")),
            ]),
        ),
        (
            t!("net_wifi_standard").into_owned(),
            info.standard
                .as_ref()
                .map(|standard| match info.generation() {
                    Some(generation) => format!("{standard} ({generation})"),
                    None => standard.clone(),
                }),
        ),
        (
            t!("net_wifi_rate").into_owned(),
            info.rate_mbps.map(|rate| format!("{rate} Mbps")),
        ),
        (t!("net_wifi_security").into_owned(), info.security.clone()),
        (t!("net_wifi_bssid").into_owned(), info.bssid.clone()),
    ];
    let items = items
        .into_iter()
        .filter_map(|(label, value)| Some((label, value?)))
        .collect();

    ui::section(
        "net-wifi",
        format!("{} · {}", t!("net_wifi"), info.interface),
    )
    .child(
        v_flex()
            .gap_3()
            .child(summary)
            .child(ui::spec_list(items, cx)),
    )
    .into_any_element()
}

/// 流量统计：今天 / 近 7 天 / 本月 / 累计，以及最近每天各走了多少。
fn data_usage(s: &AppState, state: &Entity<AppState>, cx: &App) -> AnyElement {
    let palette = chart::palette(cx);
    let (down, up) = (palette[2], palette[3]);
    let settings = &s.settings;
    let usage = &s.usage;
    let today = usage::today();
    let week = usage.last_days(today, 7);

    let tile = |label: String, traffic: Traffic, note: Option<String>| {
        v_flex()
            .w(rems(11.))
            .gap_0p5()
            .child(ui::muted(label, cx).text_xs())
            .child(readout("↓", down, settings.bytes(traffic.rx)))
            .child(readout("↑", up, settings.bytes(traffic.tx)))
            .when_some(note, |this, note| this.child(ui::muted(note, cx).text_xs()))
    };
    let since = usage
        .since()
        .map(|day| t!("net_usage_since", date = day.format("%Y-%m-%d").to_string()).into_owned());
    let tiles = h_flex()
        .gap_6()
        .flex_wrap()
        .child(tile(
            t!("net_usage_today").into_owned(),
            usage.day(today),
            None,
        ))
        .child(tile(
            t!("net_usage_week").into_owned(),
            Traffic::sum(week.iter().map(|(_, traffic)| *traffic)),
            None,
        ))
        .child(tile(
            t!("net_usage_month").into_owned(),
            usage.month(today),
            None,
        ))
        .child(tile(
            t!("net_usage_total").into_owned(),
            usage.total(),
            since,
        ));

    // 每天一条：条的长度按这几天里最多的那天算，下载 / 上传各占一段。
    let peak = week
        .iter()
        .map(|(_, traffic)| traffic.total())
        .max()
        .unwrap_or(0)
        .max(1) as f32;
    let days = week
        .iter()
        .rev()
        .enumerate()
        .map(|(index, (day, traffic))| {
            let name = if index == 0 {
                t!("net_usage_today").into_owned()
            } else {
                day.format("%m/%d").to_string()
            };
            h_flex()
                .items_center()
                .gap_3()
                .child(div().w(rems(3.5)).flex_none().child(ui::muted(name, cx)))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h(px(10.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(cx.theme().muted)
                        .child(
                            div()
                                .h_full()
                                .w(relative(traffic.rx as f32 / peak))
                                .bg(down),
                        )
                        .child(div().h_full().w(relative(traffic.tx as f32 / peak)).bg(up)),
                )
                .child(
                    div()
                        .w(rems(13.))
                        .flex_none()
                        .text_sm()
                        .text_right()
                        .child(format!(
                            "↓ {}   ↑ {}",
                            settings.bytes(traffic.rx),
                            settings.bytes(traffic.tx)
                        )),
                )
        });

    let reset_state = state.clone();
    let reset = Button::new("net-usage-reset")
        .small()
        .ghost()
        .icon(IconName::RotateCcw)
        .label(t!("net_usage_reset").into_owned())
        .tooltip(t!("net_usage_reset_hint").into_owned())
        .on_click(move |_, _, cx| {
            reset_state.update(cx, |s, cx| s.reset_usage(cx));
        });

    ui::section("net-usage", t!("net_usage").into_owned())
        .child(
            v_flex()
                .gap_4()
                .child(tiles)
                .child(
                    v_flex()
                        .gap_1p5()
                        .child(ui::muted(t!("net_usage_daily").into_owned(), cx).text_xs())
                        .children(days),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(ui::muted(t!("net_usage_hint").into_owned(), cx).text_xs())
                        .child(reset),
                ),
        )
        .into_any_element()
}

/// 带方向箭头的数值，箭头颜色和图线一致，兼作图例。
fn readout(arrow: &'static str, color: gpui_kit::Hsla, value: String) -> AnyElement {
    h_flex()
        .gap_1p5()
        .child(div().text_color(color).child(arrow))
        .child(div().child(value))
        .into_any_element()
}

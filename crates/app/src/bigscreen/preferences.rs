//! 设置页「监控大屏」：最上面是随设置实时变化的预览，下面依次是显示、外观、布局。

use super::config::{
    BigScreenSettings, CELL_RANGE, ChartPref, Density, Preset, Rank, SCALE_RANGE, SCALE_STEP,
    Shape, WidgetConfig,
};
use super::theme::ThemeId;
use super::view::{Canvas, wall};
use crate::{state::AppState, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    switch::Switch,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px, rems,
};
use pc_specs_core::util::guarded;
use rust_i18n::t;

/// 预览框最大能占多大（逻辑像素）。横屏、竖屏、超宽屏都按这个框等比缩进去。
const PREVIEW_BOX: (f32, f32) = (560., 360.);
/// 预览按「短边 720」的屏幕排版，字号和疏密都和这个分辨率的真实大屏一样。
const PREVIEW_SHORT_SIDE: f32 = 720.;

pub struct BigScreenPage {
    state: Entity<AppState>,
    title: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl BigScreenPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = state.read(cx).settings.bigscreen.title.clone();
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("bs_title_placeholder").into_owned())
                .default_value(current)
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&title, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    this.state.update(cx, |s, cx| {
                        s.update_settings(|v| v.bigscreen.title = text, cx)
                    });
                }
            }),
        ];
        Self {
            state,
            title,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for BigScreenPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 设置被重置之后，输入框里的字要跟着回去。
        let wanted = self.state.read(cx).settings.bigscreen.title.clone();
        let typed = self.title.read(cx).value();
        if *typed != *wanted {
            self.title
                .update(cx, |input, cx| input.set_value(wanted, window, cx));
        }
        guarded("render big screen settings", || {
            content(&self.state, &self.title, cx)
        })
        .unwrap_or_else(|| div().into_any_element())
    }
}

fn content(state: &Entity<AppState>, title: &Entity<InputState>, cx: &App) -> AnyElement {
    let s = state.read(cx);
    let cfg = &s.settings.bigscreen;
    let open = super::is_open(cx);

    let actions = vec![
        {
            let state = state.clone();
            Button::new("bs-reset")
                .small()
                .outline()
                .label(t!("bs_reset").into_owned())
                .on_click(move |_, _, cx| {
                    edit(&state, |c| *c = BigScreenSettings::default(), cx);
                    super::reopen(cx);
                })
                .into_any_element()
        },
        Button::new("bs-wall")
            .small()
            .primary()
            .icon(IconName::MonitorPlay)
            .label(if open {
                t!("bs_close_wall").into_owned()
            } else {
                t!("bs_open_wall").into_owned()
            })
            .on_click(move |_, _, cx| {
                if open {
                    super::close(cx);
                } else {
                    super::open(cx);
                }
            })
            .into_any_element(),
    ];

    ui::page(
        "page-big-screen",
        t!("bs_title").into_owned(),
        Some(t!("bs_description").into_owned().into()),
        actions,
        vec![
            preview_section(s, state, cx).into_any_element(),
            display_section(state, cfg, cx).into_any_element(),
            appearance_section(state, title, cfg, cx).into_any_element(),
            layout_section(state, cfg, cx).into_any_element(),
            keys_section(cx).into_any_element(),
        ],
        cx,
    )
}

// ---- 预览 -------------------------------------------------------------------

fn preview_section(s: &AppState, state: &Entity<AppState>, cx: &App) -> impl IntoElement {
    let cfg = &s.settings.bigscreen;
    let screen = guarded("render preview", || preview(s, cfg, cx)).unwrap_or_else(div);
    ui::section("bs-preview", t!("bs_preview").into_owned()).child(
        v_flex()
            .gap_3()
            .items_start()
            .child(screen)
            .child(choice(
                state,
                "bs-shape",
                t!("bs_shape").into_owned(),
                Shape::ALL
                    .into_iter()
                    .map(|shape| (shape, shape.title()))
                    .collect(),
                cfg.preview,
                |c, v| c.preview = v,
            ))
            .child(ui::muted(t!("bs_preview_hint").into_owned(), cx).text_xs()),
    )
}

/// 把一整块屏幕缩进一个小框：按真实大屏的参考分辨率排版，再整体缩小来画。
fn preview(s: &AppState, cfg: &BigScreenSettings, cx: &App) -> Div {
    let (long, short) = cfg.preview.ratio();
    let ratio = long / short;
    let (width, height) = if ratio >= 1. {
        (PREVIEW_SHORT_SIDE * ratio, PREVIEW_SHORT_SIDE)
    } else {
        (PREVIEW_SHORT_SIDE, PREVIEW_SHORT_SIDE / ratio)
    };
    let factor = (PREVIEW_BOX.0 / width).min(PREVIEW_BOX.1 / height);
    let mono = cfg
        .theme
        .palette()
        .mono
        .then(|| cx.theme().mono_font_family.clone());
    div()
        .relative()
        .flex_none()
        .w(px(width * factor))
        .h(px(height * factor))
        .overflow_hidden()
        .rounded(px(10.))
        .border_1()
        .border_color(cx.theme().border)
        .child(wall(
            s,
            cfg,
            Canvas {
                width,
                height,
                factor,
            },
            0.,
            mono,
        ))
}

// ---- 显示 -------------------------------------------------------------------

fn display_section(
    state: &Entity<AppState>,
    cfg: &BigScreenSettings,
    cx: &App,
) -> impl IntoElement {
    let screens = super::screens(cx);
    // 设置里记着的那块屏现在没接上：提醒一声，大屏会自动挑一块。
    let missing = cfg
        .display
        .as_ref()
        .is_some_and(|key| screens.iter().all(|screen| &screen.key != key));

    let mut options = vec![{
        let state = state.clone();
        Button::new("bs-display-auto")
            .small()
            .outline()
            .label(t!("bs_display_auto").into_owned())
            .selected(cfg.display.is_none())
            .on_click(move |_, _, cx| {
                edit(&state, |c| c.display = None, cx);
                super::reopen(cx);
            })
    }];
    for (index, screen) in screens.iter().enumerate() {
        let state = state.clone();
        let key = screen.key.clone();
        let mut label = t!("bs_display_n", n = index + 1).into_owned();
        if screen.primary {
            label.push_str(&format!(" · {}", t!("bs_display_primary")));
        }
        label.push_str(&format!(
            " · {}×{}",
            screen.width as u32, screen.height as u32
        ));
        options.push(
            Button::new(SharedString::from(format!("bs-display-{index}")))
                .small()
                .outline()
                .label(label)
                .selected(cfg.display.as_ref() == Some(&screen.key))
                .on_click(move |_, _, cx| {
                    let key = key.clone();
                    edit(&state, move |c| c.display = Some(key), cx);
                    super::reopen(cx);
                }),
        );
    }

    let chooser = v_flex()
        .gap_2()
        .child(div().text_sm().child(t!("bs_display").into_owned()))
        .child(h_flex().flex_wrap().gap_2().children(options))
        .child(
            ui::muted(
                if screens.is_empty() {
                    t!("bs_display_none")
                } else if missing {
                    t!("bs_display_missing")
                } else {
                    t!("bs_display_hint")
                }
                .into_owned(),
                cx,
            )
            .text_xs(),
        );

    ui::section("bs-display", t!("bs_section_display").into_owned()).child(
        v_flex()
            .gap_4()
            .child(chooser)
            .child(hinted(
                toggle_then(
                    state,
                    "bs-fullscreen",
                    t!("bs_fullscreen").into_owned(),
                    cfg.fullscreen,
                    |c, v| c.fullscreen = v,
                    super::reopen,
                    cx,
                ),
                t!("bs_fullscreen_hint").into_owned(),
                cx,
            ))
            .child(hinted(
                toggle(
                    state,
                    "bs-launch",
                    t!("bs_open_on_launch").into_owned(),
                    cfg.open_on_launch,
                    |c, v| c.open_on_launch = v,
                    cx,
                ),
                t!("bs_open_on_launch_hint").into_owned(),
                cx,
            ))
            .child(hinted(
                toggle(
                    state,
                    "bs-awake",
                    t!("bs_keep_awake").into_owned(),
                    cfg.keep_awake,
                    |c, v| c.keep_awake = v,
                    cx,
                ),
                t!("bs_keep_awake_hint").into_owned(),
                cx,
            )),
    )
}

// ---- 外观 -------------------------------------------------------------------

fn appearance_section(
    state: &Entity<AppState>,
    title: &Entity<InputState>,
    cfg: &BigScreenSettings,
    cx: &App,
) -> impl IntoElement {
    let themes = h_flex().flex_wrap().gap_3().children(
        ThemeId::ALL
            .into_iter()
            .map(|id| theme_card(state, id, id == cfg.theme, cx)),
    );

    let title_row = h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(div().text_sm().child(t!("bs_header_title").into_owned()))
        .child(Input::new(title).small().w(rems(16.)).cleanable(true));

    ui::section("bs-appearance", t!("bs_section_appearance").into_owned()).child(
        v_flex()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .child(div().text_sm().child(t!("bs_theme").into_owned()))
                    .child(themes),
            )
            .child(scale_row(state, cfg.scale, cx))
            .child(choice(
                state,
                "bs-density",
                t!("bs_density").into_owned(),
                vec![
                    (Density::Compact, t!("bs_density_compact").into_owned()),
                    (Density::Normal, t!("bs_density_normal").into_owned()),
                    (Density::Relaxed, t!("bs_density_relaxed").into_owned()),
                ],
                cfg.density,
                |c, v| c.density = v,
            ))
            .child(choice(
                state,
                "bs-chart",
                t!("bs_chart").into_owned(),
                vec![
                    (ChartPref::Auto, t!("bs_chart_auto").into_owned()),
                    (ChartPref::Area, t!("bs_chart_area").into_owned()),
                    (ChartPref::Line, t!("bs_chart_line").into_owned()),
                    (ChartPref::Bars, t!("bs_chart_bars").into_owned()),
                ],
                cfg.chart,
                |c, v| c.chart = v,
            ))
            .child(choice(
                state,
                "bs-chart-minutes",
                t!("bs_chart_minutes").into_owned(),
                [1u32, 2, 5, 10]
                    .into_iter()
                    .map(|n| (n, t!("bs_minutes", n = n).into_owned()))
                    .collect(),
                cfg.chart_minutes,
                |c, v| c.chart_minutes = v,
            ))
            .child(toggle(
                state,
                "bs-header",
                t!("bs_show_header").into_owned(),
                cfg.show_header,
                |c, v| c.show_header = v,
                cx,
            ))
            .when(cfg.show_header, |column| {
                column.child(title_row).child(toggle(
                    state,
                    "bs-clock",
                    t!("bs_show_clock").into_owned(),
                    cfg.show_clock,
                    |c, v| c.show_clock = v,
                    cx,
                ))
            })
            .when(cfg.show_header && cfg.show_clock, |column| {
                column
                    .child(toggle(
                        state,
                        "bs-seconds",
                        t!("bs_show_seconds").into_owned(),
                        cfg.show_seconds,
                        |c, v| c.show_seconds = v,
                        cx,
                    ))
                    .child(toggle(
                        state,
                        "bs-hour24",
                        t!("bs_hour24").into_owned(),
                        cfg.hour24,
                        |c, v| c.hour24 = v,
                        cx,
                    ))
            }),
    )
}

/// 一张主题卡：用主题自己的底色和磁贴色画一个迷你大屏，一眼看出风格。
fn theme_card(state: &Entity<AppState>, id: ThemeId, selected: bool, cx: &App) -> Stateful<Div> {
    let pal = id.palette();
    let state = state.clone();
    let tile = |accent| {
        v_flex()
            .flex_1()
            .gap(px(4.))
            .p(px(5.))
            .rounded(px(3.))
            .bg(pal.tile)
            .border_1()
            .border_color(pal.tile_edge)
            .child(div().w(px(16.)).h(px(3.)).rounded_full().bg(accent))
            .child(div().w(px(30.)).h(px(3.)).rounded_full().bg(pal.faint))
    };
    let swatch = div()
        .relative()
        .w_full()
        .h(px(74.))
        .overflow_hidden()
        .rounded(px(8.))
        .bg(pal.background())
        .border_2()
        .border_color(if selected {
            cx.theme().primary
        } else {
            cx.theme().border
        })
        .child(
            h_flex()
                .absolute()
                .top(px(6.))
                .left(px(6.))
                .right(px(6.))
                .bottom(px(6.))
                .gap(px(4.))
                .child(tile(pal.cpu))
                .child(
                    v_flex()
                        .flex_1()
                        .gap(px(4.))
                        .child(tile(pal.mem))
                        .child(tile(pal.down)),
                ),
        );
    v_flex()
        .id(SharedString::from(format!("bs-theme-{}", id.key())))
        .w(px(132.))
        .gap(px(5.))
        .cursor_pointer()
        .child(swatch)
        .child(
            div()
                .text_xs()
                .text_center()
                .when(selected, |label| label.font_semibold())
                .text_color(if selected {
                    cx.theme().foreground
                } else {
                    cx.theme().muted_foreground
                })
                .child(id.title()),
        )
        .on_click(move |_, _, cx| edit(&state, |c| c.theme = id, cx))
}

/// 「− 100% +」：每按一下改一个步长，到头了按钮变灰。
fn scale_row(state: &Entity<AppState>, value: u32, cx: &App) -> AnyElement {
    let button = |name: &'static str, icon: IconName, enabled: bool, next: u32| {
        let state = state.clone();
        Button::new(name)
            .small()
            .outline()
            .icon(icon)
            .disabled(!enabled)
            .on_click(move |_, _, cx| edit(&state, |c| c.scale = next, cx))
    };
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .text_color(cx.theme().foreground)
        .child(
            v_flex()
                .gap_1()
                .child(div().text_sm().child(t!("bs_scale").into_owned()))
                .child(ui::muted(t!("bs_scale_hint").into_owned(), cx).text_xs()),
        )
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(button(
                    "bs-scale-down",
                    IconName::Minus,
                    value > *SCALE_RANGE.start(),
                    value.saturating_sub(SCALE_STEP),
                ))
                .child(
                    div()
                        .w(px(56.))
                        .text_center()
                        .text_sm()
                        .font_medium()
                        .child(format!("{value}%")),
                )
                .child(button(
                    "bs-scale-up",
                    IconName::Plus,
                    value < *SCALE_RANGE.end(),
                    value + SCALE_STEP,
                )),
        )
        .into_any_element()
}

// ---- 布局 -------------------------------------------------------------------

fn layout_section(state: &Entity<AppState>, cfg: &BigScreenSettings, cx: &App) -> impl IntoElement {
    let presets = v_flex()
        .gap_2()
        .child(div().text_sm().child(t!("bs_presets").into_owned()))
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .children(Preset::ALL.into_iter().enumerate().map(|(index, preset)| {
                    let state = state.clone();
                    Button::new(SharedString::from(format!("bs-preset-{index}")))
                        .small()
                        .outline()
                        .label(preset.title())
                        .on_click(move |_, _, cx| edit(&state, |c| preset.apply(c), cx))
                })),
        )
        .child(ui::muted(t!("bs_presets_hint").into_owned(), cx).text_xs());

    let count = cfg.widgets.len();
    let widgets = v_flex().children(
        cfg.widgets
            .iter()
            .enumerate()
            .map(|(index, widget)| widget_row(state, index, count, widget, cx)),
    );

    ui::section("bs-layout", t!("bs_section_layout").into_owned()).child(
        v_flex()
            .gap_4()
            .child(presets)
            .child(hinted(
                toggle(
                    state,
                    "bs-hide-missing",
                    t!("bs_hide_missing").into_owned(),
                    cfg.hide_missing,
                    |c, v| c.hide_missing = v,
                    cx,
                ),
                t!("bs_hide_missing_hint").into_owned(),
                cx,
            ))
            .child(toggle(
                state,
                "bs-per-core",
                t!("bs_per_core").into_owned(),
                cfg.per_core,
                |c, v| c.per_core = v,
                cx,
            ))
            .child(choice(
                state,
                "bs-rank",
                t!("bs_rank").into_owned(),
                vec![
                    (Rank::Cpu, t!("bs_rank_cpu").into_owned()),
                    (Rank::Memory, t!("bs_rank_memory").into_owned()),
                ],
                cfg.rank,
                |c, v| c.rank = v,
            ))
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child(t!("bs_widgets").into_owned()))
                    .child(ui::muted(t!("bs_widgets_hint").into_owned(), cx).text_xs())
                    .child(widgets),
            ),
    )
}

/// 一个组件的一行：开关、名字、宽高、上下移动。
fn widget_row(
    state: &Entity<AppState>,
    index: usize,
    count: usize,
    widget: &WidgetConfig,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let kind = widget.kind;
    let key = kind.key();
    let accent = theme.primary;

    let size = |axis: &'static str, label: String, value: u8, set: fn(&mut WidgetConfig, u8)| {
        let state = state.clone();
        h_flex()
            .items_center()
            .gap_1p5()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(label),
            )
            .child(ui::segmented(
                SharedString::from(format!("bs-{key}-{axis}")),
                CELL_RANGE.map(|n| (n, n.to_string())).collect(),
                value,
                move |n, cx| edit(&state, |c| c.edit_widget(kind, |w| set(w, n)), cx),
            ))
    };

    let mut order = h_flex().gap_1().w(px(64.)).justify_end();
    if index > 0 {
        let state = state.clone();
        order = order.child(
            Button::new(SharedString::from(format!("bs-up-{key}")))
                .ghost()
                .small()
                .icon(IconName::ArrowUp)
                .on_click(move |_, _, cx| edit(&state, |c| c.move_widget(kind, true), cx)),
        );
    }
    if index + 1 < count {
        let state = state.clone();
        order = order.child(
            Button::new(SharedString::from(format!("bs-down-{key}")))
                .ghost()
                .small()
                .icon(IconName::ArrowDown)
                .on_click(move |_, _, cx| edit(&state, |c| c.move_widget(kind, false), cx)),
        );
    }

    let toggle_state = state.clone();
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .py_2()
        .when(index + 1 < count, |row| {
            row.border_b_1().border_color(theme.border)
        })
        .child(
            Switch::new(SharedString::from(format!("bs-on-{key}")))
                .accessibility_label(kind.title())
                .checked(widget.enabled)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    edit(
                        &toggle_state,
                        |c| c.edit_widget(kind, |w| w.enabled = checked),
                        cx,
                    );
                }),
        )
        .child(
            div()
                .size(px(30.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .bg(accent.opacity(0.16))
                .child(Icon::new(kind.icon()).small().text_color(accent)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .font_medium()
                .truncate()
                .when(!widget.enabled, |name| {
                    name.text_color(theme.muted_foreground)
                })
                .child(kind.title()),
        )
        .when(widget.enabled, |row| {
            row.child(size("w", t!("bs_width").into_owned(), widget.w, |w, n| {
                w.w = n
            }))
            .child(size("h", t!("bs_height").into_owned(), widget.h, |w, n| {
                w.h = n
            }))
        })
        .child(order)
}

// ---- 快捷键 -----------------------------------------------------------------

fn keys_section(cx: &App) -> impl IntoElement {
    let key = |keys: &str, what: String| {
        h_flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .w(px(84.))
                    .flex_shrink_0()
                    .child(ui::chip(keys.to_string())),
            )
            .child(ui::muted(what, cx).text_xs())
    };
    ui::section("bs-keys", t!("bs_section_keys").into_owned()).child(
        v_flex()
            .gap_2()
            .child(key("F / F11", t!("bs_key_fullscreen").into_owned()))
            .child(key("Esc", t!("bs_key_leave").into_owned()))
            .child(key("← / →", t!("bs_key_theme").into_owned()))
            .child(key("+ / −", t!("bs_key_zoom").into_owned()))
            .child(key("H", t!("bs_key_header").into_owned()))
            .child(key("S", t!("bs_key_settings").into_owned()))
            .child(ui::muted(t!("bs_keys_hint").into_owned(), cx).text_xs()),
    )
}

// ---- 小部件 -----------------------------------------------------------------

fn edit(state: &Entity<AppState>, change: impl FnOnce(&mut BigScreenSettings), cx: &mut App) {
    state.update(cx, |s, cx| {
        s.update_settings(|v| change(&mut v.bigscreen), cx)
    });
}

/// 一行设置加上紧贴在它下面的说明：说明跟这一行靠得近，跟下一行隔开。
fn hinted(row: AnyElement, hint: String, cx: &App) -> Div {
    v_flex()
        .gap_1()
        .child(row)
        .child(ui::muted(hint, cx).text_xs())
}

fn toggle(
    state: &Entity<AppState>,
    id: &'static str,
    label: String,
    value: bool,
    set: fn(&mut BigScreenSettings, bool),
    cx: &App,
) -> AnyElement {
    toggle_then(state, id, label, value, set, |_| {}, cx)
}

/// 开关，改完设置之后再做点什么（比如让开着的大屏按新设置重开）。
fn toggle_then(
    state: &Entity<AppState>,
    id: &'static str,
    label: String,
    value: bool,
    set: fn(&mut BigScreenSettings, bool),
    then: fn(&mut App),
    cx: &App,
) -> AnyElement {
    let state = state.clone();
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .text_color(cx.theme().foreground)
        .child(div().text_sm().child(label.clone()))
        .child(
            Switch::new(id)
                .accessibility_label(label)
                .checked(value)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    edit(&state, |c| set(c, checked), cx);
                    then(cx);
                }),
        )
        .into_any_element()
}

fn choice<T: Copy + PartialEq + 'static>(
    state: &Entity<AppState>,
    id: &'static str,
    label: String,
    options: Vec<(T, String)>,
    value: T,
    set: fn(&mut BigScreenSettings, T),
) -> AnyElement {
    let state = state.clone();
    h_flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(div().text_sm().child(label))
        .child(ui::segmented(id, options, value, move |value, cx| {
            edit(&state, |c| set(c, value), cx)
        }))
        .into_any_element()
}

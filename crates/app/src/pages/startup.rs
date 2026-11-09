//! 启动项页：开机 / 登录时会自己启动的东西，可以逐项关掉、再打开。
//!
//! 读和改都在核心库的 `startup` 里（只做可以反悔的开关，不删任何东西）；这里管后台线程和界面。
//! 要记住读到的列表、有没有操作在进行，所以是独立的 `Entity`。

use crate::health::Level;
use crate::state::{AppState, spawn_thread};
use crate::ui;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::NotificationType;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, Context, Entity, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use pc_specs_core::startup::{self, Entry, Listing, Source, Trigger};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::time::{Duration, Instant};

/// 列表放了这么久，再回到这一页就重新读一遍。
const STALE: Duration = Duration::from_secs(60);

pub struct StartupPage {
    state: Entity<AppState>,
    /// 读到的启动项；第一次读完之前是 `None`。
    listing: Option<Listing>,
    loaded_at: Option<Instant>,
    loading: bool,
    /// 一次开关正在进行：这期间不接受第二个，也不重新读，免得结果和界面对不上。
    busy: bool,
    _subscription: Subscription,
}

impl StartupPage {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            listing: None,
            loaded_at: None,
            loading: false,
            busy: false,
            _subscription: subscription,
        }
    }

    /// 在后台把启动项读一遍。
    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || self.busy {
            return;
        }
        let (tx, rx) = async_channel::bounded(1);
        let started = spawn_thread("pc-specs-startup", move || {
            let listing = guarded("startup list", startup::list).unwrap_or_default();
            let _ = tx.send_blocking(listing);
        });
        // 线程起不来也记一笔时间：不然每次重画都会再试一次。
        self.loaded_at = Some(Instant::now());
        if !started {
            return;
        }
        self.loading = true;
        cx.spawn(async move |this, cx| {
            let listing = rx.recv().await.ok();
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.loaded_at = Some(Instant::now());
                if let Some(listing) = listing {
                    this.listing = Some(listing);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// 打开或关掉第 `index` 项；做完再读一遍，让开关和系统里的实际状态一致。
    fn toggle(&mut self, index: usize, enabled: bool, cx: &mut Context<Self>) {
        if self.busy || self.loading {
            return;
        }
        let Some(entry) = self
            .listing
            .as_ref()
            .and_then(|listing| listing.entries.get(index))
            .cloned()
        else {
            return;
        };
        let name = entry.name.clone();
        let (tx, rx) = async_channel::bounded(1);
        let started = spawn_thread("pc-specs-startup-toggle", move || {
            let result = guarded("startup toggle", || startup::set_enabled(&entry, enabled))
                .unwrap_or_else(|| Err("the change crashed".to_string()));
            let _ = tx.send_blocking(result);
        });
        if !started {
            let message = t!("sa_error_thread").into_owned();
            self.state
                .update(cx, |s, cx| s.toast(NotificationType::Error, message, cx));
            return;
        }
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = rx
                .recv()
                .await
                .unwrap_or_else(|_| Err("the change was interrupted".to_string()));
            let _ = this.update(cx, |this, cx| this.toggled(&name, enabled, result, cx));
        })
        .detach();
        cx.notify();
    }

    fn toggled(
        &mut self,
        name: &str,
        enabled: bool,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        let (kind, message) = match result {
            Ok(()) if enabled => (
                NotificationType::Success,
                t!("startup_turned_on", name = name).into_owned(),
            ),
            Ok(()) => (
                NotificationType::Success,
                t!("startup_turned_off", name = name).into_owned(),
            ),
            Err(error) => (
                NotificationType::Error,
                t!("startup_failed", name = name, error = error).into_owned(),
            ),
        };
        self.state.update(cx, |s, cx| s.toast(kind, message, cx));
        self.refresh(cx);
    }

    fn content(&self, cx: &Context<Self>) -> AnyElement {
        let entity = cx.entity();
        let theme = cx.theme();
        let (border, muted, warning, radius) = (
            theme.border,
            theme.muted_foreground,
            theme.warning,
            theme.radius,
        );
        let locked = self.busy || self.loading;

        let refresh = {
            let page = entity.clone();
            Button::new("startup-refresh")
                .small()
                .outline()
                .icon(IconName::RefreshCw)
                .label(t!("startup_refresh").into_owned())
                .disabled(locked)
                .on_click(move |_, _, cx| page.update(cx, |page, cx| page.refresh(cx)))
                .into_any_element()
        };

        let Some(listing) = &self.listing else {
            return ui::page(
                "page-startup",
                t!("nav_startup").into_owned(),
                None,
                vec![refresh],
                ui::skeleton_body(),
                cx,
            );
        };

        let total = listing.entries.len();
        let off = listing
            .entries
            .iter()
            .filter(|entry| !entry.enabled)
            .count();
        let subtitle = (total > 0).then(|| {
            SharedString::from(if off == 0 {
                t!("startup_summary", n = total).into_owned()
            } else {
                t!("startup_summary_off", n = total, off = off).into_owned()
            })
        });

        let mut body: Vec<AnyElement> = Vec::new();
        if listing.login_items_denied {
            body.push(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(radius)
                    .border_1()
                    .border_color(warning.opacity(0.6))
                    .child(Icon::new(IconName::TriangleAlert).text_color(warning))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .child(t!("startup_denied_title").into_owned()),
                            )
                            .child(
                                ui::muted(t!("startup_denied_desc").into_owned(), cx).text_xs(),
                            ),
                    )
                    .child(
                        Button::new("startup-automation")
                            .small()
                            .outline()
                            .label(t!("startup_denied_open").into_owned())
                            .on_click(|_, _, cx| {
                                cx.open_url(
                                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Automation",
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
        body.push(ui::muted(t!("startup_intro").into_owned(), cx).into_any_element());

        if listing.entries.is_empty() {
            body.push(ui::empty_state(
                IconName::Power,
                t!("startup_empty").into_owned(),
                "",
            ));
        }

        // 条目已经按来源排好，一组组切开。
        let mut start = 0;
        while start < listing.entries.len() {
            let source = listing.entries[start].source;
            let len = listing.entries[start..]
                .iter()
                .take_while(|entry| entry.source == source)
                .count();
            let rows: Vec<AnyElement> = (start..start + len)
                .map(|index| {
                    let last = index + 1 == start + len;
                    row(
                        index,
                        &listing.entries[index],
                        locked,
                        last,
                        &entity,
                        (border, muted),
                        cx,
                    )
                })
                .collect();
            body.push(
                ui::section(
                    SharedString::from(format!("startup-{source:?}")),
                    t!(group_title_key(source)).into_owned(),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(ui::muted(t!(group_desc_key(source)).into_owned(), cx).text_xs())
                        .children(rows),
                )
                .into_any_element(),
            );
            start += len;
        }

        ui::page(
            "page-startup",
            t!("nav_startup").into_owned(),
            subtitle,
            vec![refresh],
            body,
            cx,
        )
    }
}

impl Render for StartupPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 第一次打开、或者离开了一阵再回来：重新读一遍。
        if self.loaded_at.is_none_or(|at| at.elapsed() > STALE) {
            self.refresh(cx);
        }
        guarded("render startup", || self.content(cx)).unwrap_or_else(|| div().into_any_element())
    }
}

fn group_title_key(source: Source) -> &'static str {
    match source {
        Source::LoginItem => "startup_group_login_item",
        Source::LaunchAgent => "startup_group_launch_agent",
        Source::LaunchDaemon => "startup_group_launch_daemon",
        Source::Registry => "startup_group_registry",
        Source::StartupFolder => "startup_group_startup_folder",
        Source::Autostart => "startup_group_autostart",
    }
}

fn group_desc_key(source: Source) -> &'static str {
    match source {
        Source::LoginItem => "startup_desc_login_item",
        Source::LaunchAgent => "startup_desc_launch_agent",
        Source::LaunchDaemon => "startup_desc_launch_daemon",
        Source::Registry => "startup_desc_registry",
        Source::StartupFolder => "startup_desc_startup_folder",
        Source::Autostart => "startup_desc_autostart",
    }
}

/// 它什么时候跑。守护进程在开机时跑，其余的在登录时。
fn trigger_label(entry: &Entry) -> String {
    match (entry.trigger, entry.source) {
        (Trigger::Login, Source::LaunchDaemon) => t!("startup_trigger_boot"),
        (Trigger::Login, _) => t!("startup_trigger_login"),
        (Trigger::KeepAlive, _) => t!("startup_trigger_keep"),
        (Trigger::Schedule, _) => t!("startup_trigger_schedule"),
        (Trigger::OnDemand, _) => t!("startup_trigger_demand"),
    }
    .into_owned()
}

/// 一个启动项：开关、名字和标签、它跑的命令，以及在文件管理器里显示的入口。
fn row(
    index: usize,
    entry: &Entry,
    locked: bool,
    last: bool,
    page: &Entity<StartupPage>,
    (border, muted): (gpui_kit::Hsla, gpui_kit::Hsla),
    cx: &gpui_kit::App,
) -> AnyElement {
    let toggle = {
        let page = page.clone();
        Switch::new(SharedString::from(format!("startup-switch-{index}")))
            .accessibility_label(entry.name.clone())
            .checked(entry.enabled)
            .disabled(locked || !entry.editable)
            .on_click(move |checked, _, cx| {
                let checked = *checked;
                page.update(cx, |page, cx| page.toggle(index, checked, cx));
            })
    };

    let reveal = entry.path.clone().map(|path| {
        Button::new(SharedString::from(format!("startup-reveal-{index}")))
            .small()
            .ghost()
            .icon(IconName::FolderSearch)
            .tooltip(ui::reveal_label())
            .on_click(move |_, _, cx| cx.reveal_path(&path))
    });

    // 名字旁边的小字：技术标识（和名字不同才显示），以及各种状态标签。
    let title = h_flex()
        .flex_wrap()
        .items_center()
        .gap_x_2()
        .gap_y_1()
        .child(div().text_sm().font_semibold().child(entry.name.clone()))
        .when(!entry.id.is_empty() && entry.id != entry.name, |this| {
            this.child(ui::muted(entry.id.clone(), cx).text_xs())
        })
        .child(ui::chip(trigger_label(entry)))
        .when(entry.system && entry.editable, |this| {
            this.child(ui::chip(t!("startup_system").into_owned()))
        })
        .when(!entry.editable, |this| {
            this.child(ui::chip(t!("startup_readonly").into_owned()))
        })
        .when(entry.running, |this| {
            this.child(ui::level_tag(Level::Healthy, t!("startup_running").into_owned()).small())
        });

    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .py_2()
        .when(!last, |this| this.border_b_1().border_color(border))
        .child(div().flex_shrink_0().child(toggle))
        .child(v_flex().flex_1().min_w_0().gap_0p5().child(title).when(
            !entry.command.is_empty(),
            |this| {
                this.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(muted)
                        .child(entry.command.clone()),
                )
            },
        ))
        .children(reveal.map(|button| div().flex_shrink_0().min_w(px(32.)).child(button)))
        .into_any_element()
}

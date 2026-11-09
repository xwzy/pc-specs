//! 规则页：自定义「某个读数越线就做点什么」。
//!
//! 列表里每条规则可以展开编辑；推送设置和最近的触发记录在同一页。
//! 带输入框和展开状态，所以是独立的 `Entity`。

use crate::health::Level;
use crate::rules::{
    self, COOLDOWN_CHOICES, Compare, HOLD_CHOICES, MAX_RULES, Metric, Rule, WebhookFormat,
};
use crate::settings::Settings;
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use pc_specs_core::util::guarded;
use rust_i18n::t;
use std::rc::Rc;

pub struct RulesPage {
    state: Entity<AppState>,
    webhook: Entity<InputState>,
    /// 命令输入框：同一时间只有展开编辑的那条规则用得到它。
    command: Entity<InputState>,
    /// 正在展开编辑的规则。
    editing: Option<u32>,
    _subscriptions: Vec<Subscription>,
}

impl RulesPage {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = state.read(cx).settings.webhook_url.clone();
        let webhook = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://…")
                .default_value(url)
        });
        let command = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("rule_command_placeholder").into_owned())
        });
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&webhook, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    this.state
                        .update(cx, |s, cx| s.update_settings(|v| v.webhook_url = text, cx));
                }
            }),
            cx.subscribe(&command, |this, input, event: &InputEvent, cx| {
                if let (InputEvent::Change, Some(id)) = (event, this.editing) {
                    let text = input.read(cx).value().to_string();
                    edit_rule(&this.state, id, |rule| rule.command = text, cx);
                }
            }),
        ];
        Self {
            state,
            webhook,
            command,
            editing: None,
            _subscriptions: subscriptions,
        }
    }

    /// 设置在别处变了（恢复默认、导入……）、或者换了一条规则编辑时，输入框里的字要跟上。
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (url, command) = {
            let s = self.state.read(cx);
            let rule = self
                .editing
                .and_then(|id| s.settings.rules.iter().find(|rule| rule.id == id));
            (
                s.settings.webhook_url.clone(),
                rule.map(|rule| rule.command.clone()),
            )
        };
        // 正在编辑的规则被删掉了。
        if command.is_none() {
            self.editing = None;
        }
        let typed = self.webhook.read(cx).value();
        if *typed != *url {
            self.webhook
                .update(cx, |input, cx| input.set_value(url, window, cx));
        }
        if let Some(command) = command {
            let typed = self.command.read(cx).value();
            if *typed != *command {
                self.command
                    .update(cx, |input, cx| input.set_value(command, window, cx));
            }
        }
    }

    fn content(&self, cx: &Context<Self>) -> AnyElement {
        let s = self.state.read(cx);
        let settings = &s.settings;
        let at_limit = settings.rules.len() >= MAX_RULES;

        let actions = vec![
            {
                let state = self.state.clone();
                Button::new("rules-restore")
                    .small()
                    .outline()
                    .label(t!("rules_restore").into_owned())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.editing = None;
                        state.update(cx, |s, cx| {
                            s.update_settings(|v| v.rules = rules::defaults(), cx)
                        });
                    }))
                    .into_any_element()
            },
            {
                let state = self.state.clone();
                Button::new("rules-add")
                    .small()
                    .primary()
                    .icon(IconName::Plus)
                    .label(t!("rules_add").into_owned())
                    .disabled(at_limit)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let mut id = 0;
                        state.update(cx, |s, cx| {
                            s.update_settings(
                                |v| {
                                    id = rules::next_id(&v.rules);
                                    v.rules.push(Rule::new(id, Metric::CpuUsage));
                                },
                                cx,
                            )
                        });
                        this.editing = Some(id);
                        cx.notify();
                    }))
                    .into_any_element()
            },
        ];

        let body = vec![
            self.delivery(settings, cx),
            self.rule_list(settings, cx),
            self.events(settings, cx),
        ];
        ui::page(
            "page-rules",
            t!("nav_rules").into_owned(),
            Some(t!("rules_description").into_owned().into()),
            actions,
            body,
            cx,
        )
    }

    // ---- 推送 ---------------------------------------------------------------

    fn delivery(&self, settings: &Settings, cx: &Context<Self>) -> AnyElement {
        let master = {
            let state = self.state.clone();
            Switch::new("rules-master")
                .checked(settings.alerts)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    state.update(cx, |s, cx| s.update_settings(|v| v.alerts = checked, cx));
                })
                .into_any_element()
        };
        let formats = {
            let state = self.state.clone();
            ui::segmented(
                "rules-format",
                vec![
                    (
                        WebhookFormat::Generic,
                        t!("rules_format_generic").into_owned(),
                    ),
                    (
                        WebhookFormat::Feishu,
                        t!("rules_format_feishu").into_owned(),
                    ),
                    (
                        WebhookFormat::DingTalk,
                        t!("rules_format_dingtalk").into_owned(),
                    ),
                ],
                settings.webhook_format,
                move |format, cx| {
                    state.update(cx, |s, cx| {
                        s.update_settings(|v| v.webhook_format = format, cx)
                    })
                },
            )
        };
        let test = {
            let state = self.state.clone();
            Button::new("rules-webhook-test")
                .small()
                .outline()
                .icon(IconName::Send)
                .label(t!("rules_webhook_test").into_owned())
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.test_webhook(cx)))
        };

        ui::section("rules-delivery", t!("rules_delivery").into_owned())
            .child(
                v_flex()
                    .gap_4()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_6()
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_medium()
                                            .child(t!("rules_master").into_owned()),
                                    )
                                    .child(
                                        ui::muted(t!("rules_master_hint").into_owned(), cx)
                                            .text_xs(),
                                    ),
                            )
                            .child(master),
                    )
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .child(t!("rules_webhook").into_owned()),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .child(Input::new(&self.webhook).small()),
                                    )
                                    .child(test),
                            )
                            .child(h_flex().items_center().gap_2().child(formats))
                            .child(ui::muted(t!("rules_webhook_hint").into_owned(), cx).text_xs()),
                    ),
            )
            .into_any_element()
    }

    // ---- 规则列表 -------------------------------------------------------------

    fn rule_list(&self, settings: &Settings, cx: &Context<Self>) -> AnyElement {
        let list = &settings.rules;
        let body = if list.is_empty() {
            ui::empty_state(
                IconName::BellRing,
                t!("rules_empty_title").into_owned(),
                t!("rules_empty_hint").into_owned(),
            )
        } else {
            v_flex()
                .w_full()
                .children(
                    list.iter()
                        .enumerate()
                        .map(|(index, rule)| self.rule_row(rule, index, list.len(), settings, cx)),
                )
                .into_any_element()
        };
        ui::section("rules-list", t!("rules_list").into_owned())
            .child(body)
            .into_any_element()
    }

    fn rule_row(
        &self,
        rule: &Rule,
        index: usize,
        count: usize,
        settings: &Settings,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = rule.id;
        let editing = self.editing == Some(id);
        let active = settings.alerts && rule.enabled && self.state.read(cx).is_rule_active(id);
        let border = cx.theme().border;

        let toggle = {
            let state = self.state.clone();
            Switch::new(SharedString::from(format!("rule-enabled-{id}")))
                .accessibility_label(t!("rule_enable").into_owned())
                .checked(rule.enabled)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    edit_rule(&state, id, |rule| rule.enabled = checked, cx);
                })
        };
        let expand = Button::new(SharedString::from(format!("rule-edit-{id}")))
            .ghost()
            .small()
            .icon(if editing {
                IconName::ChevronUp
            } else {
                IconName::ChevronDown
            })
            .tooltip(if editing {
                t!("rule_close").into_owned()
            } else {
                t!("rule_edit").into_owned()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.editing = if this.editing == Some(id) {
                    None
                } else {
                    Some(id)
                };
                cx.notify();
            }));
        let delete = {
            let state = self.state.clone();
            Button::new(SharedString::from(format!("rule-delete-{id}")))
                .ghost()
                .small()
                .icon(IconName::Trash)
                .tooltip(t!("rule_delete").into_owned())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.editing == Some(id) {
                        this.editing = None;
                    }
                    state.update(cx, |s, cx| {
                        s.update_settings(|v| v.rules.retain(|rule| rule.id != id), cx)
                    });
                    cx.notify();
                }))
        };

        v_flex()
            .w_full()
            .gap_3()
            .py_3()
            .when(index + 1 < count, |row| {
                row.border_b_1().border_color(border)
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_3()
                    .child(toggle)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .when(!rule.enabled, |text| {
                                        text.text_color(cx.theme().muted_foreground)
                                    })
                                    .child(rule.summary(settings)),
                            )
                            .child(ui::muted(rule.actions_text(), cx).text_xs().truncate()),
                    )
                    .when(active, |row| {
                        row.child(ui::level_tag(
                            Level::Warning,
                            t!("rule_active").into_owned(),
                        ))
                    })
                    .child(expand)
                    .child(delete),
            )
            .when(editing, |row| row.child(self.editor(rule, settings, cx)))
            .into_any_element()
    }

    /// 展开后的编辑区：条件、持续多久、多久重复一次、触发后做什么。
    fn editor(&self, rule: &Rule, settings: &Settings, cx: &Context<Self>) -> AnyElement {
        let id = rule.id;
        let metric = rule.metric;

        let metric_pick = dropdown(
            format!("rule-metric-{id}"),
            metric.name(),
            Metric::ALL.iter().map(|m| (*m, m.name())).collect(),
            metric,
            {
                let state = self.state.clone();
                move |picked, cx| {
                    if picked != metric {
                        edit_rule(
                            &state,
                            id,
                            |rule| {
                                rule.metric = picked;
                                rule.compare = picked.default_compare();
                                rule.threshold = picked.default_threshold();
                            },
                            cx,
                        );
                    }
                }
            },
        );
        let mut when = h_flex()
            .items_center()
            .gap_2()
            .flex_wrap()
            .child(metric_pick);
        if !metric.is_event() {
            let state = self.state.clone();
            let compare = ui::segmented(
                SharedString::from(format!("rule-compare-{id}")),
                vec![
                    (Compare::Above, t!("rule_compare_above").into_owned()),
                    (Compare::Below, t!("rule_compare_below").into_owned()),
                ],
                rule.compare,
                move |compare, cx| edit_rule(&state, id, |rule| rule.compare = compare, cx),
            );
            let threshold = dropdown(
                format!("rule-threshold-{id}"),
                rules::format_value(metric, rule.threshold, settings),
                metric
                    .presets()
                    .iter()
                    .map(|&value| (value, rules::format_value(metric, value, settings)))
                    .collect(),
                rule.threshold,
                {
                    let state = self.state.clone();
                    move |value, cx| edit_rule(&state, id, |rule| rule.threshold = value, cx)
                },
            );
            when = when.child(compare).child(threshold);
        }

        let hold = dropdown(
            format!("rule-hold-{id}"),
            rules::duration_text(rule.for_secs),
            HOLD_CHOICES
                .iter()
                .map(|&secs| (secs, rules::duration_text(secs)))
                .collect(),
            rule.for_secs,
            {
                let state = self.state.clone();
                move |secs, cx| edit_rule(&state, id, |rule| rule.for_secs = secs, cx)
            },
        );
        let cooldown = dropdown(
            format!("rule-cooldown-{id}"),
            rules::duration_text(rule.cooldown_mins * 60),
            COOLDOWN_CHOICES
                .iter()
                .map(|&mins| (mins, rules::duration_text(mins * 60)))
                .collect(),
            rule.cooldown_mins,
            {
                let state = self.state.clone();
                move |mins, cx| edit_rule(&state, id, |rule| rule.cooldown_mins = mins, cx)
            },
        );

        let notify = {
            let state = self.state.clone();
            Switch::new(SharedString::from(format!("rule-notify-{id}")))
                .accessibility_label(t!("rule_action_notify").into_owned())
                .checked(rule.notify)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    edit_rule(&state, id, |rule| rule.notify = checked, cx);
                })
        };
        let webhook = {
            let state = self.state.clone();
            Switch::new(SharedString::from(format!("rule-webhook-{id}")))
                .accessibility_label(t!("rule_action_webhook").into_owned())
                .checked(rule.webhook)
                .on_click(move |checked, _, cx| {
                    let checked = *checked;
                    edit_rule(&state, id, |rule| rule.webhook = checked, cx);
                })
        };
        let try_it = {
            let state = self.state.clone();
            Button::new(SharedString::from(format!("rule-test-{id}")))
                .small()
                .outline()
                .icon(IconName::Play)
                .label(t!("rule_test").into_owned())
                .tooltip(t!("rule_test_hint").into_owned())
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.test_rule(id, cx)))
        };

        let hint = match metric {
            Metric::ProcessCpu => Some(t!("rule_hint_process_cpu")),
            Metric::MemoryPressure => Some(t!("rule_hint_memory_pressure")),
            Metric::FanSpeed => Some(t!("rule_hint_fan_speed")),
            Metric::OnBattery => Some(t!("rule_hint_on_battery")),
            Metric::NetLatency => Some(t!("rule_hint_net_latency")),
            Metric::Offline => Some(t!("rule_hint_offline")),
            _ => None,
        };
        let webhook_note = settings
            .webhook_url
            .trim()
            .is_empty()
            .then(|| ui::muted(t!("rule_webhook_unset").into_owned(), cx).text_xs());

        v_flex()
            .w_full()
            .gap_3()
            .pl(px(46.))
            .child(field(t!("rule_when").into_owned(), when, cx))
            .when_some(hint, |this, hint| {
                this.child(
                    div()
                        .pl(px(150. + 12.))
                        .child(ui::muted(hint.into_owned(), cx).text_xs()),
                )
            })
            .child(field(t!("rule_hold").into_owned(), hold, cx))
            .child(field(t!("rule_cooldown").into_owned(), cooldown, cx))
            .child(field(
                t!("rule_action_notify").into_owned(),
                h_flex().items_center().child(notify),
                cx,
            ))
            .child(field(
                t!("rule_action_webhook").into_owned(),
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(webhook)
                    .children(webhook_note),
                cx,
            ))
            .child(field(
                t!("rule_action_command").into_owned(),
                Input::new(&self.command).small(),
                cx,
            ))
            .child(
                div()
                    .pl(px(150. + 12.))
                    .child(ui::muted(t!("rule_command_hint").into_owned(), cx).text_xs()),
            )
            .child(div().pl(px(150. + 12.)).child(try_it))
            .into_any_element()
    }

    // ---- 最近触发 -------------------------------------------------------------

    fn events(&self, _settings: &Settings, cx: &Context<Self>) -> AnyElement {
        let s = self.state.read(cx);
        let body = if s.rule_events.is_empty() {
            ui::muted(t!("rules_events_empty").into_owned(), cx).into_any_element()
        } else {
            let state = self.state.clone();
            let clear = Button::new("rules-events-clear")
                .ghost()
                .small()
                .icon(IconName::Trash)
                .label(t!("rules_events_clear").into_owned())
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.clear_rule_events(cx)));
            v_flex()
                .w_full()
                .gap_2()
                .child(h_flex().w_full().justify_end().child(clear))
                .children(s.rule_events.iter().rev().map(|event| {
                    h_flex()
                        .w_full()
                        .items_baseline()
                        .gap_3()
                        .child(
                            div()
                                .w(px(140.))
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(fmt::datetime(event.timestamp_ms / 1000)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .child(event.message.clone()),
                        )
                }))
                .into_any_element()
        };
        ui::section("rules-events", t!("rules_events").into_owned())
            .child(body)
            .into_any_element()
    }
}

impl Render for RulesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_inputs(window, cx);
        guarded("render rules", || self.content(cx)).unwrap_or_else(|| div().into_any_element())
    }
}

/// 改某条规则；改完设置会自动保存。
fn edit_rule(state: &Entity<AppState>, id: u32, change: impl FnOnce(&mut Rule), cx: &mut App) {
    state.update(cx, |s, cx| {
        s.update_settings(
            |v| {
                if let Some(rule) = v.rules.iter_mut().find(|rule| rule.id == id) {
                    change(rule);
                }
            },
            cx,
        )
    });
}

/// 左边一个说明、右边一个控件的表单行。
fn field(label: String, control: impl IntoElement, cx: &App) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .gap_3()
        .child(
            div()
                .w(px(150.))
                .flex_shrink_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(control))
        .into_any_element()
}

/// 「当前值 ▾」按钮，点开列出全部选项，当前选中的带勾。
fn dropdown<T: Copy + PartialEq + 'static>(
    id: String,
    current: String,
    options: Vec<(T, String)>,
    selected: T,
    on_pick: impl Fn(T, &mut App) + 'static,
) -> AnyElement {
    let on_pick = Rc::new(on_pick);
    Button::new(SharedString::from(id))
        .small()
        .outline()
        .label(current)
        .dropdown_caret(true)
        .dropdown_menu(move |menu, _, _| {
            options.iter().fold(menu, |menu, (value, label)| {
                let (value, on_pick) = (*value, on_pick.clone());
                menu.item(
                    PopupMenuItem::new(label.clone())
                        .checked(value == selected)
                        .on_click(move |_, _, cx| on_pick(value, cx)),
                )
            })
        })
        .into_any_element()
}

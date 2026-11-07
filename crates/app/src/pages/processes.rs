//! 进程：可排序、可筛选的虚拟列表表格，数据来自 `AppState` 的进程采样线程。
//!
//! 这是唯一一个自带状态的页面（排序列、筛选词、表格滚动位置），所以是独立的 `Entity`。

use crate::settings::{NetUnit, SizeUnit};
use crate::state::AppState;
use crate::{fmt, ui};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::table::{Column, ColumnSort, DataTable, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, rems,
};
use pc_specs_core::model::ProcessInfo;
use rust_i18n::t;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Pid,
    Name,
    Cpu,
    Memory,
    Read,
    Write,
}

const COLUMNS: [(&str, SortKey); 6] = [
    ("pid", SortKey::Pid),
    ("name", SortKey::Name),
    ("cpu", SortKey::Cpu),
    ("memory", SortKey::Memory),
    ("read", SortKey::Read),
    ("write", SortKey::Write),
];

struct ProcessDelegate {
    /// 第一批采样是否已经到达（没到之前显示「加载中」而不是「没有匹配」）。
    loaded: bool,
    source: Vec<ProcessInfo>,
    rows: Vec<ProcessInfo>,
    filter: String,
    sort: (SortKey, ColumnSort),
    size_unit: SizeUnit,
    net_unit: NetUnit,
}

impl ProcessDelegate {
    fn new() -> Self {
        Self {
            loaded: false,
            source: Vec::new(),
            rows: Vec::new(),
            filter: String::new(),
            sort: (SortKey::Cpu, ColumnSort::Descending),
            size_unit: SizeUnit::default(),
            net_unit: NetUnit::default(),
        }
    }

    /// 按当前筛选词与排序重新生成可见行。
    fn rebuild(&mut self) {
        let needle = self.filter.trim().to_lowercase();
        self.rows = self
            .source
            .iter()
            .filter(|p| {
                needle.is_empty()
                    || p.name.to_lowercase().contains(&needle)
                    || p.pid.to_string().contains(&needle)
            })
            .cloned()
            .collect();

        let (key, order) = self.sort;
        self.rows.sort_by(|a, b| {
            let ord = match key {
                SortKey::Pid => a.pid.cmp(&b.pid),
                SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortKey::Cpu => a.cpu_percent.total_cmp(&b.cpu_percent),
                SortKey::Memory => a.mem_bytes.cmp(&b.mem_bytes),
                SortKey::Read => a.read_bps.cmp(&b.read_bps),
                SortKey::Write => a.write_bps.cmp(&b.write_bps),
            };
            if order == ColumnSort::Descending {
                ord.reverse()
            } else {
                ord
            }
        });
    }
}

impl TableDelegate for ProcessDelegate {
    fn columns_count(&self, _: &App) -> usize {
        COLUMNS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let (id, key) = COLUMNS[col_ix];
        let title = match key {
            SortKey::Pid => t!("proc_col_pid"),
            SortKey::Name => t!("proc_col_name"),
            SortKey::Cpu => t!("proc_col_cpu"),
            SortKey::Memory => t!("proc_col_mem"),
            SortKey::Read => t!("proc_col_read"),
            SortKey::Write => t!("proc_col_write"),
        }
        .into_owned();
        let column = Column::new(id, title)
            .sortable()
            .movable(false)
            .resizable(false);
        let column = match key {
            SortKey::Pid => column.width(72.),
            SortKey::Name => column.width(240.),
            SortKey::Cpu => column.width(84.).text_right(),
            SortKey::Memory => column.width(110.).text_right(),
            SortKey::Read | SortKey::Write => column.width(120.).text_right(),
        };
        // 当前排序列要带上方向，表头才会显示箭头。
        if self.sort.0 == key {
            column.sort(self.sort.1)
        } else {
            column
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(p) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        match COLUMNS[col_ix].1 {
            SortKey::Pid => div()
                .text_color(cx.theme().muted_foreground)
                .child(p.pid.to_string())
                .into_any_element(),
            SortKey::Name => div().truncate().child(p.name.clone()).into_any_element(),
            SortKey::Cpu => {
                let color = ui::usage_color(p.cpu_percent, cx);
                let color = if p.cpu_percent < 20.0 {
                    cx.theme().foreground
                } else {
                    color
                };
                div()
                    .text_color(color)
                    .child(format!("{:.1}%", p.cpu_percent))
                    .into_any_element()
            }
            SortKey::Memory => div()
                .child(fmt::bytes(p.mem_bytes, self.size_unit))
                .into_any_element(),
            SortKey::Read => rate_cell(p.read_bps, self.net_unit, cx),
            SortKey::Write => rate_cell(p.write_bps, self.net_unit, cx),
        }
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColumnSort,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) {
        self.sort = (COLUMNS[col_ix].1, sort);
        self.rebuild();
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child(if self.loaded || !self.filter.trim().is_empty() {
                t!("proc_empty").into_owned()
            } else {
                t!("common_loading").into_owned()
            })
    }
}

/// 读写速率为 0 时显示「—」，避免满屏 `0 B/s`。
fn rate_cell(bps: u64, unit: NetUnit, cx: &App) -> AnyElement {
    if bps == 0 {
        div()
            .text_color(cx.theme().muted_foreground)
            .child("—")
            .into_any_element()
    } else {
        div().child(fmt::rate(bps, unit)).into_any_element()
    }
}

pub struct ProcessesPage {
    app: Entity<AppState>,
    table: Entity<TableState<ProcessDelegate>>,
    search: Entity<InputState>,
    seen_rev: u64,
    _subscriptions: Vec<Subscription>,
}

impl ProcessesPage {
    pub fn new(app: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let table = cx.new(|cx| {
            TableState::new(ProcessDelegate::new(), window, cx)
                .col_movable(false)
                .col_selectable(false)
        });
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("proc_search").into_owned()));
        let subscriptions = vec![
            cx.observe(&app, |this, _, cx| this.sync(cx)),
            cx.subscribe(&search, |this, search, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = search.read(cx).value().to_string();
                    this.table.update(cx, |table, cx| {
                        table.delegate_mut().filter = text;
                        table.delegate_mut().rebuild();
                        cx.notify();
                    });
                }
            }),
        ];
        Self {
            app,
            table,
            search,
            seen_rev: u64::MAX,
            _subscriptions: subscriptions,
        }
    }

    /// 进程列表或单位变了就把新数据灌进表格；每个采样 tick 都会走到这里，所以先判断版本。
    fn sync(&mut self, cx: &mut Context<Self>) {
        let (rev, size_unit, net_unit) = {
            let s = self.app.read(cx);
            (s.process_rev, s.settings.size_unit, s.settings.net_unit)
        };
        let units_changed = {
            let d = self.table.read(cx).delegate();
            d.size_unit != size_unit || d.net_unit != net_unit
        };
        if rev == self.seen_rev && !units_changed {
            return;
        }
        self.seen_rev = rev;
        let list = self.app.read(cx).processes.clone();
        self.table.update(cx, |table, cx| {
            let d = table.delegate_mut();
            d.loaded = rev > 0;
            d.source = list;
            d.size_unit = size_unit;
            d.net_unit = net_unit;
            d.rebuild();
            cx.notify();
        });
    }
}

impl Render for ProcessesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (count, loaded) = {
            let d = self.table.read(cx).delegate();
            (d.rows.len(), d.loaded)
        };
        let search = Input::new(&self.search)
            .small()
            .w(rems(16.))
            .cleanable(true)
            .prefix(Icon::new(IconName::Search).small())
            .into_any_element();
        ui::page_fill(
            t!("proc_title").into_owned(),
            loaded.then(|| SharedString::from(t!("proc_count", n = count).into_owned())),
            vec![search],
            DataTable::new(&self.table)
                .bordered(false)
                .stripe(true)
                .small(),
            cx,
        )
    }
}

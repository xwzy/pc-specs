//! 各页面。页面都是 `AppState` 的只读视图：`render(&AppState, ..) -> AnyElement`，
//! 只有带自身状态的页面（进程表、存储的空间分析、规则编辑……）才是独立的 `Entity`。

mod apps;
mod battery;
pub mod chart;
mod cpu;
pub mod dashboard;
mod dev_env;
mod display;
mod export;
mod gpu;
mod memory;
mod monitor;
mod motherboard;
mod network;
mod os;
mod peripherals;
mod processes;
mod rules;
mod sensors;
mod settings;
mod startup;
mod storage;

use crate::state::{AppState, Page};
use crate::ui;
use crate::workspace::Workspace;
use gpui_kit::{AnyElement, AppContext as _, Context, Entity, IntoElement as _, Window, div};
use pc_specs_core::util::guarded;

/// 带有自身状态的页面实体。
pub struct Pages {
    processes: Entity<processes::ProcessesPage>,
    rules: Entity<rules::RulesPage>,
    startup: Entity<startup::StartupPage>,
    apps: Entity<apps::AppsPage>,
    storage: Entity<storage::StoragePage>,
    bigscreen: Entity<crate::bigscreen::preferences::BigScreenPage>,
}

impl Pages {
    pub fn new(state: &Entity<AppState>, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let state = state.clone();
        Self {
            processes: cx.new(|cx| processes::ProcessesPage::new(state.clone(), window, cx)),
            rules: cx.new(|cx| rules::RulesPage::new(state.clone(), window, cx)),
            startup: cx.new(|cx| startup::StartupPage::new(state.clone(), cx)),
            apps: cx.new(|cx| apps::AppsPage::new(state.clone(), window, cx)),
            storage: cx.new(|cx| storage::StoragePage::new(state.clone(), window, cx)),
            bigscreen: cx
                .new(|cx| crate::bigscreen::preferences::BigScreenPage::new(state, window, cx)),
        }
    }
}

/// `content_width` 是页面正文区的可用宽度（px，已扣除侧栏和左右内边距），
/// 用网格排版的页面据此决定列数。
pub fn render(
    page: Page,
    state: &Entity<AppState>,
    pages: &Pages,
    content_width: f32,
    cx: &mut Context<Workspace>,
) -> AnyElement {
    // 进程表的名称列要随窗口宽度伸缩；它是独立的实体，得在读取 `state` 之前先告诉它。
    if page == Page::Processes {
        pages.processes.update(cx, |processes, cx| {
            processes.set_content_width(content_width, cx)
        });
    }
    // 存储页的位置卡片按宽度排网格。
    if page == Page::Storage {
        pages
            .storage
            .update(cx, |storage, _| storage.set_content_width(content_width));
    }

    let s = state.read(cx);

    // 模块数据还没到：先给骨架，避免闪出空白页。
    if let Some(module) = page.module()
        && !s.is_loaded(module)
    {
        return ui::page(
            "page-loading",
            page.title(),
            None,
            vec![],
            ui::skeleton_body(),
            cx,
        );
    }

    // 绘制由系统的回调直接驱动，页面里任何一处 panic 都会让整个进程 abort。
    // 一页画不出来就让它这一帧空着：窗口、托盘和其它页面都不受影响，切走再切回来会重画。
    guarded("render page", || match page {
        Page::Dashboard => dashboard::render(s, state, content_width, cx),
        Page::Cpu => cpu::render(s, cx),
        Page::Gpu => gpu::render(s, cx),
        Page::Memory => memory::render(s, cx),
        Page::Storage => pages.storage.clone().into_any_element(),
        Page::Motherboard => motherboard::render(s, cx),
        Page::Display => display::render(s, cx),
        Page::Network => network::render(s, state, cx),
        Page::Battery => battery::render(s, cx),
        Page::Sensors => sensors::render(s, content_width, cx),
        Page::Peripherals => peripherals::render(s, cx),
        Page::Os => os::render(s, cx),
        Page::DevEnv => dev_env::render(s, state, cx),
        Page::Monitor => monitor::render(s, state, content_width, cx),
        Page::MenuBar => crate::menubar::preferences::render(state, cx),
        Page::BigScreen => pages.bigscreen.clone().into_any_element(),
        Page::Processes => pages.processes.clone().into_any_element(),
        Page::Rules => pages.rules.clone().into_any_element(),
        Page::Startup => pages.startup.clone().into_any_element(),
        Page::Apps => pages.apps.clone().into_any_element(),
        Page::Export => export::render(s, state, cx),
        Page::Settings => settings::render(state, cx),
    })
    .unwrap_or_else(|| div().into_any_element())
}

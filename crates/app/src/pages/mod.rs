//! 各页面。页面都是 `AppState` 的只读视图：`render(&AppState, ..) -> AnyElement`，
//! 只有带自身状态的页面（进程表）才是独立的 `Entity`。

mod battery;
pub mod chart;
mod cpu;
mod dashboard;
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
mod sensors;
mod settings;
mod storage;

use crate::state::{AppState, Page};
use crate::ui;
use crate::workspace::Workspace;
use gpui_kit::{AnyElement, AppContext as _, Context, Entity, IntoElement as _, Window, div};
use pc_specs_core::util::guarded;

/// 带有自身状态的页面实体。
pub struct Pages {
    processes: Entity<processes::ProcessesPage>,
}

impl Pages {
    pub fn new(state: &Entity<AppState>, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let state = state.clone();
        Self {
            processes: cx.new(|cx| processes::ProcessesPage::new(state, window, cx)),
        }
    }
}

pub fn render(
    page: Page,
    state: &Entity<AppState>,
    pages: &Pages,
    _window: &mut Window,
    cx: &mut Context<Workspace>,
) -> AnyElement {
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
        Page::Dashboard => dashboard::render(s, state, cx),
        Page::Cpu => cpu::render(s, cx),
        Page::Gpu => gpu::render(s, cx),
        Page::Memory => memory::render(s, cx),
        Page::Storage => storage::render(s, cx),
        Page::Motherboard => motherboard::render(s, cx),
        Page::Display => display::render(s, cx),
        Page::Network => network::render(s, state, cx),
        Page::Battery => battery::render(s, cx),
        Page::Sensors => sensors::render(s, cx),
        Page::Peripherals => peripherals::render(s, cx),
        Page::Os => os::render(s, cx),
        Page::DevEnv => dev_env::render(s, state, cx),
        Page::Monitor => monitor::render(s, state, cx),
        Page::MenuBar => crate::menubar::preferences::render(state, cx),
        Page::Processes => pages.processes.clone().into_any_element(),
        Page::Export => export::render(s, state, cx),
        Page::Settings => settings::render(state, cx),
    })
    .unwrap_or_else(|| div().into_any_element())
}

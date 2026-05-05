//! 桌面悬浮窗（暂时只有"网速悬浮窗"一种）。
//!
//! 设计：
//! - 标识符固定 `floating-net-speed`；前端按 hash `#/floating/net-speed` 路由，
//!   复用同一个 dist bundle。
//! - 创建参数：`always_on_top`、`decorations: false`、`skip_taskbar: true`、
//!   `resizable: false`，让悬浮窗像 macOS 的状态条小工具一样轻量、不打扰。
//!   不启用 `transparent` —— Tauri 2 默认 feature 下要求 `macos-private-api`，
//!   引入不便且对发布签名有额外要求。改用不透明深色矩形小条 + decorations:false。
//! - 默认尺寸 168x44；位置：屏幕主显示器右下角内 24px。用户可通过窗口内的
//!   `data-tauri-drag-region` 自由拖动；下次启动会从右下角重置（避免持久化
//!   多平台位置兼容性问题）。
//! - 关闭通过：① 设置开关关掉 → invoke `set_floating_net_speed(false)`；
//!   ② 双击悬浮窗 → 走 `close_floating_window` 命令；③ 退出整应用。
//! - `set_net_speed_window` 是幂等的：启用时若窗口已存在仅 show+focus；禁用时
//!   若窗口不存在直接 return Ok。前端可以放心反复调用。

use parking_lot::Mutex;
use std::sync::OnceLock;
use tauri::{LogicalPosition, LogicalSize, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

pub const NET_SPEED_LABEL: &str = "floating-net-speed";

const W: f64 = 168.0;
const H: f64 = 44.0;
const MARGIN: f64 = 24.0;

/// 序列化对悬浮窗的开 / 关操作。React StrictMode 下 effect 会双 mount，
/// 触发两次 `invoke('set_floating_net_speed', true)`，两次先后进入。如果不串行化，
/// 第二次会在第一次 `WebviewWindowBuilder::build` 完成前发现 window 不存在，
/// 也尝试 build 同 label 的 window → 第二次 build 失败（label 冲突）。
fn lock() -> &'static Mutex<()> {
    static L: OnceLock<Mutex<()>> = OnceLock::new();
    L.get_or_init(|| Mutex::new(()))
}

pub fn set_net_speed_window<R: Runtime>(
    app: &tauri::AppHandle<R>,
    enabled: bool,
) -> tauri::Result<()> {
    let _guard = lock().lock();
    if enabled {
        if let Some(w) = app.get_webview_window(NET_SPEED_LABEL) {
            // 已经存在，确保前置（用户可能误触发"开"按钮）。
            let _ = w.show();
            let _ = w.set_focus();
            return Ok(());
        }
        create_net_speed_window(app)?;
    } else if let Some(w) = app.get_webview_window(NET_SPEED_LABEL) {
        w.close()?;
    }
    Ok(())
}

fn create_net_speed_window<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    // 通过 hash 让前端路由到 FloatingNetSpeed 组件；不触发 BrowserRouter，
    // 由 main.tsx 入口判定渲染。
    let url = WebviewUrl::App("index.html#/floating/net-speed".into());

    let window = WebviewWindowBuilder::new(app, NET_SPEED_LABEL, url)
        .title("PC Specs · Net Speed")
        .inner_size(W, H)
        .min_inner_size(W, H)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .visible(true)
        .build()?;

    // 计算右下角位置。优先使用主显示器（primary_monitor），失败时退回到第一个
    // 可用 monitor，再失败就不挪动（停在系统默认位置）。
    let monitor = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| {
            window
                .available_monitors()
                .ok()
                .and_then(|v| v.into_iter().next())
        });
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let size = monitor.size().to_logical::<f64>(scale);
        let pos = monitor.position().to_logical::<f64>(scale);
        let x = pos.x + size.width - W - MARGIN;
        let y = pos.y + size.height - H - MARGIN;
        let _ = window.set_position(LogicalPosition::new(x, y));
    }

    // 重置一次 size，部分平台首次创建会把 inner_size 当成 outer_size。
    let _ = window.set_size(LogicalSize::new(W, H));
    Ok(())
}

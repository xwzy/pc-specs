mod commands;
mod exporter;
mod floating;
mod local_server;
mod model;
mod modules;
mod monitor;
mod platform;
mod state;
mod tray;

use tauri::Manager;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_shell::init())
        .manage(state::AppState::new())
        .on_window_event(|window, event| match event {
            // 主窗口"关闭"按钮拦截：改为隐藏到托盘，让 monitor / 本地 HTTP 服务继续后台运行。
            // 用户从托盘菜单选 Quit 才会真正退出。其他窗口（未来可能新增）走默认行为。
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            // 悬浮窗被关闭时通知前端（让 settings 同步回退到 disabled），
            // 避免用户从悬浮窗右键关掉但 setting 还显示"已开启"。
            tauri::WindowEvent::Destroyed => {
                if window.label() == floating::NET_SPEED_LABEL {
                    let app = window.app_handle().clone();
                    let _ = tauri::Emitter::emit(&app, "floating://net-speed-closed", ());
                }
            }
            _ => {}
        })
        .setup(|app| {
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "PC Specs starting"
            );
            let _ = app.get_webview_window("main");
            // 启动本地 HTTP 采集服务（0.0.0.0:16089）。bind 失败只会写一条 warn，
            // 不影响主程序使用 —— 比如同机已有另一个 pc-specs 实例占用端口。
            let app_handle = app.handle().clone();
            let shared = app.state::<state::AppState>().sys.clone();
            local_server::spawn(app_handle, shared);
            // 安装系统托盘（关窗后保持运行）。失败仅记日志，不阻断启动。
            if let Err(e) = tray::install(app.handle()) {
                tracing::warn!("install tray icon failed: {e}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_full_snapshot,
            commands::get_host,
            commands::get_os,
            commands::get_cpu,
            commands::get_gpus,
            commands::get_memory,
            commands::get_storages,
            commands::get_motherboard,
            commands::get_network,
            commands::get_displays,
            commands::get_sensors,
            commands::get_battery,
            commands::get_peripherals,
            commands::get_dev_env,
            commands::get_public_ip,
            commands::start_monitor,
            commands::stop_monitor,
            commands::export_markdown,
            commands::export_json,
            commands::save_export,
            commands::apply_tray_settings,
            commands::set_floating_net_speed,
            commands::close_floating_window,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .try_init();
}

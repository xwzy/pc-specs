mod commands;
mod error;
mod exporter;
mod model;
mod modules;
mod monitor;
mod platform;
mod state;

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
        .setup(|app| {
            tracing::info!(
                version = env!("CARGO_PKG_VERSION"),
                "PC Specs starting"
            );
            let _ = app.get_webview_window("main");
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

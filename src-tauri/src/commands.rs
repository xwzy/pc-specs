use crate::exporter;
use crate::model::*;
use crate::modules;
use crate::monitor;
use crate::state::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::sync::Notify;

#[tauri::command]
pub async fn get_full_snapshot(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SystemSnapshot, String> {
    let shared = state.sys.clone();
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || modules::collect_full_snapshot(&shared, &app_clone))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_host(_state: State<'_, AppState>) -> Result<HostInfo, String> {
    Ok(modules::host::collect())
}

#[tauri::command]
pub async fn get_os(_state: State<'_, AppState>) -> Result<OsInfo, String> {
    Ok(modules::os::collect())
}

#[tauri::command]
pub async fn get_cpu(state: State<'_, AppState>) -> Result<CpuInfo, String> {
    let shared = state.sys.clone();
    let mut cpu = modules::cpu::collect(&shared);
    let sensors = modules::sensors::collect(&shared);
    modules::cpu::enrich_with_sensors(&mut cpu, &sensors);
    Ok(cpu)
}

#[tauri::command]
pub async fn get_gpus(_state: State<'_, AppState>) -> Result<Vec<GpuInfo>, String> {
    Ok(modules::gpu::collect())
}

#[tauri::command]
pub async fn get_memory(state: State<'_, AppState>) -> Result<MemoryInfo, String> {
    let shared = state.sys.clone();
    Ok(modules::memory::collect(&shared))
}

#[tauri::command]
pub async fn get_storages(state: State<'_, AppState>) -> Result<Vec<StorageInfo>, String> {
    let shared = state.sys.clone();
    Ok(modules::storage::collect(&shared))
}

#[tauri::command]
pub async fn get_motherboard(
    _state: State<'_, AppState>,
) -> Result<Option<MotherboardInfo>, String> {
    Ok(crate::platform::motherboard())
}

#[tauri::command]
pub async fn get_network(state: State<'_, AppState>) -> Result<NetworkInfo, String> {
    let shared = state.sys.clone();
    Ok(modules::network::collect(&shared))
}

#[tauri::command]
pub async fn get_displays(
    app: AppHandle,
    _state: State<'_, AppState>,
) -> Result<Vec<DisplayInfo>, String> {
    Ok(modules::display::collect(&app))
}

#[tauri::command]
pub async fn get_sensors(state: State<'_, AppState>) -> Result<Vec<SensorReading>, String> {
    let shared = state.sys.clone();
    Ok(modules::sensors::collect(&shared))
}

#[tauri::command]
pub async fn get_battery(_state: State<'_, AppState>) -> Result<Option<BatteryInfo>, String> {
    Ok(modules::battery::collect())
}

#[tauri::command]
pub async fn get_peripherals(_state: State<'_, AppState>) -> Result<Vec<PeripheralInfo>, String> {
    Ok(modules::peripherals::collect())
}

#[tauri::command]
pub async fn get_dev_env(_state: State<'_, AppState>) -> Result<DevEnvInfo, String> {
    tokio::task::spawn_blocking(modules::dev_env::collect)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_public_ip(_state: State<'_, AppState>) -> Result<Option<String>, String> {
    let h = tokio::task::spawn_blocking(modules::network::fetch_public_ip)
        .await
        .map_err(|e| e.to_string())?;
    Ok(h)
}

#[tauri::command]
pub async fn start_monitor(
    app: AppHandle,
    state: State<'_, AppState>,
    interval_ms: u64,
) -> Result<(), String> {
    {
        let mut slot = state.monitor_stop.lock();
        let mut interval_slot = state.monitor_interval_ms.lock();
        let mut alive_slot = state.monitor_alive.lock();
        // 幂等：若已经在以相同 interval 运行（主窗口启动后悬浮窗也会调一次），且
        // task 还活着，跳过重启 —— 否则每次都 notify_waiters() 杀线程，丢失 elapsed
        // 基线，第一帧 BPS 显示为 0。
        let task_alive = alive_slot
            .as_ref()
            .map(|a| a.load(Ordering::SeqCst))
            .unwrap_or(false);
        if slot.is_some() && *interval_slot == Some(interval_ms) && task_alive {
            return Ok(());
        }
        if let Some(prev) = slot.take() {
            prev.notify_waiters();
        }
        let stop = Arc::new(Notify::new());
        let alive = Arc::new(AtomicBool::new(false));
        *slot = Some(stop.clone());
        *interval_slot = Some(interval_ms);
        *alive_slot = Some(alive.clone());
        let shared = state.sys.clone();
        let mon = state.monitor.clone();
        {
            let mut last = mon.last_sample_at.lock();
            *last = None;
            mon.diskstats_prev.lock().clear();
        }
        monitor::spawn_monitor(app, shared, mon, interval_ms, stop, alive);
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_monitor(state: State<'_, AppState>) -> Result<(), String> {
    let mut slot = state.monitor_stop.lock();
    let mut interval_slot = state.monitor_interval_ms.lock();
    let mut alive_slot = state.monitor_alive.lock();
    if let Some(prev) = slot.take() {
        prev.notify_waiters();
    }
    *interval_slot = None;
    *alive_slot = None;
    Ok(())
}

#[tauri::command]
pub async fn export_markdown(
    app: AppHandle,
    state: State<'_, AppState>,
    include_sensitive: Option<bool>,
) -> Result<String, String> {
    let shared = state.sys.clone();
    let app_clone = app.clone();
    let inc = include_sensitive.unwrap_or(false);
    tokio::task::spawn_blocking(move || {
        let snap = modules::collect_full_snapshot(&shared, &app_clone);
        exporter::to_markdown(&snap, inc)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_json(
    app: AppHandle,
    state: State<'_, AppState>,
    pretty: bool,
    include_sensitive: Option<bool>,
) -> Result<String, String> {
    let shared = state.sys.clone();
    let app_clone = app.clone();
    let inc = include_sensitive.unwrap_or(false);
    tokio::task::spawn_blocking(move || {
        let snap = modules::collect_full_snapshot(&shared, &app_clone);
        exporter::to_json(&snap, pretty, inc)
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn apply_tray_settings(
    app: AppHandle,
    settings: crate::tray::TraySettings,
) -> Result<(), String> {
    crate::tray::apply_settings(&app, settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_floating_net_speed(app: AppHandle, enabled: bool) -> Result<(), String> {
    crate::floating::set_net_speed_window(&app, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_floating_window(app: AppHandle, label: String) -> Result<(), String> {
    // 仅允许关闭已知的悬浮窗 label，避免前端错传把主窗口关掉。
    const ALLOWED: &[&str] = &[crate::floating::NET_SPEED_LABEL];
    if !ALLOWED.contains(&label.as_str()) {
        return Err(format!("label not allowed: {label}"));
    }
    if let Some(w) = tauri::Manager::get_webview_window(&app, &label) {
        w.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn save_export(path: String, content: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("empty path".into());
    }
    tokio::task::spawn_blocking(move || {
        if let Some(parent) = std::path::Path::new(&path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|e| e.to_string())?
}

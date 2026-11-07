//! PC Specs：跨平台的电脑配置查看与实时监控，界面基于 GPUI Kit。

// 发布版 Windows 程序不要弹出控制台窗口。
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod autostart;
mod bigscreen;
mod floating;
mod fmt;
mod health;
mod history;
mod instance;
mod menubar;
mod pages;
mod poster;
mod settings;
mod state;
mod tray;
mod ui;
mod workspace;

rust_i18n::i18n!("locales", fallback = "en");

use gpui_kit::{AppContext as _, QuitMode};
use settings::Settings;
use state::{AppState, Hub};
use tracing_subscriber::EnvFilter;

/// 崩溃日志的大小上限。超过就从头写：它只是线索，不是档案。
const CRASH_LOG_LIMIT: u64 = 256 * 1024;

/// 让每一次 panic 都留下记录。
///
/// 常驻托盘的程序没有终端可看，用户只会发现「它不见了」。被 `guarded` 挡下来的 panic
/// 和真正致命的 panic 都会先经过这里，所以崩溃日志里总有位置和原因可查。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log_panic(info);
        default_hook(info);
    }));
}

fn log_panic(info: &std::panic::PanicHookInfo<'_>) {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};
    // 每帧都 panic 的 bug 不该把磁盘写个不停：同一秒里只记第一条。
    static LAST_LOGGED: AtomicU64 = AtomicU64::new(0);

    // 钩子里再 panic 会直接 abort：这里只用不会 panic 的调用，任何错误都忽略。
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    if LAST_LOGGED.swap(seconds, Ordering::Relaxed) == seconds {
        return;
    }
    let Some(dir) = dirs::data_local_dir().map(|dir| dir.join("pc-specs")) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("crash.log");
    if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > CRASH_LOG_LIMIT) {
        let _ = std::fs::remove_file(&path);
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let thread = std::thread::current();
    let location = info
        .location()
        .map_or_else(String::new, |at| format!("{}:{}", at.file(), at.line()));
    let _ = writeln!(
        file,
        "[unix {seconds}] v{} thread '{}' panicked at {location}: {}",
        env!("CARGO_PKG_VERSION"),
        thread.name().unwrap_or("<unnamed>"),
        pc_specs_core::util::panic_message(info.payload()),
    );
}

fn main() {
    install_panic_hook();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();
    // 采集要同时起很多子进程和管道；打开文件数的上限（macOS 默认只有 256）耗尽时，
    // 各种系统调用会开始失败，所以趁早放宽。
    pc_specs_core::util::raise_fd_limit();

    // 开机自启启动的：有托盘就只留在托盘里。
    let background = std::env::args().any(|arg| arg == autostart::BACKGROUND_ARG);

    // 每个用户只留一个实例：已经有人在跑就把它叫醒（开机自启的那份不打扰用户）然后退出。
    let (wake_tx, wake_rx) = async_channel::unbounded::<()>();
    let Some(_instance) = instance::acquire(wake_tx, !background) else {
        return;
    };

    let settings = Settings::load();

    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);

    // macOS：点击 Dock 图标时，如果窗口已经关到托盘，重新打开它。
    app.on_reopen(workspace::show_main_window);

    app.run(move |cx| {
        {
            use gpui_kit::component as gpui_component;
            rust_i18n::extend!(gpui_component);
        }
        gpui_kit::init(cx);

        cx.set_app_identity("com.xwzy.pc-specs", "PC Specs");
        // 是否退出由我们自己决定：关闭窗口后可能要继续驻留在托盘。
        cx.set_quit_mode(QuitMode::Explicit);

        workspace::init(cx);
        bigscreen::init(cx);

        let state = cx.new(|cx| AppState::new(settings, cx));
        cx.set_global(Hub(state.clone()));

        cx.on_window_closed(|cx, _| workspace::on_window_closed(cx))
            .detach();

        // 后启动的实例来叫醒我们。
        cx.spawn(async move |cx| {
            while wake_rx.recv().await.is_ok() {
                cx.update(workspace::show_main_window);
            }
        })
        .detach();

        // 没有托盘就没法再把窗口叫出来，那种情况下即使是开机自启也要开窗口。
        if !(background && state.read(cx).has_tray()) {
            workspace::open_main_window(cx);
        }
        state.update(cx, |s, cx| s.sync_floating(cx));
        // 做成常驻看板的人会希望开机自启之后大屏直接出现在副屏上。
        bigscreen::open_on_launch(cx);
    });
}

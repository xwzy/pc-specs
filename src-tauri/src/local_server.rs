//! 本地局域网 HTTP 采集服务，监听 `0.0.0.0:16089`。
//!
//! 设计目标：
//! 1. 占用低 —— 没有请求时**完全没有**后台采样；轻量 metrics 端点带 800ms TTL 缓存，
//!    全量 snapshot 端点带 3s TTL 缓存 + single-flight 锁，多个分段端点共用同一份快照。
//! 2. 稳定 —— 复用已经经过验证的采集 module；axum + tower-http 提供 CORS / 超时 /
//!    body 大小限制等开箱即用的中间件。
//! 3. 易用 —— 全部 API 走 POST + JSON body（项目规范），返回字段统一 `snake_case`；
//!    根路径 `GET /` 给出可读文档，`GET /healthz` 给监控系统快速探活。
//!
//! 与前端 `MonitorSys` 的共享：HTTP 服务**不**复用 `MonitorSys` 的 `Networks` /
//! diskstats 基线，自己单独维护一套，避免两边互相 refresh 偷走累计字节。

use crate::model::{
    BatteryInfo, CpuInfo, DevEnvInfo, DisplayInfo, GpuInfo, HostInfo, MemoryInfo, MotherboardInfo,
    NetworkInfo, OsInfo, PeripheralInfo, SensorReading, StorageInfo, SystemSnapshot,
};
use crate::modules;
use crate::state::SharedSys;
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, Networks};
use tauri::AppHandle;
use tokio::sync::Mutex as AsyncMutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

/// 服务监听端口。固定 16089，便于其他机器约定访问。
pub const LOCAL_SERVER_PORT: u16 = 16089;

/// 全量 snapshot 缓存窗口。3s 内请求都返回同一份对象。
const SNAPSHOT_TTL: Duration = Duration::from_millis(3000);
/// 轻量 metrics 缓存窗口。800ms 内复用上次结果，避免 1Hz 以上轮询时反复 refresh。
const METRICS_TTL: Duration = Duration::from_millis(800);
/// 单个请求处理时间上限。snapshot 收集（含 sensors / dev_env）最坏情况 2~3s，
/// 给一点 buffer。超时返回 504。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// 限制请求体最大 64KiB —— 我们所有端点都只接受小型 JSON 配置。
const MAX_BODY_BYTES: usize = 64 * 1024;

pub fn spawn(app: AppHandle, shared: Arc<SharedSys>) {
    let state = Arc::new(ServerState::new(app, shared));
    // 使用 tauri::async_runtime::spawn 而非 tokio::spawn ——
    // Tauri 的 `setup()` 回调在主线程同步执行，调用点本身并未"进入" Tokio runtime，
    // 直接 tokio::spawn 会触发 "no reactor running" panic。
    // tauri::async_runtime 内部托管一个常驻的多线程 tokio runtime，
    // axum / tokio::net 在该 runtime 上正常工作。
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_server(state).await {
            tracing::warn!("local http server stopped: {e}");
        }
    });
}

async fn run_server(state: Arc<ServerState>) -> std::io::Result<()> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let router = Router::new()
        .route("/", get(root_index))
        .route("/healthz", get(get_health))
        .route("/api/v1/health", post(post_health).get(post_health))
        .route("/api/v1/info", post(post_info))
        .route("/api/v1/metrics", post(post_metrics))
        .route("/api/v1/snapshot", post(post_snapshot))
        .route("/api/v1/host", post(post_host))
        .route("/api/v1/os", post(post_os))
        .route("/api/v1/cpu", post(post_cpu))
        .route("/api/v1/gpus", post(post_gpus))
        .route("/api/v1/memory", post(post_memory))
        .route("/api/v1/storages", post(post_storages))
        .route("/api/v1/motherboard", post(post_motherboard))
        .route("/api/v1/network", post(post_network))
        .route("/api/v1/displays", post(post_displays))
        .route("/api/v1/sensors", post(post_sensors))
        .route("/api/v1/battery", post(post_battery))
        .route("/api/v1/peripherals", post(post_peripherals))
        .route("/api/v1/dev_env", post(post_dev_env))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        // 504 Gateway Timeout 比默认 408 更符合"上游采集慢"的语义。
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(cors)
        .with_state(state);

    let addr = format!("0.0.0.0:{}", LOCAL_SERVER_PORT);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(addr = %addr, "bind failed: {e}");
            return Err(e);
        }
    };
    tracing::info!(addr = %addr, "local http server listening");
    axum::serve(listener, router).await
}

// ---------- State ---------------------------------------------------------

pub struct ServerState {
    app: AppHandle,
    shared: Arc<SharedSys>,
    started_at: Instant,

    /// Snapshot 缓存（atomic refresh + single-flight）。`Arc<SystemSnapshot>` 让多次
    /// 请求 clone 时只增加引用计数，不复制数据。
    snapshot_cache: Mutex<Option<(Instant, Arc<SystemSnapshot>)>>,
    snapshot_refresh_lock: AsyncMutex<()>,

    /// 轻量 metrics 采样状态：自带 networks 实例 + diskstats 基线，不和前端 monitor 抢累计值。
    metrics: Arc<Mutex<MetricsState>>,
    metrics_refresh_lock: AsyncMutex<()>,
}

impl ServerState {
    fn new(app: AppHandle, shared: Arc<SharedSys>) -> Self {
        Self {
            app,
            shared,
            started_at: Instant::now(),
            snapshot_cache: Mutex::new(None),
            snapshot_refresh_lock: AsyncMutex::new(()),
            metrics: Arc::new(Mutex::new(MetricsState::new())),
            metrics_refresh_lock: AsyncMutex::new(()),
        }
    }
}

struct MetricsState {
    networks: Networks,
    last_sample_at: Option<Instant>,
    diskstats_prev: HashMap<String, (u64, u64)>,
    cached: Option<(Instant, LightMetrics)>,
}

impl MetricsState {
    fn new() -> Self {
        Self {
            networks: Networks::new_with_refreshed_list(),
            last_sample_at: None,
            diskstats_prev: HashMap::new(),
            cached: None,
        }
    }
}

// ---------- 轻量 metrics 数据结构 -----------------------------------------

#[derive(Clone, Serialize)]
pub struct LightMetrics {
    pub timestamp_ms: u64,
    pub cpu_overall: f32,
    pub cpu_per_core: Vec<f32>,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    /// 自上次采样以来的实际经过秒数；首次为 0（此时各速率均为 0，避免误算）。
    pub elapsed_secs: f64,
}

// ---------- 公共响应 ------------------------------------------------------

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
    name: &'static str,
    version: &'static str,
    uptime_secs: u64,
    timestamp_ms: u64,
}

#[derive(Serialize)]
struct ApiInfoResponse {
    hostname: String,
    username: String,
    os_family: String,
    os_name: String,
    os_version: String,
    arch: String,
    app_version: &'static str,
    api_version: u32,
    server_uptime_secs: u64,
    timestamp_ms: u64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SnapshotRequest {
    /// 是否在响应中包含敏感字段（hostname/username/MAC/序列号/公网 IP）。
    /// 不传或 false 时按 `exporter::redact` 的策略遮蔽。
    include_sensitive: Option<bool>,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    let body = ErrorBody {
        ok: false,
        error: msg.into(),
    };
    (code, Json(body)).into_response()
}

// ---------- 路由处理函数 --------------------------------------------------

async fn root_index() -> Response {
    let html = include_str!("local_server_index.html");
    let mut resp = (StatusCode::OK, html).into_response();
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    resp
}

async fn get_health(State(state): State<Arc<ServerState>>) -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        name: "pc-specs",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.started_at.elapsed().as_secs(),
        timestamp_ms: modules::now_ms(),
    })
}

async fn post_health(state: State<Arc<ServerState>>) -> Json<HealthResponse> {
    get_health(state).await
}

async fn post_info(State(state): State<Arc<ServerState>>) -> Response {
    // host / os 都是廉价采集，不需要走 snapshot cache。
    let result = tokio::task::spawn_blocking(|| {
        let h = modules::host::collect();
        let o = modules::os::collect();
        (h, o)
    })
    .await;
    match result {
        Ok((host, os)) => Json(ApiInfoResponse {
            hostname: host.hostname,
            username: host.username,
            os_family: os.family,
            os_name: os.name,
            os_version: os.version,
            arch: os.arch,
            app_version: env!("CARGO_PKG_VERSION"),
            api_version: 1,
            server_uptime_secs: state.started_at.elapsed().as_secs(),
            timestamp_ms: modules::now_ms(),
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn post_metrics(State(state): State<Arc<ServerState>>) -> Response {
    match metrics_throttled(&state).await {
        Ok(m) => Json(m).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn post_snapshot(
    State(state): State<Arc<ServerState>>,
    body: Option<Json<SnapshotRequest>>,
) -> Response {
    let req = body.map(|Json(r)| r).unwrap_or_default();
    match snapshot_cached(&state).await {
        Ok(snap) => {
            let include_sensitive = req.include_sensitive.unwrap_or(false);
            let payload = if include_sensitive {
                (*snap).clone()
            } else {
                redact(&snap)
            };
            Json(payload).into_response()
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// 分段端点：从同一份 snapshot cache 派生，多端点共享同一次采集成本。
macro_rules! section_endpoint {
    ($name:ident, $ret:ty, $extract:expr) => {
        async fn $name(State(state): State<Arc<ServerState>>) -> Response {
            match snapshot_cached(&state).await {
                Ok(snap) => {
                    let value: $ret = $extract(&*snap);
                    Json(value).into_response()
                }
                Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
            }
        }
    };
}

section_endpoint!(post_host, HostInfo, |s: &SystemSnapshot| s.host.clone());
section_endpoint!(post_os, OsInfo, |s: &SystemSnapshot| s.os.clone());
section_endpoint!(post_cpu, CpuInfo, |s: &SystemSnapshot| s.cpu.clone());
section_endpoint!(post_gpus, Vec<GpuInfo>, |s: &SystemSnapshot| s.gpus.clone());
section_endpoint!(post_memory, MemoryInfo, |s: &SystemSnapshot| s.memory.clone());
section_endpoint!(post_storages, Vec<StorageInfo>, |s: &SystemSnapshot| s
    .storages
    .clone());
section_endpoint!(
    post_motherboard,
    Option<MotherboardInfo>,
    |s: &SystemSnapshot| s.motherboard.clone()
);
section_endpoint!(post_network, NetworkInfo, |s: &SystemSnapshot| s
    .network
    .clone());
section_endpoint!(post_displays, Vec<DisplayInfo>, |s: &SystemSnapshot| s
    .displays
    .clone());
section_endpoint!(post_sensors, Vec<SensorReading>, |s: &SystemSnapshot| s
    .sensors
    .clone());
section_endpoint!(post_battery, Option<BatteryInfo>, |s: &SystemSnapshot| s
    .battery
    .clone());
section_endpoint!(
    post_peripherals,
    Vec<PeripheralInfo>,
    |s: &SystemSnapshot| s.peripherals.clone()
);
section_endpoint!(post_dev_env, DevEnvInfo, |s: &SystemSnapshot| s
    .dev_env
    .clone());

// ---------- snapshot 缓存 -------------------------------------------------

async fn snapshot_cached(state: &Arc<ServerState>) -> Result<Arc<SystemSnapshot>, String> {
    if let Some(s) = read_fresh_snapshot(state) {
        return Ok(s);
    }
    // 同一时间只允许一个请求触发 refresh，其余等待复用结果。
    let _g = state.snapshot_refresh_lock.lock().await;
    if let Some(s) = read_fresh_snapshot(state) {
        return Ok(s);
    }
    let shared = state.shared.clone();
    let app = state.app.clone();
    let snap = tokio::task::spawn_blocking(move || modules::collect_full_snapshot(&shared, &app))
        .await
        .map_err(|e| format!("snapshot collect join error: {e}"))?;
    let arc = Arc::new(snap);
    *state.snapshot_cache.lock() = Some((Instant::now(), arc.clone()));
    Ok(arc)
}

fn read_fresh_snapshot(state: &Arc<ServerState>) -> Option<Arc<SystemSnapshot>> {
    let cache = state.snapshot_cache.lock();
    let (t, s) = cache.as_ref()?;
    if t.elapsed() < SNAPSHOT_TTL {
        Some(s.clone())
    } else {
        None
    }
}

/// 最少必要遮蔽：复制 `exporter::redact` 的策略，但仅作用于通过 HTTP 暴露的副本。
fn redact(snap: &SystemSnapshot) -> SystemSnapshot {
    const REDACTED: &str = "[redacted]";
    let mut s = snap.clone();
    s.host.hostname = REDACTED.to_string();
    s.host.username = REDACTED.to_string();
    if let Some(mb) = s.motherboard.as_mut() {
        mb.serial = mb.serial.as_ref().map(|_| REDACTED.to_string());
    }
    for st in s.storages.iter_mut() {
        st.serial = st.serial.as_ref().map(|_| REDACTED.to_string());
    }
    for n in s.network.interfaces.iter_mut() {
        n.mac = n.mac.as_ref().map(|_| REDACTED.to_string());
    }
    s.network.public_ip = s.network.public_ip.as_ref().map(|_| REDACTED.to_string());
    s
}

// ---------- 轻量 metrics 采样 ---------------------------------------------

async fn metrics_throttled(state: &Arc<ServerState>) -> Result<LightMetrics, String> {
    if let Some(m) = read_fresh_metrics(&state.metrics) {
        return Ok(m);
    }
    let _g = state.metrics_refresh_lock.lock().await;
    if let Some(m) = read_fresh_metrics(&state.metrics) {
        return Ok(m);
    }
    let shared = state.shared.clone();
    let metrics = state.metrics.clone();
    let result = tokio::task::spawn_blocking(move || sample_light_metrics(&shared, &metrics))
        .await
        .map_err(|e| format!("metrics join error: {e}"))?;
    Ok(result)
}

fn read_fresh_metrics(metrics: &Arc<Mutex<MetricsState>>) -> Option<LightMetrics> {
    let m = metrics.lock();
    let (t, v) = m.cached.as_ref()?;
    if t.elapsed() < METRICS_TTL {
        Some(v.clone())
    } else {
        None
    }
}

fn sample_light_metrics(
    shared: &Arc<SharedSys>,
    metrics: &Arc<Mutex<MetricsState>>,
) -> LightMetrics {
    let now = Instant::now();
    let elapsed_secs = {
        let mut m = metrics.lock();
        let prev = m.last_sample_at;
        m.last_sample_at = Some(now);
        prev.map(|t| now.saturating_duration_since(t).as_secs_f64())
            .unwrap_or(0.0)
    };
    let has_prev = elapsed_secs > 0.05;

    let (cpu_overall, cpu_per_core, mem_used, mem_total) = {
        let mut sys = shared.system.lock();
        sys.refresh_cpu_specifics(CpuRefreshKind::everything().without_frequency());
        sys.refresh_memory_specifics(MemoryRefreshKind::everything());
        let cpus = sys.cpus();
        let per: Vec<f32> = cpus.iter().map(|c| c.cpu_usage()).collect();
        let g = sys.global_cpu_usage();
        let overall = if g.is_finite() && g > 0.0 {
            g
        } else if per.is_empty() {
            0.0
        } else {
            per.iter().sum::<f32>() / per.len() as f32
        };
        (overall, per, sys.used_memory(), sys.total_memory())
    };

    let (rx, tx) = {
        let mut m = metrics.lock();
        m.networks.refresh();
        let mut rx = 0u64;
        let mut tx = 0u64;
        for (_, d) in m.networks.iter() {
            rx = rx.saturating_add(d.received());
            tx = tx.saturating_add(d.transmitted());
        }
        (rx, tx)
    };
    let net_rx_bps = bps(rx, elapsed_secs, has_prev);
    let net_tx_bps = bps(tx, elapsed_secs, has_prev);

    let (disk_r, disk_w) = if has_prev {
        sample_disk_io(metrics, elapsed_secs)
    } else {
        prime_disk_io(metrics);
        (0, 0)
    };

    let result = LightMetrics {
        timestamp_ms: modules::now_ms(),
        cpu_overall,
        cpu_per_core,
        mem_used_bytes: mem_used,
        mem_total_bytes: mem_total,
        net_rx_bps,
        net_tx_bps,
        disk_read_bps: disk_r,
        disk_write_bps: disk_w,
        elapsed_secs: if has_prev { elapsed_secs } else { 0.0 },
    };

    {
        let mut m = metrics.lock();
        m.cached = Some((Instant::now(), result.clone()));
    }
    result
}

fn bps(value: u64, elapsed_secs: f64, has_prev: bool) -> u64 {
    if !has_prev || elapsed_secs <= 0.0 {
        return 0;
    }
    ((value as f64) / elapsed_secs).round() as u64
}

fn prime_disk_io(metrics: &Arc<Mutex<MetricsState>>) {
    let raw = crate::monitor::read_platform_diskstats();
    if raw.is_empty() {
        return;
    }
    let mut m = metrics.lock();
    for (dev, rb, wb) in raw {
        m.diskstats_prev.insert(dev, (rb, wb));
    }
}

fn sample_disk_io(metrics: &Arc<Mutex<MetricsState>>, elapsed_secs: f64) -> (u64, u64) {
    let raw = crate::monitor::read_platform_diskstats();
    if raw.is_empty() {
        return (0, 0);
    }
    let mut m = metrics.lock();
    let mut total_r = 0u64;
    let mut total_w = 0u64;
    for (dev, rb, wb) in &raw {
        let prior = m.diskstats_prev.get(dev).copied().unwrap_or((0, 0));
        total_r = total_r.saturating_add(rb.saturating_sub(prior.0));
        total_w = total_w.saturating_add(wb.saturating_sub(prior.1));
    }
    for (dev, rb, wb) in raw {
        m.diskstats_prev.insert(dev, (rb, wb));
    }
    let r = ((total_r as f64) / elapsed_secs).round() as u64;
    let w = ((total_w as f64) / elapsed_secs).round() as u64;
    (r, w)
}

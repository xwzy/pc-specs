//! 本地 HTTP 采集服务，默认监听 `0.0.0.0:16089`。
//!
//! 设计目标：
//! 1. 占用低 —— 没有请求时**完全没有**后台采样；轻量 metrics 端点带 800ms TTL 缓存，
//!    全量 snapshot 端点带 3s TTL 缓存 + single-flight 锁，多个分段端点共用同一份快照。
//! 2. 稳定 —— 复用与界面相同的采集模块；axum + tower-http 提供 CORS / 超时 / body 大小限制。
//! 3. 易用 —— 全部 API 走 POST + JSON body，返回字段统一 `snake_case`；
//!    根路径 `GET /` 给出可读文档，`GET /healthz` 给监控系统快速探活。
//!
//! 服务自带一个 tokio runtime 和独立线程，不依赖调用方的异步环境；
//! 实时速率用自己的 `Sampler`，与界面的监控线程互不干扰。

use crate::model::{
    BatteryInfo, CpuInfo, DevEnvInfo, DiskTick, DisplayInfo, GpuInfo, GpuTick, HostInfo,
    InterfaceTick, MemoryInfo, MonitorTick, MotherboardInfo, NetworkInfo, OsInfo, PeripheralInfo,
    SensorReading, StorageInfo, SystemSnapshot,
};
use crate::monitor::Sampler;
use crate::util::{guarded, now_ms};
use crate::{exporter, modules};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

/// 默认端口。固定 16089，便于其他机器约定访问。
pub const DEFAULT_PORT: u16 = 16089;

/// 全量 snapshot 缓存窗口。3s 内请求都返回同一份对象。
const SNAPSHOT_TTL: Duration = Duration::from_millis(3000);
/// 轻量 metrics 缓存窗口。800ms 内复用上次结果，避免 1Hz 以上轮询时反复刷新。
const METRICS_TTL: Duration = Duration::from_millis(800);
/// 单个请求处理时间上限。snapshot 采集最坏情况 2~3s，留一些余量。超时返回 504。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// 请求体上限 64KiB —— 所有端点都只接受很小的 JSON。
const MAX_BODY_BYTES: usize = 64 * 1024;

/// 服务当前状态，供设置页展示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Starting,
    Listening(SocketAddr),
    /// 通常是端口被占用（例如已有另一个实例在运行）。
    Failed(String),
}

/// 服务句柄。丢弃它会优雅关闭服务并等待端口释放（最多等到在途请求结束），
/// 需要在界面线程上关闭时请把它丢到后台线程里。
pub struct ServerHandle {
    status: Arc<Mutex<ServerStatus>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ServerHandle {
    pub fn status(&self) -> ServerStatus {
        self.status.lock().clone()
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// 在独立线程上启动服务。绑定失败不会 panic，状态里会给出原因。
pub fn spawn(addr: SocketAddr) -> ServerHandle {
    let status = Arc::new(Mutex::new(ServerStatus::Starting));
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let thread_status = status.clone();
    let thread = std::thread::Builder::new()
        .name("pc-specs-http".to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("pc-specs-http-worker")
                .enable_all()
                .build();
            let runtime = match runtime {
                Ok(r) => r,
                Err(e) => {
                    *thread_status.lock() = ServerStatus::Failed(e.to_string());
                    return;
                }
            };
            if let Err(e) = runtime.block_on(serve(addr, &thread_status, rx)) {
                tracing::warn!(%addr, "local http server stopped: {e}");
                *thread_status.lock() = ServerStatus::Failed(e.to_string());
            }
            // 正在跑采集的 blocking 线程最多再等 1 秒，不拖住关闭流程。
            runtime.shutdown_timeout(Duration::from_secs(1));
        });

    match thread {
        Ok(t) => ServerHandle {
            status,
            shutdown: Some(tx),
            thread: Some(t),
        },
        Err(e) => {
            *status.lock() = ServerStatus::Failed(e.to_string());
            ServerHandle {
                status,
                shutdown: None,
                thread: None,
            }
        }
    }
}

async fn serve(
    addr: SocketAddr,
    status: &Mutex<ServerStatus>,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<()> {
    let state = Arc::new(ServerState::new());
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
        // 504 比默认的 408 更符合"上游采集慢"的语义。
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    *status.lock() = ServerStatus::Listening(listener.local_addr()?);
    tracing::info!(%addr, "local http server listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = shutdown.await;
        })
        .await
}

// ---------- State ---------------------------------------------------------

struct ServerState {
    started_at: Instant,
    /// Snapshot 缓存（原子刷新 + single-flight）。`Arc` 让多次请求只增加引用计数。
    snapshot_cache: Mutex<Option<(Instant, Arc<SystemSnapshot>)>>,
    snapshot_refresh_lock: AsyncMutex<()>,
    /// 首个 metrics 请求到来时才创建，没有请求就不占任何资源。
    sampler: OnceLock<Option<SamplerWorker>>,
    metrics_cache: Mutex<Option<(Instant, Arc<LightMetrics>)>>,
    metrics_refresh_lock: AsyncMutex<()>,
}

impl ServerState {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            snapshot_cache: Mutex::new(None),
            snapshot_refresh_lock: AsyncMutex::new(()),
            sampler: OnceLock::new(),
            metrics_cache: Mutex::new(None),
            metrics_refresh_lock: AsyncMutex::new(()),
        }
    }
}

/// 采样器只在自己的线程里使用：Windows 上 sysinfo 的 GPU 性能计数器句柄不是 `Send`，
/// 不能放进多线程共享的状态。请求通过 channel 送过去，结果通过 oneshot 送回来。
struct SamplerWorker {
    requests: std::sync::mpsc::Sender<tokio::sync::oneshot::Sender<MonitorTick>>,
}

impl SamplerWorker {
    fn start() -> Option<Self> {
        let (requests, inbox) =
            std::sync::mpsc::channel::<tokio::sync::oneshot::Sender<MonitorTick>>();
        std::thread::Builder::new()
            .name("pc-specs-sampler".to_string())
            .spawn(move || {
                let mut sampler: Option<Sampler> = None;
                // 服务关闭、`requests` 被丢弃后 recv 返回 Err，线程随之退出。
                while let Ok(reply) = inbox.recv() {
                    if sampler.is_none() {
                        sampler = guarded("http sampler init", Sampler::new);
                    }
                    let tick = sampler
                        .as_mut()
                        .and_then(|s| guarded("http sampler", || s.sample_now()));
                    match tick {
                        Some(tick) => {
                            let _ = reply.send(tick);
                        }
                        // 采样器 panic 过，内部状态就不可信了：丢掉它，这次请求得不到结果
                        // （`reply` 随之丢弃，调用方按「暂不可用」处理），下次请求再重建。
                        None => sampler = None,
                    }
                }
            })
            .ok()?;
        Some(Self { requests })
    }

    async fn sample(&self) -> Option<MonitorTick> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.requests.send(tx).ok()?;
        rx.await.ok()
    }
}

// ---------- 数据结构 ------------------------------------------------------

/// 轻量 metrics。前十个字段与 v1 初版保持一致；其后是新增的可选信息，旧客户端忽略即可。
#[derive(Clone, Serialize)]
struct LightMetrics {
    timestamp_ms: u64,
    cpu_overall: f32,
    cpu_per_core: Vec<f32>,
    mem_used_bytes: u64,
    mem_total_bytes: u64,
    net_rx_bps: u64,
    net_tx_bps: u64,
    disk_read_bps: u64,
    disk_write_bps: u64,
    /// 相对上一次采样的真实秒数。
    elapsed_secs: f64,
    swap_used_bytes: u64,
    swap_total_bytes: u64,
    cpu_temperature_c: Option<f32>,
    max_temperature_c: Option<f32>,
    gpus: Vec<GpuTick>,
    per_interface: Vec<InterfaceTick>,
    per_disk: Vec<DiskTick>,
}

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
    /// 是否在响应中包含敏感字段（hostname / username / MAC / 序列号 / 公网 IP）。
    /// 不传或 false 时遮蔽。
    include_sensitive: Option<bool>,
}

#[derive(Serialize)]
struct ErrorBody {
    ok: bool,
    error: String,
}

fn err(code: StatusCode, msg: impl Into<String>) -> Response {
    (
        code,
        Json(ErrorBody {
            ok: false,
            error: msg.into(),
        }),
    )
        .into_response()
}

// ---------- 路由处理函数 --------------------------------------------------

async fn root_index() -> Response {
    let mut resp = (StatusCode::OK, include_str!("server_index.html")).into_response();
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
        timestamp_ms: now_ms(),
    })
}

async fn post_health(state: State<Arc<ServerState>>) -> Json<HealthResponse> {
    get_health(state).await
}

async fn post_info(State(state): State<Arc<ServerState>>) -> Response {
    // host / os 都是廉价采集，不需要走 snapshot 缓存。
    let result =
        tokio::task::spawn_blocking(|| (modules::host::collect(), modules::os::collect())).await;
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
            timestamp_ms: now_ms(),
        })
        .into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn post_metrics(State(state): State<Arc<ServerState>>) -> Response {
    match metrics_throttled(&state).await {
        Ok(m) => Json(&*m).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

async fn post_snapshot(
    State(state): State<Arc<ServerState>>,
    body: Option<Json<SnapshotRequest>>,
) -> Response {
    let include_sensitive = body
        .and_then(|Json(r)| r.include_sensitive)
        .unwrap_or(false);
    match snapshot_cached(&state).await {
        Ok(snap) => Json(exporter::redacted(&snap, include_sensitive)).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, e),
    }
}

// 分段端点：从同一份 snapshot 缓存派生，多个端点共享同一次采集成本。
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
section_endpoint!(post_memory, MemoryInfo, |s: &SystemSnapshot| s
    .memory
    .clone());
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
    // 同一时间只允许一个请求触发采集，其余等待并复用结果。
    let _guard = state.snapshot_refresh_lock.lock().await;
    if let Some(s) = read_fresh_snapshot(state) {
        return Ok(s);
    }
    let snap = tokio::task::spawn_blocking(modules::collect_snapshot)
        .await
        .map_err(|e| format!("snapshot collect join error: {e}"))?;
    let snap = Arc::new(snap);
    *state.snapshot_cache.lock() = Some((Instant::now(), snap.clone()));
    Ok(snap)
}

fn read_fresh_snapshot(state: &ServerState) -> Option<Arc<SystemSnapshot>> {
    let cache = state.snapshot_cache.lock();
    let (t, s) = cache.as_ref()?;
    (t.elapsed() < SNAPSHOT_TTL).then(|| s.clone())
}

// ---------- 轻量 metrics --------------------------------------------------

async fn metrics_throttled(state: &Arc<ServerState>) -> Result<Arc<LightMetrics>, String> {
    if let Some(m) = read_fresh_metrics(state) {
        return Ok(m);
    }
    let _guard = state.metrics_refresh_lock.lock().await;
    if let Some(m) = read_fresh_metrics(state) {
        return Ok(m);
    }
    let worker = state
        .sampler
        .get_or_init(SamplerWorker::start)
        .as_ref()
        .ok_or("sampler thread unavailable")?;
    let tick = worker.sample().await.ok_or("sampler thread stopped")?;
    let metrics = Arc::new(LightMetrics {
        timestamp_ms: tick.timestamp,
        cpu_temperature_c: tick.cpu_temperature(),
        max_temperature_c: tick.max_temperature(),
        cpu_overall: tick.cpu_overall,
        cpu_per_core: tick.cpu_per_core,
        mem_used_bytes: tick.mem_used_bytes,
        mem_total_bytes: tick.mem_total_bytes,
        net_rx_bps: tick.net_rx_bps,
        net_tx_bps: tick.net_tx_bps,
        disk_read_bps: tick.disk_read_bps,
        disk_write_bps: tick.disk_write_bps,
        elapsed_secs: tick.elapsed_secs,
        swap_used_bytes: tick.swap_used_bytes,
        swap_total_bytes: tick.swap_total_bytes,
        gpus: tick.gpus,
        per_interface: tick.per_interface,
        per_disk: tick.per_disk,
    });
    *state.metrics_cache.lock() = Some((Instant::now(), metrics.clone()));
    Ok(metrics)
}

fn read_fresh_metrics(state: &ServerState) -> Option<Arc<LightMetrics>> {
    let cache = state.metrics_cache.lock();
    let (t, m) = cache.as_ref()?;
    (t.elapsed() < METRICS_TTL).then(|| m.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn concurrent_metrics_requests_share_a_valid_first_sample() {
        let state = Arc::new(ServerState::new());
        // 两个冷启动请求应只触发一次采集，且首帧已有有效的统计窗口。
        let (first, second) = tokio::join!(metrics_throttled(&state), metrics_throttled(&state));
        let first = first.expect("first metrics request");
        let second = second.expect("concurrent metrics request");
        assert!(first.elapsed_secs >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.as_secs_f64());
        assert!(Arc::ptr_eq(&first, &second));

        let cached = metrics_throttled(&state).await.expect("cached metrics");
        assert!(Arc::ptr_eq(&first, &cached));
    }
}

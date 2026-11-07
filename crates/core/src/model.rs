use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub timestamp: u64,
    pub host: HostInfo,
    pub os: OsInfo,
    pub cpu: CpuInfo,
    pub gpus: Vec<GpuInfo>,
    pub memory: MemoryInfo,
    pub storages: Vec<StorageInfo>,
    pub motherboard: Option<MotherboardInfo>,
    pub network: NetworkInfo,
    pub displays: Vec<DisplayInfo>,
    pub sensors: Vec<SensorReading>,
    pub battery: Option<BatteryInfo>,
    pub peripherals: Vec<PeripheralInfo>,
    pub dev_env: DevEnvInfo,
}

impl SystemSnapshot {
    /// 把一次实时采样里的"速率类"数据合入快照：磁盘 / 网卡 / GPU 的实时读数只有采样器能给。
    pub fn merge_tick(&mut self, tick: &MonitorTick) {
        for st in &mut self.storages {
            let hit = tick
                .per_disk
                .iter()
                .find(|d| st.mount_point.as_deref() == Some(d.mount_point.as_str()));
            if let Some(d) = hit {
                st.read_bytes_per_sec = d.read_bps;
                st.write_bytes_per_sec = d.write_bps;
            }
        }
        for n in &mut self.network.interfaces {
            if let Some(t) = tick.per_interface.iter().find(|t| t.name == n.name) {
                n.rx_bytes_per_sec = t.rx_bps;
                n.tx_bytes_per_sec = t.tx_bps;
            }
        }
        for g in &mut self.gpus {
            if let Some(t) = tick.gpus.iter().find(|t| t.name == g.name) {
                g.utilization = t.utilization.or(g.utilization);
                g.vram_used_bytes = t.vram_used_bytes.or(g.vram_used_bytes);
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    pub username: String,
    pub uptime_secs: u64,
    pub boot_time: u64,
    pub app_version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OsInfo {
    pub family: String,
    pub name: String,
    pub version: String,
    pub kernel: String,
    pub arch: String,
    pub locale: String,
    pub shell: Option<String>,
    pub desktop: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuInfo {
    pub vendor: String,
    pub brand: String,
    pub arch: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub base_frequency_hz: u64,
    pub max_frequency_hz: u64,
    pub current_frequency_hz: u64,
    pub cache_l1_bytes: Option<u64>,
    pub cache_l2_bytes: Option<u64>,
    pub cache_l3_bytes: Option<u64>,
    pub features: Vec<String>,
    pub virtualization: Option<bool>,
    pub usage_per_core: Vec<f32>,
    pub usage_overall: f32,
    pub temperature_c: Option<f32>,
    pub topology: Option<CpuTopology>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CpuTopology {
    pub sockets: u32,
    pub p_cores: Option<u32>,
    pub e_cores: Option<u32>,
    pub numa_nodes: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GpuInfo {
    pub index: u32,
    pub vendor: String,
    pub name: String,
    /// 该平台的原生图形 API：Metal / Direct3D / Vulkan。
    pub backend: String,
    pub driver: Option<String>,
    /// 独显为专用显存；Apple Silicon 等统一内存平台为 `None`。
    pub vram_total_bytes: Option<u64>,
    pub vram_used_bytes: Option<u64>,
    pub utilization: Option<f32>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
    pub pcie_link: Option<String>,
    pub is_discrete: bool,
    /// GPU 核心数（目前仅 Apple Silicon 可得）。
    #[serde(default)]
    pub cores: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub modules: Vec<MemoryModule>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryModule {
    pub slot: String,
    pub manufacturer: Option<String>,
    pub part_number: Option<String>,
    pub capacity_bytes: u64,
    pub speed_mt_s: Option<u32>,
    pub kind: Option<String>,
    pub form_factor: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageInfo {
    pub name: String,
    pub mount_point: Option<String>,
    pub filesystem: Option<String>,
    pub kind: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub read_bytes_per_sec: u64,
    pub write_bytes_per_sec: u64,
    pub temperature_c: Option<f32>,
    pub smart_health: Option<String>,
    pub serial: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MotherboardInfo {
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub version: Option<String>,
    pub serial: Option<String>,
    pub bios_vendor: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,
    pub chassis: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub interfaces: Vec<NetworkInterface>,
    pub public_ip: Option<String>,
    pub default_gateway: Option<String>,
    pub dns_servers: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub mac: Option<String>,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub is_up: bool,
    pub is_loopback: bool,
    pub kind: String,
    pub link_speed_mbps: Option<u64>,
    pub rx_bytes_per_sec: u64,
    pub tx_bytes_per_sec: u64,
    pub rx_total_bytes: u64,
    pub tx_total_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub name: String,
    pub width_px: u32,
    pub height_px: u32,
    pub refresh_hz: Option<u32>,
    pub scale_factor: Option<f32>,
    pub is_primary: bool,
    pub physical_width_mm: Option<u32>,
    pub physical_height_mm: Option<u32>,
    pub color_depth: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SensorReading {
    pub source: String,
    pub label: String,
    /// temperature / fan / voltage / current / power
    pub kind: String,
    pub value: f32,
    pub unit: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub state: String,
    pub percentage: f32,
    pub cycle_count: Option<u32>,
    pub design_capacity_mwh: Option<u64>,
    pub full_capacity_mwh: Option<u64>,
    pub current_capacity_mwh: Option<u64>,
    pub temperature_c: Option<f32>,
    pub time_to_empty_secs: Option<u64>,
    pub time_to_full_secs: Option<u64>,
    pub power_now_mw: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeripheralInfo {
    pub kind: String,
    pub name: String,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub bus: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DevEnvInfo {
    pub languages: Vec<RuntimeInfo>,
    pub package_managers: Vec<RuntimeInfo>,
    pub vcs: Vec<RuntimeInfo>,
    pub editors: Vec<RuntimeInfo>,
    pub containers: Vec<RuntimeInfo>,
    pub shells: Vec<RuntimeInfo>,
    pub env_keys: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeInfo {
    pub name: String,
    pub version: Option<String>,
    pub path: Option<String>,
}

/// 一次实时采样。所有速率类字段已按真实 elapsed 换算成"每秒"。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonitorTick {
    pub timestamp: u64,
    /// 距上一次采样的真实秒数；首次采样为 0（此时各速率均为 0）。
    pub elapsed_secs: f64,
    pub cpu_overall: f32,
    pub cpu_per_core: Vec<f32>,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub gpus: Vec<GpuTick>,
    pub temperatures: Vec<SensorReading>,
    /// 每接口的实时速率，按名字关联到 `NetworkInterface`。
    pub per_interface: Vec<InterfaceTick>,
    /// 每块物理盘的实时读写速率（已按物理设备去重）。
    pub per_disk: Vec<DiskTick>,
}

impl MonitorTick {
    /// CPU / SoC 相关的最高温度。
    pub fn cpu_temperature(&self) -> Option<f32> {
        crate::modules::cpu::cpu_temperature(&self.temperatures)
    }

    /// 所有温度传感器里的最高值。
    pub fn max_temperature(&self) -> Option<f32> {
        self.temperatures
            .iter()
            .map(|s| s.value)
            .filter(|v| v.is_finite())
            .fold(None, |m, v| Some(m.map_or(v, |m: f32| m.max(v))))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GpuTick {
    pub name: String,
    pub utilization: Option<f32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InterfaceTick {
    pub name: String,
    pub rx_bps: u64,
    pub tx_bps: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiskTick {
    /// 该物理设备上挂载点最短的那个卷，与 `StorageInfo.mount_point` 对应。
    pub mount_point: String,
    pub read_bps: u64,
    pub write_bps: u64,
}

/// 进程列表中的一行（任务管理器式的轻量视图）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub mem_bytes: u64,
    pub read_bps: u64,
    pub write_bps: u64,
}

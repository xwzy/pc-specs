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
        // 温度只在界面开着温度读数时才有；没有就保留快照里的。
        crate::modules::cpu::enrich_with_sensors(&mut self.cpu, &tick.temperatures);
        crate::modules::gpu::enrich_with_sensors(&mut self.gpus, &tick.temperatures);
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

/// 内存的构成（字节）。各平台能区分的项不一样，区分不出来的就是 0。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryBreakdown {
    /// 应用程序占用。
    pub app_bytes: u64,
    /// 系统内核和驱动占用，不能被换出。
    pub wired_bytes: u64,
    /// 被压缩存放的部分。
    pub compressed_bytes: u64,
    /// 文件缓存等可以随时回收的部分。
    pub cached_bytes: u64,
    pub free_bytes: u64,
    /// 内存压力 0–100，越高越紧张；系统不提供时为 `None`。
    pub pressure_percent: Option<f32>,
    /// 内存压力等级：0 正常、1 警告、2 严重。
    pub pressure_level: Option<u8>,
    /// 自开机累计换入 / 换出交换空间的字节数，采样器据此算出下面的速率。
    pub swapped_in_bytes: u64,
    pub swapped_out_bytes: u64,
    pub swap_in_bps: u64,
    pub swap_out_bps: u64,
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
    /// 电池健康度（%）：macOS 取系统自己报的「最大容量」（和「系统设置」里一致），
    /// 其他平台是满电容量 / 设计容量。
    #[serde(default)]
    pub health_percent: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PeripheralInfo {
    pub kind: String,
    pub name: String,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub bus: Option<String>,
    /// 设备自己的电量（百分比）。目前只有已连接的蓝牙设备（键盘、鼠标、耳机……）能读到。
    #[serde(default)]
    pub battery_percent: Option<u8>,
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
    /// 内存构成、压力和换页速率；平台读不到时为 `None`。
    #[serde(default)]
    pub memory: Option<MemoryBreakdown>,
    /// 1 / 5 / 15 分钟平均负载；Windows 没有这个概念。
    #[serde(default)]
    pub load_average: Option<[f32; 3]>,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub gpus: Vec<GpuTick>,
    pub temperatures: Vec<SensorReading>,
    /// 随温度一起刷新的其余传感器：风扇转速、功率、电压、电流。
    #[serde(default)]
    pub other_sensors: Vec<SensorReading>,
    /// 系统的热压力等级：0 正常、1 中等、2 较重、3 严重、4 休眠。数字大了说明系统已经在压低性能来降温。
    /// 和温度一起刷新；只有 macOS 提供。
    #[serde(default)]
    pub thermal_pressure: Option<u8>,
    /// 每接口的实时速率，按名字关联到 `NetworkInterface`。
    pub per_interface: Vec<InterfaceTick>,
    /// 每块物理盘的实时读写速率（已按物理设备去重）。
    pub per_disk: Vec<DiskTick>,
}

impl MonitorTick {
    /// CPU 温度。Apple 芯片取逐核传感器的平均值（和 iStat Menus / Stats 一致），
    /// 其他平台取封装温度。口径见 [`crate::modules::cpu::cpu_temperature`]。
    pub fn cpu_temperature(&self) -> Option<f32> {
        crate::modules::cpu::cpu_temperature(&self.temperatures)
    }

    /// GPU 温度（各传感器平均）；读不到时为 `None`。
    pub fn gpu_temperature(&self) -> Option<f32> {
        crate::modules::gpu::gpu_temperature(&self.temperatures)
    }

    /// 所有温度传感器里的最高值：CPU 最热的那颗核心、SSD、电池……谁最热就是谁。
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
    /// 网络下行 / 上行速率（字节/秒）。只有请求了按进程统计网络流量、且系统支持时才有值，其余情况是 0。
    #[serde(default)]
    pub net_rx_bps: u64,
    #[serde(default)]
    pub net_tx_bps: u64,
}

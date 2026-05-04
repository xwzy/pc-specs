# PC Specs · 技术方案文档

> 与 `design.md` 配套。本文档描述项目结构、技术栈、模块拆分、跨平台数据采集策略、IPC 协议、数据模型、构建与发布流程。

---

## 1. 技术栈选型

| 层 | 选型 | 理由 |
|----|------|------|
| 桌面外壳 | **Tauri 2** | 单包 < 15MB；Rust 后端 + 系统 WebView 前端，跨平台一致；权限模型清晰 |
| 后端语言 | **Rust** | 跨平台原生 API 接入能力（windows-rs / objc2 / sysfs）+ 性能 + 内存安全 |
| 前端框架 | **React 18 + TypeScript 5** | 生态最广，现代极客 UI 库齐全 |
| 构建 | **Vite 5** | 启动 / HMR 快 |
| 包管理 | **pnpm** | 快速、节省磁盘、严格依赖（项目规范） |
| 样式 | **Tailwind CSS 3** + CSS 变量主题 | 高密度 UI 友好，深色主题简单 |
| 组件库 | **shadcn/ui 子集** + **Radix Primitives** | 可定制、无侵入 |
| 图标 | **lucide-react** | 风格统一、轻量 |
| 状态 | **Zustand** | 极简、无 boilerplate |
| 数据流 | **@tanstack/react-query** | 缓存、自动刷新、订阅式 |
| 图表 | **Recharts** | 体积适中，组件式 API 与 React 契合 |
| 路由 | **react-router v6** | 声明式 |
| 国际化 | 简易自实现 i18n（中 / 英） | 避免引入大库 |

---

## 2. 项目结构

```
pc-specs/
├── docs/                   # 设计 / 技术 / UI 文档
│   ├── design.md
│   ├── tech.md
│   └── ui.md
├── src/                    # 前端源码（React + TS）
│   ├── main.tsx
│   ├── App.tsx
│   ├── index.css           # Tailwind + 主题变量
│   ├── lib/
│   │   ├── api.ts          # 与 Rust 通信封装（invoke）
│   │   ├── types.ts        # 后端数据模型 TS 镜像
│   │   ├── format.ts       # 单位 / 字节 / 频率格式化
│   │   ├── i18n.ts
│   │   └── store.ts        # zustand store
│   ├── components/
│   │   ├── layout/         # AppShell / Sidebar / Topbar
│   │   ├── ui/             # 基础元件（Card / Stat / Badge / Bar / Spark）
│   │   └── charts/         # LineChart / Gauge / RingProgress
│   └── pages/
│       ├── Dashboard.tsx
│       ├── Cpu.tsx
│       ├── Gpu.tsx
│       ├── Memory.tsx
│       ├── Storage.tsx
│       ├── Motherboard.tsx
│       ├── OsPage.tsx
│       ├── Network.tsx
│       ├── Display.tsx
│       ├── Sensors.tsx
│       ├── Battery.tsx
│       ├── Peripherals.tsx
│       ├── DevEnv.tsx
│       ├── Monitor.tsx
│       ├── Export.tsx
│       └── Settings.tsx
├── src-tauri/              # Rust 后端
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── build.rs
│   ├── icons/
│   └── src/
│       ├── main.rs
│       ├── lib.rs          # tauri::Builder + invoke 注册
│       ├── error.rs
│       ├── model.rs        # 全部数据模型（serde）
│       ├── commands.rs     # 暴露给前端的 invoke handler
│       ├── monitor.rs      # 实时监控发布器（tokio + emit）
│       ├── exporter.rs     # JSON / Markdown 导出
│       ├── platform/
│       │   ├── mod.rs      # 平台抽象 trait + cfg 路由
│       │   ├── common.rs   # 跨平台通用（基于 sysinfo）
│       │   ├── windows.rs  # Windows 特化（WMI / SetupAPI）
│       │   ├── macos.rs    # macOS 特化（sysctl / IOKit）
│       │   └── linux.rs    # Linux 特化（/sys、/proc、DMI）
│       └── modules/
│           ├── cpu.rs
│           ├── gpu.rs
│           ├── memory.rs
│           ├── storage.rs
│           ├── motherboard.rs
│           ├── os.rs
│           ├── network.rs
│           ├── display.rs
│           ├── sensors.rs
│           ├── battery.rs
│           ├── peripherals.rs
│           └── dev_env.rs
├── package.json
├── tsconfig.json
├── vite.config.ts
├── tailwind.config.js
├── postcss.config.js
├── index.html
├── .gitignore
└── README.md
```

> **设计原则**：
> 1. `modules/*` 只定义数据采集逻辑（一组件一文件）。
> 2. `platform/*` 提供平台特化的取数细节（WMI/IOKit/sysfs），由 `modules` 调用。
> 3. `commands.rs` 是唯一对前端暴露层，命名 `get_*`/`subscribe_*`/`export_*`。
> 4. 后端任何采集失败都应回退为 `Option::None` 或字符串 `"unknown"`，不在 IPC 层返回错误把整页打挂。

---

## 3. 数据模型（Rust 侧定义；前端 TS 同名镜像）

所有结构体派生 `Serialize, Deserialize, Clone, Debug`，字段命名 `snake_case`。

```rust
// model.rs（节选，详细类型见实现）

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

pub struct HostInfo {
    pub hostname: String,
    pub username: String,
    pub uptime_secs: u64,
    pub boot_time: u64,         // unix 秒
    pub app_version: String,
}

pub struct OsInfo {
    pub family: String,          // "windows" / "macos" / "linux"
    pub name: String,            // "Windows 11 Pro" / "macOS Sonoma 14.3" / "Ubuntu 22.04"
    pub version: String,
    pub kernel: String,
    pub arch: String,            // "x86_64" / "aarch64"
    pub locale: String,
    pub shell: Option<String>,
    pub desktop: Option<String>, // GNOME/KDE/Aqua/Explorer
}

pub struct CpuInfo {
    pub vendor: String,          // "GenuineIntel" / "AuthenticAMD" / "Apple"
    pub brand: String,           // "Intel(R) Core(TM) i7-13700K"
    pub arch: String,            // "x86_64" / "aarch64"
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub base_frequency_hz: u64,
    pub max_frequency_hz: u64,
    pub current_frequency_hz: u64,
    pub cache_l1_bytes: Option<u64>,
    pub cache_l2_bytes: Option<u64>,
    pub cache_l3_bytes: Option<u64>,
    pub features: Vec<String>,   // sse4_2, avx2, avx512f, aes, vmx, ...
    pub virtualization: Option<bool>,
    pub usage_per_core: Vec<f32>,// 0..100
    pub usage_overall: f32,
    pub temperature_c: Option<f32>,
    pub topology: Option<CpuTopology>,
}

pub struct CpuTopology {
    pub sockets: u32,
    pub p_cores: Option<u32>,
    pub e_cores: Option<u32>,
    pub numa_nodes: u32,
}

pub struct GpuInfo {
    pub index: u32,
    pub vendor: String,          // NVIDIA / AMD / Intel / Apple
    pub name: String,
    pub backend: String,         // Vulkan / Metal / Dx12 / OpenGL / Software
    pub driver: Option<String>,
    pub vram_total_bytes: Option<u64>,
    pub vram_used_bytes: Option<u64>,
    pub utilization: Option<f32>,
    pub temperature_c: Option<f32>,
    pub power_w: Option<f32>,
    pub pcie_link: Option<String>,// e.g. "PCIe 4.0 ×16"
    pub is_discrete: bool,
}

pub struct MemoryInfo {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub modules: Vec<MemoryModule>,
}

pub struct MemoryModule {
    pub slot: String,            // "DIMM_A1"
    pub manufacturer: Option<String>,
    pub part_number: Option<String>,
    pub capacity_bytes: u64,
    pub speed_mt_s: Option<u32>, // MT/s
    pub kind: Option<String>,    // DDR4 / DDR5 / LPDDR5
    pub form_factor: Option<String>,
}

pub struct StorageInfo {
    pub name: String,            // "Samsung SSD 990 PRO 2TB"
    pub mount_point: Option<String>,
    pub filesystem: Option<String>,
    pub kind: String,            // SSD / HDD / NVMe / Removable / Unknown
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub read_bytes_per_sec: u64,
    pub write_bytes_per_sec: u64,
    pub temperature_c: Option<f32>,
    pub smart_health: Option<String>, // OK / Warning / Failing / unknown
    pub serial: Option<String>,
}

pub struct MotherboardInfo {
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub version: Option<String>,
    pub serial: Option<String>,
    pub bios_vendor: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,
    pub chassis: Option<String>,  // Desktop / Laptop / Server / Mini-PC
}

pub struct NetworkInfo {
    pub interfaces: Vec<NetworkInterface>,
    pub public_ip: Option<String>, // 仅当用户开启
    pub default_gateway: Option<String>,
    pub dns_servers: Vec<String>,
}

pub struct NetworkInterface {
    pub name: String,
    pub mac: Option<String>,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub is_up: bool,
    pub is_loopback: bool,
    pub kind: String,            // ethernet / wifi / virtual / bluetooth
    pub link_speed_mbps: Option<u64>,
    pub rx_bytes_per_sec: u64,
    pub tx_bytes_per_sec: u64,
    pub rx_total_bytes: u64,
    pub tx_total_bytes: u64,
}

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

pub struct SensorReading {
    pub source: String,          // "coretemp" / "smc" / "wmi"
    pub label: String,           // "CPU Package" / "GPU Hot Spot" / "Fan1"
    pub kind: String,            // temperature / fan / voltage / power
    pub value: f32,
    pub unit: String,            // C / RPM / V / W
}

pub struct BatteryInfo {
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub state: String,           // charging / discharging / full / unknown
    pub percentage: f32,
    pub cycle_count: Option<u32>,
    pub design_capacity_mwh: Option<u64>,
    pub full_capacity_mwh: Option<u64>,
    pub current_capacity_mwh: Option<u64>,
    pub temperature_c: Option<f32>,
    pub time_to_empty_secs: Option<u64>,
    pub time_to_full_secs: Option<u64>,
    pub power_now_mw: Option<i64>, // 负=放电，正=充电
}

pub struct PeripheralInfo {
    pub kind: String,            // usb / bluetooth / audio / camera
    pub name: String,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub bus: Option<String>,
}

pub struct DevEnvInfo {
    pub languages: Vec<RuntimeInfo>,
    pub package_managers: Vec<RuntimeInfo>,
    pub vcs: Vec<RuntimeInfo>,
    pub editors: Vec<RuntimeInfo>,
    pub containers: Vec<RuntimeInfo>,
    pub shells: Vec<RuntimeInfo>,
    pub env_keys: Vec<String>,    // 仅展示 KEY，不展示 VALUE
}

pub struct RuntimeInfo {
    pub name: String,             // "node" / "python" / "rustc" / "git"
    pub version: Option<String>,
    pub path: Option<String>,
}
```

> 前端在 `src/lib/types.ts` 中维护与之 1:1 对齐的 TS 接口。

---

## 4. IPC 协议（Tauri Commands）

所有命令均为 `async`，无入参或简单入参，返回 `Result<T, String>`（错误转字符串避免类型膨胀）。
事件名常量见 `src-tauri/src/monitor.rs`。

### 4.1 一次性查询

| Command | 返回 | 说明 |
|--------|------|------|
| `get_full_snapshot` | `SystemSnapshot` | 一次拉取完整快照（首屏使用） |
| `get_cpu` | `CpuInfo` |  |
| `get_gpus` | `Vec<GpuInfo>` |  |
| `get_memory` | `MemoryInfo` |  |
| `get_storages` | `Vec<StorageInfo>` |  |
| `get_motherboard` | `Option<MotherboardInfo>` |  |
| `get_os` | `OsInfo` |  |
| `get_network` | `NetworkInfo` |  |
| `get_displays` | `Vec<DisplayInfo>` |  |
| `get_sensors` | `Vec<SensorReading>` |  |
| `get_battery` | `Option<BatteryInfo>` |  |
| `get_peripherals` | `Vec<PeripheralInfo>` |  |
| `get_dev_env` | `DevEnvInfo` |  |
| `get_host` | `HostInfo` |  |

### 4.2 实时监控

| Command | 入参 | 行为 |
|---------|------|------|
| `start_monitor` | `interval_ms: u64`（最小 500） | 在后台 task 周期采样并通过事件 `monitor://tick` 推送 `MonitorTick` |
| `stop_monitor` | — | 停止后台 task |

`MonitorTick`（精简快照，仅含变化频繁的字段，降低 IPC 体积）：

```rust
pub struct MonitorTick {
    pub timestamp: u64,
    pub cpu_overall: f32,
    pub cpu_per_core: Vec<f32>,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub net_rx_bps: u64,
    pub net_tx_bps: u64,
    pub disk_read_bps: u64,
    pub disk_write_bps: u64,
    pub gpu_utilizations: Vec<f32>,
    pub temperatures: Vec<SensorReading>,
}
```

### 4.3 导出 / 实用

| Command | 入参 | 返回 | 说明 |
|---------|------|------|------|
| `export_markdown` | — | `String` | 完整 Markdown |
| `export_json` | `pretty: bool` | `String` |  |
| `copy_to_clipboard` | `text: String` | `()` | 通过 Tauri clipboard 插件 |

---

## 5. 跨平台数据采集策略

### 5.1 通用打底（所有平台）
- `sysinfo` crate：CPU 占用 / 内存 / 进程 / 磁盘 IO / 网卡流量 / 操作系统名 / 主机名 / Uptime。
- `wgpu` crate：枚举所有 GPU 适配器，得到 vendor、name、backend、是否独显。
- `os_info` crate：发行版细分（Linux）。
- `whoami` crate：用户、桌面环境、shell。
- `local-ip-address` crate：默认网关 / 主 IP。
- 自定义：调用 `node --version` / `python --version` / `git --version` 等探测开发环境。

### 5.2 Windows 特化
- **WMI**（`wmi` crate）：
  - 主板：`Win32_BaseBoard`、`Win32_BIOS`
  - 内存模块：`Win32_PhysicalMemory`（厂商 / 容量 / 速度 / SPD 精度有限）
  - GPU：`Win32_VideoController`（驱动 / 显存）
  - 磁盘：`Win32_DiskDrive` + `MSStorageDriver_FailurePredictStatus`
  - 显示：`Win32_DesktopMonitor`
- **温度**：通过 `MSAcpi_ThermalZoneTemperature`（部分主板支持），失败则空。
- **PCIe 通道**：`SetupAPI`（可选，不依赖第三方）。

### 5.3 macOS 特化
- `sysctl` 解析 hw.* 给出 CPU brand、cores、l1/l2/l3、memsize。
- `system_profiler -json SPDisplaysDataType / SPHardwareDataType / SPMemoryDataType / SPNVMeDataType / SPPowerDataType`。
- IOKit（`io-kit-sys`）枚举显示器、电池。
- SMC 温度需要 root，未取到则空。
- M 系列芯片：`sysctl` 直接给出 P/E core 数量、神经网络引擎。

### 5.4 Linux 特化
- `/proc/cpuinfo`、`/proc/meminfo`、`/proc/stat`、`/proc/uptime`、`/proc/diskstats`。
- DMI：`/sys/class/dmi/id/{board_vendor,board_name,bios_*,product_name,sys_vendor}`。
- 内存模块：`dmidecode -t memory`（需 root，可选）；不可用时只显示总量。
- 风扇 / 温度：`/sys/class/hwmon/*/{temp*,fan*,in*,name}` 聚合。
- NVMe：`/sys/class/nvme/`。
- 显示：通过 X11 / Wayland 接口（直接读 `/sys/class/drm/*/edid`）。
- 网络：`/sys/class/net/*/{address,speed,operstate,statistics/*}`。
- 桌面环境：`XDG_CURRENT_DESKTOP`。

### 5.5 GPU 增强（可选 feature）
- NVIDIA：`nvml-wrapper`（feature flag `nvml`，构建时若 lib 不存在自动跳过）。
- AMD：`rocm-smi` 命令行（兜底解析）。
- Intel iGPU：暂不深度支持，依靠 wgpu/WMI 给出。

> **降级原则**：所有平台特化模块都有 try / fallback。任何一项不可用，对应字段为 `None` 或 `"unknown"`，UI 显示破折号 "—" 即可，不影响其他字段。

---

## 6. 实时监控架构

```
┌────────────┐    invoke(start_monitor)     ┌────────────────────┐
│  Frontend  │ ───────────────────────────▶ │ tokio::spawn loop  │
│ (React)    │                              │ (interval_ms tick) │
│            │ ◀───── emit("monitor://tick")│                    │
└────────────┘                              └────────────────────┘
       ▲                                             │
       │                                             ▼
       │                              ┌─────────────────────────┐
       └──────  listen via @tauri    │  采样：sysinfo refresh + │
                /api/event           │  GPU util + 传感器       │
                                     └─────────────────────────┘
```

- 采样使用全局 `Arc<Mutex<sysinfo::System>>`，每个 tick `refresh_specifics`。
- 通过 `tokio::sync::Notify` 优雅停止任务。
- 频率前端可调（500ms ~ 5s）。
- 前端使用 ring buffer（最多 600 点 = 10min @ 1Hz）做曲线图。

---

## 7. 错误处理与日志

- Rust 端：自定义 `AppError` 枚举（IO / Wmi / Sysinfo / Parse），`thiserror` 派生。
- IPC 边界：`AppError` → `String`（保留 `to_string()`，不暴露内部细节）。
- 前端：所有 query 在 React Query 中捕获，UI 渲染 "—" + Tooltip 错误描述。
- 启动时初始化 `tracing_subscriber`，文件日志位于 `${APP_DATA}/pc-specs/log/app.log`，DEBUG 级别可设置项控制。

---

## 8. 安全 & 权限

- Tauri capabilities 仅开放：`core:default`, `clipboard:write`, `dialog:save`, `fs:write-text-file`（用于导出）, `shell:spawn`（白名单：`node` / `python` / `git` / `rustc` / `go` 等只读探测命令）。
- 网络：默认禁用所有 fetch；仅当用户在设置中勾选 "查询公网 IP"，使用 `https://api.ipify.org` 单一域名。
- 本地敏感数据（环境变量值、磁盘序列号等）默认遮罩，导出时用户主动勾选才包含。

---

## 9. 构建 & 发布

- 开发：`pnpm i` → `pnpm tauri dev`（用户日常开发用）。
- 仅前端（用户可单独跑 UI）：`pnpm dev`。
- 验证编译（CI / 本地非完整构建）：
  - 前端：`pnpm typecheck`（`tsc -b --noEmit`）。
  - 后端：`cargo check --manifest-path src-tauri/Cargo.toml`（按项目规范，本仓库默认只 check，不构建完整 app 包）。
- 发布：`pnpm tauri build`（CI/CD 使用，签名 / 公证另行配置）。

---

## 10. 测试策略（轻量）

- Rust 模块函数对纯逻辑（格式化、解析）写 unit tests（`#[cfg(test)]`）。
- 平台采集逻辑放置 mock 友好的 trait，但本期不强制覆盖。
- 前端组件不做强测试，依赖 typecheck + 实际运行。
- 临时调试代码在验证后删除（项目规范）。

---

## 11. 命名规范

- Rust crate 内：`snake_case` 模块、`UpperCamelCase` 类型、`SCREAMING_SNAKE_CASE` 常量。
- IPC 字段：全部 `snake_case`（项目规范硬约束）。
- 前端 TS 类型：与 Rust 一致 `snake_case`（直接 1:1，避免转换层成本与 bug）。
- 前端组件 / 文件：`PascalCase.tsx`，目录名 `kebab-case` 或 `lowercase`。
- React hooks：`useXxx`。

---

## 12. 关键 crate 列表（src-tauri/Cargo.toml 约定）

```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-clipboard-manager = "2"
tauri-plugin-dialog = "2"
tauri-plugin-fs = "2"
tauri-plugin-shell = "2"
tauri-plugin-os = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time"] }
sysinfo = "0.32"
wgpu = "22"
os_info = "3"
whoami = "1"
local-ip-address = "0.6"
thiserror = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
once_cell = "1"
parking_lot = "0.12"

[target.'cfg(target_os = "windows")'.dependencies]
wmi = "0.14"

[target.'cfg(target_os = "linux")'.dependencies]
nix = { version = "0.29", features = ["fs"] }

[target.'cfg(target_os = "macos")'.dependencies]
# IOKit / sysctl 通过 std 命令调用 + system_profiler，不强引入 objc
```

> 实际版本以 `cargo check` 通过为准；如发现某 crate 与 Tauri 2 冲突，记录在 README 中。

---

## 13. 与设计 / UI 的映射

| design.md §2.1 模块 | 后端 command | 前端页面 |
|---|---|---|
| Dashboard | `get_full_snapshot` | `pages/Dashboard.tsx` |
| CPU | `get_cpu` + monitor | `pages/Cpu.tsx` |
| GPU | `get_gpus` + monitor | `pages/Gpu.tsx` |
| Memory | `get_memory` + monitor | `pages/Memory.tsx` |
| Storage | `get_storages` + monitor | `pages/Storage.tsx` |
| Motherboard | `get_motherboard` | `pages/Motherboard.tsx` |
| OS | `get_os` + `get_host` | `pages/OsPage.tsx` |
| Network | `get_network` + monitor | `pages/Network.tsx` |
| Display | `get_displays` | `pages/Display.tsx` |
| Sensors | `get_sensors` + monitor | `pages/Sensors.tsx` |
| Battery | `get_battery` + monitor | `pages/Battery.tsx` |
| Peripherals | `get_peripherals` | `pages/Peripherals.tsx` |
| Dev Env | `get_dev_env` | `pages/DevEnv.tsx` |
| Monitor | `start/stop_monitor` + tick event | `pages/Monitor.tsx` |
| Export | `export_markdown` / `export_json` | `pages/Export.tsx` |
| Settings | 本地存储 | `pages/Settings.tsx` |


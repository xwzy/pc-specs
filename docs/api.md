# PC Specs · 本地 HTTP 接口文档

> 本文档面向想从**其他机器**调用 pc-specs 采集本机硬件 / 系统 / 实时指标的开发者。  
> 服务在 pc-specs 桌面应用启动后自动运行，无需额外配置。

---

## 1. 基本信息

| 项 | 值 |
|---|---|
| 监听地址 | `0.0.0.0:16089` |
| 协议 | HTTP/1.1（明文） |
| API 版本 | `v1` |
| 字符编码 | UTF-8 |
| Content-Type | `application/json`（响应永远是 JSON，错误页除外） |
| 鉴权 | 无（依赖网络层 / 防火墙做访问控制，详见 [§7 安全](#7-安全建议)） |
| CORS | `Access-Control-Allow-Origin: *`（允许浏览器跨域调用） |

启动后访问 `http://<host>:16089/` 可看到自带的 HTML 文档页。

---

## 2. 通用约定

### 2.1 请求方式

- 除 `GET /` 与 `GET /healthz` 外，**所有 `/api/v1/*` 端点都使用 `POST`**，便于以后无侵入地扩展请求参数。
- 请求体格式：`application/json`。**不需要参数时也可以传空对象 `{}`，或者完全省略 body。**
- 请求体最大 **64 KiB**（超出返回 `413 Payload Too Large`）。
- 单个请求处理超时 **15 秒**（超出返回 `504 Gateway Timeout`）。

### 2.2 字段命名

所有响应字段统一采用 `snake_case`（小写 + 下划线）。

### 2.3 错误响应

非 2xx 响应返回如下结构：

```json
{
  "ok": false,
  "error": "human readable error message"
}
```

| HTTP 状态 | 含义 |
|---|---|
| `200 OK` | 成功 |
| `400 Bad Request` | 请求 body JSON 解析失败 |
| `404 Not Found` | 路径不存在 |
| `405 Method Not Allowed` | 用了非 POST/GET 的方法 |
| `413 Payload Too Large` | 请求体超过 64 KiB |
| `500 Internal Server Error` | 后端采集失败（详见 `error` 字段） |
| `504 Gateway Timeout` | 单次请求超过 15 秒未完成 |

### 2.4 时间戳

所有 `timestamp_ms` / `timestamp` 字段都是 **Unix 毫秒**（UTC）。

### 2.5 字节单位

所有字节相关字段（`*_bytes`）统一使用 **bytes**；带宽 / IO 速率字段（`*_bps`）单位为 **bytes per second**（不是 bits）。

---

## 3. 缓存与采集成本

| 端点 | TTL | 说明 |
|---|---|---|
| `/api/v1/metrics` | **800 ms** | 高频实时指标，1Hz 轮询不会触发额外采集 |
| `/api/v1/snapshot` 与 `/api/v1/{host\|os\|cpu\|...}` | **3 s** | 共用同一份全量快照缓存，多端点轮询只会触发一次采集 |
| `/api/v1/health`、`/api/v1/info` | 无缓存 | 本身就是廉价操作 |

> 缓存使用 single-flight 锁保护：即使缓存刚好失效，多客户端并发请求也只会触发**一次**底层采集，其余请求复用结果。

为了进一步降低占用：

- **没有客户端请求时，服务后台 0 CPU 占用**（不预采样）。
- 高频监控请使用 `/api/v1/metrics`，不要轮询 `/api/v1/snapshot`（snapshot 包含 sensors/dev_env 等慢查询）。

---

## 4. 端点速查

| 方法 | 路径 | 用途 |
|---|---|---|
| `GET` | `/` | HTML 文档页 |
| `GET` | `/healthz` | 探活（监控系统友好） |
| `GET` / `POST` | `/api/v1/health` | 同上（POST 形式） |
| `POST` | `/api/v1/info` | 轻量身份信息 |
| `POST` | `/api/v1/metrics` | **实时指标**（CPU/内存/网络/磁盘 BPS） |
| `POST` | `/api/v1/snapshot` | **全量系统快照** |
| `POST` | `/api/v1/host` | 主机信息 |
| `POST` | `/api/v1/os` | 操作系统信息 |
| `POST` | `/api/v1/cpu` | CPU 信息 |
| `POST` | `/api/v1/gpus` | GPU 列表 |
| `POST` | `/api/v1/memory` | 内存信息 |
| `POST` | `/api/v1/storages` | 存储设备列表 |
| `POST` | `/api/v1/motherboard` | 主板 / BIOS |
| `POST` | `/api/v1/network` | 网络接口 / 公网 IP |
| `POST` | `/api/v1/displays` | 显示器列表 |
| `POST` | `/api/v1/sensors` | 温度 / 风扇等传感器 |
| `POST` | `/api/v1/battery` | 电池（笔记本） |
| `POST` | `/api/v1/peripherals` | USB / 输入设备 |
| `POST` | `/api/v1/dev_env` | 编程环境（语言 / 包管理器 / 编辑器） |

---

## 5. 端点详细说明

### 5.1 `GET /healthz` · 探活

最便宜的探活端点。

**响应：**

```json
{
  "ok": true,
  "name": "pc-specs",
  "version": "0.1.0",
  "uptime_secs": 1234,
  "timestamp_ms": 1746427980000
}
```

**字段：**

| 字段 | 类型 | 说明 |
|---|---|---|
| `ok` | `bool` | 始终 `true`（服务存活） |
| `name` | `string` | 应用名，固定 `"pc-specs"` |
| `version` | `string` | 应用语义化版本 |
| `uptime_secs` | `u64` | HTTP 服务自启动以来的秒数 |
| `timestamp_ms` | `u64` | 服务器当前 Unix 毫秒 |

`POST /api/v1/health` 等价。

---

### 5.2 `POST /api/v1/info` · 身份信息

返回最轻量的机器身份信息（不会触发硬件扫描）。适合"批量发现局域网内 pc-specs 实例"。

**请求 body：** `{}` 或省略

**响应：**

```json
{
  "hostname": "studio.local",
  "username": "wzy",
  "os_family": "macos",
  "os_name": "macOS Sonoma",
  "os_version": "14.5",
  "arch": "aarch64",
  "app_version": "0.1.0",
  "api_version": 1,
  "server_uptime_secs": 1234,
  "timestamp_ms": 1746427980000
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `hostname` | `string` | 主机名 |
| `username` | `string` | 当前登录用户名 |
| `os_family` | `string` | `"linux"` / `"macos"` / `"windows"` |
| `os_name` | `string` | 操作系统完整名 |
| `os_version` | `string` | 操作系统版本号 |
| `arch` | `string` | CPU 架构（`x86_64` / `aarch64` 等） |
| `app_version` | `string` | pc-specs 应用版本 |
| `api_version` | `u32` | 接口大版本号，当前 `1` |
| `server_uptime_secs` | `u64` | HTTP 服务运行时长 |
| `timestamp_ms` | `u64` | 服务器当前时间 |

> ⚠️ 此端点不会做敏感字段遮蔽，因为信息本身轻量公开。如果担心 hostname/username 泄露，可在防火墙层限制访问。

---

### 5.3 `POST /api/v1/metrics` · 实时指标

**这是高频轮询的推荐端点。** 内部带 800ms 缓存 + single-flight，1~10Hz 轮询都不会增加显著压力。

**请求 body：** `{}` 或省略

**响应（`LightMetrics`）：**

```json
{
  "timestamp_ms": 1746427980123,
  "cpu_overall": 18.4,
  "cpu_per_core": [12.5, 21.3, 8.7, 30.1, ...],
  "mem_used_bytes": 16223584256,
  "mem_total_bytes": 34359738368,
  "net_rx_bps": 124589,
  "net_tx_bps": 9821,
  "disk_read_bps": 0,
  "disk_write_bps": 524288,
  "elapsed_secs": 1.012
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `timestamp_ms` | `u64` | 采样时间 |
| `cpu_overall` | `f32` | 全局 CPU 使用率（0–100，单位百分比） |
| `cpu_per_core` | `f32[]` | 每核使用率（顺序与系统返回一致） |
| `mem_used_bytes` | `u64` | 已用物理内存 |
| `mem_total_bytes` | `u64` | 物理内存总量 |
| `net_rx_bps` | `u64` | 网络下行速率（bytes/s，所有非 loopback 接口聚合） |
| `net_tx_bps` | `u64` | 网络上行速率 |
| `disk_read_bps` | `u64` | 磁盘读速率（所有物理盘聚合） |
| `disk_write_bps` | `u64` | 磁盘写速率 |
| `elapsed_secs` | `f64` | 自上次采样以来的真实经过秒数；**首次请求为 `0`** |

**首次请求注意：** 速率类字段需要"上次"基线才有意义。第一次调用 `/api/v1/metrics` 时各 BPS 字段都是 `0`，且 `elapsed_secs == 0`。从第二次起返回真实值。

---

### 5.4 `POST /api/v1/snapshot` · 全量系统快照

返回 [`SystemSnapshot`](#65-systemsnapshot) 完整对象，包含所有可采集的硬件 / 软件信息。3 秒缓存。

**请求 body（可选）：**

```json
{
  "include_sensitive": false
}
```

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `include_sensitive` | `bool` | `false` | 是否原样返回敏感字段；`false` 时按下表遮蔽为 `"[redacted]"` |

**敏感字段遮蔽规则**（`include_sensitive: false` 时）：

| 字段路径 | 处理 |
|---|---|
| `host.hostname` | 替换为 `"[redacted]"` |
| `host.username` | 替换为 `"[redacted]"` |
| `network.public_ip` | 替换为 `"[redacted]"`（如果存在） |
| `network.interfaces[].mac` | 替换为 `"[redacted]"`（如果存在） |
| `motherboard.serial` | 替换为 `"[redacted]"`（如果存在） |
| `storages[].serial` | 替换为 `"[redacted]"`（如果存在） |

**响应：** 见 [§6 数据类型](#6-数据类型)。

---

### 5.5 分段端点

下列端点都从同一份 `/api/v1/snapshot` 缓存派生，返回 `SystemSnapshot` 对应字段的子树。**敏感字段始终不遮蔽**（除 `/api/v1/snapshot` 本身外）。请按需选择：

| 端点 | 响应类型 | 链接 |
|---|---|---|
| `POST /api/v1/host` | `HostInfo` | [↓](#62-hostinfo) |
| `POST /api/v1/os` | `OsInfo` | [↓](#63-osinfo) |
| `POST /api/v1/cpu` | `CpuInfo` | [↓](#64-cpuinfo) |
| `POST /api/v1/gpus` | `GpuInfo[]` | [↓](#65-gpuinfo) |
| `POST /api/v1/memory` | `MemoryInfo` | [↓](#66-memoryinfo--memorymodule) |
| `POST /api/v1/storages` | `StorageInfo[]` | [↓](#67-storageinfo) |
| `POST /api/v1/motherboard` | `MotherboardInfo \| null` | [↓](#68-motherboardinfo) |
| `POST /api/v1/network` | `NetworkInfo` | [↓](#69-networkinfo--networkinterface) |
| `POST /api/v1/displays` | `DisplayInfo[]` | [↓](#610-displayinfo) |
| `POST /api/v1/sensors` | `SensorReading[]` | [↓](#611-sensorreading) |
| `POST /api/v1/battery` | `BatteryInfo \| null` | [↓](#612-batteryinfo) |
| `POST /api/v1/peripherals` | `PeripheralInfo[]` | [↓](#613-peripheralinfo) |
| `POST /api/v1/dev_env` | `DevEnvInfo` | [↓](#614-devenvinfo--runtimeinfo) |

请求 body 都可省略或传 `{}`。

---

## 6. 数据类型

> 类型表示沿用 Rust 风格：`Option<T>` 在 JSON 中表示 `T | null`。`u64`/`f32` 等是 JSON 的 `number`，但请注意 64-bit 整数在 JS 端可能丢精度（详见 [§7.3](#73-javascript-的-bigint-精度)）。

### 6.1 `SystemSnapshot`

`/api/v1/snapshot` 的顶层对象：

| 字段 | 类型 | 说明 |
|---|---|---|
| `timestamp` | `u64` | 快照采集时间（Unix 毫秒） |
| `host` | [`HostInfo`](#62-hostinfo) | |
| `os` | [`OsInfo`](#63-osinfo) | |
| `cpu` | [`CpuInfo`](#64-cpuinfo) | |
| `gpus` | [`GpuInfo[]`](#65-gpuinfo) | |
| `memory` | [`MemoryInfo`](#66-memoryinfo--memorymodule) | |
| `storages` | [`StorageInfo[]`](#67-storageinfo) | |
| `motherboard` | [`MotherboardInfo`](#68-motherboardinfo) `\| null` | |
| `network` | [`NetworkInfo`](#69-networkinfo--networkinterface) | |
| `displays` | [`DisplayInfo[]`](#610-displayinfo) | |
| `sensors` | [`SensorReading[]`](#611-sensorreading) | |
| `battery` | [`BatteryInfo`](#612-batteryinfo) `\| null` | |
| `peripherals` | [`PeripheralInfo[]`](#613-peripheralinfo) | |
| `dev_env` | [`DevEnvInfo`](#614-devenvinfo--runtimeinfo) | |

### 6.2 `HostInfo`

```json
{
  "hostname": "studio.local",
  "username": "wzy",
  "uptime_secs": 86400,
  "boot_time": 1746341580,
  "app_version": "0.1.0"
}
```

| 字段 | 类型 | 说明 |
|---|---|---|
| `hostname` | `string` | 主机名 |
| `username` | `string` | 当前登录用户 |
| `uptime_secs` | `u64` | 系统运行秒数 |
| `boot_time` | `u64` | 系统启动 Unix 秒（不是毫秒） |
| `app_version` | `string` | pc-specs 应用版本 |

### 6.3 `OsInfo`

| 字段 | 类型 | 说明 |
|---|---|---|
| `family` | `string` | `"linux"` / `"macos"` / `"windows"` |
| `name` | `string` | 完整 OS 名（如 `"macOS Sonoma"`、`"Ubuntu 22.04 LTS"`） |
| `version` | `string` | 版本号 |
| `kernel` | `string` | 内核版本（Linux 为 uname、macOS 为 Darwin、Windows 为 NT 版本） |
| `arch` | `string` | `"x86_64"` / `"aarch64"` 等 |
| `locale` | `string` | 系统区域，如 `"zh_CN.UTF-8"` |
| `shell` | `string \| null` | 登录 shell |
| `desktop` | `string \| null` | 桌面环境（GNOME / KDE / Aqua / Windows 等） |

### 6.4 `CpuInfo`

| 字段 | 类型 | 说明 |
|---|---|---|
| `vendor` | `string` | 厂商（GenuineIntel / AuthenticAMD / Apple 等） |
| `brand` | `string` | CPU 型号字符串 |
| `arch` | `string` | 架构 |
| `physical_cores` | `u32` | 物理核心数 |
| `logical_cores` | `u32` | 逻辑线程数 |
| `base_frequency_hz` | `u64` | 基础频率（Hz） |
| `max_frequency_hz` | `u64` | 最大频率（Hz） |
| `current_frequency_hz` | `u64` | 当前频率（Hz，sysinfo 抽样） |
| `cache_l1_bytes` | `u64 \| null` | L1 总容量 |
| `cache_l2_bytes` | `u64 \| null` | L2 总容量 |
| `cache_l3_bytes` | `u64 \| null` | L3 总容量 |
| `features` | `string[]` | 指令集（如 `["sse4_2","avx2","aes"]`） |
| `virtualization` | `bool \| null` | 是否支持硬件虚拟化（VT-x / SVM） |
| `usage_per_core` | `f32[]` | 各核利用率（0–100） |
| `usage_overall` | `f32` | 整体利用率（0–100） |
| `temperature_c` | `f32 \| null` | 当前 CPU 温度（°C），平台无可用传感器时为 `null` |
| `topology` | `CpuTopology \| null` | 拓扑信息 |

`CpuTopology`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `sockets` | `u32` | 物理插槽数 |
| `p_cores` | `u32 \| null` | 性能核心数（混合架构） |
| `e_cores` | `u32 \| null` | 能效核心数 |
| `numa_nodes` | `u32` | NUMA 节点数 |

### 6.5 `GpuInfo`

| 字段 | 类型 | 说明 |
|---|---|---|
| `index` | `u32` | 序号（0 起） |
| `vendor` | `string` | 厂商（NVIDIA / AMD / Intel / Apple） |
| `name` | `string` | 型号 |
| `backend` | `string` | wgpu 后端（`Vulkan`/`Metal`/`Dx12`/`Gl`） |
| `driver` | `string \| null` | 驱动版本 |
| `vram_total_bytes` | `u64 \| null` | 显存总量 |
| `vram_used_bytes` | `u64 \| null` | 显存已用 |
| `utilization` | `f32 \| null` | 使用率（0–100，需平台支持） |
| `temperature_c` | `f32 \| null` | 温度 |
| `power_w` | `f32 \| null` | 当前功耗（W） |
| `pcie_link` | `string \| null` | PCIe 链路（如 `"PCIe 4.0 x16"`） |
| `is_discrete` | `bool` | 是否独立显卡 |

### 6.6 `MemoryInfo` / `MemoryModule`

`MemoryInfo`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `total_bytes` | `u64` | 物理内存总量 |
| `used_bytes` | `u64` | 已用 |
| `available_bytes` | `u64` | 可用（已扣 cache/buffer） |
| `swap_total_bytes` | `u64` | swap 总量 |
| `swap_used_bytes` | `u64` | swap 已用 |
| `modules` | `MemoryModule[]` | 内存条详情（部分平台为空） |

`MemoryModule`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `slot` | `string` | 插槽位标识 |
| `manufacturer` | `string \| null` | 厂商 |
| `part_number` | `string \| null` | 零件号 |
| `capacity_bytes` | `u64` | 容量 |
| `speed_mt_s` | `u32 \| null` | 速率（MT/s） |
| `kind` | `string \| null` | 类型（`DDR4`、`DDR5` 等） |
| `form_factor` | `string \| null` | 封装形态（`DIMM`、`SODIMM` 等） |

### 6.7 `StorageInfo`

| 字段 | 类型 | 说明 |
|---|---|---|
| `name` | `string` | 设备名（`/dev/nvme0n1`、`disk0` 等） |
| `mount_point` | `string \| null` | 挂载点 |
| `filesystem` | `string \| null` | 文件系统（apfs/ntfs/ext4 等） |
| `kind` | `string` | `"SSD"` / `"HDD"` / `"NVMe"` / `"Removable"` 等 |
| `total_bytes` | `u64` | 容量 |
| `used_bytes` | `u64` | 已用 |
| `read_bytes_per_sec` | `u64` | 当前读速率 |
| `write_bytes_per_sec` | `u64` | 当前写速率 |
| `temperature_c` | `f32 \| null` | 温度 |
| `smart_health` | `string \| null` | SMART 健康状态（`"OK"`/`"Failing"` 等） |
| `serial` | `string \| null` | 序列号（敏感字段，遮蔽规则见 [§5.4](#54-post-apiv1snapshot--全量系统快照)） |

### 6.8 `MotherboardInfo`

| 字段 | 类型 | 说明 |
|---|---|---|
| `vendor` | `string \| null` | 主板厂商 |
| `model` | `string \| null` | 型号 |
| `version` | `string \| null` | 版本 |
| `serial` | `string \| null` | 序列号（敏感） |
| `bios_vendor` | `string \| null` | BIOS 厂商 |
| `bios_version` | `string \| null` | BIOS 版本 |
| `bios_date` | `string \| null` | BIOS 日期（厂商原文，未必标准化） |
| `chassis` | `string \| null` | 机型（Desktop/Laptop/...） |

### 6.9 `NetworkInfo` / `NetworkInterface`

`NetworkInfo`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `interfaces` | `NetworkInterface[]` | 全部网卡 |
| `public_ip` | `string \| null` | 出口公网 IP；不一定每次都拉取（首次请求 `/api/v1/snapshot` 后才会探测） |
| `default_gateway` | `string \| null` | 默认网关 |
| `dns_servers` | `string[]` | DNS 列表 |

`NetworkInterface`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `name` | `string` | 接口名（`en0` / `eth0` / `Wi-Fi`） |
| `mac` | `string \| null` | MAC 地址（敏感） |
| `ipv4` | `string[]` | 此接口的 IPv4 地址列表 |
| `ipv6` | `string[]` | IPv6 地址列表 |
| `is_up` | `bool` | 是否启用 |
| `is_loopback` | `bool` | 是否回环 |
| `kind` | `string` | `"Ethernet"`/`"WiFi"`/`"Loopback"`/`"Tun"`/`"Other"` |
| `link_speed_mbps` | `u64 \| null` | 链路速率（Mbps） |
| `rx_bytes_per_sec` | `u64` | 即时下行速率 |
| `tx_bytes_per_sec` | `u64` | 即时上行速率 |
| `rx_total_bytes` | `u64` | 自启动以来累计接收 |
| `tx_total_bytes` | `u64` | 自启动以来累计发送 |

### 6.10 `DisplayInfo`

| 字段 | 类型 | 说明 |
|---|---|---|
| `name` | `string` | 显示器名称 |
| `width_px` | `u32` | 横向像素 |
| `height_px` | `u32` | 纵向像素 |
| `refresh_hz` | `u32 \| null` | 刷新率 |
| `scale_factor` | `f32 \| null` | DPI 缩放（如 1.0 / 2.0） |
| `is_primary` | `bool` | 是否主显示器 |
| `physical_width_mm` | `u32 \| null` | 物理宽度（毫米） |
| `physical_height_mm` | `u32 \| null` | 物理高度（毫米） |
| `color_depth` | `u8 \| null` | 色深（位） |

### 6.11 `SensorReading`

| 字段 | 类型 | 说明 |
|---|---|---|
| `source` | `string` | 数据源（`"smc"` / `"hwmon"` / `"wmi"` / ...） |
| `label` | `string` | 传感器名（如 `"CPU Die Temperature"`） |
| `kind` | `string` | `"temperature"` / `"fan"` / `"voltage"` / `"power"` |
| `value` | `f32` | 数值 |
| `unit` | `string` | 单位字符串（`"C"` / `"rpm"` / `"V"` / `"W"`） |

### 6.12 `BatteryInfo`

笔记本才返回，台式机为 `null`。

| 字段 | 类型 | 说明 |
|---|---|---|
| `vendor` | `string \| null` | 厂商 |
| `model` | `string \| null` | 型号 |
| `state` | `string` | `"Charging"` / `"Discharging"` / `"Full"` / `"Empty"` / `"Unknown"` |
| `percentage` | `f32` | 电量百分比（0–100） |
| `cycle_count` | `u32 \| null` | 循环次数 |
| `design_capacity_mwh` | `u64 \| null` | 设计容量（mWh） |
| `full_capacity_mwh` | `u64 \| null` | 实际满电容量 |
| `current_capacity_mwh` | `u64 \| null` | 当前剩余 |
| `temperature_c` | `f32 \| null` | 电池温度 |
| `time_to_empty_secs` | `u64 \| null` | 放电时预计剩余时间（秒） |
| `time_to_full_secs` | `u64 \| null` | 充电时预计剩余时间（秒） |
| `power_now_mw` | `i64 \| null` | 当前充/放电功率（mW，正数充电、负数放电） |

### 6.13 `PeripheralInfo`

| 字段 | 类型 | 说明 |
|---|---|---|
| `kind` | `string` | `"keyboard"` / `"mouse"` / `"audio"` / `"camera"` / `"usb"` 等 |
| `name` | `string` | 设备名 |
| `vendor_id` | `string \| null` | USB Vendor ID（4 位 hex） |
| `product_id` | `string \| null` | USB Product ID |
| `bus` | `string \| null` | 总线（`"usb"` / `"bluetooth"` / `"pci"`） |

### 6.14 `DevEnvInfo` / `RuntimeInfo`

`DevEnvInfo`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `languages` | `RuntimeInfo[]` | 编程语言（node/python/go/rust/java/...） |
| `package_managers` | `RuntimeInfo[]` | 包管理器（npm/pnpm/pip/cargo/...） |
| `vcs` | `RuntimeInfo[]` | 版本控制（git/hg/svn） |
| `editors` | `RuntimeInfo[]` | 编辑器（code/cursor/vim/emacs/...） |
| `containers` | `RuntimeInfo[]` | 容器工具（docker/podman/k8s） |
| `shells` | `RuntimeInfo[]` | shell（bash/zsh/fish/pwsh） |
| `env_keys` | `string[]` | 检测到的关键环境变量名（仅 key，不含值） |

`RuntimeInfo`：

| 字段 | 类型 | 说明 |
|---|---|---|
| `name` | `string` | 工具名 |
| `version` | `string \| null` | 版本字符串（采集自 `--version` 输出） |
| `path` | `string \| null` | 可执行路径 |

---

## 7. 安全建议

### 7.1 默认裸 HTTP，无鉴权

服务监听 `0.0.0.0:16089`，**任何能 ping 到本机的设备都能调用**。请按场景做访问控制：

- **个人内网**：依赖路由器的隔离即可。
- **办公 / 共享网络**：用系统防火墙限制 16089 端口的源 IP 段。

  ```bash
  # macOS 示例（pf）
  echo "block in proto tcp from any to any port 16089" | sudo pfctl -ef -
  echo "pass in proto tcp from 192.168.1.0/24 to any port 16089" | sudo pfctl -ef -

  # Linux 示例（ufw）
  sudo ufw allow from 192.168.1.0/24 to any port 16089 proto tcp
  sudo ufw deny 16089/tcp

  # Windows 示例（netsh）
  netsh advfirewall firewall add rule name="pc-specs LAN" dir=in action=allow ^
        protocol=TCP localport=16089 remoteip=192.168.1.0/24
  ```

- **公网暴露**：**强烈不建议**。如必须，请放在 nginx / Caddy 反代后加 TLS + Basic Auth。

### 7.2 敏感字段保护

`/api/v1/snapshot` 默认遮蔽敏感字段（详见 [§5.4](#54-post-apiv1snapshot--全量系统快照)）。只有传递 `{"include_sensitive": true}` 才会原样返回。

> ⚠️ 注意：分段端点（如 `/api/v1/host`、`/api/v1/network`、`/api/v1/storages`）**不会自动遮蔽**，因为它们的目标用户通常是本机/局域网信任环境。如果你担心，请用 `/api/v1/snapshot` 后从结果里提取自己需要的字段。

### 7.3 JavaScript 的 BigInt 精度

`u64` 字段（如 `*_bytes`、`boot_time`）理论上可以超过 `Number.MAX_SAFE_INTEGER`（2^53 − 1 ≈ 9 PB）。在浏览器 / Node 解析时会丢精度。

绕过方法：

```js
// 方法 1：用 BigInt JSON parser
import JSONbig from 'json-bigint';
const data = JSONbig({ useNativeBigInt: true }).parse(text);

// 方法 2：自己重写大字段
const text = await (await fetch(url)).text();
const data = JSON.parse(text.replace(
  /"(\w+_bytes|boot_time)":(\d{16,})/g,
  '"$1":"$2"'
));
```

实际场景：日常硬件 / 内存 / 时间戳都在 64 位精度安全范围内（< 2^53）。除非你处理 EB 级冷存储，可以无视这条。

---

## 8. 接入示例

### 8.1 curl

```bash
HOST=192.168.1.42

# 探活
curl -s http://$HOST:16089/healthz

# 实时指标
curl -s -X POST http://$HOST:16089/api/v1/metrics

# 全量快照（遮蔽敏感）
curl -s -X POST http://$HOST:16089/api/v1/snapshot

# 全量快照（含敏感）
curl -s -X POST http://$HOST:16089/api/v1/snapshot \
     -H 'Content-Type: application/json' \
     -d '{"include_sensitive": true}'

# 仅 CPU
curl -s -X POST http://$HOST:16089/api/v1/cpu | jq .
```

### 8.2 JavaScript / TypeScript

```ts
const HOST = '192.168.1.42';

async function fetchMetrics() {
  const res = await fetch(`http://${HOST}:16089/api/v1/metrics`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: '{}',
  });
  if (!res.ok) throw new Error(`HTTP ${res.status}`);
  return res.json() as Promise<{
    timestamp_ms: number;
    cpu_overall: number;
    cpu_per_core: number[];
    mem_used_bytes: number;
    mem_total_bytes: number;
    net_rx_bps: number;
    net_tx_bps: number;
    disk_read_bps: number;
    disk_write_bps: number;
    elapsed_secs: number;
  }>;
}

setInterval(async () => {
  const m = await fetchMetrics();
  console.log(`CPU=${m.cpu_overall.toFixed(1)}%  MEM=${(m.mem_used_bytes / m.mem_total_bytes * 100).toFixed(1)}%`);
}, 1000);
```

### 8.3 Python

```python
import requests, time

HOST = "192.168.1.42"
BASE = f"http://{HOST}:16089"

def metrics():
    r = requests.post(f"{BASE}/api/v1/metrics", json={}, timeout=5)
    r.raise_for_status()
    return r.json()

while True:
    m = metrics()
    print(f"CPU={m['cpu_overall']:.1f}%  "
          f"NET ↓ {m['net_rx_bps']/1e6:.2f} MB/s ↑ {m['net_tx_bps']/1e6:.2f} MB/s")
    time.sleep(1)
```

### 8.4 Go

```go
package main

import (
    "bytes"
    "encoding/json"
    "fmt"
    "net/http"
    "time"
)

type Metrics struct {
    TimestampMs   int64     `json:"timestamp_ms"`
    CPUOverall    float32   `json:"cpu_overall"`
    CPUPerCore    []float32 `json:"cpu_per_core"`
    MemUsedBytes  uint64    `json:"mem_used_bytes"`
    MemTotalBytes uint64    `json:"mem_total_bytes"`
    NetRxBps      uint64    `json:"net_rx_bps"`
    NetTxBps      uint64    `json:"net_tx_bps"`
    DiskReadBps   uint64    `json:"disk_read_bps"`
    DiskWriteBps  uint64    `json:"disk_write_bps"`
    ElapsedSecs   float64   `json:"elapsed_secs"`
}

func main() {
    base := "http://192.168.1.42:16089"
    client := &http.Client{Timeout: 5 * time.Second}

    for {
        resp, err := client.Post(base+"/api/v1/metrics",
            "application/json", bytes.NewReader([]byte("{}")))
        if err != nil { fmt.Println(err); time.Sleep(time.Second); continue }
        var m Metrics
        if err := json.NewDecoder(resp.Body).Decode(&m); err != nil {
            fmt.Println(err); resp.Body.Close(); continue
        }
        resp.Body.Close()
        fmt.Printf("CPU=%.1f%%  RX=%d  TX=%d\n", m.CPUOverall, m.NetRxBps, m.NetTxBps)
        time.Sleep(time.Second)
    }
}
```

### 8.5 Prometheus / 监控系统接入

服务原生不暴露 `/metrics` Prometheus 文本格式（避免引入额外依赖）。可以用一段 5 行的 sidecar 适配：

```python
# prometheus_exporter.py — 把 pc-specs JSON 转成 Prometheus 文本
from flask import Flask, Response
import requests

app = Flask(__name__)
BASE = "http://localhost:16089"

@app.route("/metrics")
def metrics():
    m = requests.post(f"{BASE}/api/v1/metrics", json={}, timeout=5).json()
    lines = [
        f"pc_specs_cpu_overall {m['cpu_overall']}",
        f"pc_specs_mem_used_bytes {m['mem_used_bytes']}",
        f"pc_specs_mem_total_bytes {m['mem_total_bytes']}",
        f"pc_specs_net_rx_bps {m['net_rx_bps']}",
        f"pc_specs_net_tx_bps {m['net_tx_bps']}",
        f"pc_specs_disk_read_bps {m['disk_read_bps']}",
        f"pc_specs_disk_write_bps {m['disk_write_bps']}",
    ]
    return Response("\n".join(lines) + "\n", mimetype="text/plain")
```

---

## 9. 常见问题

**Q: 端口被占用怎么办？**  
A: 服务启动时 bind 失败会写一条 `warn` 日志，主程序继续运行。检查是否有上一个 pc-specs 实例没退干净，或被其他进程占用。当前版本不支持改端口（约定优于配置）。

**Q: 为什么首次 `/api/v1/metrics` 各 BPS 都是 0？**  
A: 速率类指标需要"上一次"基线做差才能算，第一次请求还没基线，所以返回 0；从第二次起返回真实速率。

**Q: 我能拿到 GPU 的实时利用率吗？**  
A: NVIDIA 在 Linux/Windows 通过 nvidia-smi 命中；其他厂商 / 平台多数返回 `null`（API 层面不会缺字段）。

**Q: snapshot 端点比 metrics 端点慢很多？**  
A: 是的。snapshot 包含 sensors（WMI / system_profiler 调用）、dev_env（探测多个外部命令），首次冷调用最慢可能 1–3 秒；3 秒缓存内复用。**高频轮询请用 `/api/v1/metrics`。**

**Q: 我能改这个端口吗？**  
A: 当前版本写死 16089。如有强烈需求请提 issue。

---

## 10. 兼容性

- **API 版本**：当前 `v1`。在 `v2` 出现前，**只增字段、不删字段、不改语义**。  
- **字段缺失**：未来新增的字段在旧客户端解析时会被忽略。请用宽容的 JSON 解析器（默认行为基本就是这样）。  
- **HTTP 状态码**：稳定承诺。  
- **响应字段命名**：稳定承诺（永远 `snake_case`）。

---

*Last updated: 2026-05-05 · pc-specs `v0.1.x`*

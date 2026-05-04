# PC Specs · 产品设计文档

> 一款面向游戏发烧友与编程极客的跨平台（Windows / macOS / Linux）电脑配置查看与实时监控软件。
> 内部代号：`pc-specs`。

---

## 1. 产品定位

### 1.1 一句话定义
> "你的电脑里到底装了什么？" —— 用一个安装包，在三大主流桌面系统上看清自己电脑的全部硬件、软件、运行时状态。

### 1.2 核心价值
- **看得全**：硬件、固件、外设、网络、显示器、电源、传感器、开发工具链一站式呈现。
- **看得准**：直接调用系统底层 API（WMI / IOKit / sysfs / DMI）获取一手数据，不依赖人工录入。
- **看得爽**：深色极客风 UI，信息密度高、动效克制、响应实时。
- **看得明白**：把 "Intel Core i7-13700K @ 5.4GHz" 解读成 "P-core×8 + E-core×8，最高睿频 5.4GHz，TDP 125W，PL2 253W"。
- **能带走**：一键导出 Markdown / JSON / 长截图，支持复制到剪贴板分享给朋友、贴在论坛、提工单给客服。

### 1.3 目标用户画像

| 用户类型 | 关注点 | 典型场景 |
|---------|--------|---------|
| **游戏发烧友** | GPU、温度、帧率瓶颈、超频参数、内存频率/时序 | 装机后验机、买二手卡前对比、游戏卡顿排查 |
| **DIY / 装机党** | 主板型号、BIOS 版本、SPD 信息、风扇曲线、PCIe 通道 | 选购升级件、确认是否被 JS（奸商） |
| **编程极客** | CPU 微架构、缓存、虚拟化、Node/Python/Rust/Go 版本、IDE、容器 | 排查环境问题、向同事 / AI 截图自己的环境 |
| **生产力 / 设计师** | 内存容量、显存、SSD 寿命、外设色域 | 项目交付前体检、出差前确认设备状态 |
| **运维 / 极客玩家** | 完整系统快照、SMART、系统日志摘要 | 远程诊断他人机器、跨平台批量盘点 |

### 1.4 与同类竞品的差异
- **CPU-Z / GPU-Z**：单一平台、单一组件，只能 Windows，UI 老旧。
- **HWiNFO / AIDA64**：信息全但收费 / 复杂、Windows 独占。
- **Neofetch / Fastfetch**：终端命令行、信息浅。
- **iStat Menus**：仅 macOS、偏监控。
- **PC Specs（本产品）**：Tauri 原生跨平台 + 现代 UI + 数据深度 + 极客向解读 + 开发者环境探测。

---

## 2. 功能模块

### 2.1 一级导航（左侧 Sidebar）
1. **Dashboard 概览**：一张图看完整机配置（封面式布局）。
2. **CPU 处理器**：完整 CPUID、缓存层级、指令集、虚拟化、调频、负载、温度。
3. **GPU 显卡**：所有显卡、显存、驱动、PCIe 链路、显示输出、实时占用。
4. **Memory 内存**：总量、通道、频率、时序、SPD、各插槽、实时占用 / 交换。
5. **Storage 存储**：所有磁盘、文件系统、占用、SMART（可用时）、读写速率。
6. **Motherboard 主板**：厂商、型号、BIOS 版本/日期、芯片组、PCIe 总线树。
7. **OS 操作系统**：发行版、内核、init 系统（Linux）、Shell、用户、Uptime、Locale。
8. **Network 网络**：网卡、IP、网关、DNS、Wi-Fi 信号、公网 IP、实时上下行。
9. **Display 显示**：所有显示器、分辨率、刷新率、色深、HDR、缩放。
10. **Sensors 传感器**：温度、电压、风扇、功耗（按可用源聚合）。
11. **Battery 电池**（笔记本）：容量、循环、设计容量、健康度、当前功率。
12. **Peripherals 外设**：USB / 蓝牙设备、摄像头、音频。
13. **Dev Env 开发环境**：编程语言、包管理器、IDE、容器、Git、Shell 配置。
14. **Monitor 实时监控**：多曲线大盘（CPU / GPU / RAM / 网络 / 磁盘 / 温度）。
15. **Export 导出 / 分享**：Markdown / JSON / PNG 长图。
16. **Settings 设置**：刷新频率、单位、语言、主题、传感器源选择。

### 2.2 关键交互场景

#### 场景 A：装机后验机
> 张三新组装了一台主机，启动 PC Specs → Dashboard → 一眼确认 CPU 型号 / 内存频率 / SSD 是否跑在 PCIe 4.0 ×4 / 显卡是否插在 PCIe 主槽。点 "导出长图" → 发到群里。

#### 场景 B：游戏卡顿排查
> 李四玩游戏掉帧。打开 Monitor 页 → 看到 CPU 单核占用 100% 但 GPU 仅 60% → 提示 "CPU 瓶颈"。点 CPU 详情看到温度 96°C → "建议清灰 / 重涂硅脂"。

#### 场景 C：开发者向 AI 求助
> 王五在论坛求助 Python 包安装失败。一键 "复制环境到剪贴板"，粘出 OS / Python / pip / 编译器版本 + 关键环境变量，省去手敲 5 条命令。

#### 场景 D：买二手对比
> 赵六加价收一台二手游戏本。对方启动 PC Specs → 截图 → 实物清单 + SMART 通电 7000 小时 + 电池循环 800 次 + 显卡未魔改 → 安心交易。

### 2.3 极客增强特性（Geek Mode）
开启后增加：
- 显示原始 CPUID（EAX/EBX/ECX/EDX）。
- 显示 PCIe 配置空间 BDF + Vendor/Device ID。
- 显示 ACPI 表（DSDT / SSDT 摘要）。
- 显示 EDID 原始字节解析。
- 显示 NUMA 拓扑（CPU socket → core → thread）。
- 内存 SPD JEDEC / XMP / EXPO Profile 全表。
- 系统调用统计（内核版本 / 加载模块数）。

---

## 3. 信息架构（Information Architecture）

```
PC Specs
├── Dashboard
│   ├── 封面卡（OS Logo + 主机昵称 + 全局健康分）
│   ├── 4×3 配置卡（CPU/GPU/RAM/SSD/NET/...）
│   └── 实时迷你折线（24h 概览）
├── 详情页（一组件一页）
│   └── 标题区 / 关键信息卡 / 详情表 / 实时图 / 极客面板
├── 监控页（全屏多图）
└── 设置 / 导出 / 关于
```

每个详情页统一遵循 **"上盖 → 中表 → 下图 → 极客"** 的版式：
- 上盖：核心型号 + 关键 3~5 项指标（大字、徽章）。
- 中表：完整属性表（Key/Value，可复制单元格）。
- 下图：该组件相关的实时折线（最近 60 秒 / 5 分钟 / 1 小时切换）。
- 极客：折叠区，默认收起，开启 Geek Mode 自动展开。

---

## 4. 数据模型（与 tech.md 对齐）

所有 API 返回字段统一为 **小写 + 下划线**（snake_case），整型走 `u64` / `i64`，浮点走 `f64`，所有大小单位统一为 **字节（bytes）**，频率统一为 **Hz**，温度统一为 **摄氏度（°C）**，时间戳为 **Unix 毫秒**。

顶层快照 `SystemSnapshot` 字段：

| 字段 | 类型 | 说明 |
|------|------|------|
| `timestamp` | u64 | 采样时间戳（毫秒） |
| `host` | HostInfo | 主机基础信息 |
| `os` | OsInfo | 操作系统 |
| `cpu` | CpuInfo | CPU |
| `gpus` | Gpu[] | 多 GPU |
| `memory` | MemoryInfo | 内存 |
| `storages` | Storage[] | 多盘 |
| `motherboard` | MotherboardInfo? | 主板（部分平台不可用） |
| `network` | NetworkInfo | 网络 |
| `displays` | Display[] | 显示器 |
| `sensors` | SensorReading[] | 传感器聚合 |
| `battery` | BatteryInfo? | 电池（无则 null） |
| `peripherals` | Peripheral[] | 外设 |
| `dev_env` | DevEnvInfo | 开发环境 |

> 详细字段表见 `docs/tech.md` §3。

---

## 5. 非功能需求

| 维度 | 目标 |
|------|------|
| **性能** | 启动 < 1.5s，首屏首数据 < 500ms，监控页 1Hz 刷新 CPU 占用 < 2% |
| **体积** | 安装包 < 15MB（Tauri 原生优势） |
| **资源** | 空闲常驻内存 < 80MB |
| **兼容性** | Windows 10 1809+ / macOS 11+ / Ubuntu 20.04+ 及主流发行版 |
| **权限** | 默认无 root；SMART / SPD 等高权限项需要时弹出提权说明，拒绝则 graceful degrade |
| **隐私** | 100% 本地，零联网；公网 IP 查询可选关闭；导出文件用户主动触发 |
| **稳定性** | 任何采集失败必须 fallback 为 `null` 或友好占位，不 panic 整个 UI |
| **可访问性** | 全键盘导航、对比度达 WCAG AA、字号可调 |
| **国际化** | 中 / 英双语，单位（°F / °C、GiB / GB）可切换 |

---

## 6. MVP 范围（首个可用版本）

**MUST HAVE（M1）：**
- Dashboard 概览
- CPU / GPU / Memory / Storage / OS / Network / Display 七个详情页
- 一级实时监控（CPU / RAM / 网络 / 磁盘）
- 导出 Markdown / JSON
- 中英文 + 深色主题

**SHOULD HAVE（M2）：**
- 主板 / 传感器 / 电池 / 外设详情
- 显卡 PCIe / 显存实时
- SMART 信息（基础）
- 长图导出

**NICE TO HAVE（M3）：**
- Geek Mode（CPUID、EDID、SPD raw）
- 开发环境完整探测
- 跑分基准（CPU/MEM 简版）
- 多机对比、配置时光机

本仓库当前迭代覆盖 **M1 + 部分 M2**，模块 / API 结构完整，方便逐步填充。

---

## 7. 风险与对策

| 风险 | 对策 |
|------|------|
| 不同平台 GPU 信息差异巨大（NVIDIA 提供 NVML，AMD/Intel 弱） | 用 wgpu 拿基础适配器信息打底，可选 NVML 增强；缺失字段标记 `unknown` 显示 "—" |
| Linux SMART / 风扇需要 root | 使用读权限优先，需要 root 时通过 `polkit` 弹窗提权；用户拒绝则隐藏面板 |
| macOS 系统对硬件细节封闭 | 使用 `sysctl` + IOKit + `system_profiler` 输出，硬件型号有限处给出 "macOS 限制" 提示 |
| 部分笔记本无独显 / 部分服务器无显示 | 模块 graceful degrade，UI 显示 "无可用 X" |
| 隐私顾虑（"会不会上传我的硬件指纹"） | 开源 + 全功能离线 + 明确的 "网络功能" 开关 + 文档说明 |

---

## 8. 路线图

| 版本 | 内容 |
|------|------|
| v0.1 (MVP) | Dashboard + 7 详情页 + 实时监控基础 + 导出 |
| v0.2 | Geek Mode + 主板/传感器 + macOS / Linux 适配收尾 |
| v0.3 | 开发环境探测 + 多机对比 + 长图导出 + 国际化完善 |
| v0.4 | 跑分基准（轻量） + 系统托盘常驻 + 报表订阅 |
| v1.0 | 公测稳定版，自动更新通道，签名公证 |

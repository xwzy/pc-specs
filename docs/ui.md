# PC Specs · UI 设计文档

> 与 `design.md` / `tech.md` 配套。本文档定义视觉风格、布局栅格、配色、字体、组件库、典型页面线框、交互动效、响应式。

---

## 1. 设计原则

1. **极客优先（Geek First）**：信息密度大、字段多，但通过卡片化与折叠让小白也能扫读关键。
2. **数据是主角（Data is the Hero）**：所有装饰服务于数据；不做无意义动效。
3. **暗色为本（Dark by Default）**：游戏 / 编程场景常驻暗色；浅色只是补充。
4. **一致大于自由（Consistency > Customization）**：所有页面"上盖 / 中表 / 下图 / 极客"四段式版式。
5. **单位明确（Unit-Aware）**：温度、字节、频率统一显示带单位的数值，避免歧义。
6. **渐进披露（Progressive Disclosure）**：高级字段折叠在 "Geek Panel"，默认隐藏。

---

## 2. 视觉系统

### 2.1 主题与色板

**深色（默认）**：受赛博朋克、终端美学启发，整体偏冷蓝绿，关键信息走青色高亮。

| Token | HEX | 用途 |
|------|-----|-----|
| `--bg-base` | `#0a0d12` | 应用底色 |
| `--bg-surface` | `#11151c` | 卡片底 |
| `--bg-surface-2` | `#171c25` | 弹层 / 深一层卡片 |
| `--bg-elevated` | `#1d2330` | hover / 选中 |
| `--border` | `#222a36` | 分割线 / 卡片边 |
| `--border-strong` | `#2c3645` |  |
| `--text-primary` | `#e7ecf3` | 主文字 |
| `--text-secondary` | `#9aa6b6` | 次要文字 |
| `--text-tertiary` | `#5b6678` | 占位 / 单位 |
| `--accent` | `#22d3ee` | 主强调（青） |
| `--accent-2` | `#a78bfa` | 次强调（紫） |
| `--success` | `#34d399` | OK / 健康 |
| `--warning` | `#fbbf24` | 警告 |
| `--danger` | `#f87171` | 危险 / 失败 |
| `--info` | `#60a5fa` |  |

**浅色（备用）**：以白底 + 深字 + 同色相 accent 提供，对应 token 名相同。

### 2.2 字体

| 用途 | 字体栈 |
|------|--------|
| UI 文字 | `Inter, "Segoe UI Variable", "PingFang SC", "Noto Sans CJK SC", system-ui` |
| 数据 / 等宽 | `"JetBrains Mono", "Cascadia Code", "Menlo", ui-monospace` |
| 标题 | 同 UI，`font-weight: 600` |

字号阶梯（rem，基准 16px）：

| Token | 大小 | 行高 |
|-------|-----|------|
| xs | 12 / 0.75 | 1.4 |
| sm | 13 / 0.8125 | 1.5 |
| base | 14 / 0.875 | 1.55 |
| lg | 16 / 1 | 1.5 |
| xl | 18 / 1.125 | 1.4 |
| 2xl | 22 / 1.375 | 1.3 |
| 3xl | 28 / 1.75 | 1.2 |
| metric | 36 / 2.25 | 1.1 (等宽) |

### 2.3 形状 / 间距

- 圆角：卡片 `12px`、按钮 `8px`、徽章 `6px`、内联标签 `4px`。
- 间距阶梯：`4 / 6 / 8 / 12 / 16 / 20 / 24 / 32 / 40 / 56 / 72`。
- 阴影：暗色场景几乎不用阴影，靠边框区分；弹层用 `0 8px 24px rgba(0,0,0,.45)`。
- 栅格：12 列，gutter 16，max-width `1440px`，超过居中。
- 图表线宽 `1.5px`，网格线 `--border` 0.4 透明度，避免抢戏。

---

## 3. 全局布局（AppShell）

```
┌──────────────────────────────────────────────────────────────────┐
│  Topbar  (40px)  · 主机名 · 全局健康分 · 刷新指示 · 主题切换 · 设置  │
├──────────┬───────────────────────────────────────────────────────┤
│          │                                                       │
│ Sidebar  │                Content (24px padding)                 │
│ (240px)  │                                                       │
│          │                                                       │
│  · 概览  │                                                       │
│  · CPU   │                                                       │
│  · GPU   │                                                       │
│  · Mem   │                                                       │
│  · ...   │                                                       │
│          │                                                       │
│  Footer  │                                                       │
│  v0.1    │                                                       │
└──────────┴───────────────────────────────────────────────────────┘
```

- Sidebar：图标 + 文字（图标取自 lucide-react，主题化），当前项左侧 2px 青色高光条 + 卡片背景。
- Topbar：左侧主机名 + 三段健康灯（温度 / 存储 / 内存），右侧主题切换、Geek Mode 开关、设置入口。
- 折叠：宽度 < 1100 时 Sidebar 自动折叠为 64px 图标条，hover 弹出文字。

---

## 4. 通用组件

### 4.1 `<Card>` 数据卡片
- 背景 `--bg-surface`；标题 + 可选右上角操作；内部 16/20 padding。
- 变体：`muted`（无边框），`accent`（左侧 2px 青色边）。

### 4.2 `<Stat>` 大数指标
```
TOP LABEL（uppercase, --text-tertiary, xs, letter-spacing .08em）
36px metric value（等宽，--text-primary）
SUB LABEL（次行，sm，--text-secondary）
```
可附 `<Trend +1.2%>` / `<Badge>`。

### 4.3 `<KeyValueTable>` 键值表
- 双列；左 `--text-secondary`、右 `--text-primary` 等宽。
- 长字符串自动 `truncate` + Tooltip 显示完整 + 单击复制。

### 4.4 `<Bar>` 进度条
- 高 6px，左圆角，背景 `--bg-elevated`，前景 accent；
- 危险阈值（>85%）转 `--warning`，>95% 转 `--danger`。

### 4.5 `<RingProgress>` 环形指示
- 直径 96 / 64 两个尺寸；中心两行：metric + label。
- 用于 CPU 总占用、内存、磁盘单盘。

### 4.6 `<Spark>` 迷你折线
- 36px 高、无网格、无坐标轴、accent 渐变填充；用于卡片右下角。

### 4.7 `<LineChart>` 主图
- 暗背景 + 微网格；横轴时间、纵轴数值；多曲线时图例在顶部右对齐；
- 悬停十字线 + Tooltip（深色卡片，等宽数值）。
- 监控页主图高 220px，详情页 160px。

### 4.8 `<Badge>` 徽章
- 圆角 6，padding 2/8，xs 字号；颜色：default / accent / success / warning / danger。
- 用于：架构标签（x86_64）、平台标签（PCIe 4.0 ×16）、状态（OK / Throttle）。

### 4.9 `<Section>` 折叠节
- 标题 + 右上角 chevron；动画 150ms ease-out 高度展开。
- 用于"Geek Panel"。

### 4.10 `<Topbar Health Light>` 健康指示
- 三个圆点（温度 / 存储 / 内存），颜色根据阈值变化；hover 显示当前数值。

---

## 5. 页面线框

### 5.1 Dashboard 概览（封面式）

```
┌──────────────────────────────────────────────────────────────────┐
│  COVER（高 200px）                                                │
│  ┌─────────────────────────────────────────────────────────────┐ │
│  │ [OS LOGO]  hostname.local              v0.1     ⚙ Geek      │ │
│  │ macOS Sonoma 14.5  ·  Up 3d 12h        Health 92  ☀         │ │
│  │ MacBook Pro 16 (2023) — Apple M3 Max                        │ │
│  └─────────────────────────────────────────────────────────────┘ │
│                                                                   │
│  GRID 12 列                                                       │
│  ┌── CPU (4列) ──────────┐ ┌── GPU (4列) ─────────┐ ┌─ MEM (4列)─┐│
│  │  Apple M3 Max         │ │ Apple M3 Max GPU      │ │ 64 GB LPDDR5 ││
│  │  16 cores · 5.4 GHz   │ │ 40 cores · Metal      │ │ 38 / 64 GB ▓▓││
│  │  ~~~~~~~~~~~~~~ 24%   │ │ ~~~~~~~~~~~~~~~ 17%   │ │ Used 38 GB    ││
│  └───────────────────────┘ └───────────────────────┘ └──────────────┘│
│                                                                   │
│  ┌── Storage (6列) ──────────────────┐ ┌── Network (6列) ─────────┐ │
│  │ 1 TB NVMe   620 / 1024 GB ▓▓▓▓░  │ │ Wi-Fi 6E · 1.2 Gbps      │ │
│  │ Read 12 MB/s  Write 4 MB/s       │ │ ↓ 2.4 MB/s  ↑ 0.3 MB/s   │ │
│  └──────────────────────────────────┘ └──────────────────────────┘ │
│                                                                   │
│  ┌── Sensors (12列) ──────────────────────────────────────────────┐ │
│  │ CPU 56°C   GPU 52°C   SSD 41°C   Fan1 1240 RPM ...             │ │
│  └────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────┘
```

每个卡片右下角 24px 高 `<Spark>` 显示最近 60 秒该指标。

### 5.2 CPU 详情页

```
[← Back]  CPU
─────────────────────────────────────────────
TOP COVER:
  Apple M3 Max                              [arch: arm64] [P-cores: 12] [E-cores: 4]
  Base 4.06 GHz · Max 4.06 GHz · 16 logical · L2 16MB · L3 32MB

GAUGES:
  ⊙ 24% Overall   |  ⊙ 56°C Temp  |  ⊙ 4.05 GHz Now

USAGE PER CORE (柱图，16 列)：
  ▍▍▆▆▍▍▎▎▎▎▍▍▎▎▎▎ ...

KEY VALUE TABLE：
  Vendor              Apple
  Brand               Apple M3 Max
  Architecture        arm64
  Physical / Logical  12P + 4E / 16
  Base / Max          4.06 GHz / 4.06 GHz
  L1 / L2 / L3        — / 16 MB / 32 MB
  Virtualization      yes
  Features            neon, fp16, sve, dotprod, ...

LIVE LINE CHART（180px 高）：
  CPU 占用 + 各核占用切换  · 60s / 5m / 10m

GEEK PANEL（折叠）：
  · Topology（NUMA / socket 树状）
  · 完整指令集列表
  · 频率历史（如可用）
```

### 5.3 GPU 详情页
- 多卡时上面切换 Tabs；
- 上盖：型号 + Backend + 显存条 + 利用率环；
- 表：VendorID/DeviceID、PCIe 链路、驱动版本、显示输出口；
- 图：利用率 + 显存 + 温度 + 功耗（功耗如不可用则隐藏）。

### 5.4 Memory 详情
- 上盖：总量、已用环、通道数、频率；
- 表：每个内存槽（按行展开）：Slot / Manufacturer / Part / Capacity / Speed / Type；
- 图：内存使用 + Swap 使用双线。

### 5.5 Storage 详情
- 列表式：每个磁盘一卡，左侧型号 / 容量 / 健康徽章；右侧已用条 + 读写速率 mini spark；
- 点击磁盘进入抽屉显示 SMART（可用时）。

### 5.6 Network 详情
- 表头：默认网关 / DNS / 公网 IP（如开启）；
- 各网卡卡片：IPv4/IPv6、MAC、链路速度徽章、实时 ↑↓。

### 5.7 Display 详情
- 网格卡片：每显示器一个矩形，按相对尺寸缩放绘制 + 主屏角标；右侧分辨率 / 刷新率 / 缩放 / 物理尺寸。

### 5.8 Monitor 实时大盘
```
顶部：刷新频率 [500ms | 1s | 2s | 5s]   暂停 ▮▮  清空
GRID:
  CPU Overall (单图，全宽)
  CPU Per-Core (柱图，全宽，bar 流动)
  RAM Used vs Total (双线)
  Network Rx/Tx (双线)
  Disk Read/Write (双线)
  GPU Utilization (多线)
  Sensors (温度堆叠)
```
每个图独立时间轴，鼠标悬停联动十字线。

### 5.9 DevEnv 开发环境
- 分组卡片：Languages / Package Managers / VCS / Editors / Containers / Shells；
- 每条目：图标 + 名称 + 版本徽章 + 路径（小字省略）；
- 顶部一键 "复制环境到剪贴板"。

### 5.10 Export 导出
- 三个大按钮：Markdown / JSON / 复制到剪贴板；
- 中部预览面板（代码视图）；
- 右上角"包含敏感信息"开关（默认关）。

### 5.11 Settings 设置
- 区块：通用 / 单位 / 网络 / 监控 / 主题 / 关于。
- 单位：温度 °C/°F；字节 GiB/GB；速度 MB/s/Mbps。
- 网络：是否查询公网 IP（默认关）。
- 监控：默认间隔、保留点数。
- 主题：暗 / 亮 / 跟随系统；强调色（青 / 紫 / 绿）。

---

## 6. 状态与反馈

- **加载**：骨架屏（带等宽数字虚位 "—"），不旋转 spinner。
- **空状态**：图标 + 一句话 + 次要按钮（如 "授权读取 SMART"）。
- **错误**：行内 Tooltip + 红点；不弹窗打断。
- **离线**：仅当设置开启网络功能时才会出现"连接失败"，其他场景没有联网概念。

---

## 7. 动效

- 数值变化：250ms `ease-out` 数字滚动；超过 ±20% 时带 0.4s accent → 默认色淡入。
- 折叠：150ms `ease-out`。
- 路由切换：内容区 100ms 透明度淡入 + 6px 上滑。
- 实时图：requestAnimationFrame 节流，最高 60fps，但默认对齐采样频率（节能）。

---

## 8. 响应式断点

| 断点 | 表现 |
|------|------|
| ≥ 1280 | 12 列舒展 |
| 1024–1280 | 9 列；某些 4-列卡变 6 列 |
| 768–1024 | Sidebar 自动折叠；卡片按 6 列编排 |
| < 768 | 极少触发（桌面应用），Sidebar 隐藏，顶栏汉堡菜单 |

---

## 9. 可访问性

- 颜色对比 ≥ 4.5:1（已用对比测试器校验主要对）。
- Focus ring：2px `--accent` outline，offset 2px。
- 全键盘导航：tab 顺序为 Sidebar → Topbar → Content；
- 重要操作均有 aria-label；表格使用 `<table role="table">`。

---

## 10. 资源与图标

- 图标：lucide-react（自带 Tree shaking）。
  - CPU = `Cpu`、GPU = `MonitorCog`/`Gpu`（用 `MonitorPlay` 替）、内存 = `MemoryStick`、存储 = `HardDrive`、网络 = `Network`、显示 = `Monitor`、传感器 = `Thermometer`、电池 = `BatteryFull`、外设 = `Usb`、开发 = `Terminal`、监控 = `Activity`、导出 = `Share2`、设置 = `Settings`、Geek = `Sparkles`。
- 应用 Logo：单字 `S` 缩写在青色圆角矩形内（占位，正式版替换）。
- 不引入第三方图片资源；所有图形通过 SVG 内联或 CSS 绘制。

---

## 11. 文案风格

- 中文：简洁名词性短句；术语保留英文（CPU / PCIe / DDR5）。
- 英文：title case for headings, sentence case for body。
- 数字附单位；未知字段统一显示半角破折号 `—`（U+2014）。
- 提示语避免威胁式文案；错误描述给出"可能原因 + 建议"。

---

## 12. 与开发任务对齐

| UI 模块 | 主要组件 | 数据来源（command） |
|---------|---------|--------------------|
| Topbar 健康灯 | `Topbar` | `get_full_snapshot` 摘取 |
| Sidebar | `Sidebar` | 静态路由 |
| Dashboard 卡片 | `<StatCard>`、`<Spark>` | `get_full_snapshot` + `monitor://tick` |
| CPU 详情 | `<Cover>`, `<KeyValueTable>`, `<LineChart>`, `<Section Geek>` | `get_cpu` + tick |
| Monitor 大盘 | `<LineChart>` × N | `start_monitor` + tick |
| Export | `<CodePreview>` + 按钮 | `export_markdown / export_json` |


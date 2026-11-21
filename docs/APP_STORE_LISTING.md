# App Store 上架文案（ASC 逐字段对照）

> 对应「Mac App Store 版」（`--features appstore`，功能范围见 `docs/APP_STORE_COMPLIANCE.md` 第 1 节）。本文件提供中英文商店资料与审核说明，逐字段复制到 App Store Connect；括号内是字段说明与字数限制。
> 主要语言为简体中文，另加 English (U.S.) 本地化。提到的网址托管在 Cloudflare Pages（`https://pc-specs.xwzy.dev/`，页面源码在 `site/`，部署步骤见 `docs/RELEASE_CHECKLIST.md` §2）；**域名和联系邮箱是按另外几个项目的习惯假定的**，换了要同步改 `crates/app/src/links.rs` 的 `WEBSITE`、`site/` 里的页面和下面的网址。
> 文案里只写商店版真有的功能：没有传感器温度、进程列表、清理、开发环境、启动项、应用管理、联网和本机接口服务。App Store Connect 会拒绝推广文本和描述里的 `⌘` 等特殊符号，所以快捷键一律不写。

## 基本信息

| 字段 | 填写 |
|---|---|
| 平台 | macOS |
| Bundle ID | `com.xwzy.pc-specs` |
| SKU | `pcspecs100` |
| 主要语言 | 简体中文（另加 English (U.S.)） |
| 最低系统版本 | macOS 13.0（`scripts/lib/common.sh` 的 `MACOS_APPSTORE_MIN_VERSION`，已写进 Info.plist；`SMAppService` 登录项要 13.0） |
| 版本号 / 构建号 | 根 `Cargo.toml` 的 `[workspace.package].version`（当前 `0.2.0`）/ `[workspace.metadata.appstore].build`（当前 `1`）。首次上架想用 `1.0.0` 的话，先 `scripts/version.sh set 1.0.0` |
| 版权（Copyright） | `2026 xwzy` |
| 许可协议 | 使用 Apple 标准 EULA |

## 简体中文（zh-Hans，主要语言）

### 名称（App Name，30 字符）

```
PC Specs - 电脑配置与实时监控
```

名称在 App Store 全局唯一，如果提示已被占用，按顺序换成：

```
PC Specs: 配置查看与系统监控
PC Specs 电脑配置监控
PC Specs - 硬件信息与监控
```

### 副标题（Subtitle，30 字符）

```
硬件信息、实时监控、磁盘空间分析
```

### 宣传文本（Promotional Text，170 字符，可随时更新，无需提审）

```
一眼看清这台 Mac 的全部配置；顶栏实时监控 CPU、内存、网速；用矩形树图和旋转图找出占满磁盘的大文件与重复文件。完全离线，不收集任何数据，界面支持简体中文与 English。
```

### 描述（Description，4000 字符）

```
PC Specs 是一款 Mac 上的配置查看与实时监控工具：打开就能看到这台 Mac 的全部硬件信息和运行状态，菜单栏里随时盯着 CPU、内存和网速，还能把整块磁盘画成一张图，一眼找出是谁占了空间。界面支持简体中文和 English，跟随系统语言。

【概览】
· 一屏看清 CPU、内存、显卡、存储、网络和散热状态，有没有异常，一句话告诉你
· 每张卡片都带实时走势，点一下直接进入对应的详情页
· 下方是硬件配置摘要：处理器、内存、显卡、主存储、主板、显示器

【硬件详情】
· 处理器：型号、核心数、频率、缓存，以及每个核心的实时占用
· 显卡、内存、存储、主板、显示器、网络、电池、外设、操作系统，逐项展开
· 内存构成一目了然：应用内存、联动内存、已压缩、缓存文件、空闲，另有内存压力和换页速度
· 存储显示每块磁盘的容量、剩余空间和实时读写；电池显示电量、健康度和充放电循环次数
· 网络页有 Wi-Fi 的信号、信噪比、信道和连接速率，以及今天、7 天、本月的流量统计
· 系统没有提供的信息会如实写明，不会编数字

【实时监控】
· CPU、内存、网络、磁盘读写和显卡的实时走势图，每个 CPU 核心单独显示
· 可暂停、清空，历史时长在 1、5、10 分钟之间切换

【顶栏监控】
· 在菜单栏里放上 CPU、内存、存储、网速，另可加上显卡和电池
· 每一项可以选双行、单行、趋势图或环形样式，顺序、标签、颜色都能调，设置页里实时预览
· 点开任意一项，弹出详细面板：CPU 每核心负载、内存构成与压力、磁盘卷、网络上下行与 Wi-Fi 信号，并能查看 1 小时、24 小时、7 天、30 天的走势

【监控大屏】
· 把实时状态铺满另一块显示器的看板：15 套配色、5 种布局，横屏、竖屏、超宽屏自动适配
· 可设置启动时自动打开，开着的时候保持屏幕常亮

【存储空间分析】
· 选一块磁盘或任意文件夹，扫描成一张可交互的图：矩形树图，或者旋转图，色块越大占用越多
· 按文件类型着色，点图例即可过滤；双击进入文件夹，面包屑一键返回
· 列表里有最大文件榜、当前文件夹、重复文件（按内容比对），以及和上次扫描相比什么变大了；可按名称、大小、未修改时间筛选
· 选中后可预览、在访达中显示、复制路径，或加入「收集箱」：核对好总量，一次性移到废纸篓
· 扫描只读取文件大小等信息，不会修改任何文件；可导出 CSV

【磁盘速度测试】
· 在系统盘上测顺序读写和 4K 随机读写，测完自动删除测试文件，并和上一次结果对比

【规则与通知】
· 自己定义：哪个读数、超过多少、持续多久，就发一条系统通知，带冷却时间，不会刷屏
· 内置 CPU、内存、磁盘三条默认规则，一键恢复

【导出与设置】
· 导出 Markdown、JSON 或 PNG 长图；默认隐去 MAC 地址、序列号、主机名等敏感信息
· 深色、浅色或跟随系统；容量、网速单位可选；采样间隔 0.5 秒到 5 秒
· 可选开机自启，只留在菜单栏，不弹窗口；设置可以导出，换一台 Mac 导入即可

【隐私】
· 完全离线：没有账号、没有广告、没有统计，应用本身也没有联网权限
· 在 macOS 沙盒中运行，只访问你在对话框里亲自选择的文件夹

看一眼配置、盯着菜单栏里的数字、清理一块快满的磁盘，PC Specs 让这些事都变得轻松。
```

### 关键词（Keywords，100 字符，逗号分隔，不要空格）

```
系统信息,硬件检测,性能,菜单栏,网速,CPU,GPU,内存,电池健康,重复文件,大文件,空间清理,树图,磁盘测速,配置
```

> 名称、副标题、类别里已有的词（电脑、配置、实时监控、硬件信息、磁盘、空间分析、工具）App Store 会自动索引，不必重复占用关键词字数。故意没有写任何其他软件或品牌的名字（准则 2.3.7）。

### 版本说明（What's New）

首次上架的版本页没有「版本说明」字段；下面是第一个更新版本可以用的写法，先留着：

```
· 新增……（按实际改动填写）
```

### 网址

| 字段 | 网址 |
|---|---|
| 技术支持网址（必填） | `https://pc-specs.xwzy.dev/support.html` |
| 营销网址（选填） | `https://pc-specs.xwzy.dev/` |
| 隐私政策网址（必填） | `https://pc-specs.xwzy.dev/privacy.html` |

## English (U.S.)（en-US，新增本地化）

### Name（30 characters）

```
PC Specs - System Monitor
```

If the name is taken, try in this order:

```
PC Specs: Hardware Monitor
PC Specs - Mac Hardware Monitor
PC Specs Hardware Info
```

### Subtitle（30 characters）

```
Hardware info and live monitor
```

### Promotional Text（170 characters，可随时更新，无需提审）

```
See every spec of your Mac at a glance. Live CPU, memory and network in the menu bar, plus a map of what fills your disk. Fully offline, no data collected.
```

### Description（4000 characters）

```
PC Specs is a hardware info and live monitoring tool for your Mac. See every spec and the current state of your machine, watch CPU, memory and network speed from the menu bar, and turn any folder or disk into a map of what is taking up the space. Available in English and Simplified Chinese; follows your system language.

OVERVIEW
- CPU, memory, GPU, storage, network and thermal state on one screen, with a one-line verdict on whether anything is wrong
- Every card has a live chart; click it to jump to the full details
- A hardware summary below: processor, memory, graphics, main storage, logic board, displays

HARDWARE DETAILS
- Processor: model, core counts, frequency, caches, and live load for every core
- Graphics, memory, storage, logic board, displays, network, battery, peripherals and operating system, one page each
- Memory at a glance: app memory, wired, compressed, cached files and free, plus memory pressure and swap activity
- Storage shows each disk's capacity, free space and live read and write speed; battery shows charge, health and cycle count
- The network page has Wi-Fi signal, noise, channel and link speed, plus traffic totals for today, 7 days and this month
- Anything the system does not provide is labelled as such. Nothing is made up

LIVE MONITOR
- Live charts for CPU, memory, network, disk read and write, and GPU, with every CPU core shown separately
- Pause, clear, or switch the history between 1, 5 and 10 minutes

MENU BAR
- Put CPU, memory, storage and network speed in the menu bar, and optionally GPU and battery
- Choose two-line, single-line, trend or ring style for each item; order, labels and colors are adjustable, with a live preview in the settings
- Click any item for a detail panel: per-core CPU load, memory breakdown and pressure, disk volumes, network upload and download with Wi-Fi signal, and 1 hour, 24 hour, 7 day and 30 day history

BIG SCREEN
- A dashboard that fills another display: 15 color themes and 5 layouts that adapt to landscape, portrait and ultrawide screens
- Can open when the app starts, and keeps the screen awake while it is showing

STORAGE ANALYSIS
- Pick a disk or any folder and see it as an interactive map: a treemap or a sunburst, where bigger blocks mean more space
- Colored by file type, click the legend to filter; double-click to go into a folder, breadcrumbs to come back
- Lists of the largest files, the current folder, duplicate files (matched by content) and what grew since the last scan; filter by name, size or age
- Preview a file, show it in Finder, copy its path, or add it to the collector: check the total, then move everything to the Trash in one step
- Scanning only reads file sizes and dates and never changes a file. Export the result as CSV

DISK SPEED TEST
- Sequential and 4K random read and write on the startup disk. The test file is deleted afterwards, and results are compared with the previous run

RULES AND NOTIFICATIONS
- Choose a reading, a threshold and how long it must last, and get a system notification; a cooldown keeps it from flooding you
- CPU, memory and disk rules are included, and can be restored with one click

EXPORT AND SETTINGS
- Export Markdown, JSON or a long PNG image; MAC addresses, serial numbers and the host name are hidden by default
- Dark, light or follow the system; choose your units; sampling interval from 0.5 to 5 seconds
- Optional launch at login that stays in the menu bar without opening a window; settings can be exported and imported on another Mac

PRIVACY
- Fully offline: no account, no ads, no analytics, and the app has no network permission
- Runs in the macOS sandbox and only touches folders you choose in a dialog

Check your specs, watch the numbers in the menu bar, or clear out a disk that is almost full. PC Specs makes it easy.
```

### Keywords（100 characters）

```
specs,hardware,monitor,menubar,cpu,gpu,memory,disk,storage,battery,network,duplicates,treemap,sysinfo
```

### What's New

```
(First release has no What's New field.)
```

### URLs

| Field | URL |
|---|---|
| Support URL（必填） | `https://pc-specs.xwzy.dev/en/support.html` |
| Marketing URL（选填） | `https://pc-specs.xwzy.dev/en/` |
| Privacy Policy URL（必填） | `https://pc-specs.xwzy.dev/en/privacy.html` |

> 「App 信息」页里的隐私政策网址也是按语言分开填的：zh-Hans 用 `/privacy.html`，en-US 用 `/en/privacy.html`。

## 类别

- 主类别：工具（Utilities）——与 Info.plist 的 `LSApplicationCategoryType = public.app-category.utilities` 一致
- 副类别（选填）：开发者工具（Developer Tools）或 效率（Productivity）都说得通，不填也行

## 价格与销售范围

- 价格：免费，没有内购
- 销售范围：参照其他项目的做法，可以先排除中国大陆。**中国大陆需要 ICP 备案号**（App 备案），备案完成后再补开

## 年龄分级（Age Rating）

问卷里所有内容项一律选「无 / 否」，结果为 **4+**。

## App 隐私（App Privacy）

- 是否收集数据：**否，不收集数据（Data Not Collected）**
- 追踪：否
- 与 `packaging/macos/appstore/PrivacyInfo.xcprivacy` 一致：不追踪、不收集数据类型；声明的必需理由 API：文件时间戳（3B52.1、C617.1、DDA9.1）、开机时间（35F9.1）、磁盘空间（85F4.1、E174.1）、UserDefaults（CA92.1）

## 截图（Mac，2880×1800，无透明通道，按序上传）

两套各 8 张，文件在 `AppStore/screenshots/zh-Hans/` 和 `AppStore/screenshots/en-US/`（一套 2880×1800 即可，ASC 会自动适配其他 Mac 尺寸）。截图都是在真实应用窗口上截取的，数据是真实运行时的读数；存储分析用的是一个临时造出来的演示文件夹（名字叫 Workspace，里面是随便命名的占位文件），**没有任何个人信息、第三方内容或其他软件的名字**。主机名在截图里换成了通用的 `MacBook-Pro.local`，局域网地址换成了 `192.168.1.23`。

| 顺序 | 文件 | zh-Hans 标题 | en-US title |
|---|---|---|---|
| 1 | `01-overview.png` | 一眼看清你的 Mac | Your Mac at a glance |
| 2 | `02-hardware.png` | 每一项硬件都看得明明白白 | Every component, in detail |
| 3 | `03-realtime.png` | 实时走势，一目了然 | Live charts for everything |
| 4 | `04-menubar.png` | 菜单栏常驻，点开即看 | Always in your menu bar |
| 5 | `05-bigscreen.png` | 监控大屏，副屏变看板 | A dashboard for your second display |
| 6 | `06-storage-map.png` | 找出谁占满了磁盘 | See what fills your disk |
| 7 | `07-duplicates.png` | 重复文件与大文件，清理有依据 | Find duplicates and big files |
| 8 | `08-appearance.png` | 深色、浅色，随你喜欢 | Dark or light, your choice |

## 审核信息（App Review Information）

- 登录信息：不需要（取消勾选「需要登录」）
- 联系人姓名 / 电话 / 邮箱：沿用其他项目已填的内容（只给审核团队看，不公开；电话号码必须带国家区号）
- 附件：不需要。应用不依赖样例文件，审核员选自己电脑上任意文件夹（比如「下载」）就能试存储分析
- 备注（Notes）：

```
PC Specs is fully offline and requires no login. The App Store edition runs in the App Sandbox with only these entitlements: app-sandbox, files.user-selected.read-write and files.bookmarks.app-scope. It has no network entitlement, no Apple Events, no temporary exceptions and no helper tools.

Test steps (the interface follows the system language: English or Simplified Chinese; Settings > Language switches it):
1. Launch the app. The Dashboard page shows live CPU, memory, GPU, storage, network and thermal state. Click a card to open its page. The sidebar has one page per hardware group.
2. "Monitor" shows real-time charts. "Menu bar" (sidebar, Desktop group) is the settings page for the status items: the app adds items (CPU, MEM, DISK, network speed) to the menu bar. Click one in the menu bar to open its detail panel; the 1h / 24h / 7d / 30d tabs show history that builds up over time.
3. "Monitoring Wall" (sidebar, Desktop group) has an "Open Monitoring Wall" button that opens a full dashboard window; the toolbar at the bottom changes theme, toggles full screen and opens its settings.
4. Storage: open the Storage page, choose the "Space analysis" tab, then "Choose a folder…" and pick any folder (for example Downloads). The scan only reads file names, sizes and dates. A treemap appears; switch to the sunburst, double-click a folder to go into it, and use the lists below the map (Largest files, Current folder, Duplicates). In Duplicates, press "Find duplicates". Items can be added to the collector and then moved to the Trash after a confirmation dialog. Files outside the folder you chose are never touched.
5. Storage > "Speed test" runs a read/write benchmark on the startup disk inside the app container and deletes its temporary file afterwards.
6. Rules: expand a rule and press "Try it" to send a test notification (macOS asks for notification permission the first time; the app works without it).
7. Export (toolbar button): "Save to file" writes a Markdown / JSON report to a location chosen in the save panel; MAC address, serial number and host name are redacted by default.
8. Settings > Launch at login uses SMAppService and is off by default.

Hardware and system information is read through public frameworks and the command line tools that ship with macOS (system_profiler, diskutil, ioreg, scutil, networksetup, route, netstat, tmutil, qlmanage and caffeinate), which run inside the sandbox. Nothing is downloaded or installed, no code is generated at run time, and no private API is used.
No camera, microphone, location, contacts or photos access.
Encryption: ITSAppUsesNonExemptEncryption = NO (the App Store edition contains no networking or TLS code).
Privacy policy: https://pc-specs.xwzy.dev/en/privacy.html

中文说明：完全离线，无账号、无联网、不收集数据，在沙盒中运行。请按以上步骤测试（界面会跟随系统语言显示中文或英文）；存储分析时在「选择文件夹…」里选任意一个文件夹即可。商店版没有传感器温度、进程列表和系统清理功能（这些只在官网版里）。
```

## 其他声明

| 项目 | 回答 |
|---|---|
| 内容版权（Content Rights） | 不包含、不显示、不访问第三方内容 |
| 出口合规（加密） | Info.plist 已声明 `ITSAppUsesNonExemptEncryption = NO`，上传后无需再作答 |
| 广告标识符（IDFA） | 不使用 |
| 欧盟 DSA 交易者身份 | 账号级设置，之前的项目已经填过，这里不会再问 |

## TestFlight（可选）

- Beta App 描述：`Mac 配置查看与实时监控。请重点试试：菜单栏里的监控项和点开后的面板、存储分析（选一个文件夹看矩形树图和重复文件）、监控大屏、规则通知、深浅外观。`
- 反馈方式：邮件 `pc-specs@xwzy.dev`

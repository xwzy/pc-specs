# PC Specs

> 跨平台的电脑配置查看与实时监控工具，给游戏玩家和程序员用。
> Windows · macOS · Linux · 原生 GPU 渲染 · 100% 本地。

界面使用 [GPUI Kit](https://gpui-kit.com/)（Zed 的 GPUI 之上的 Rust 原生 UI 框架）；整个应用是一个纯 Rust 程序，不含 WebView 和 JS 运行时。

外部 API：`docs/api.md` · 设计稿：`docs/design.md`、`docs/ui.md`

## 技术栈

- **界面**：GPUI Kit 0.9（gpui-component + 主题 / 图标 / 图表），`rust-i18n` 做中英文
- **采集层**：`crates/core`，与界面完全解耦。以 sysinfo 为主，再按平台补专用路径（Windows WMI / Win32，macOS sysctl / system_profiler / ioreg，Linux sysfs / DMI）
- **系统托盘**：GPUI 没有托盘 API，用 `tray-icon`（Linux 走 StatusNotifierItem，不依赖 GTK）
- **本地 HTTP 采集服务**：axum，默认 `0.0.0.0:16089`，可在设置里关闭或改为仅本机

## 目录结构

```
pc-specs/
├── crates/core/      # 数据采集、导出（Markdown / JSON）、本地 HTTP 服务
├── crates/app/       # GPUI 应用：页面、托盘、悬浮窗、海报导出
├── assets/icons/     # 各平台图标（source.png 是母版，scripts/gen_icons.py 从它生成其余）
├── packaging/        # 各平台的打包素材：macos/（Info.plist）、linux/（desktop、AppStream、rpm spec）、windows/（Inno Setup 脚本）
├── scripts/          # 版本管理、各平台的编译 / 打包 / 签名脚本，见下面「打包与发布」
├── .github/workflows/release.yml   # 手动触发的发布工作流
└── docs/             # 文档
```

## 开发

```bash
cargo run -p pc-specs              # 开发运行（框架依赖已开优化，不会卡）
cargo check --workspace            # 仅编译校验
cargo build --release -p pc-specs  # 发布构建
scripts/build-and-install.sh       # 编译发布版并装到本机（macOS 装成 /Applications/PC Specs.app，Linux 装到 ~/.local），装完自动启动
```

Rust 需要 1.92+。Linux 的编译依赖用 `scripts/linux/install-deps.sh` 安装（Debian / Ubuntu 和 Fedora / RHEL）。

运行时 Linux 需要可用的 Vulkan 驱动与 Wayland / X11 会话。发行包在 Ubuntu 22.04（glibc 2.35）上构建，Ubuntu 22.04+、Debian 12+、Fedora 36+ 及同期的发行版可以运行。deb / rpm 会自动安装依赖库；tar.gz 和 AppImage 不带库，需要系统里有 libvulkan、libxkbcommon-x11、libwayland-client、libX11-xcb、libfontconfig。

## 打包与发布

### 版本号

唯一的来源是根 `Cargo.toml` 的 `[workspace.package].version`：各个 crate、应用里显示的版本、所有安装包和发布的 tag 都从它来，不要在别处写版本号。用 `scripts/version.sh` 管理：

```bash
scripts/version.sh                # 看当前版本
scripts/version.sh set 0.3.0      # 设置版本号（同步 Cargo.lock）；预发布写成 0.3.0-rc.1
scripts/version.sh bump minor     # patch | minor | major
scripts/version.sh check          # 检查各处版本一致、Cargo.lock 没有落后
```

### 本地编译与打包

每个平台一套脚本，产物都在 `dist/`。签名、公证用的环境变量和选项见各脚本的 `--help`（Windows 是 `Get-Help`）。

| 平台 | 编译 | 打包 |
| --- | --- | --- |
| macOS | `scripts/macos/build.sh [--arch universal\|arm64\|x64]` | `scripts/macos/package.sh` → `.dmg`（Developer ID 签名 + 公证 + 盖章） |
| Linux | `scripts/linux/build.sh` | `scripts/linux/package.sh` → `.tar.gz` / `.deb` / `.rpm` / `.AppImage` |
| Windows | `scripts\windows\build.ps1 [-Arch x64\|arm64]` | `scripts\windows\package.ps1` → 安装程序（Inno Setup）+ 便携 `.zip`，exe 和安装程序都签名 |

`scripts/build.sh [--platform macos|linux] [--arch ...] [--package]` 按本机系统和目标平台自动选编译方式。能在哪里编译：

- **macOS**：只能在 Mac 上；Apple 芯片的 Mac 也能编 Intel 和 universal。
- **Linux**：在 Linux 上原生编译；在 macOS 等其他环境，或要编别的架构时用 Docker：`scripts/linux/docker.sh --arch x64|arm64`。
- **Windows**：只能在 Windows 上编译（GPUI 要用 Windows SDK 里的 `fxc.exe` 编着色器，没法交叉编译）。

`scripts/checksums.sh dist` 为 `dist/` 里的文件生成 `SHA256SUMS.txt`；设置了 `GPG_PRIVATE_KEY` 时再加上 GPG 签名。Linux 的 deb / AppImage / tar.gz 本身没有系统级签名，用户靠它校验下载。

不带任何证书时也能打包：macOS 退到 ad-hoc 签名（本机能运行，发给别人会被 Gatekeeper 拦下），Windows 和 Linux 打出未签名的包。

### 发布到 GitHub

`.github/workflows/release.yml` 只能手动触发，提交、PR、tag 都不会触发它：

1. `scripts/version.sh set X.Y.Z`，提交并推送到默认分支。
2. GitHub 上进入 Actions → Release → Run workflow（或 `gh workflow run release.yml`）。
3. 它在 5 个原生 runner 上（macOS arm64、Linux x64 / arm64、Windows x64 / arm64）编译、签名、打包，汇总成一个 tag 为 `vX.Y.Z` 的 Release，并附上 `SHA256SUMS.txt`。

运行时可选：`dry_run`（只编译打包，不创建 Release）、`require_signing`（默认勾上：缺签名 secrets 或签名失败就报错）、`draft`（先建成草稿）、`notes`（附加说明）。需要配置的 secrets 写在 `release.yml` 的开头注释里。

## 主要功能

- 概览（健康状态 + 关键指标卡 + 配置摘要）
- CPU / GPU / 内存 / 存储 / 主板 / 显示 / 网络 / 电池 / 传感器 / 外设（含蓝牙设备电量）/ 操作系统 / 开发环境详情页
- 实时监控（CPU、内存、网络、磁盘 IO、温度、GPU 走势，每核心占用，可暂停 / 清空 / 切换历史时长）
- 进程列表（排序、按名称或 PID 筛选）
- **存储空间分析**（存储页 → 空间分析）：把整块磁盘或任意文件夹扫描成可交互的图——矩形树图（类似 SpaceSniffer）或旋转图（类似 DaisyDisk），色块越大占用越多。按文件类型着色并可点图例过滤，双击下钻、面包屑返回；列表里有最大文件榜、重复文件、和上次扫描相比什么变大了；可按名称、大小、时间搜索筛选。选中后可打开、预览（macOS 用 Quick Look）、在文件夹中显示、复制路径，或加入「收集箱」——核对总量后一次性移到废纸篓 / 回收站；也能导出 CSV。扫描整盘时还会说出「看不见的占用」（扫描没覆盖到的空间、macOS 的本地快照与可清除空间）。扫描只读取文件大小，不改动任何文件；macOS 上扫描整盘需要「完全磁盘访问」才能读到受保护的目录
- **一键清理**（存储页 → 清理）：缓存、浏览器缓存、日志、临时文件、开发工具缓存（Cargo / Go / Gradle / Xcode / npm 等）、闲置项目里的构建产物（两周没动过的项目里的 `node_modules`、`target`、虚拟环境等，默认不勾）、废纸篓。只认写死的位置（项目产物要旁边有对应的项目文件才认）、只动位置里面、不跟随符号链接，清理前列出清单并确认
- **磁盘速度测试**（存储页 → 速度测试）：在所选磁盘上写一个临时文件，绕开系统缓存测顺序读写和 4K 随机读写（单线程 / 多线程，IOPS 与延迟），测完就删，哪怕测到一半程序被杀也不会留下文件；每块盘记下最近几次的结果，和上一次对比
- **启动项管理**（工具 → 启动项）：登录项、后台代理和守护进程（macOS）、注册表 Run 项和启动文件夹（Windows）、autostart（Linux）；一键停用 / 启用，随时可以改回来
- **应用卸载**（工具 → 应用，macOS / Windows）：按占用空间排序，看最近使用时间，找出很久没用的应用。macOS 连同残留文件（偏好设置、缓存、日志……）一起移到废纸篓，能看清每一项的完整路径；Windows 调用应用自带的卸载程序
- **顶栏实时监控条**：CPU / 内存 / 磁盘容量 / 温度 / 上下行网速，另可开启 GPU 与电池。macOS 使用系统菜单栏独立监控项（项间距可调，应用图标可隐藏），其它平台通过托盘菜单与提示查看；主窗口底部另有一条状态栏。在「顶栏监控」页实时预览、开关、排序，逐项选择双行 / 紧凑 / 趋势图样式，配置刷新周期、标签、项间距、分隔线、颜色与进程排行条数
- **点击展开详情**：内存用量与进程排行、CPU 每核心负载与进程排行、磁盘容量与进程 IO、网络上下行与按进程的流量排行（macOS）、传感器、GPU、剩余电量。面板可固定，支持 Esc / 点击外部收起；进程采样仅在相关页面或详情打开时运行。设计与验收说明见 `docs/menu-bar.md`
- **内存构成与压力**：应用 / 内核 / 压缩 / 缓存 / 空闲各占多少，内存压力和换页速率；CPU 平均负载与开机时长
- **传感器**：macOS 上读 CPU / GPU 逐核温度（SMC，CPU 温度取各核平均，和 iStat Menus、Stats 一致）、风扇转速、功率、电压、电流，顶栏风扇转速实时刷新
- **网络体检**：绕过代理的延迟探测（路由器 / DNS / 公网，看的是真实链路而不是代理出口）、Wi-Fi 详情、今日 / 7 天 / 本月 / 累计的流量用量
- 导出 Markdown / JSON / PNG 长图，默认隐去 MAC、序列号、主机名等敏感信息
- **系统托盘常驻**：菜单实时显示已启用指标，选择指标可展开详情，关闭主窗口后继续后台运行。Linux 需要桌面支持 StatusNotifier（GNOME 要装 AppIndicator 扩展），检测不到时不创建托盘，关闭窗口即退出
- **开机自启**：设置里一键开关（macOS LaunchAgent / Windows 注册表 Run 键 / Linux XDG autostart）；有托盘时开机只驻留托盘，不弹窗口
- **单实例**：重复启动只会唤起已经在运行的窗口，不会多出一份进程和托盘图标
- **桌面网速悬浮窗**：始终置顶的小条，可拖动（位置会记住），双击或右键关闭
- 运行时长、磁盘占用、网络地址、电池这些「会变的配置」自动刷新（窗口可见时每 30 秒，隐藏时每 5 分钟），不用手动点刷新
- **规则**：自己定义「什么指标、满足什么条件、持续多久」就触发，带冷却；触发后发系统通知、推送 Webhook、运行一条命令。默认带 CPU / 内存 / 磁盘 / 温度四条
- 深色 / 浅色 / 跟随系统主题、Geek 模式、采样间隔与单位可调、中英文；设置可以导出成文件、在另一台电脑上导入（规则里的命令和接口服务开关出于安全不导入）
- **本地 HTTP 采集服务**：方便其他机器或脚本拉取本机指标（详见 `docs/api.md`）

## License

MIT

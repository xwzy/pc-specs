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
├── assets/icons/     # 各平台图标
├── packaging/        # macOS Info.plist、Linux .desktop
├── scripts/          # 图标生成、打包脚本
└── docs/             # 文档
```

## 开发

```bash
cargo run -p pc-specs              # 开发运行（框架依赖已开优化，不会卡）
cargo check --workspace            # 仅编译校验
cargo build --release -p pc-specs  # 发布构建
scripts/build-and-install.sh       # 编译发布版并装到本机（macOS 装成 /Applications/PC Specs.app，Linux 装到 ~/.local），装完自动启动
```

Rust 需要 1.92+。Linux 额外依赖：

```bash
sudo apt install -y clang pkg-config libfontconfig-dev libwayland-dev \
  libxkbcommon-x11-dev libx11-xcb-dev libssl-dev libzstd-dev
```

运行时 Linux 需要可用的 Vulkan 驱动与 Wayland / X11 会话。

## 打包

CI（`.github/workflows/build.yml`）在 5 个原生 runner 上构建：macOS arm64 → `.dmg`，Linux amd64 / arm64 → `.tar.gz` + `.deb`，Windows amd64 / arm64 → `.zip`。本地打 macOS / Linux 包：

```bash
cargo build --release --locked -p pc-specs
scripts/package.sh macos-arm64   # 或 linux-amd64 / linux-arm64
```

macOS 的包只做了 ad-hoc 签名，没有公证；从网上下载后首次打开需要在「系统设置 → 隐私与安全性」里放行。

## 主要功能

- 概览（健康状态 + 关键指标卡 + 配置摘要）
- CPU / GPU / 内存 / 存储 / 主板 / 显示 / 网络 / 电池 / 传感器 / 外设 / 操作系统 / 开发环境详情页
- 实时监控（CPU、内存、网络、磁盘 IO、温度、GPU 走势，每核心占用，可暂停 / 清空 / 切换历史时长）
- 进程列表（排序、按名称或 PID 筛选）
- **顶栏实时监控条**：CPU / 内存 / 磁盘容量 / 温度 / 上下行网速，另可开启 GPU 与电池。macOS 使用系统菜单栏独立监控项（项间距可调，应用图标可隐藏），其它平台通过托盘菜单与提示查看；主窗口底部另有一条状态栏。在「顶栏监控」页实时预览、开关、排序，逐项选择双行 / 紧凑 / 趋势图样式，配置刷新周期、标签、项间距、分隔线、颜色与进程排行条数
- **点击展开详情**：内存用量与进程排行、CPU 每核心负载与进程排行、磁盘容量与进程 IO、各网卡实时速率、传感器、GPU、剩余电量。面板可固定，支持 Esc / 点击外部收起；进程采样仅在相关页面或详情打开时运行。设计与验收说明见 `docs/menu-bar.md`
- 导出 Markdown / JSON / PNG 长图，默认隐去 MAC、序列号、主机名等敏感信息
- **系统托盘常驻**：菜单实时显示已启用指标，选择指标可展开详情，关闭主窗口后继续后台运行。Linux 需要桌面支持 StatusNotifier（GNOME 要装 AppIndicator 扩展），检测不到时不创建托盘，关闭窗口即退出
- **开机自启**：设置里一键开关（macOS LaunchAgent / Windows 注册表 Run 键 / Linux XDG autostart）；有托盘时开机只驻留托盘，不弹窗口
- **单实例**：重复启动只会唤起已经在运行的窗口，不会多出一份进程和托盘图标
- **桌面网速悬浮窗**：始终置顶的小条，可拖动（位置会记住），双击或右键关闭
- 运行时长、磁盘占用、网络地址、电池这些「会变的配置」自动刷新（窗口可见时每 30 秒，隐藏时每 5 分钟），不用手动点刷新
- 阈值告警（CPU / 内存 / 磁盘 / 温度，带冷却）
- 深色 / 浅色 / 跟随系统主题、Geek 模式、采样间隔与单位可调、中英文
- **本地 HTTP 采集服务**：方便其他机器或脚本拉取本机指标（详见 `docs/api.md`）

## License

MIT

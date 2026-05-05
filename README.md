# PC Specs

> A cross-platform PC configuration viewer for gaming enthusiasts and programming geeks.
> Windows · macOS · Linux · 单包 < 15MB · 100% 本地。

设计文档：`docs/design.md` · 技术方案：`docs/tech.md` · UI 设计：`docs/ui.md` · 外部 API：`docs/api.md`

## 技术栈

- **桌面外壳**：Tauri 2 + Rust
- **前端**：React 18 + TypeScript + Vite + Tailwind CSS + Recharts + Zustand + TanStack Query
- **跨平台数据采集**：sysinfo、wgpu、os_info、whoami、local-ip-address，配合 Win32_BaseBoard / sysctl / `/sys/class/dmi` 等平台特化路径

## 目录结构

```
pc-specs/
├── docs/             # 三份核心设计文档
├── src/              # 前端 (React + TS)
├── src-tauri/        # 后端 (Rust + Tauri)
└── scripts/          # 辅助脚本（图标生成等）
```

## 快速开始

```bash
# 安装前端依赖（必须使用 pnpm）
pnpm install

# 开发模式（启动 Tauri + Vite HMR）
pnpm tauri:dev

# 仅编译校验（不打完整 app）
pnpm typecheck                                  # 前端 TS 类型检查
cargo check --manifest-path src-tauri/Cargo.toml # 后端 Rust 编译校验

# 仅前端调试（浏览器访问，会回落到 mock 数据）
pnpm dev
```

## 主要功能

- Dashboard 总览（封面 + 关键指标卡）
- CPU / GPU / Memory / Storage / Motherboard / OS / Network / Display / Sensors / Battery / Peripherals 详情页
- 实时监控（多曲线 + 各核柱图 + 温度堆叠 + 各网卡实时 BPS）
- 开发环境探测（Languages / PM / VCS / Editors / Containers / Shells）
- 一键导出 Markdown / JSON / **PNG 长图**（适合发分享）
- **系统托盘常驻**：菜单实时显示 CPU / 内存 / 磁盘 / 网络 / 温度，关闭主窗口仍后台运行；macOS 状态栏支持显示实时网速文字
- **桌面网速悬浮窗**：始终置顶的极简小条，可拖动，可在设置里启用 / 关闭
- 深色 / 浅色主题、Geek Mode、采样间隔可调
- 国际化基础（中 / 英）
- **本地 HTTP 采集服务**：`0.0.0.0:16089`，方便其他机器实时拉取本机指标（详见 `docs/api.md`）

## 路线图

见 `docs/design.md` §8。当前实现覆盖 M1（MVP）与部分 M2 模块。

## License

MIT

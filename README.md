# PC Specs

> A cross-platform PC configuration viewer for gaming enthusiasts and programming geeks.
> Windows · macOS · Linux · 单包 < 15MB · 100% 本地。

设计文档：`docs/design.md` · 技术方案：`docs/tech.md` · UI 设计：`docs/ui.md`

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
- 实时监控（多曲线 + 各核柱图 + 温度堆叠）
- 开发环境探测（Languages / PM / VCS / Editors / Containers / Shells）
- 一键导出 Markdown / JSON
- 深色 / 浅色主题、Geek Mode、采样间隔可调
- 国际化基础（中 / 英）

## 路线图

见 `docs/design.md` §8。当前实现覆盖 M1（MVP）与部分 M2 模块。

## License

MIT

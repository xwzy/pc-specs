# 后台稳定性审查

审查日期：2026-11-21。范围包括应用生命周期、采集与后台任务、HTTP 服务、历史持久化、扫描任务、依赖和构建脚本。

## 已修复

| 问题 | 修复后的行为 |
| --- | --- |
| 系统采集卡住后，定时和手动重试可能重复启动同一模块 | 进程内每个模块最多一个采集；忙时复用上一份结果，单个模块出错保留已有数据 |
| 超时或较早的轻量采集可能覆盖新结果、清除新任务的状态 | 用轮次隔离结果，超时立即作废该轮次 |
| HTTP 请求超时释放了锁，但阻塞采集还在运行 | 采集任务持有锁直至实际结束，并自行更新缓存 |
| HTTP 采样请求可在采样器卡住后继续排队 | 只允许一个待处理请求，已取消的请求直接跳过；info 阻塞采集最多两个 |
| 未发完请求体的客户端可能阻止 HTTP 服务关闭 | 最多等待两秒优雅关闭，runtime 最多再等一秒；新增真实 TCP 回归测试 |
| 外部命令的后代保留 stdout/stderr，会留下永久等待的管道读取线程 | 在命令自身线程读取两条管道：Unix 使用非阻塞读取，Windows 使用 PeekNamedPipe；无需额外读取线程 |
| 命令排队时间不计入超时，工作线程可能无限等待名额 | 排队和执行共用超时预算；命令并发限制为 6，stdout/stderr 各保留最多 16 MiB |
| 慢磁盘时历史写盘队列不断积累整份历史快照 | 待写任务按档位合并，最多两个快照、一次删除和一次退出确认，UI 不等待磁盘 |
| 系统时间停止推进时大屏历史无数量上限 | 同时按时间和数量限制，最多 2421 个点 |
| 长期创建和删除 VPN/容器接口会积累分类缓存 | 删除已消失接口的缓存；物理网卡空闲时也不把虚拟接口流量错误计入总量 |
| 隐藏窗口仍保持前台的高频采样 | 后台按菜单刷新需求降频；无可见读数或规则时默认五秒；保留用户主动选择的更慢间隔 |
| macOS 每轮重复采集 CPU，传感器在没有需求时提前初始化 | 原生 CPU 读取成功时跳过备用读取，按需初始化传感器 |
| 关闭主窗口后扫描、测速、清理统计和应用统计可能继续消耗资源 | 销毁所属页面时设置取消标志；应用列表消息队列限制为 16 项，接收方消失后停止生产 |
| 清理目录的计量过程中取消，计量结束后仍然开始删除 | 删除前再次检查取消标志，避免取消后还启动递归删除；已经开始的单次系统删除无法中途打断 |
| 规则集中触发或反复点击测试会不断创建线程 | 四个固定动作线程，最多 128 个排队任务；队列满时明确提示 |
| 唤醒已有实例失败后又启动一个后台实例 | 已有实例持锁时始终退出；唤醒消息合并为一条，监听错误不再空转 |
| 旧电池依赖引入 nix 已知越界写漏洞 | 替换为维护中的 starship-battery 0.12；heapless 更新至 0.9.4，修复其内存安全公告涉及的问题 |

这些措施限制资源增长和重复工作。操作系统接口本身卡住时，Rust 无法安全强杀正在调用它的线程：该模块复用之前的结果，其读数可能过期；首次还没有结果时显示缺失值。程序重启后可重新采集。

## 自动验证

本机全功能版 255 项测试通过，商店版 230 项测试通过；两种特性的严格 Clippy 检查通过。ShellCheck、格式和版本一致性检查通过。

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --workspace --no-default-features --features appstore --locked
cargo clippy --workspace --all-targets --no-default-features --features appstore --locked -- -D warnings
cargo build --release --locked -p pc-specs
cargo audit
shellcheck scripts/*.sh scripts/lib/*.sh scripts/macos/*.sh scripts/linux/*.sh
scripts/version.sh check
```

App Store 特性只支持 macOS。发布流程增加 Windows 原生测试、三平台格式与 Clippy 检查，以及 macOS 商店版测试；仍只在手动触发发布流程时执行。

新增回归测试覆盖：卡住模块的 10000 次重试、panic 后恢复、HTTP 取消与缓存、采样排队上限、不完整请求下的退出、命令双管道大输出和超时、慢磁盘合并、历史压缩与最后一格保存、时钟停止推进，以及动作队列满载后恢复。

发布二进制的 HTTP 实测：64 次 metrics 请求（16 并发）在 0.536 秒内完成，共用一次有效采样；8 次 snapshot 并发请求共用一份快照，内存总量均有效。结果保存在 `dist/http-smoke.json`。

## 实机持续检查

先退出已运行的 PC Specs，保留托盘开启；脚本仅启动和停止自己的子进程。建议关闭大屏和悬浮窗，记录所使用的采样、菜单栏和规则设置，方便比较。

```sh
cargo build --release --locked -p pc-specs
python3 scripts/check-background.py --duration 300
python3 scripts/check-background.py --duration 86400 --output dist/background-24h.json
```

脚本适用于 macOS / Linux，默认预热 30 秒、每五秒记录一次进程累计 CPU 时间、RSS 和线程数。默认检查预热后峰值内存增长不超过 32 MiB、线程数增长不超过 4；这些阈值可通过参数调整。CPU 汇总以单核的 100% 为基准。详细采样和应用日志保存在输出 JSON 及同名 `.log` 中，位于被 Git 忽略的 `dist/`。

本次 macOS ARM64 发布构建后台检查运行 300 秒，每十秒采样，预热后的 270 秒汇总如下（结果为本机、当前设置下的观察值）：

| 指标 | 结果 |
| --- | --- |
| 平均 CPU（单核 100%） | 1.166% |
| RSS 起点 / 终点 / 峰值 | 86.641 / 87.172 / 87.219 MiB |
| 线程数起点 / 峰值 | 11 / 12 |
| 提前退出或资源增长超限 | 无 |

应用日志只有 GPUI 跳过重复系统字体的启动警告，没有观察到 panic 或采集错误。完整结果在 `dist/background-check.json`，该文件不纳入源码提交。

短时间检查不能证明连续数周稳定，也不能替代合盖唤醒、热插拔、网络切换和不同硬件上的验证。持续检查期间不要打开额外窗口或运行存储扫描，否则测到的是交互负载。脚本结束会终止其启动的进程；历史完整保存由退出回归测试单独验证。

## 依赖与平台限制

更新后本次 RustSec 审计的已知漏洞和 unsound 公告均为零；仍有九项上游停止维护公告：chrono、instant、paste、rustls-pemfile、rustybuzz、ttf-parser、zbus_names、zvariant、zvariant_derive。多数来自 GPUI 及其平台依赖；本次没有通过忽略公告来隐藏这些结果，后续升级 UI 框架时需要继续跟踪。

本机为 macOS ARM64。Windows / Linux 的命令执行模块已分别通过目标平台编译检查；完整项目的跨平台检查被缺少 Windows SDK C 头文件及 Linux musl C 编译器阻止，应以对应原生 runner 的结果为准。未将跨平台静态检查表述为实机运行验证。

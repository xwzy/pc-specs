# 苹果商店（Mac App Store）合规审计

对照 [App Review Guidelines](https://developer.apple.com/app-store/review/guidelines/) 与 Mac App Store 的沙盒、签名、打包要求，
逐条检查本项目，找出的问题已经改掉；**无法在本机自查、要等 Apple 审核才知道的项目**列在最后一节。

- 审计对象：`pc-specs` 0.2.0，macOS 商店版（`--no-default-features --features appstore`）和全功能版（默认）。
- 结论：商店版按本文的措施处理之后，没有已知的违反准则之处；全功能版**不能**上架（它读私有传感器、枚举进程、开本地端口，沙盒不允许），只通过 GitHub Releases 自行发布。
- 两个版本怎么切、怎么编译：`src` 里只有一处开关，见 `crates/core/src/edition.rs`；脚本见文末「怎么构建与复查」。

## 1. 两个版本的差别

判断「这个版本有没有某项能力」一律用 `pc_specs_core::edition` 里的常量。重型或私有的代码用 `cfg(feature = "appstore")` 在编译期整段去掉（不是运行时关掉），
所以商店版的二进制里**没有**对应的代码和依赖：用 `cargo tree -p pc-specs --no-default-features --features appstore -i <包名>` 逐个查过，
`ureq`、`axum`、`tokio`、`hyper`、`rustls`、`ring`、`openssl`、`native-tls`、`reqwest`、`plist`、`tower-http` 都不在它的依赖树里（全功能版里有）。

| 能力 | 全功能版（自行发布） | 商店版 | 原因 |
|---|---|---|---|
| 概览、实时监控、顶栏监控、监控大屏、处理器 / 显卡 / 内存 / 存储 / 主板 / 显示 / 网络 / 电池 / 外设 / 操作系统、规则 | ✔ | ✔ | 都只用公开接口 |
| 传感器页，温度 / 风扇 / 电压 / 功耗，顶栏温度项，规则里的温度类条件 | ✔ | ✘ | macOS 上读它们要走私有的 IOHID / SMC 接口（准则 2.5.1），沙盒也不让碰（`edition::SENSORS`） |
| 进程页，各进程的 CPU / 内存 / 磁盘 / 网络占用，面板里的「占用最多的进程」 | ✔ | ✘ | 沙盒里不能枚举别的进程（`edition::PROCESS_LIST`）；实测网络流量能量到、却认不出是哪个进程 |
| 公网 IP 查询、网络延迟探测、Webhook | ✔ | ✘ | 商店版不申请任何网络权限，完全不联网（`edition::NETWORK_ACCESS`） |
| 本机 HTTP 采集接口 | ✔ | ✘ | 监听端口需要网络权限（`edition::LOCAL_SERVER`） |
| 规则触发时运行用户写的命令 | ✔ | ✘ | 沙盒里只能运行系统自带的工具，没有意义也不该提供（`edition::RUN_COMMANDS`） |
| 开发环境检测、启动项管理、应用管理（卸载）、系统缓存清理 | ✔ | ✘ | 都要读写用户目录之外或别的应用的数据（`edition::SYSTEM_MANAGEMENT`） |
| 存储分析 | 整块磁盘或任意文件夹 | 用户在「选择文件夹」对话框里亲自选的位置；选过的用安全作用域书签记到下次启动 | 沙盒只允许访问用户亲自授权的位置（`edition::SANDBOXED`） |
| 磁盘速度测试 | 任意磁盘 | 只能测应用容器所在的系统盘 | 沙盒里只能往自己的容器写测试文件 |
| 开机自启 | 登录项 plist | `SMAppService.mainApp` 登录项，用户在设置里主动打开 | 准则 2.4.5：不得在用户不知情时自启 |
| 单实例叫醒 | 回环端口 | 只用文件锁，不监听任何端口 | 沙盒里没有网络权限 |
| 菜单栏项被点中时的矩形高亮 | 有（挂钩 `-[NSStatusItem _setSelectedMenuContentFrame:]`） | 没有，用系统自己的圆角底色 | 那是私有方法（准则 2.5.1），见第 3 节 |

商店版的侧栏只有：概览、实时监控、顶栏监控、监控大屏、处理器、显卡、内存、存储、主板、显示、网络、电池、外设、操作系统、规则。

## 2. 对照准则逐条检查

| 准则 | 要求 | 本项目 | 结论 |
|---|---|---|---|
| 2.4.5(i) | Mac App Store 的应用必须启用沙盒 | `packaging/macos/appstore/entitlements.plist` 只申请 `app-sandbox`、`files.user-selected.read-write`、`files.bookmarks.app-scope`；故意没有网络、Apple Events、temporary-exception | ✔ |
| 2.4.5(ii) | 一个自包含的安装包，不得把代码或资源装到共享位置 | 单个 `PC Specs.app`，只装进 `/Applications`；用 `productbuild` 打 `.pkg`，没有第三方安装器 | ✔ |
| 2.4.5(iii) | 不得在没有用户同意时自启；退出应用后不得留下继续运行的进程；不得自己往 Dock 或桌面放图标 | 默认不自启；设置里的开关用 `SMAppService.mainApp` 登记，系统设置的「登录项」里也看得见、能关；退出（⌘Q / 菜单「退出」）即整个进程结束；没有往 Dock 或桌面放任何东西 | ✔ |
| 2.4.5(iv) | 不得下载或安装独立应用、内核扩展、额外代码来改变功能 | 商店版没有联网代码，不可能下载；不安装内核扩展、辅助进程 | ✔ |
| 2.4.5(v) | 不得请求 root 权限、不得用 setuid | 没有任何 `sudo`、`AuthorizationExecuteWithPrivileges`、`osascript … with administrator privileges`（全功能版里的「系统管理」类功能不在商店版里） | ✔ |
| 2.4.5(vi) | 不得在启动时弹出许可协议、要求序列号、自己做防拷贝 | 没有 | ✔ |
| 2.4.5(vii) | 更新只通过 App Store | 没有自更新代码；商店版也没有联网代码 | ✔ |
| 2.5.1 | 只能使用公开 API | 审计出 4 处私有接口，全部处理（第 3 节）；`scripts/macos/scan-private-api.sh` 对最终二进制扫描，结果干净 | ✔（见第 6 节的未验证项） |
| 2.5.2 | 不得读写容器之外的数据；不得下载、安装或执行会改变功能的代码 | 没有插件机制；数据只在应用容器和用户在对话框里选的位置；二进制里的 `dlopen / dlsym` 来自 Rust 标准库和系统库自身（扫描脚本只给出一条提示），加载的都是系统框架 | ✔ |
| 2.4.2 | 节能：不得让设备持续高负载、发热，不得无故大量写 SSD | 默认 1 秒采样一次，间隔可在设置里调；只有「磁盘速度测试」会写大文件，由用户点击触发、测完立刻删，测试大小 1 / 4 / 16 GB 由用户选；其余只写几 KB 的设置与历史记录 | ✔ |
| 2.3.x | 元数据准确：名称、截图、描述都要真实 | 截图全是真实界面（含演示用的无害文件夹），不含个人信息；描述里没有未实现的功能、没有其他平台和价格 | ✔（文案见 `docs/APP_STORE_LISTING.md`） |
| 2.3.7 / 5.2.1 | 不得在名称、关键词、图标里用别人的商标 | 图标原稿里的显卡上有「GEFORCE RTX」字样（NVIDIA 商标），已抹掉并重新生成全部尺寸；元数据里没有任何第三方品牌名（`README` 里提到的同类软件不属于应用和上架材料） | ✔ 已修复 |
| 4.1 / 4.2 | 不得山寨；要有足够的功能 | 原创界面与实现；功能完整、不是网页包装 | ✔ |
| 5.1.1(i) | 要有隐私政策，在 App Store Connect 里填链接，应用里也要能找到 | 应用「设置」页有「隐私政策」和「技术支持」两个按钮（`crates/app/src/links.rs`，跟随界面语言）；政策正文在 `site/privacy.html` | ✔（站点要先上线，见 `docs/RELEASE_CHECKLIST.md`） |
| 5.1.1(ii) / 5.1.2 | 收集数据要申报、要征得同意 | 不收集、不上传、不追踪；App Privacy 选「不收集数据」；`PrivacyInfo.xcprivacy` 声明无追踪、无收集 | ✔ |
| 5.1.1(iii) | 只请求必要的权限 | 唯一会出现的系统授权是「通知」，而且只在规则第一次触发、需要发通知时才请求（UserNotifications，公开接口）；不要定位、通讯录、相机、麦克风；文件夹只在用户点「选择文件夹」时才问 | ✔ |
| 3.1.x | 付费与内购 | 免费，没有内购 | ✔ |
| 2.1 | 应用要完整，没有占位内容、没有失效链接 | 没有占位内容；**应用里的隐私政策 / 技术支持链接指向 `pc-specs.xwzy.dev`，提审前必须已上线** | ⚠ 待办 |
| 出口合规 | 加密声明 | 商店版没有联网代码，依赖树里没有任何 TLS 或加密库，`Info.plist` 里 `ITSAppUsesNonExemptEncryption = false` | ✔ |
| 隐私清单（2024 年起） | 用到「需要申报理由」的 API 要声明 | `PrivacyInfo.xcprivacy` 声明了 FileTimestamp（3B52.1 / C617.1 / DDA9.1）、SystemBootTime（35F9.1）、DiskSpace（85F4.1 / E174.1）、UserDefaults（CA92.1） | ✔ |
| 打包与签名 | 沙盒权限、描述文件、构建号、分类、图标 | `scripts/macos/appstore.sh` 负责：Apple Distribution 证书签 `.app`、安装包证书签 `.pkg`、嵌入描述文件并合并 `application-identifier` / `team-identifier`、构建号递增、`LSApplicationCategoryType`、`icon.icns` 含 1024 像素那一档 | ✔（要有证书与描述文件，见清单） |

## 3. 审计中发现的私有接口，以及处理

Mac App Store 上传时会静态扫描二进制里的私有方法名和符号（ITMS-90338），所以不只要看自己的代码，依赖里的也要查。
`scripts/macos/scan-private-api.sh` 的做法：把二进制导入的符号、字符串里长得像选择子的名字，和当前 SDK 公开头文件里出现过的名字比对，再加一份已知私有名字的黑名单。

| # | 位置 | 私有接口 | 处理 |
|---|---|---|---|
| 1 | 依赖 `sysinfo` 的 `component` 功能 | 4 个私有的 `IOHIDEventSystemClient*` / `IOHIDServiceClient*` 函数（读温度传感器） | 商店版不开这个功能（`crates/core/Cargo.toml`：`full` 才有 `sysinfo/component`） |
| 2 | 依赖 `gpui-pre-macos` 0.3.11 | 5 处：`_opaqueRectForWindowMoveWhenInTitlebar`、`_windowRestorationOptions`、`_zoomFill:`、两个斜向缩放光标 `_windowResize…Cursor`、图层类名 `CAChameleonLayer` | 商店版构建时由 `scripts/macos/build.sh` 把 `packaging/macos/appstore/gpui-pre-macos.patch` 打在注册表源码的**副本**上，不动原件；全功能版不受影响。代价见下 |
| 3 | 本项目 `crates/app/src/menubar/selection.rs` | `-[NSStatusItem _setSelectedMenuContentFrame:]`（把菜单栏被点中的圆角底色改成矩形） | 商店版整段编译掉，用系统自己的圆角高亮 |
| 4 | 本项目 `crates/core/src/platform/macos_smc.rs` | 通过 `AppleSMC` 读温度 / 风扇 / 功耗：不是文档化的接口，沙盒也拒绝 | 商店版不编译这一部分 |

**GPUI 补丁的代价**（都是体验上的小折扣，没有功能缺失）：

- 窗口在重启之后不一定回到原来所在的桌面（Space）。
- 窗口菜单里的「填充」退化成普通的「缩放」。
- 标题栏拖动和双击的区域声明被去掉后，交给 AppKit 的默认行为；在合成事件下按钮、双击缩放、拖动都正常，但**真实鼠标下是否会出现「双击标题栏缩放两次」本机没法验证**（见第 6 节）。

GPUI 升级后补丁对不上，编译会直接报错，按新版本重新生成补丁即可（做法见第 7 节）。

## 4. 沙盒下的取舍与替代做法

- **存储分析**：沙盒读不了没被授权的位置，所以「选位置」页换成「选择文件夹…」，用 `NSOpenPanel` 让用户亲自选整块磁盘或某个文件夹；选过的位置用应用范围的安全作用域书签存进设置，下次启动时「最近分析」里一点就能再扫（`crates/app/src/bookmarks.rs`）。书签失效（文件夹没了、授权被撤销）就从列表里去掉并提示重选。
- **移到废纸篓**：直接调系统的 `NSFileManager trashItem`（不经访达，所以不会弹「想要控制访达」），只对用户亲自选过的位置里的文件有效。导出报告、设置文件都走系统的保存 / 打开对话框，读写用户选的地方。
- **不能做的就不摆出来**：被去掉的页面、按钮、设置项一律不出现在界面里，而不是出现一个点了没用的入口；规则页只剩「发通知」这一种动作，条件里没有温度、风扇、延迟、断网、单个进程占用这几类。
- **子进程**：沙盒里启动的都是系统自带的命令行工具，由它们继承沙盒来读公开信息：`system_profiler`、`diskutil`、`scutil`、`networksetup`、`route`、`netstat`、`ioreg`（以上读硬件、磁盘、网络）、`qlmanage`（空格键预览）、`caffeinate`（监控大屏保持屏幕常亮）、`tmutil`（只列本地快照）。没有 `sudo`、没有运行用户给的命令。
- **偏好与历史**：都在应用容器里（`~/Library/Containers/com.xwzy.pc-specs/Data/…`）：设置、规则、长期走势、流量累计（`usage.json`，按天的上下行总字节数）、扫描历史。

## 5. 隐私与数据

- 不收集、不上传、不追踪；没有账号，没有统计，没有崩溃上报；商店版没有任何网络权限，想上传也上传不了。
- 读取的内容只用来在界面上显示：硬件与系统信息、实时占用、用户选过的文件夹里的文件名与大小（扫描只读元数据，不读文件内容；找重复文件时才读内容算校验值，只在内存里比较）。
- 导出的诊断报告 / 设置文件由用户亲自点导出、选地方保存；默认遮蔽主机名、用户名、MAC、序列号、公网 IP，要带上这些得用户手动打开「包含敏感信息」。
- 全功能版额外的联网行为（都是用户触发或用户配置的）：公网 IP 查询（`api.ipify.org` / `ifconfig.co` / `icanhazip.com`）、规则的 Webhook（用户自己填的地址）、可选的本机 HTTP 接口（默认只监听回环地址，要给局域网用得用户主动打开）。隐私政策里如实写了两个版本的区别。

## 6. 已知风险与没能验证的事

| 项目 | 说明 | 怎么办 |
|---|---|---|
| Apple 服务端的二进制扫描 | 本机的扫描脚本只能做到「按公开头文件和已知黑名单比对」，Apple 的规则不公开，没法保证一次通过 | 上传后若被 ITMS-90338 拒绝，邮件里会列出具体名字：按名字再补进补丁 / 黑名单 |
| 真实鼠标下的标题栏行为 | 本机用合成事件验证过，真实鼠标没法在自动化里验证；可能的问题是双击标题栏缩放了两次，或 macOS 27 上标题栏点击略有延迟 | 提审前人工试一次（`docs/RELEASE_CHECKLIST.md`）；若重复缩放，把补丁里的 `titlebar_double_click` 改成空操作 |
| 沙盒里逐页点过一遍 | 沙盒版（ad-hoc 签名）在本机启动、渲染、各后台采样都正常，日志里没有 panic，也只有无害的拒绝记录；但沙盒窗口没法用自动化工具点（辅助功能对它不可用），所以每个页面的交互没有在沙盒里逐一点过 | 提审前用 `scripts/macos/appstore.sh package --adhoc` 打一个沙盒版，人工把每一页点一遍 |
| 登录时启动的判断 | `launched_at_login()` 区分「系统在登录时拉起」和「用户自己打开」，只做了编译检查，没有真的注销再登录验证 | 提审前人工验证一次：打开自启、注销再登录，窗口不应该弹出来（只留在菜单栏） |
| 其他芯片 | 传感器键表只在 M2 Max 上验证过——不过传感器只在全功能版里 | 与商店版无关 |
| 站点链接 | 应用里的隐私政策与技术支持链接要在提审时能打开 | 先部署 `site/`，见清单 |
| 第三方许可证 | 商店版依赖树（495 个包，`cargo metadata` 逐个查过）里没有 GPL / LGPL / AGPL；有两个 MPL-2.0：`cbindgen`（只在编译时运行，不进二进制）和 `option-ext`（经 `dirs` 静态链接，未修改，源码公开在 crates.io），都不妨碍上架。尚未在应用里附带第三方许可证清单 | 可选：加一页「开源许可」 |

## 7. 怎么构建与复查

```bash
# 商店版（只在 macOS 上）：编译 → 组装 .app → 扫描私有 API → 签名 → .pkg
scripts/build-appstore.sh --package          # 等同 scripts/macos/appstore.sh package
scripts/macos/appstore.sh check              # 自检：工具、证书、描述文件、API 密钥
scripts/macos/appstore.sh validate           # 交给 Apple 校验（不上传）
scripts/macos/appstore.sh upload             # 校验通过后上传

# 本机沙盒试跑（不要证书）：ad-hoc 签名并带上沙盒权限，换了 Bundle ID，不能用来上架
scripts/macos/appstore.sh package --adhoc

# 单独复查二进制里有没有私有接口
scripts/macos/scan-private-api.sh "dist/appstore-adhoc/PC Specs.app"
codesign -d --entitlements - "dist/appstore-adhoc/PC Specs.app"

# 全功能版（自行发布）
scripts/build-full.sh --package
```

升级 GPUI 或依赖之后要重新做的事：

1. `scripts/macos/build.sh --edition appstore --arch arm64`：补丁对不上就会在这里报错；
2. 把新版本源码里的 `window.rs` 等按 `packaging/macos/appstore/gpui-pre-macos.patch` 开头的五条重新改一遍，用 `diff -u` 重新生成补丁；
3. 跑 `scripts/macos/scan-private-api.sh`，确认仍然干净；新增的私有名字照第 3 节的办法处理；
4. 两个版本都跑 `cargo clippy --workspace --all-targets` 和 `cargo test --workspace`（商店版加 `--no-default-features --features appstore`）。

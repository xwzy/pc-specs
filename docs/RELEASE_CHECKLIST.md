# PC Specs 发布与上架检查清单

> 两条发布线：**Mac App Store 版**（`appstore` 版本，沙盒、功能收窄）走 App Store Connect；**全功能版**（默认）走 GitHub Releases 的手动发布工作流。按顺序做；每一步都标了「谁来做」。
> 合规结论与两个版本的功能差别见 `docs/APP_STORE_COMPLIANCE.md`，逐字段的商店文案见 `docs/APP_STORE_LISTING.md`。

## 0. 现状

**工程侧已就绪（已自查）**：

- 编译开关：`full`（默认）与 `appstore` 两个互斥的 Cargo 特性，代码在 `crates/core/src/edition.rs`；商店版的依赖树里没有任何联网 / TLS / 传感器相关的包
- 沙盒与签名素材：`packaging/macos/appstore/` 里有 `entitlements.plist`（只含沙盒、用户选择文件读写、应用范围书签）、`Info.plist` 模板（含分类、`ITSAppUsesNonExemptEncryption`）、`PrivacyInfo.xcprivacy`、GPUI 私有 API 补丁
- 私有 API：自己的代码和依赖（sysinfo、GPUI）里发现的私有调用都处理了，`scripts/macos/scan-private-api.sh` 对成品扫描结果干净
- 沙盒下的功能：存储分析改用「选择文件夹」对话框 + 安全作用域书签；开机自启改用 `SMAppService`；单实例不再监听端口；被去掉的功能不在界面里留入口
- 应用里的「隐私政策」「技术支持」入口（设置页）、中英文界面
- 图标：已去掉 NVIDIA 商标字样并重新生成全部尺寸
- 上架材料：`AppStore/screenshots/{zh-Hans,en-US}/`（各 8 张，2880×1800）、`docs/APP_STORE_LISTING.md`、`site/`（隐私政策与支持页，中英文）
- 脚本：见第 4 节

**本机没法做、要你来做的（后面各节展开）**：

- 开发者账号里的 App ID、证书、描述文件、API 密钥、App Store Connect 记录
- 部署 `site/`、绑定域名、配置邮件转发
- 沙盒版的人工逐页自测（第 5 节）
- 上传、提审

## 1. 账号与签名（你来做）

| 项目 | 说明 |
|---|---|
| Team ID | `W6684V32JU`（和其他项目一样；脚本里用环境变量 `APPLE_TEAM_ID` 覆盖） |
| Bundle ID | `com.xwzy.pc-specs`：到 [Identifiers](https://developer.apple.com/account/resources/identifiers/list) 新建 macOS 的 App ID，**不需要勾选任何额外 Capabilities**（App Sandbox 由权限文件带上） |
| 证书 | 两张：**Apple Distribution**（签 `.app`）和 **Mac Installer Distribution**（旧名 3rd Party Mac Developer Installer，签 `.pkg`），连同私钥放进登录钥匙串。**Developer ID** 只用于站外分发 + 公证，**不能**用于 Mac App Store |
| 描述文件 | 新建类型为「Mac App Store Connect」的描述文件，App ID 选 `com.xwzy.pc-specs`，证书选上面的 Apple Distribution；下载后放到 `~/Library/Developer/Xcode/UserData/Provisioning Profiles/PCSpecs_Mac_App_Store.provisionprofile`（或用 `PROVISIONING_PROFILE_PATH` 指定）。`*.provisionprofile`、`*.cer` 已加进 `.gitignore`，不会误提交 |
| API 密钥 | 沿用 `APPLE_API_KEY`、`APPLE_API_ISSUER`、`APPLE_API_KEY_PATH`（`.p8` 文件建议 `chmod 600`）。用来校验和上传构建；该密钥通常没有「新建版本 / 修改文案 / 上传截图」的权限，这些在网页里做（第 3 节） |
| 沙盒权限 | `app-sandbox`、`files.user-selected.read-write`、`files.bookmarks.app-scope`；故意没有网络权限 |
| Hardened Runtime | 正式签名时开启 |

自检：`scripts/macos/appstore.sh check`——它会列出缺哪个工具、证书、描述文件、密钥，并核对描述文件里的 App ID 与 Team ID。

## 2. 托管隐私政策与支持页（提审前必须能打开）

网页源文件在 `site/`，建议和其他项目一样托管在 **Cloudflare Pages**（直连上传，不用把仓库连到 Cloudflare）。应用里的两个按钮和商店资料里的网址都指向这些页面：

| 语言 | 首页 | 技术支持 | 隐私政策 |
|---|---|---|---|
| 简体中文 | `/` | `/support.html` | `/privacy.html` |
| English | `/en/` | `/en/support.html` | `/en/privacy.html` |

- [ ] 新建 Pages 项目并上传：`npx wrangler pages deploy site --project-name pc-specs`（先 `wrangler login`；Wrangler 需要较新的 Node）
- [ ] 绑定自定义域名 `pc-specs.xwzy.dev`（**这是按其他项目的习惯假定的**，换了域名要同步改 `crates/app/src/links.rs` 的 `WEBSITE`、`site/` 页面里的链接和商店文案）
- [ ] 邮件转发：Cloudflare Email Routing 把 `pc-specs@xwzy.dev` 转到你自己的邮箱（支持页和隐私政策里写的联系邮箱）
- [ ] 用浏览器逐个打开上面 6 个页面，确认链接、语言切换、图片都正常
- [ ] 首页的下载按钮目前指向 GitHub Releases；如果还没发布全功能版，先到仓库建一个 Release，或把按钮换成你想要的地址（**不要放还不存在的 App Store 链接**，准则 2.1）

## 3. App Store Connect 建档与资料（你来做）

逐字段文案都在 `docs/APP_STORE_LISTING.md`，截图在 `AppStore/screenshots/`。

1. [ ] 「App」→「+」→「新 App」：平台 macOS，名称 `PC Specs - 电脑配置与实时监控`（被占用就用列表里的备选），主要语言简体中文，Bundle ID `com.xwzy.pc-specs`，SKU `pcspecs100`
2. [ ] 「App 信息」：添加英语（美国）本地化（名称、副标题）；类别「工具」；内容版权「不包含第三方内容」；隐私政策网址（中、英各一个）
3. [ ] 「定价与销售范围」：免费；销售地区自行决定（中国大陆要 ICP 备案）
4. [ ] 「App 隐私」：选「不收集数据」；追踪选「否」
5. [ ] 版本页：中英文各填推广文本、描述、关键词、技术支持网址、营销网址；各上传 8 张截图（`01…08` 顺序）
6. [ ] 年龄分级问卷全选「无」，得到 4+
7. [ ] 「App 审核」：不需要登录；联系人信息；把备注整段贴进去
8. [ ] 构建版本：上传后等 5–30 分钟处理完，在版本页的「构建版本」里选中
9. [ ] 发布方式建议选手动发布，过审后核对商店页再发

## 4. 构建与上传

**脚本一览**（都在仓库的 `scripts/` 下，`--help` 看完整选项）：

| 脚本 | 用途 |
|---|---|
| `scripts/build-full.sh [--arch ...] [--package]` | **全功能版**：编译，加 `--package` 就连安装包一起打（macOS 是 `.dmg`，Linux 是 `.deb` 等，Windows 要在 Windows 上用 `scripts\windows\*.ps1`） |
| `scripts/build-appstore.sh [--arch ...] [--package]` | **商店版**：编译，加 `--package` 就签名并打成 `.pkg`（要证书和描述文件） |
| `scripts/build.sh --edition full\|appstore [--platform ...] [--arch ...] [--package]` | 上面两个的通用入口 |
| `scripts/macos/build.sh [--edition appstore] [--arch ...]` | 只编译 macOS 版 |
| `scripts/macos/appstore.sh check\|build\|package\|validate\|upload [--arch ...] [--adhoc]` | 商店版全流程：自检、编译、打包、交给 Apple 校验、上传；`package --adhoc` 在本机沙盒里试跑，不要证书 |
| `scripts/macos/scan-private-api.sh <PC Specs.app>` | 扫描成品里有没有非公开 API |
| `scripts/macos/package.sh` | 全功能版的 `.dmg`（Developer ID 签名 + 公证） |
| `scripts/version.sh [get\|set\|bump\|check\|build ...]` | 版本号与商店构建号（单一来源是根 `Cargo.toml`） |

商店版上架流程：

```bash
scripts/version.sh set 1.0.0              # 首次上架想用 1.0.0 的话；之后每次对外发布再改
scripts/version.sh build                  # 看当前构建号；每次上传前 build bump，必须比上一次大
scripts/macos/appstore.sh check           # 工具、证书、描述文件、API 密钥
scripts/macos/appstore.sh package         # 编译 → 扫描 → 签名 → .pkg（dist/pc-specs-<版本>-b<构建号>-macos-appstore.pkg）
scripts/macos/appstore.sh validate        # 交给 Apple 校验，不上传
scripts/macos/appstore.sh upload          # 校验通过后上传
```

- 脚本会检查安装器与应用签名、应用标识符（`<团队 ID>.com.xwzy.pc-specs`）、描述文件、隐私清单；沙盒权限文件与描述文件里的权限合并后再签名
- 构建号只增不减：`BUILD_NUMBER=N` 可以临时覆盖，但最好用 `scripts/version.sh build bump` 记下来
- 上传后若收到 ITMS 开头的邮件，按邮件内容处理；ITMS-90338（非公开 API）的处理办法见 `docs/APP_STORE_COMPLIANCE.md` 第 6 节

全功能版发布：`scripts/version.sh set X.Y.Z` → 提交并推送 → GitHub 上手动运行 **Release** 工作流（它在 5 个原生 runner 上编译、签名、打包）；也可以本机 `scripts/build-full.sh --package`。签名和公证用的 secrets 写在 `.github/workflows/release.yml` 开头的注释里。

## 5. 签名包人工自测（尚未逐项执行）

用 `scripts/macos/appstore.sh package --adhoc` 打出 `dist/appstore-adhoc/PC Specs.app`（ad-hoc 签名、带沙盒权限、Bundle ID 换成 `com.xwzy.pc-specs.sandbox-test`），双击运行，逐项点：

- [ ] 首次启动不崩溃，界面正常；系统语言是英文时是英文，是中文时是中文；设置里切换语言、主题立即生效
- [ ] 概览、实时监控、处理器、显卡、内存、存储、主板、显示、网络、电池、外设、操作系统每页都能打开，没有空白或乱码
- [ ] 菜单栏出现监控项，点开面板正常；面板里「完整详情」「配置监控条」能跳转；在「顶栏监控」页改样式、顺序立即生效
- [ ] 存储 → 空间分析 → 选择文件夹…：扫描、树图 / 旋转图、列表、重复文件、加入收集箱、移到废纸篓（有确认）都正常；退出重开后「最近分析」里还能直接点开
- [ ] 存储 → 速度测试能跑完，测试文件被删掉
- [ ] 监控大屏能打开、全屏、关闭
- [ ] 规则：展开一条点「试一试」，第一次会出现系统的通知授权提示，允许后能收到通知
- [ ] 导出：保存到文件、PNG 长图都能用；设置页的导出 / 导入设置文件能用
- [ ] 设置里的「隐私政策」「技术支持」按钮能打开对应页面（要先完成第 2 节）
- [ ] **开机自启**：打开开关 → 注销再登录 → 应用在菜单栏里，不弹窗口；系统设置的「登录项」里能看到并能关闭
- [ ] **标题栏的真实鼠标行为**（GPUI 补丁的改动）：拖动窗口、双击标题栏缩放（应缩放一次，不是来回两次）、红黄绿三个按钮；全屏进出。如果双击会缩放两次，把 `packaging/macos/appstore/gpui-pre-macos.patch` 里的 `titlebar_double_click` 改成空操作再重新打包
- [ ] 关闭窗口后应用留在菜单栏；应用菜单里的「退出」和 Command+Q 能让整个进程结束

本机小贴士：同一个 Bundle ID 的沙盒版，如果之前在这台电脑上用别的签名跑过，系统里会留着旧容器，新签名的包可能卡在沙盒初始化（进程在、不出窗口）。先看有没有系统弹窗（「想访问其他 App 的数据」）点允许；没有的话在访达里删掉 `~/Library/Containers/com.xwzy.pc-specs`（终端 `rm` 删不掉），再启动。测试用过的 `com.xwzy.pc-specs.sandbox-test` 容器、以及 ad-hoc 试跑留下的文件夹授权，也可以一起清理。只影响本机。

## 6. 提审与响应

- [ ] 提审；审核结果出来后：通过就在 ASC 手动发布并核对商店页；被拒就按下面的预案回复
  - **2.1 信息不全 / 无法测试**：备注里已经写了完整的测试路径；存储分析用任意文件夹即可，不需要样例文件
  - **2.4.5 沙盒 / 文件访问**：只申请了用户选择读写和应用范围书签；所有文件操作都来自用户在对话框里亲自选的位置；开机自启用 `SMAppService`，默认关闭
  - **2.5.1 非公开 API（ITMS-90338）**：商店版编译时给 GPUI 打了补丁、关掉了传感器和菜单栏高亮的私有调用；邮件里列出具体名字的话，照 `docs/APP_STORE_COMPLIANCE.md` 第 3 节的办法补进补丁，再 `scripts/macos/scan-private-api.sh` 复查
  - **5.1.1 隐私**：隐私政策已托管，应用没有网络权限、不收集数据；隐私清单与 ASC 答复一致
  - **2.3 截图或描述与功能不符**：截图是真实界面；描述里只写了商店版有的功能（没有温度传感器、进程列表、清理等）
  - **5.2.1 / 2.3.7 商标**：应用图标和元数据里没有第三方商标（图标上原有的显卡品牌字样已抹掉）
  - **4.2 功能太少**：这是一个完整的系统信息与监控工具，不是网页包装，可以指出菜单栏监控、存储分析、监控大屏、规则通知这几块

## 7. 可选 / 之后再做

- [ ] 中国大陆 ICP 备案，备案完成后再开放中国大陆区
- [ ] 在应用里加一页「开源许可」（依赖里没有 GPL 类协议，但 MIT / Apache 等许可证要求保留版权声明）
- [ ] macOS 26 的 Liquid Glass 新图标（Icon Composer 做 `.icon`），现在的传统图标会被系统套一层底板，不影响上架
- [ ] App Preview 视频（可选，最多 3 个）
- [ ] Windows、Linux 上的实机验证：这两个平台目前只做了编译和单元测试，没有在真机上逐页点过
- [ ] 传感器键表目前只在 M2 Max 上验证过（只影响全功能版）

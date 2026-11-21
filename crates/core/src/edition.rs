//! 发行版开关：同一份代码编出两个版本，差别在编译期就定了。
//!
//! | | 全功能版（自行发布） | 苹果商店版 |
//! |---|---|---|
//! | 构建 | `cargo build --release` | `cargo build --release --no-default-features --features appstore` |
//! | 沙盒 | 无 | App Sandbox（Mac App Store 强制） |
//! | 系统接口 | 能读的都读 | 只用公开接口 |
//!
//! 商店版必须在沙盒里运行，并且不能引用私有接口，所以下面这些能力在它里面不存在：界面里不摆出来，
//! 相关代码也不编进去（不是运行时关掉）。判断「这个版本有没有某项能力」一律用这里的常量，
//! 不要在别处直接写 `cfg!(feature = "appstore")`，这样想知道两个版本差在哪，看这一个文件就够了。
//! 依据和逐条理由见 `docs/APP_STORE_COMPLIANCE.md`。

#[cfg(all(feature = "full", feature = "appstore"))]
compile_error!(
    "features `full` and `appstore` are mutually exclusive; \
     build the App Store edition with `--no-default-features --features appstore`"
);
#[cfg(not(any(feature = "full", feature = "appstore")))]
compile_error!("enable exactly one of the features `full` (default) or `appstore`");
#[cfg(all(feature = "appstore", not(target_os = "macos")))]
compile_error!("the App Store edition only exists on macOS");

/// 当前是不是苹果商店版。
pub const APP_STORE: bool = cfg!(feature = "appstore");

/// 版本代号：`full` 或 `appstore`。写进导出的诊断报告和接口返回的 `host.edition`，
/// 方便从一份报告看出它来自哪个版本（商店版没有传感器、进程这几块数据）；「关于」页按 `APP_STORE` 显示本地化的名字。
pub const NAME: &str = if APP_STORE { "appstore" } else { "full" };

/// 温度、风扇、电压、功耗传感器。macOS 上读它们要走私有的 HID / SMC 接口，沙盒也不让碰。
pub const SENSORS: bool = !APP_STORE;

/// 别的进程的 CPU / 内存 / 磁盘占用（进程列表）。沙盒里系统不允许一个应用枚举、查看别的进程。
pub const PROCESS_LIST: bool = !APP_STORE;

/// 联网：公网 IP、网络延迟探测、Webhook。商店版不申请任何网络权限，所以完全不联网。
pub const NETWORK_ACCESS: bool = !APP_STORE;

/// 本机 HTTP 采集接口（监听端口需要网络权限）。
pub const LOCAL_SERVER: bool = !APP_STORE;

/// 规则触发时运行用户写的命令（沙盒里只能运行系统自带的工具，没有意义，也不该提供）。
pub const RUN_COMMANDS: bool = !APP_STORE;

/// 管理别的软件和系统：应用卸载、启动项管理、编程环境检测、系统缓存清理。
/// 它们都要读写用户目录之外或别的应用的数据，沙盒不允许。
pub const SYSTEM_MANAGEMENT: bool = !APP_STORE;

/// 在沙盒里运行：只能访问用户在「选择文件夹」对话框里亲自选的位置，
/// 所以存储分析要用对话框授权，授权用书签记到下次启动。
pub const SANDBOXED: bool = APP_STORE;

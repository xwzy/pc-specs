//! PC Specs 数据层：采集、实时采样、导出与本地 HTTP 服务。
//!
//! 与任何界面框架无关，可以被 GPUI 应用、命令行工具或测试直接使用。

pub mod apps;
pub mod bench;
pub mod clean;
pub mod dupes;
pub mod edition;
pub mod exporter;
pub mod model;
pub mod modules;
pub mod monitor;
pub mod netprobe;
pub mod platform;
pub mod process_net;
#[cfg(not(feature = "appstore"))]
pub mod server;
/// 商店版没有本地 HTTP 服务，留一个同样接口的空壳（见 `server_off.rs`）。
#[cfg(feature = "appstore")]
#[path = "server_off.rs"]
pub mod server;
pub mod space;
pub mod startup;
pub mod util;

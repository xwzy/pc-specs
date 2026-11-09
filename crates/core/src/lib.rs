//! PC Specs 数据层：采集、实时采样、导出与本地 HTTP 服务。
//!
//! 与任何界面框架无关，可以被 GPUI 应用、命令行工具或测试直接使用。

pub mod apps;
pub mod bench;
pub mod clean;
pub mod dupes;
pub mod exporter;
pub mod model;
pub mod modules;
pub mod monitor;
pub mod netprobe;
pub mod platform;
pub mod process_net;
pub mod server;
pub mod space;
pub mod startup;
pub mod util;

//! 商店版没有本地 HTTP 服务：沙盒里监听端口要网络权限，商店版不申请。
//!
//! 接口和 `server` 保持一致，上层不用为此写条件编译：`spawn` 永远返回「启动失败」，
//! 界面按 `edition::LOCAL_SERVER` 不提供相关开关，所以用户不会看到这个失败。

use std::net::SocketAddr;

/// 和全功能版的默认端口一致，上层拿它拼提示用的地址。
pub const DEFAULT_PORT: u16 = 16089;

/// 服务当前状态，字段含义同 `server::ServerStatus`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Starting,
    Listening(SocketAddr),
    Failed(String),
}

/// 服务句柄。商店版里它什么也不持有。
pub struct ServerHandle;

/// 全功能版的句柄被丢弃时会关掉服务、等端口释放，上层靠 `drop(handle)` 表达这件事。
/// 商店版没有东西可关，留一个空实现，让这些调用在两个版本里写法一样，也不会触发 `drop_non_drop`。
impl Drop for ServerHandle {
    fn drop(&mut self) {}
}

impl ServerHandle {
    pub fn status(&self) -> ServerStatus {
        ServerStatus::Failed("the local HTTP server is not part of the App Store edition".into())
    }
}

pub fn spawn(_addr: SocketAddr) -> ServerHandle {
    ServerHandle
}

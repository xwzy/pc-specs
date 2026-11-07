//! macOS 网络接口计数器的轻量读取。
//!
//! sysinfo 每次刷新网络，都会对每个接口调一次 `if_indextoname`（内部建一个套接字再关掉），
//! 再 `getifaddrs` 把所有地址收集一遍：二十来个接口，一秒钟就是上百次系统调用和几百次内存分配，
//! 而实时监控只要「接口名 + 累计收发字节」。这里先用一次 `NET_RT_IFLIST2` 拿到接口序号，
//! 再对每个序号读一次 `IFMIB_IFDATA`——它已经带着接口名和 64 位计数，别的一概不碰。

use std::{ffi::c_int, mem, ptr};

/// 一个接口开机以来累计收发的字节数。
pub struct Counters {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Default)]
pub struct Reader {
    /// 系统调用的接收缓冲，跨次复用，避免每秒分配一次。
    buf: Vec<u8>,
}

impl Reader {
    /// 读出所有接口的累计计数，追加到 `out`。系统调用失败时返回 `false`，`out` 保持不变。
    pub fn read(&mut self, out: &mut Vec<Counters>) -> bool {
        let Some(len) = self.list() else {
            return false;
        };
        let header = mem::size_of::<libc::if_msghdr2>();
        let mut offset = 0;
        // 消息按 `ifm_msglen` 首尾相接；除了 `RTM_IFINFO2`（接口本身），还夹着各接口的地址消息，跳过。
        while offset + header <= len {
            // SAFETY: 上面已确认 `offset + header <= len <= buf.len()`；消息头不一定对齐，所以按未对齐读取。
            let message = unsafe {
                ptr::read_unaligned(self.buf.as_ptr().add(offset).cast::<libc::if_msghdr2>())
            };
            let size = usize::from(message.ifm_msglen);
            if size == 0 {
                break;
            }
            if c_int::from(message.ifm_type) == libc::RTM_IFINFO2
                && let Some(counters) = interface(message.ifm_index)
            {
                out.push(counters);
            }
            offset += size;
        }
        true
    }

    /// 取回接口列表，返回有效字节数。缓冲不够就按系统给出的大小扩容再取。
    fn list(&mut self) -> Option<usize> {
        let mut mib = [libc::CTL_NET, libc::PF_ROUTE, 0, 0, libc::NET_RT_IFLIST2, 0];
        if self.buf.is_empty() {
            self.buf.resize(16 * 1024, 0);
        }
        for _ in 0..4 {
            let mut len = self.buf.len();
            // SAFETY: `buf` 是一块长度为 `len` 的可写缓冲，`mib` 是合法的六段名字。
            let status = unsafe {
                libc::sysctl(
                    mib.as_mut_ptr(),
                    mib.len() as _,
                    self.buf.as_mut_ptr().cast(),
                    &mut len,
                    ptr::null_mut(),
                    0,
                )
            };
            if status == 0 {
                return Some(len.min(self.buf.len()));
            }
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOMEM) {
                return None;
            }
            // 接口数量变多了：先问需要多大，多留一点余量应付两次调用之间又冒出来的接口。
            let mut needed = 0usize;
            // SAFETY: 传空缓冲只是询问所需长度。
            let status = unsafe {
                libc::sysctl(
                    mib.as_mut_ptr(),
                    mib.len() as _,
                    ptr::null_mut(),
                    &mut needed,
                    ptr::null_mut(),
                    0,
                )
            };
            if status != 0 {
                return None;
            }
            self.buf.resize(needed + needed / 4 + 1024, 0);
        }
        None
    }
}

/// 读一个接口的名字和计数。接口刚好消失、系统不给数据时返回 `None`。
fn interface(index: u16) -> Option<Counters> {
    let mut mib = [
        libc::CTL_NET,
        libc::PF_LINK,
        libc::NETLINK_GENERIC,
        libc::IFMIB_IFDATA,
        c_int::from(index),
        libc::IFDATA_GENERAL,
    ];
    let mut data = mem::MaybeUninit::<libc::ifmibdata>::zeroed();
    let mut size = mem::size_of::<libc::ifmibdata>();
    // SAFETY: `data` 是一块够大的可写缓冲，`size` 是它的长度；结构体全是整数，全零也是合法值。
    let status = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as _,
            data.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if status != 0 || size < mem::size_of::<libc::ifmibdata>() {
        return None;
    }
    // SAFETY: 见上，内核写满或保持全零，都是有效的整数结构体。
    let data = unsafe { data.assume_init() };
    let name: Vec<u8> = data
        .ifmd_name
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    if name.is_empty() {
        return None;
    }
    Some(Counters {
        name: String::from_utf8_lossy(&name).into_owned(),
        rx_bytes: data.ifmd_data.ifi_ibytes,
        tx_bytes: data.ifmd_data.ifi_obytes,
    })
}

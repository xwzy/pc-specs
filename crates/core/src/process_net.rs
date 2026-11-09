//! 按进程统计网络流量。
//!
//! macOS 的 `netstat -anv` 会列出每个套接字累计收发的字节数和所属进程。隔一段时间读两次，
//! 每个套接字的增量按进程加起来，就是这段时间里这个进程的上下行速率。读一次只要十来毫秒；
//! `nettop` 也能给出同样的数据，但它常驻时要占掉大半个核。代价是两次读取之间开了又关的
//! 短连接看不到，对「谁在大量上传、下载」这个问题影响不大。本机回环不算网络流量，会被略过。
//!
//! 其它平台没有不需要特权就能拿到每个进程流量的办法，[`SUPPORTED`] 为假，调用方应当提示「不支持」。
use std::{collections::HashMap, time::Instant};

/// 当前系统能不能按进程统计网络流量。
pub const SUPPORTED: bool = cfg!(target_os = "macos");

/// 两次读取至少隔这么久（秒）才算速率，免得除出一个夸张的数。
const MIN_INTERVAL: f64 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    udp: bool,
    /// 内核给每个套接字的流水号（`gencnt`），在同一种协议里唯一。
    id: u64,
}

#[derive(Debug, PartialEq, Eq)]
struct Socket {
    key: Key,
    pid: u32,
    rx: u64,
    tx: u64,
}

/// 每个进程的网络速率采样器。第一次读取只是建立基线。
#[derive(Default)]
pub struct NetSampler {
    /// 上一次读到的每个套接字累计的（收，发）字节数。
    last: HashMap<Key, (u64, u64)>,
    at: Option<Instant>,
}

impl NetSampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// 读一遍套接字，返回距上次读取这段时间里每个进程的 `(下行, 上行)`，单位字节/秒，
    /// 没有流量的进程不在其中。建立基线的第一次、读取失败、两次读取离得太近时返回 `None`。
    pub fn sample(&mut self) -> Option<HashMap<u32, (u64, u64)>> {
        let sockets = read_sockets()?;
        let now = Instant::now();
        let elapsed = self
            .at
            .map(|at| now.saturating_duration_since(at).as_secs_f64());
        if elapsed.is_some_and(|secs| secs < MIN_INTERVAL) {
            return None;
        }
        let last = std::mem::replace(
            &mut self.last,
            sockets.iter().map(|s| (s.key, (s.rx, s.tx))).collect(),
        );
        self.at = Some(now);
        Some(rates(&sockets, &last, elapsed?))
    }
}

/// 两次读取之间每个进程的速率。这次新出现的套接字是在这段时间里才建立的，它的累计值就是这段时间的流量。
fn rates(
    sockets: &[Socket],
    last: &HashMap<Key, (u64, u64)>,
    seconds: f64,
) -> HashMap<u32, (u64, u64)> {
    let mut bytes: HashMap<u32, (u64, u64)> = HashMap::new();
    for socket in sockets {
        let (rx, tx) = last.get(&socket.key).copied().unwrap_or((0, 0));
        let (rx, tx) = (socket.rx.saturating_sub(rx), socket.tx.saturating_sub(tx));
        if rx | tx != 0 {
            let total = bytes.entry(socket.pid).or_default();
            total.0 = total.0.saturating_add(rx);
            total.1 = total.1.saturating_add(tx);
        }
    }
    let per_second = |bytes: u64| (bytes as f64 / seconds).round() as u64;
    bytes
        .into_iter()
        .map(|(pid, (rx, tx))| (pid, (per_second(rx), per_second(tx))))
        .collect()
}

#[cfg(target_os = "macos")]
fn read_sockets() -> Option<Vec<Socket>> {
    use crate::util::run_stdout_timeout;
    use std::time::Duration;

    let mut sockets = Vec::new();
    let mut any = false;
    for protocol in ["tcp", "udp"] {
        if let Some(text) = run_stdout_timeout(
            "/usr/sbin/netstat",
            &["-anv", "-p", protocol],
            Duration::from_secs(3),
        ) {
            any = true;
            sockets.extend(parse_netstat(&text));
        }
    }
    any.then_some(sockets)
}

#[cfg(not(target_os = "macos"))]
fn read_sockets() -> Option<Vec<Socket>> {
    None
}

/// 本机回环地址。`netstat` 把端口接在地址后面，用点隔开：`127.0.0.1.8080`、`::1.8080`。
fn is_loopback(address: &str) -> bool {
    address.starts_with("127.") || address.starts_with("::1.") || address == "::1"
}

/// 连接的两端是同一个 IPv4 地址：自己连自己（比如用本机的局域网地址访问本机的服务），
/// 没有真正走网络，接收方的计数还会翻倍。IPv6 地址在 `netstat` 里被截断过，不能拿来比较。
fn same_host(local: &str, foreign: &str) -> bool {
    let host = |address: &'_ str| address.rsplit_once('.').map(|(host, _)| host.to_string());
    match (host(local), host(foreign)) {
        (Some(a), Some(b)) => a == b && a.matches('.').count() == 3,
        _ => false,
    }
}

/// 解析 `netstat -anv -p tcp|udp` 的输出。每行是
///
/// ```text
/// tcp4  0  0  本地地址  对端地址  状态  收字节  发字节  rhiwat  shiwat  进程名:pid  state options gencnt flags flags1 usecnt rtncnt fltrs
/// udp4  0  0  本地地址  对端地址        收字节  发字节  rhiwat  shiwat  进程名:pid  ...（UDP 没有状态列）
/// ```
///
/// 进程名里可能有空格，所以后面固定的八列从右往左数、前面固定的几列从左往右数，中间都算名字。
/// 本机回环、自己连自己、没有属主的套接字略过。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_netstat(text: &str) -> Vec<Socket> {
    text.lines().filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Socket> {
    /// `process:pid` 之后的列：state options gencnt flags flags1 usecnt rtncnt fltrs。
    const TRAILING: usize = 8;
    let fields: Vec<&str> = line.split_whitespace().collect();
    let protocol = *fields.first()?;
    let udp = protocol.starts_with("udp");
    if !udp && !protocol.starts_with("tcp") {
        return None;
    }
    // TCP 多一列状态，后面的列整体后移一位。
    let first_byte_column = if udp { 5 } else { 6 };
    // 收、发、rhiwat、shiwat 四列，至少一个 `进程名:pid`，再加上结尾的固定几列。
    if fields.len() < first_byte_column + 4 + 1 + TRAILING {
        return None;
    }
    let (local, foreign) = (fields[3], fields[4]);
    if is_loopback(local) || is_loopback(foreign) || same_host(local, foreign) {
        return None;
    }
    let rx: u64 = fields[first_byte_column].parse().ok()?;
    let tx: u64 = fields[first_byte_column + 1].parse().ok()?;
    let id = u64::from_str_radix(fields[fields.len() - TRAILING + 2], 16).ok()?;
    let pid: u32 = fields[fields.len() - TRAILING - 1]
        .rsplit_once(':')?
        .1
        .parse()
        .ok()?;
    (pid != 0).then_some(Socket {
        key: Key { udp, id },
        pid,
        rx,
        tx,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TCP: &str = "\
Active Internet connections (including servers)
Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)          rxbytes      txbytes  rhiwat  shiwat          process:pid    state  options           gencnt    flags   flags1 usecnt rtncnt fltrs
tcp4       0    517  192.168.5.6.55108      134.195.101.187.443    ESTABLISHED            0          517  131376  131376     verge-mihomo:2289   00182 00000008 0000000000a8bc16 00000080 04000800      2      0 000000
tcp4       0      0  198.18.0.1.55107       198.18.0.52.443        ESTABLISHED       100000      2000000  277388  134220 Cursor Helper (P:1439   00102 00000108 0000000000a8bc15 00000001 04000800      2      0 000000
tcp6       0      0  2408:8207:1912:9.55035 2408:8740:c1fc:4.443   ESTABLISHED        11970        14613  262144  131072     verge-mihomo:2289   00182 00000008 0000000000a8baf1 00000080 04000800      2      0 000000
tcp4       0      0  127.0.0.1.8080         127.0.0.1.50000        ESTABLISHED         5000         5000  131072  131072          workerd:2820   02131 00000000 0000000000a8bc20 00000080 04000800      2      0 000000
tcp6       0      0  ::1.8080               ::1.50001              ESTABLISHED         5000         5000  131072  131072          workerd:2820   02131 00000000 0000000000a8bc21 00000080 04000800      2      0 000000
tcp4       0      0  *.22                   *.*                    LISTEN                 0            0  131072  131072          launchd:1      00100 00000006 0000000000a8bc22 00000000 04000800      1      0 000000
tcp4       0      0  192.168.5.6.62071      192.168.5.6.8767       ESTABLISHED    151375822           82  131072  131072             curl:90637   02131 00000000 0000000000a8bc24 00000080 04000800      2      0 000000
tcp4       0      0  10.0.0.2.5000          10.0.0.3.6000          TIME_WAIT              7            9  131072  131072               :0         00000 00000000 0000000000a8bc23 00000000 04000800      1      0 000000
garbage line that is not a socket
";

    const UDP: &str = "\
Active Internet connections (including servers)
Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)          rxbytes      txbytes  rhiwat  shiwat          process:pid    state  options           gencnt    flags   flags1 usecnt rtncnt fltrs
udp4       0      0  *.58208                *.*                                       42540         7511       0    9216    WeatherWidget:910    ffffa130 00020000 0000000000a6eca3 00100009 04280808      1      0 000002
udp46      0      0  *.5353                 *.*                                           0            0  786896    9216 Google Chrome He:25547  00100 00000204 0000000000a58877 00000000 00000800      1      0 000002
udp4       0      0  127.0.0.1.9999         *.*                                         500          500  786896    9216       dnsmasq:300    00100 00000204 0000000000a58878 00000000 00000800      1      0 000002
";

    #[test]
    fn parses_tcp_and_skips_loopback_self_and_ownerless_sockets() {
        let sockets = parse_netstat(TCP);
        assert_eq!(
            sockets,
            vec![
                Socket {
                    key: Key {
                        udp: false,
                        id: 0xa8bc16
                    },
                    pid: 2289,
                    rx: 0,
                    tx: 517,
                },
                // 名字里有空格、被截断成一个括号：从右往左数仍然能找到 pid。
                Socket {
                    key: Key {
                        udp: false,
                        id: 0xa8bc15
                    },
                    pid: 1439,
                    rx: 100_000,
                    tx: 2_000_000,
                },
                Socket {
                    key: Key {
                        udp: false,
                        id: 0xa8baf1
                    },
                    pid: 2289,
                    rx: 11970,
                    tx: 14613,
                },
                // 监听中的套接字没有流量，但照样解析出来（增量为零，不会进排行）。
                Socket {
                    key: Key {
                        udp: false,
                        id: 0xa8bc22
                    },
                    pid: 1,
                    rx: 0,
                    tx: 0,
                },
            ]
        );
    }

    #[test]
    fn parses_udp_without_a_state_column() {
        let sockets = parse_netstat(UDP);
        assert_eq!(sockets.len(), 2, "the loopback socket is skipped");
        assert_eq!(
            sockets[0],
            Socket {
                key: Key {
                    udp: true,
                    id: 0xa6eca3
                },
                pid: 910,
                rx: 42540,
                tx: 7511,
            }
        );
        assert_eq!(sockets[1].pid, 25547);
    }

    #[test]
    fn rates_count_new_sockets_whole_and_old_ones_by_difference() {
        let key = |id| Key { udp: false, id };
        let socket = |id, pid, rx, tx| Socket {
            key: key(id),
            pid,
            rx,
            tx,
        };
        // 套接字 1 之前就在：只算增量；套接字 2 是这段时间新建的：累计值全算；
        // 套接字 3 没有变化；两个套接字属于同一个进程时要加起来。
        let last = HashMap::from([(key(1), (1000, 500)), (key(3), (50, 50))]);
        let now = [
            socket(1, 10, 3000, 700),
            socket(2, 10, 400, 100),
            socket(3, 20, 50, 50),
        ];
        let rates = rates(&now, &last, 2.);
        assert_eq!(rates.get(&10), Some(&(1200, 150)));
        assert_eq!(rates.get(&20), None, "idle processes are left out");
    }

    #[test]
    fn counters_that_go_backwards_do_not_underflow() {
        let key = Key { udp: true, id: 7 };
        let last = HashMap::from([(key, (900, 900))]);
        let now = [Socket {
            key,
            pid: 5,
            rx: 100,
            tx: 100,
        }];
        assert!(rates(&now, &last, 1.).is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn reads_the_real_socket_table() {
        // 不断言具体的流量，只确认命令能跑、输出能被解析。
        let mut sampler = NetSampler::new();
        assert!(sampler.sample().is_none(), "the first read is the baseline");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(sampler.sample().is_some());
    }
}

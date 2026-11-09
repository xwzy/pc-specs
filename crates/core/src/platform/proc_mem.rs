//! Linux 的内存构成：解析 `/proc/meminfo`、`/proc/vmstat` 和内存压力文件（PSI）。
//! 解析和读文件分开，样例文本就能在任何平台上测。

use crate::model::MemoryBreakdown;

/// `Key:   1234 kB` → 字节数。
fn field(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let rest = line.strip_prefix(key)?.strip_prefix(':')?;
        let kib: u64 = rest.split_whitespace().next()?.parse().ok()?;
        Some(kib.saturating_mul(1024))
    })
}

/// `/proc/vmstat` 里的累计计数。
fn counter(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(' ')?;
        (name == key).then(|| value.trim().parse().ok())?
    })
}

/// `/proc/pressure/memory` 第一行 `some avg10=0.00 ...`：最近 10 秒里有任务因等内存而停顿的时间占比。
fn pressure(psi: &str) -> Option<f32> {
    let line = psi.lines().find(|l| l.starts_with("some"))?;
    line.split_whitespace()
        .find_map(|part| part.strip_prefix("avg10="))?
        .parse()
        .ok()
        .filter(|v: &f32| v.is_finite())
}

pub fn parse(meminfo: &str, vmstat: &str, psi: Option<&str>) -> Option<MemoryBreakdown> {
    let total = field(meminfo, "MemTotal")?;
    let free = field(meminfo, "MemFree")?;
    let get = |key| field(meminfo, key).unwrap_or(0);

    // 文件缓存：页缓存 + 缓冲 + 可回收的内核缓存，共享内存（tmpfs）算在页缓存里但不能丢，要扣掉。
    let cached =
        (get("Buffers") + get("Cached") + get("SReclaimable")).saturating_sub(get("Shmem"));
    let wired = get("SUnreclaim") + get("KernelStack") + get("PageTables") + get("Percpu");
    let compressed = get("Zswap");
    // 其余都算应用占用，这样几部分加起来正好是总量。
    let app = total
        .saturating_sub(free)
        .saturating_sub(cached)
        .saturating_sub(wired)
        .saturating_sub(compressed);

    let pressure_percent = psi.and_then(pressure);
    // 阈值取自 PSI 的常见用法：10% 开始明显卡顿，30% 以上已经在抖动。
    let pressure_level = pressure_percent.map(|p| match p {
        p if p >= 30.0 => 2,
        p if p >= 10.0 => 1,
        _ => 0,
    });
    let page = 4096;
    Some(MemoryBreakdown {
        app_bytes: app,
        wired_bytes: wired,
        compressed_bytes: compressed,
        cached_bytes: cached,
        free_bytes: free,
        pressure_percent,
        pressure_level,
        swapped_in_bytes: counter(vmstat, "pswpin").unwrap_or(0) * page,
        swapped_out_bytes: counter(vmstat, "pswpout").unwrap_or(0) * page,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:       16000000 kB\nMemFree:         2000000 kB\nMemAvailable:    9000000 kB\nBuffers:          500000 kB\nCached:          6000000 kB\nShmem:           1000000 kB\nSReclaimable:     500000 kB\nSUnreclaim:       300000 kB\nKernelStack:       20000 kB\nPageTables:        80000 kB\n";

    #[test]
    fn parts_add_up_to_the_total() {
        let m = parse(MEMINFO, "pswpin 10\npswpout 20\n", None).unwrap();
        assert_eq!(m.free_bytes, 2_000_000 * 1024);
        // 缓冲 + 页缓存 + 可回收 − 共享内存
        assert_eq!(m.cached_bytes, 6_000_000 * 1024);
        assert_eq!(m.wired_bytes, 400_000 * 1024);
        let sum = m.app_bytes + m.wired_bytes + m.compressed_bytes + m.cached_bytes + m.free_bytes;
        assert_eq!(sum, 16_000_000 * 1024);
        assert_eq!(m.swapped_in_bytes, 10 * 4096);
        assert_eq!(m.swapped_out_bytes, 20 * 4096);
        assert_eq!(m.pressure_percent, None);
    }

    #[test]
    fn pressure_levels_follow_the_stall_time() {
        let level = |psi: &str| parse(MEMINFO, "", Some(psi)).unwrap().pressure_level;
        assert_eq!(
            level("some avg10=0.50 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00"),
            Some(0)
        );
        assert_eq!(
            level("some avg10=12.00 avg60=0.00 avg300=0.00 total=1"),
            Some(1)
        );
        assert_eq!(
            level("some avg10=45.00 avg60=0.00 avg300=0.00 total=1"),
            Some(2)
        );
        assert_eq!(level("garbage"), None);
    }

    #[test]
    fn incomplete_files_do_not_panic() {
        assert!(parse("", "", None).is_none());
        assert!(parse("MemTotal: 1 kB\n", "", None).is_none());
    }
}

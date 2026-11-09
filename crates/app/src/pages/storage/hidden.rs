//! 文件树里看不见的那部分空间：macOS 的「可清除空间」和时间机器的本地快照。

use std::path::Path;

/// 一个卷上，扫描本身交代不了的空间。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Extra {
    /// 系统在需要时会自己腾出来的字节数（缓存、可重新下载的内容等）。
    /// 它们是实实在在的文件，已经算在扫描结果里，只是不用自己动手清；Finder 把它们算进「可用」。
    pub purgeable: Option<u64>,
    /// 时间机器本地快照的个数：快照占的空间在文件树里一个字节也看不到，却占着磁盘。
    pub snapshots: usize,
}

/// 问一下 `path` 所在的卷。只有 macOS 有这两样东西，别的系统返回空。
pub fn probe(path: &Path) -> Extra {
    #[cfg(target_os = "macos")]
    {
        Extra {
            purgeable: purgeable(path),
            snapshots: snapshots(path),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Extra::default()
    }
}

/// `path` 所在卷上可清除的字节数：Finder 里的「可用」减去 statfs 报的「空闲」。
#[cfg(not(target_os = "macos"))]
pub fn purgeable(_: &Path) -> Option<u64> {
    None
}

#[cfg(target_os = "macos")]
pub fn purgeable(path: &Path) -> Option<u64> {
    use objc2_foundation::{
        NSArray, NSNumber, NSString, NSURL, NSURLVolumeAvailableCapacityForImportantUsageKey,
        NSURLVolumeAvailableCapacityKey,
    };
    let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
    // SAFETY: 这两个是系统框架导出的常量，永远有效。
    let (plain, important) = unsafe {
        (
            NSURLVolumeAvailableCapacityKey,
            NSURLVolumeAvailableCapacityForImportantUsageKey,
        )
    };
    let values = url
        .resourceValuesForKeys_error(&NSArray::from_slice(&[plain, important]))
        .ok()?;
    let read = |key| {
        let value = values.objectForKey(key)?;
        value
            .downcast_ref::<NSNumber>()
            .map(NSNumber::longLongValue)
    };
    let purgeable = read(important)?.checked_sub(read(plain)?)?;
    u64::try_from(purgeable).ok().filter(|&bytes| bytes > 0)
}

#[cfg(target_os = "macos")]
fn snapshots(path: &Path) -> usize {
    std::process::Command::new("tmutil")
        .arg("listlocalsnapshots")
        .arg(path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or(0, |output| {
            count_snapshots(&String::from_utf8_lossy(&output.stdout))
        })
}

/// `tmutil listlocalsnapshots` 的输出：一行标题，之后每个快照一行 `com.apple.TimeMachine.….local`。
#[cfg(any(target_os = "macos", test))]
fn count_snapshots(output: &str) -> usize {
    output
        .lines()
        .filter(|line| line.trim_start().starts_with("com.apple."))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_are_counted_from_tmutil_output() {
        assert_eq!(count_snapshots("Snapshots for disk /:\n"), 0);
        assert_eq!(
            count_snapshots(
                "Snapshots for disk /:\n\
                 com.apple.TimeMachine.2026-11-07-083012.local\n\
                 com.apple.TimeMachine.2026-11-08-091500.local\n"
            ),
            2
        );
        assert_eq!(count_snapshots(""), 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn purgeable_space_of_the_system_volume_is_readable() {
        // 取得到时不能比整块盘还大；取不到（没有可清除空间）也是合法的。
        if let Some(bytes) = purgeable(Path::new("/")) {
            let total = pc_specs_core::space::volume_of(Path::new("/"))
                .expect("volume")
                .total;
            assert!(bytes < total);
        }
    }
}

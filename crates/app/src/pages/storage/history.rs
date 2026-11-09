//! 扫描历史：每次扫完，把各个文件夹的大小存一份；下次再扫同一个位置，就能说出这段时间里什么变大了、什么变小了。
//!
//! 存档只记够大的文件夹（核心库的 `space::Snapshot`），一个位置一个文件，只留最近一次。
//! 存档文件的命名和写法（`file_in`、`save`）速度测试的记录也用。

use pc_specs_core::space::{Change, Snapshot};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// 这一次和上一次扫描比出来的结果。
pub(super) struct History {
    /// 上一次扫描的时间，Unix 秒。
    pub since: u64,
    /// 上一次扫描时整个位置有多大。
    pub before: u64,
    pub changes: Vec<Change>,
}

/// 扫描起点对应的存档文件：文件名是起点路径的哈希，不同位置互不干扰。
pub(super) fn file_in(dir: &Path, root: &Path) -> PathBuf {
    // FNV-1a：不用密码学强度，只要稳定、分得开。`DefaultHasher` 不保证跨版本稳定。
    let hash = root
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        });
    dir.join(format!("{hash:016x}.json"))
}

/// 存档放哪：和别的设置、记录一起，在用户配置目录下。
fn default_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("pc-specs").join("space"))
}

/// 读出上一次的存档，换成这一次的，并比较两者。第一次扫描这个位置，没有可比的，返回 `None`。
pub(super) fn record(root: &Path, current: &Snapshot) -> Option<History> {
    record_in(&default_dir()?, root, current)
}

fn record_in(dir: &Path, root: &Path, current: &Snapshot) -> Option<History> {
    let path = file_in(dir, root);
    let previous: Option<Snapshot> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    save(&path, current);
    let old = previous?;
    Some(History {
        since: old.taken,
        before: old.total,
        changes: current.changes_since(&old),
    })
}

pub(super) fn save(path: &Path, value: &impl Serialize) {
    let Ok(text) = serde_json::to_string(value) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // 先写临时文件再改名：写到一半断电，也不会把上一次的存档弄坏。
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, text).is_ok() {
        let _ = std::fs::rename(&temp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(taken: u64, dirs: &[(&str, u64)]) -> Snapshot {
        Snapshot {
            taken,
            total: dirs.iter().map(|(_, size)| size).sum(),
            dirs: dirs
                .iter()
                .map(|&(path, size)| (path.to_string(), size))
                .collect(),
        }
    }

    #[test]
    fn the_second_scan_of_a_place_is_compared_with_the_first() {
        let dir = std::env::temp_dir().join(format!("pcs-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (here, there) = (Path::new("/data/here"), Path::new("/data/there"));
        const GB: u64 = 1 << 30;

        let first = snapshot(100, &[("logs", GB)]);
        assert!(
            record_in(&dir, here, &first).is_none(),
            "nothing to compare"
        );

        let second = snapshot(200, &[("logs", 3 * GB)]);
        let history = record_in(&dir, here, &second).expect("has a predecessor");
        assert_eq!((history.since, history.before), (100, GB));
        assert_eq!(history.changes.len(), 1);
        assert_eq!(history.changes[0].delta(), 2 * GB as i64);

        // 另一个位置有自己的存档；同一个位置第三次比的是第二次，不是第一次。
        assert!(record_in(&dir, there, &second).is_none());
        let third = record_in(&dir, here, &second).expect("has a predecessor");
        assert_eq!(third.since, 200);
        assert!(third.changes.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_archive_is_treated_as_no_history() {
        let dir = std::env::temp_dir().join(format!("pcs-history-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = Path::new("/data/broken");
        std::fs::write(file_in(&dir, root), "{ not json").unwrap();
        assert!(record_in(&dir, root, &snapshot(1, &[])).is_none());
        // 坏档已经被这次的覆盖，下一次就能正常比较。
        assert!(record_in(&dir, root, &snapshot(2, &[])).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

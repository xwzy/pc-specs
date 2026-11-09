//! 重复文件查找：找出内容完全相同的文件。读文件是最贵的事，所以一步步缩小范围：
//!
//! 1. 按字节数分组——长度不同的文件不可能相同，绝大多数文件在这一步就淘汰了；
//! 2. 同样长的，再读头尾各 64 KiB 比一比——几乎所有「长度碰巧一样」的文件在这一步分开；
//! 3. 还没分开的才把整个文件读一遍、算哈希（2 个互相独立的 64 位 SipHash，合起来 128 位）。
//!
//! 只读文件，不改动、不删除任何东西。找出来之后怎么处理由界面决定。

use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::{self, File};
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed};

/// 第二步读文件头、尾各多少字节。
const SAMPLE: u64 = 64 * 1024;
/// 同时读几个文件：再多对固态盘没有好处，对机械盘反而更慢。
const READERS: usize = 4;

/// 要比较的一个文件。
#[derive(Clone, Debug)]
pub struct Candidate {
    pub path: PathBuf,
    /// 最后修改时间，Unix 秒；0 表示不知道。用来在一组里挑出「原件」。
    pub modified: u32,
}

/// 内容完全相同的一组文件。
#[derive(Debug)]
pub struct Group {
    /// 每个文件的字节数。
    pub len: u64,
    /// 至少两个。最老的在前（通常就是原件），一样老的路径短的在前。
    pub files: Vec<Candidate>,
}

impl Group {
    /// 只留一份的话能省多少字节。
    pub fn wasted(&self) -> u64 {
        self.len * (self.files.len() as u64 - 1)
    }
}

/// 现在做到哪一步了；界面隔一会儿读一次。
#[derive(Default)]
pub struct Progress {
    /// [`Stage`] 的编号。
    stage: AtomicU8,
    /// 这一步已经处理了多少、一共有多少：第一、二步数的是文件，第三步数的是字节。
    done: AtomicU64,
    total: AtomicU64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// 量每个文件的长度。
    Sizes,
    /// 读头尾比一比。
    Samples,
    /// 读整个文件算哈希。
    Content,
}

impl Progress {
    pub fn stage(&self) -> Stage {
        match self.stage.load(Relaxed) {
            0 => Stage::Sizes,
            1 => Stage::Samples,
            _ => Stage::Content,
        }
    }

    /// `(已处理, 一共)`。
    pub fn counts(&self) -> (u64, u64) {
        (self.done.load(Relaxed), self.total.load(Relaxed))
    }

    fn begin(&self, stage: Stage, total: u64) {
        self.stage.store(stage as u8, Relaxed);
        self.done.store(0, Relaxed);
        self.total.store(total, Relaxed);
    }
}

/// 一个文件的「指纹」：两个独立的 64 位哈希。
type Print = (u64, u64);

struct Hashers {
    a: RandomState,
    b: RandomState,
}

struct Digest {
    a: std::hash::DefaultHasher,
    b: std::hash::DefaultHasher,
}

impl Hashers {
    fn digest(&self) -> Digest {
        Digest {
            a: self.a.build_hasher(),
            b: self.b.build_hasher(),
        }
    }
}

impl Digest {
    fn write(&mut self, bytes: &[u8]) {
        self.a.write(bytes);
        self.b.write(bytes);
    }

    fn finish(&self) -> Print {
        (self.a.finish(), self.b.finish())
    }
}

/// 读 `path`（长 `len`）算指纹。`sample` 为 `Some` 且文件比头尾两块加起来还长时，只读头尾；
/// 否则读整个文件。读不出来（被删了、没权限、中途被取消）返回 `None`。
fn fingerprint(
    path: &Path,
    len: u64,
    sample: Option<u64>,
    hashers: &Hashers,
    progress: Option<&Progress>,
    cancel: &AtomicBool,
) -> Option<Print> {
    let mut file = File::open(path).ok()?;
    let mut digest = hashers.digest();
    // 长度也算进指纹，头尾一样、长度不同的不会混在一起。
    digest.write(&len.to_le_bytes());
    let mut buffer = vec![0u8; 256 * 1024];
    let mut read_range = |file: &mut File, mut left: u64| -> Option<()> {
        while left > 0 {
            if cancel.load(Relaxed) {
                return None;
            }
            let want = left.min(buffer.len() as u64) as usize;
            let got = file.read(&mut buffer[..want]).ok()?;
            if got == 0 {
                // 比量的时候短了：文件在这期间被截断，不再是同一个文件。
                return None;
            }
            digest.write(&buffer[..got]);
            left -= got as u64;
            if let Some(progress) = progress {
                progress.done.fetch_add(got as u64, Relaxed);
            }
        }
        Some(())
    };
    match sample.filter(|&s| len > 2 * s) {
        Some(sample) => {
            read_range(&mut file, sample)?;
            file.seek(SeekFrom::Start(len - sample)).ok()?;
            read_range(&mut file, sample)?;
        }
        None => read_range(&mut file, len)?,
    }
    Some(digest.finish())
}

/// 把一批（长度相同的）文件按指纹分组，只留下至少两个的。
fn regroup(len: u64, prints: Vec<(Print, Candidate)>) -> Vec<(u64, Vec<Candidate>)> {
    let mut groups: HashMap<Print, Vec<Candidate>> = HashMap::new();
    for (print, candidate) in prints {
        groups.entry(print).or_default().push(candidate);
    }
    groups
        .into_values()
        .filter(|files| files.len() > 1)
        .map(|files| (len, files))
        .collect()
}

/// 在 `candidates` 里找出内容完全相同的文件，浪费空间最多的一组在前。
/// `cancel` 被置位后尽快返回，结果为空。
pub fn find(candidates: Vec<Candidate>, progress: &Progress, cancel: &AtomicBool) -> Vec<Group> {
    // 第一步：按长度分组。
    progress.begin(Stage::Sizes, candidates.len() as u64);
    let mut by_len: HashMap<u64, Vec<Candidate>> = HashMap::new();
    for candidate in candidates {
        if cancel.load(Relaxed) {
            return Vec::new();
        }
        progress.done.fetch_add(1, Relaxed);
        let Ok(meta) = fs::metadata(&candidate.path) else {
            continue;
        };
        if meta.is_file() && meta.len() > 0 {
            by_len.entry(meta.len()).or_default().push(candidate);
        }
    }
    by_len.retain(|_, files| files.len() > 1);

    let hashers = Hashers {
        a: RandomState::new(),
        b: RandomState::new(),
    };
    let run = |work: Vec<(u64, Vec<Candidate>)>, sample: Option<u64>, count_bytes: bool| {
        let one = |(len, files): (u64, Vec<Candidate>)| {
            let prints: Vec<(Print, Candidate)> = files
                .into_iter()
                .filter_map(|candidate| {
                    let print = fingerprint(
                        &candidate.path,
                        len,
                        sample,
                        &hashers,
                        count_bytes.then_some(progress),
                        cancel,
                    )?;
                    if !count_bytes {
                        progress.done.fetch_add(1, Relaxed);
                    }
                    Some((print, candidate))
                })
                .collect();
            regroup(len, prints)
        };
        match rayon::ThreadPoolBuilder::new().num_threads(READERS).build() {
            Ok(pool) => pool.install(|| work.into_par_iter().flat_map(one).collect::<Vec<_>>()),
            Err(_) => work.into_iter().flat_map(one).collect::<Vec<_>>(),
        }
    };

    // 第二步：头尾比一比。
    let work: Vec<(u64, Vec<Candidate>)> = by_len.into_iter().collect();
    progress.begin(
        Stage::Samples,
        work.iter().map(|(_, files)| files.len() as u64).sum(),
    );
    let sampled = run(work, Some(SAMPLE), false);
    if cancel.load(Relaxed) {
        return Vec::new();
    }

    // 第三步：头尾一样、而且文件比头尾两块长的，才把整个文件读一遍。
    // 不比头尾两块长的，第二步读的已经是整个文件，不用再读。
    let (settled, pending): (Vec<_>, Vec<_>) =
        sampled.into_iter().partition(|(len, _)| *len <= 2 * SAMPLE);
    progress.begin(
        Stage::Content,
        pending
            .iter()
            .map(|(len, files)| len * files.len() as u64)
            .sum(),
    );
    let confirmed = run(pending, None, true);
    if cancel.load(Relaxed) {
        return Vec::new();
    }

    let mut groups: Vec<Group> = settled
        .into_iter()
        .chain(confirmed)
        .map(|(len, mut files)| {
            files.sort_by(|a, b| {
                // 没有修改时间（0）的排到最后，免得被误当成「最老的」。
                let age = |c: &Candidate| {
                    if c.modified == 0 {
                        u32::MAX
                    } else {
                        c.modified
                    }
                };
                age(a)
                    .cmp(&age(b))
                    .then_with(|| a.path.as_os_str().len().cmp(&b.path.as_os_str().len()))
                    .then_with(|| a.path.cmp(&b.path))
            });
            Group { len, files }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.wasted()
            .cmp(&a.wasted())
            .then_with(|| a.files[0].path.cmp(&b.files[0].path))
    });
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 放在临时目录里的一批文件，用完自动删掉。
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Scratch {
            let path = std::env::temp_dir().join(format!("pcs-dupes-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }

        fn write(&self, name: &str, bytes: &[u8]) -> Candidate {
            let path = self.0.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            Candidate { path, modified: 0 }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn run(candidates: Vec<Candidate>) -> Vec<Group> {
        find(candidates, &Progress::default(), &AtomicBool::new(false))
    }

    fn file_names(group: &Group) -> Vec<String> {
        group
            .files
            .iter()
            .map(|c| c.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn identical_files_are_grouped_whatever_they_are_called() {
        let dir = Scratch::new("same");
        let small = vec![1u8; 10_000];
        let big: Vec<u8> = (0..400_000u32).map(|i| (i % 251) as u8).collect();
        let groups = run(vec![
            dir.write("a/small.bin", &small),
            dir.write("b/copy-of-small.bin", &small),
            dir.write("a/big.bin", &big),
            dir.write("c/big (1).bin", &big),
            dir.write("d/big copy.bin", &big),
            dir.write("a/unique.bin", &[9u8; 5_000]),
        ]);
        assert_eq!(groups.len(), 2);
        // 三份大文件浪费得多，排在前面；浪费 = 单个大小 × (份数 − 1)。
        assert_eq!((groups[0].len, groups[0].files.len()), (400_000, 3));
        assert_eq!(groups[0].wasted(), 800_000);
        assert_eq!((groups[1].len, groups[1].files.len()), (10_000, 2));
    }

    #[test]
    fn same_length_and_same_ends_but_different_middle_are_told_apart() {
        let dir = Scratch::new("middle");
        // 头尾各 64 KiB 完全一样，只有正中间差一个字节：只有读整个文件才分得开。
        let one = vec![5u8; 500_000];
        let mut two = one.clone();
        two[250_000] = 6;
        let groups = run(vec![
            dir.write("one.bin", &one),
            dir.write("two.bin", &two),
            dir.write("twin.bin", &one),
        ]);
        assert_eq!(groups.len(), 1);
        let mut found = file_names(&groups[0]);
        found.sort();
        assert_eq!(found, ["one.bin", "twin.bin"]);
    }

    #[test]
    fn empty_missing_and_lone_files_are_ignored() {
        let dir = Scratch::new("odd");
        let missing = Candidate {
            path: dir.0.join("not-there"),
            modified: 0,
        };
        let all = vec![
            dir.write("empty-1", b""),
            dir.write("empty-2", b""),
            dir.write("lonely", &[1, 2, 3]),
            missing,
        ];
        assert!(run(all).is_empty());
        assert!(run(Vec::new()).is_empty());
    }

    #[test]
    fn the_oldest_copy_comes_first() {
        let dir = Scratch::new("order");
        let mut original = dir.write("deep/folder/original.bin", &[7u8; 2_000]);
        let mut copy = dir.write("copy.bin", &[7u8; 2_000]);
        let unknown = dir.write("unknown.bin", &[7u8; 2_000]);
        original.modified = 1_000;
        copy.modified = 2_000;
        // 修改时间不详的排最后，不会被当成原件。
        let groups = run(vec![unknown, copy, original]);
        assert_eq!(
            file_names(&groups[0]),
            ["original.bin", "copy.bin", "unknown.bin"]
        );
    }

    #[test]
    fn progress_ends_complete_and_cancelling_returns_nothing() {
        let dir = Scratch::new("progress");
        let bytes = vec![3u8; 300_000];
        let all = vec![dir.write("a", &bytes), dir.write("b", &bytes)];

        let progress = Progress::default();
        let groups = find(all.clone(), &progress, &AtomicBool::new(false));
        assert_eq!(groups.len(), 1);
        // 最后一步读的是整个文件：两个 300 000 字节，一个不多一个不少。
        assert_eq!(progress.stage(), Stage::Content);
        assert_eq!(progress.counts(), (600_000, 600_000));

        assert!(find(all, &Progress::default(), &AtomicBool::new(true)).is_empty());
    }
}

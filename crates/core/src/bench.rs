//! 磁盘速度测试：顺序读写，和 4K 随机读写。
//!
//! 在要测的卷上写一个临时文件，绕开系统缓存去读写它，测完就删：
//!
//! - 绕开缓存：macOS 用 `F_NOCACHE`，Linux 用 `O_DIRECT`，Windows 用 `FILE_FLAG_NO_BUFFERING`。
//!   不绕的话读出来的是内存的速度。文件系统不认的（tmpfs 之类）会退回普通读写，
//!   结果里的 [`Report::buffered`] 会说明，数字可能偏高。
//! - 顺序读写按 8 MiB 一块（块太小，固态盘喂不饱，测出来比真实速度低一大截），写完再从头读一遍。
//!   慢盘（U 盘、机械盘）受 [`Options::seq_secs`] 限制，写到时间就停，读的也只读写下来的那一段，
//!   不会为了凑够大小而跑上几分钟。
//! - 4K 随机读写在写下来的那段里随机挑 4 KiB 对齐的位置，各测 [`Options::random_secs`] 秒：
//!   读测一个线程（队列深度 1，看延迟）和 8 个线程（看固态盘的并行能力），写测一个线程。
//! - 写的内容是不可压缩的随机数据，每块还不一样，压缩和去重都占不到便宜。
//! - 临时文件不会被留下：Unix 上创建之后马上从目录里摘掉（句柄还开着，照常读写，
//!   空间在进程退出时归还），Windows 上用「句柄关闭就删除」。所以哪怕测到一半程序被杀，
//!   也不会留下一个几 GiB 的垃圾文件。
//!
//! 测的只是这个临时文件，不碰卷上别的东西；留出 64 MiB 空闲空间不占。

use crate::space;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 顺序读写一次处理多少。在苹果的固态盘上实测：1 MiB 一块约 2.6 GB/s，8 MiB 一块约 5.3 GB/s。
const BLOCK: usize = 8 << 20;
/// 随机读写的单位，也是所有缓冲区、偏移量、长度的对齐（绕开缓存时系统要求按扇区对齐）。
const SECTOR: usize = 4096;
/// 比这还小的测试文件（不到几块）没有意义。
const MIN_SIZE: u64 = 32 << 20;
/// 不去占的空闲空间。
const RESERVE: u64 = 64 << 20;
/// 并行随机读用几个线程。
pub const THREADS: usize = 8;
/// 临时文件的名字。
const FILE_NAME: &str = ".pc-specs-speedtest.tmp";
/// 延迟样本最多记这么多个，免得极快的盘把内存撑大。
const MAX_SAMPLES: usize = 1 << 20;

#[cfg(windows)]
const FILE_FLAG_NO_BUFFERING: u32 = 0x2000_0000;
#[cfg(windows)]
const FILE_FLAG_WRITE_THROUGH: u32 = 0x8000_0000;
#[cfg(windows)]
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;

/// 测试的各个阶段，也是界面上依次显示的顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    SeqWrite,
    SeqRead,
    RandRead,
    RandReadParallel,
    RandWrite,
}

impl Phase {
    pub const ALL: [Phase; 5] = [
        Phase::SeqWrite,
        Phase::SeqRead,
        Phase::RandRead,
        Phase::RandReadParallel,
        Phase::RandWrite,
    ];

    /// 顺序读写的速度以字节每秒计，随机读写以每秒多少次计。
    pub fn is_sequential(self) -> bool {
        matches!(self, Phase::SeqWrite | Phase::SeqRead)
    }

    fn index(self) -> u8 {
        Self::ALL.iter().position(|p| *p == self).unwrap_or(0) as u8
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    /// 测试文件有多大（字节）。空闲空间不够会自动缩小。
    pub size: u64,
    /// 顺序写、顺序读各最多测多少秒。
    pub seq_secs: f32,
    /// 每一项随机读写测多少秒。
    pub random_secs: f32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            size: 1 << 30,
            seq_secs: 8.0,
            random_secs: 3.0,
        }
    }
}

/// 一项 4K 随机读写的结果。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Random {
    /// 每秒完成多少次 4 KiB 的读写。
    pub iops: u32,
    /// 平均每次花多少微秒。
    pub avg_us: u32,
    /// 99% 的次数都在这么多微秒之内完成。
    pub p99_us: u32,
}

impl Random {
    /// 折算成吞吐量，字节每秒。
    pub fn bytes_per_sec(&self) -> u64 {
        u64::from(self.iops) * SECTOR as u64
    }
}

/// 一次测试的全部结果。还没测到的阶段是 0。
/// 会存成历史记录：缺的字段按默认值补，多出来的字段忽略，新旧版本的记录互相都读得了。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Report {
    /// 测试时间，Unix 秒。
    pub taken: u64,
    /// 实际写下来、拿来测的字节数。
    pub size: u64,
    /// 没能绕开系统缓存：读出来的数字可能是内存的速度。
    pub buffered: bool,
    /// 顺序写、顺序读，字节每秒。
    pub seq_write: u64,
    pub seq_read: u64,
    pub rand_read: Random,
    pub rand_read_parallel: Random,
    pub rand_write: Random,
}

/// 测试没有做完的原因。
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Cancelled,
    /// 空闲空间连最小的测试文件都放不下。
    NoSpace,
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Cancelled => f.write_str("cancelled"),
            Error::NoSpace => f.write_str("not enough free space"),
            Error::Io(message) => f.write_str(message),
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::StorageFull {
            Error::NoSpace
        } else {
            Error::Io(error.to_string())
        }
    }
}

/// 测试的进度，给界面轮询。后台线程写、界面读。
#[derive(Default)]
pub struct Progress {
    phase: AtomicU8,
    /// 当前阶段做了千分之几。
    permille: AtomicU32,
    /// 当前阶段此刻的速度：顺序读写是字节每秒，随机读写是每秒多少次。
    rate: AtomicU64,
    /// 已经测完的阶段的结果。
    report: Mutex<Report>,
    /// 哪些阶段已经测完（按 `Phase` 的顺序一位一位）。
    finished: AtomicU8,
}

impl Progress {
    pub fn phase(&self) -> Phase {
        Phase::ALL[usize::from(self.phase.load(Relaxed)).min(Phase::ALL.len() - 1)]
    }

    pub fn permille(&self) -> u32 {
        self.permille.load(Relaxed)
    }

    pub fn rate(&self) -> u64 {
        self.rate.load(Relaxed)
    }

    pub fn is_finished(&self, phase: Phase) -> bool {
        self.finished.load(Relaxed) & (1 << phase.index()) != 0
    }

    /// 到目前为止测完的结果。
    pub fn report(&self) -> Report {
        self.report.lock().clone()
    }

    fn begin(&self, phase: Phase) {
        self.phase.store(phase.index(), Relaxed);
        self.permille.store(0, Relaxed);
        self.rate.store(0, Relaxed);
    }

    fn update(&self, permille: u32, rate: u64) {
        self.permille.store(permille.min(1000), Relaxed);
        self.rate.store(rate, Relaxed);
    }

    fn finish(&self, phase: Phase, apply: impl FnOnce(&mut Report)) {
        apply(&mut self.report.lock());
        self.permille.store(1000, Relaxed);
        self.finished.fetch_or(1 << phase.index(), Relaxed);
    }
}

/// 在 `dir`（要测的卷上一个能写的目录）里跑一遍测试。`cancel` 置位就尽快停下并删掉临时文件。
pub fn run(
    dir: &Path,
    options: &Options,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<Report, Error> {
    fs::create_dir_all(dir)?;
    let free = space::volume_of(dir).map(|volume| volume.total.saturating_sub(volume.used));
    let wanted = fit(options.size, free).ok_or(Error::NoSpace)?;
    let path = dir.join(FILE_NAME);
    // 正常结束、出错、被取消都把文件删掉。（Unix 上它早就不在目录里了，这里是兜底。）
    let _remove = Remove(path.clone());
    let (file, direct) = create(&path)?;
    // 文件还开着，照常读写；但从目录里摘掉了：测到一半进程被杀，空间也会随进程退出归还。
    #[cfg(unix)]
    let _ = fs::remove_file(&path);

    let mut report = Report {
        taken: unix_now(),
        buffered: !direct,
        ..Report::default()
    };
    *progress.report.lock() = report.clone();
    let seq = Duration::from_secs_f32(options.seq_secs.max(0.1));
    let random = Duration::from_secs_f32(options.random_secs.max(0.1));
    let mut buffer = Buffer::new(BLOCK);
    Rng::new().fill(buffer.bytes_mut());

    // 顺序写。
    progress.begin(Phase::SeqWrite);
    let (written, rate) = sequential(&file, Op::Write, &mut buffer, wanted, seq, progress, cancel)?;
    file.sync_all()?;
    report.size = written;
    report.seq_write = rate;
    progress.finish(Phase::SeqWrite, |r| {
        r.size = written;
        r.buffered = !direct;
        r.seq_write = rate;
    });

    // 顺序读：只读写下来的那一段。
    forget_cache(&file);
    progress.begin(Phase::SeqRead);
    let (_, rate) = sequential(&file, Op::Read, &mut buffer, written, seq, progress, cancel)?;
    report.seq_read = rate;
    progress.finish(Phase::SeqRead, |r| r.seq_read = rate);

    // 4K 随机读（一个线程）、随机读（多个线程）、随机写。
    // 多个线程读的时候，Windows 的同步句柄一次只处理一个请求，要各用各的句柄才并得起来。
    let reopen = || -> io::Result<File> {
        #[cfg(windows)]
        {
            reopen_for_read(&path, direct)
        }
        #[cfg(not(windows))]
        {
            file.try_clone()
        }
    };
    let scatter = Scatter {
        file: &file,
        reopen: &reopen,
        extent: written,
        duration: random,
        progress,
        cancel,
    };
    progress.begin(Phase::RandRead);
    report.rand_read = scatter.run(Op::Read, 1)?;
    progress.finish(Phase::RandRead, |r| r.rand_read = report.rand_read.clone());

    progress.begin(Phase::RandReadParallel);
    report.rand_read_parallel = scatter.run(Op::Read, THREADS)?;
    progress.finish(Phase::RandReadParallel, |r| {
        r.rand_read_parallel = report.rand_read_parallel.clone()
    });

    progress.begin(Phase::RandWrite);
    report.rand_write = scatter.run(Op::Write, 1)?;
    file.sync_all()?;
    progress.finish(Phase::RandWrite, |r| {
        r.rand_write = report.rand_write.clone()
    });

    Ok(report)
}

/// 空闲 `free` 字节的卷上，想要 `wanted` 字节的测试文件，实际能用多大（按顺序读写的块取整）。
/// 连 [`MIN_SIZE`] 都放不下就是 `None`。不知道空闲多少（`None`）就照想要的来。
fn fit(wanted: u64, free: Option<u64>) -> Option<u64> {
    let usable = free.map_or(u64::MAX, |free| free.saturating_sub(RESERVE));
    let size = wanted.min(usable) / BLOCK as u64 * BLOCK as u64;
    (size >= MIN_SIZE).then_some(size)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

fn check(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Relaxed) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

/// 离开时（做完、出错、被取消）把临时文件删掉。
struct Remove(PathBuf);

impl Drop for Remove {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Read,
    Write,
}

/// 按扇区对齐的缓冲区。绕开缓存读写时，系统要求缓冲区的地址也对齐；
/// 多要一点内存、从对齐的位置开始用，就不必手写 `unsafe` 的分配。
struct Buffer {
    raw: Vec<u8>,
    start: usize,
    len: usize,
}

impl Buffer {
    fn new(len: usize) -> Buffer {
        let raw = vec![0; len + SECTOR];
        let start = raw.as_ptr().align_offset(SECTOR);
        Buffer { raw, start, len }
    }

    fn bytes(&self) -> &[u8] {
        &self.raw[self.start..self.start + self.len]
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.raw[self.start..self.start + self.len]
    }
}

/// xorshift64*：不用密码学强度，只要快、而且压不下去。
struct Rng(u64);

impl Rng {
    fn new() -> Rng {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64);
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn fill(&mut self, bytes: &mut [u8]) {
        for chunk in bytes.chunks_mut(8) {
            let word = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&word[..chunk.len()]);
        }
    }
}

/// 顺序读或写，直到读写够 `size` 字节、或者用完 `limit` 这么长时间。返回做了多少字节，和速度（字节每秒）。
fn sequential(
    file: &File,
    op: Op,
    buffer: &mut Buffer,
    size: u64,
    limit: Duration,
    progress: &Progress,
    cancel: &AtomicBool,
) -> Result<(u64, u64), Error> {
    let started = Instant::now();
    let (mut done, mut window) = (0u64, (Instant::now(), 0u64));
    while done < size && started.elapsed() < limit {
        check(cancel)?;
        match op {
            Op::Write => {
                // 每块的开头不一样：不然整个文件是同一块的重复，去重的文件系统会占便宜。
                let index = done / BLOCK as u64;
                buffer.bytes_mut()[..8].copy_from_slice(&index.to_le_bytes());
                write_all_at(file, buffer.bytes(), done)?;
            }
            Op::Read => read_exact_at(file, buffer.bytes_mut(), done)?,
        }
        done += BLOCK as u64;
        // 每 0.2 秒更新一次实时速度。
        if window.0.elapsed() >= Duration::from_millis(200) {
            let live = (done - window.1) as f64 / window.0.elapsed().as_secs_f64();
            window = (Instant::now(), done);
            let by_size = done as f64 / size as f64;
            let by_time = started.elapsed().as_secs_f64() / limit.as_secs_f64();
            progress.update((by_size.max(by_time) * 1000.0) as u32, live as u64);
        }
    }
    let seconds = started.elapsed().as_secs_f64().max(1e-6);
    Ok((done, (done as f64 / seconds) as u64))
}

/// 4K 随机读写这一轮里不变的东西。
#[derive(Clone, Copy)]
struct Scatter<'a> {
    file: &'a File,
    /// 另开一个句柄：多个线程同时读的时候，每个线程要有自己的。
    reopen: &'a (dyn Fn() -> io::Result<File> + Sync),
    /// 在文件开头多长的一段里挑位置。
    extent: u64,
    duration: Duration,
    progress: &'a Progress,
    cancel: &'a AtomicBool,
}

impl Scatter<'_> {
    /// 4K 随机读或写，持续 `duration`，用 `threads` 个线程同时做。
    fn run(self, op: Op, threads: usize) -> Result<Random, Error> {
        let Scatter {
            file,
            reopen,
            extent,
            duration,
            progress,
            cancel,
        } = self;
        let slots = (extent / SECTOR as u64).max(1);
        let stop = AtomicBool::new(false);
        let ops = AtomicU64::new(0);
        let started = Instant::now();

        let results: Vec<Result<Vec<u32>, Error>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| -> Result<Vec<u32>, Error> {
                        // 一个线程就用调用方的句柄；多个线程各拿一个自己的。
                        let own;
                        let file = if threads == 1 {
                            file
                        } else {
                            own = reopen()?;
                            &own
                        };
                        let mut buffer = Buffer::new(SECTOR);
                        let mut rng = Rng::new();
                        rng.fill(buffer.bytes_mut());
                        let mut samples = Vec::new();
                        while !stop.load(Relaxed) {
                            let offset = rng.next_u64() % slots * SECTOR as u64;
                            let began = Instant::now();
                            match op {
                                Op::Read => read_exact_at(file, buffer.bytes_mut(), offset)?,
                                Op::Write => {
                                    // 内容每次都变一点，不让设备把相同的写入合并掉。
                                    buffer.bytes_mut()[..8]
                                        .copy_from_slice(&rng.next_u64().to_le_bytes());
                                    write_all_at(file, buffer.bytes(), offset)?;
                                }
                            }
                            if samples.len() < MAX_SAMPLES / threads {
                                samples.push(
                                    began.elapsed().as_nanos().min(u128::from(u32::MAX)) as u32
                                );
                            }
                            ops.fetch_add(1, Relaxed);
                        }
                        Ok(samples)
                    })
                })
                .collect();

            // 干活的线程只管读写；这一个负责计时、报进度、看有没有被取消。
            while started.elapsed() < duration && !cancel.load(Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
                let elapsed = started.elapsed();
                progress.update(
                    (elapsed.as_secs_f64() / duration.as_secs_f64() * 1000.0) as u32,
                    (ops.load(Relaxed) as f64 / elapsed.as_secs_f64().max(1e-6)) as u64,
                );
            }
            stop.store(true, Relaxed);
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .join()
                        .unwrap_or_else(|_| Err(Error::Io("the test thread crashed".into())))
                })
                .collect()
        });
        check(cancel)?;

        let elapsed = started.elapsed().as_secs_f64().max(1e-6);
        let mut samples: Vec<u32> = Vec::new();
        for result in results {
            samples.extend(result?);
        }
        let total = ops.load(Relaxed);
        Ok(Random {
            iops: (total as f64 / elapsed) as u32,
            avg_us: average_us(&samples),
            p99_us: percentile_us(&mut samples, 0.99),
        })
    }
}

/// 延迟样本（纳秒）的平均值，微秒。
fn average_us(samples: &[u32]) -> u32 {
    if samples.is_empty() {
        return 0;
    }
    let sum: u64 = samples.iter().map(|ns| u64::from(*ns)).sum();
    (sum / samples.len() as u64 / 1000) as u32
}

/// 第 `p` 分位的延迟（样本是纳秒），微秒。会把样本排序。
fn percentile_us(samples: &mut [u32], p: f64) -> u32 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    let index = ((samples.len() - 1) as f64 * p).round() as usize;
    samples[index] / 1000
}

/// 创建测试文件（已有的就清空），尽量绕开系统缓存。第二个返回值是有没有绕开。
fn create(path: &Path) -> io::Result<(File, bool)> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(true);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // O_DIRECT：tmpfs、overlayfs 之类不认，认不得就退回普通方式。
        match options.clone().custom_flags(libc::O_DIRECT).open(path) {
            Ok(file) => Ok((file, true)),
            Err(error) if error.raw_os_error() == Some(libc::EINVAL) => {
                options.open(path).map(|file| (file, false))
            }
            Err(error) => Err(error),
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::AsRawFd as _;
        let file = options.open(path)?;
        // F_NOCACHE：这个文件的读写不进系统缓存。
        // SAFETY: `fd` 是刚打开的、属于 `file` 的有效描述符；`F_NOCACHE` 只带一个整数参数。
        let bypassed = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1) } != -1;
        Ok((file, bypassed))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // 先要「不走缓存、句柄关闭（包括进程被杀）就删文件」，不行就一项一项退一步。
        let direct = FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH;
        let mut failure = None;
        for (flags, bypassed) in [
            (direct | FILE_FLAG_DELETE_ON_CLOSE, true),
            (direct, true),
            (0, false),
        ] {
            match options.clone().custom_flags(flags).open(path) {
                Ok(file) => return Ok((file, bypassed)),
                Err(error) => failure = Some(error),
            }
        }
        Err(failure.unwrap_or_else(|| io::Error::other("cannot create the test file")))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        options.open(path).map(|file| (file, false))
    }
}

/// 给多个线程读用的另一个只读句柄（Windows 专用，见 `run`）。
#[cfg(windows)]
fn reopen_for_read(path: &Path, direct: bool) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    let flags = if direct {
        FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH
    } else {
        0
    };
    OpenOptions::new().read(true).custom_flags(flags).open(path)
}

/// 没能绕开缓存的 Linux：让内核把这个文件已经缓存的页扔掉，读的才是盘。其余系统用不着。
fn forget_cache(file: &File) {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd as _;
        // SAFETY: `fd` 是 `file` 的有效描述符；偏移和长度都是 0，表示整个文件。
        unsafe {
            libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = file;
}

#[cfg(unix)]
fn read_exact_at(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buffer, offset)
}

#[cfg(unix)]
fn write_all_at(file: &File, buffer: &[u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, buffer, offset)
}

/// Windows 的 `seek_read` 一次不一定读满，循环到读满为止。
#[cfg(windows)]
fn read_exact_at(file: &File, mut buffer: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt as _;
    while !buffer.is_empty() {
        match file.seek_read(buffer, offset)? {
            0 => return Err(io::ErrorKind::UnexpectedEof.into()),
            read => {
                buffer = &mut buffer[read..];
                offset += read as u64;
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn write_all_at(file: &File, mut buffer: &[u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt as _;
    while !buffer.is_empty() {
        match file.seek_write(buffer, offset)? {
            0 => return Err(io::ErrorKind::WriteZero.into()),
            written => {
                buffer = &buffer[written..];
                offset += written as u64;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pcs-bench-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn quick() -> Options {
        Options {
            size: 32 << 20,
            seq_secs: 5.0,
            random_secs: 0.3,
        }
    }

    #[test]
    fn a_whole_run_measures_every_phase_and_leaves_nothing_behind() {
        let dir = scratch("run");
        let progress = Progress::default();
        let report = run(&dir, &quick(), &progress, &AtomicBool::new(false)).expect("a report");

        assert_eq!(report.size, 32 << 20);
        assert!(report.seq_write > 0 && report.seq_read > 0, "{report:?}");
        for random in [
            &report.rand_read,
            &report.rand_read_parallel,
            &report.rand_write,
        ] {
            assert!(random.iops > 0, "{report:?}");
        }
        // 进度里留着同样的结果，每个阶段都标成了做完。
        assert_eq!(progress.report(), report);
        assert!(Phase::ALL.iter().all(|phase| progress.is_finished(*phase)));
        assert!(
            !dir.join(FILE_NAME).exists(),
            "the test file must be removed"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// Unix 上文件一创建就从目录里摘掉了：测到一半进程被杀，也不会留下几个 GiB 的临时文件。
    #[cfg(unix)]
    #[test]
    fn the_file_is_not_in_the_directory_while_the_test_runs() {
        let dir = scratch("hidden");
        let progress = Arc::new(Progress::default());
        let worker = {
            let (dir, progress) = (dir.clone(), progress.clone());
            std::thread::spawn(move || {
                let options = Options {
                    random_secs: 1.0,
                    ..quick()
                };
                run(&dir, &options, &progress, &AtomicBool::new(false))
            })
        };
        // 顺序写做完，文件早就创建了；后面还有三秒多的随机读写。
        let waited = Instant::now();
        while !progress.is_finished(Phase::SeqWrite) && waited.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(progress.is_finished(Phase::SeqWrite));
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        worker.join().unwrap().expect("a report");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancelling_stops_the_run_and_removes_the_file() {
        let dir = scratch("cancel");
        let cancel = Arc::new(AtomicBool::new(false));
        let stopper = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                cancel.store(true, Relaxed);
            })
        };
        let options = Options {
            size: 256 << 20,
            seq_secs: 30.0,
            random_secs: 30.0,
        };
        let started = Instant::now();
        let result = run(&dir, &options, &Progress::default(), &cancel);
        stopper.join().unwrap();
        assert_eq!(result, Err(Error::Cancelled));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "stopped promptly"
        );
        assert!(!dir.join(FILE_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slow_disk_is_cut_off_by_the_time_limit_not_the_size() {
        let dir = scratch("limit");
        let options = Options {
            size: 16 << 30,
            seq_secs: 0.1,
            random_secs: 0.1,
        };
        let started = Instant::now();
        let report = run(
            &dir,
            &options,
            &Progress::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
        // 16 GiB 在 0.1 秒内不可能写完：写下来的比想要的少，整轮也很快。
        assert!(report.size < 16 << 30, "{report:?}");
        assert!(started.elapsed() < Duration::from_secs(20));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_test_file_shrinks_to_the_free_space_and_never_below_the_minimum() {
        const GIB: u64 = 1 << 30;
        assert_eq!(fit(GIB, None), Some(GIB));
        assert_eq!(fit(GIB, Some(100 * GIB)), Some(GIB));
        // 空闲空间只够一半：留出保留空间，按块取整。
        assert_eq!(fit(GIB, Some(512 << 20)), Some(448 << 20));
        // 放不下最小的测试文件。
        assert_eq!(fit(GIB, Some(RESERVE + MIN_SIZE - 1)), None);
        assert_eq!(fit(GIB, Some(0)), None);
        assert_eq!(fit(4 << 20, None), None, "asking for less than the minimum");
    }

    #[test]
    fn latency_statistics_come_from_nanosecond_samples() {
        // 1..=100 微秒各一个。
        let mut samples: Vec<u32> = (1..=100).map(|us| us * 1000).collect();
        assert_eq!(average_us(&samples), 50);
        assert_eq!(percentile_us(&mut samples, 0.99), 99);
        assert_eq!(percentile_us(&mut samples, 0.0), 1);
        assert_eq!(average_us(&[]), 0);
        assert_eq!(percentile_us(&mut [], 0.99), 0);
    }

    #[test]
    fn buffers_are_aligned_and_the_noise_does_not_compress() {
        let mut buffer = Buffer::new(BLOCK);
        assert_eq!(buffer.bytes().as_ptr() as usize % SECTOR, 0);
        assert_eq!(buffer.bytes().len(), BLOCK);
        Rng::new().fill(buffer.bytes_mut());
        // 随机数据里几乎每个字节值都会出现，而且不是同一个字节的重复。
        let mut seen = [false; 256];
        for byte in buffer.bytes() {
            seen[usize::from(*byte)] = true;
        }
        assert!(seen.iter().filter(|seen| **seen).count() > 250);
        assert_ne!(buffer.bytes()[..4096], buffer.bytes()[4096..8192]);
    }
}

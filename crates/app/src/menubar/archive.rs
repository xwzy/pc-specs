//! 长期走势：把每秒一次的实时采样滚成两档粗粒度的历史，压缩后落盘。
//!
//! 面板里的图只有几十根柱子，没有人需要一个月前某一秒的读数，所以不存原始采样。
//! 做法和 RRD 一样：按时间分档，每档的格数固定，写满就覆盖最旧的。
//!
//! | 档位 | 一格    | 保留  | 格数  |
//! |------|---------|-------|-------|
//! | 近期 | 1 分钟  | 7 天  | 10080 |
//! | 长期 | 30 分钟 | 30 天 | 1440  |
//!
//! 每格、每条曲线只留「平均」和「峰值」两个数，各量化成 16 位；落盘时按列做差分再 zlib。
//! 两档都直接吃原始采样（而不是长期档去吃近期档），所以长期档的平均值是准的。
//!
//! 体量因此是固定的：内存里和未压缩的文件都是 11520 格 × 9 条曲线 × 4 字节 ≈ 415 KB，
//! 压缩后通常不到 100 KB——不管程序跑多久，都不会再长。
//!
//! 线程约定：界面线程只更新内存里的格子；压缩和写盘都交给后台线程，写入是「先写临时文件
//! 再改名」，进程在任何时刻被杀都不会留下半个文件。

use super::config::TimeRange;
use super::data::{Channel, Scale};
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use pc_specs_core::util::guarded;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 曲线条数，也是文件里的列数。
const N: usize = Channel::ALL.len();
/// 「这一格没有读数」。量化后的合法值都比它小。
const MISSING: u16 = u16::MAX;
/// 文件超过这个大小一律当作损坏：正常的文件远小于它，别把一个坏文件读进内存。
const MAX_FILE: u64 = 1 << 20;
/// 退出时最多等后台线程多久。
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

struct Spec {
    /// 一格多宽（毫秒）。
    width: u64,
    /// 最多留多少格。
    capacity: usize,
    /// 文件名。换了档位就换个名字，旧文件由启动时的清理带走。
    file: &'static str,
    /// 有新数据时，最久多久落一次盘。
    every: Duration,
}

const TIERS: [Spec; 2] = [
    Spec {
        width: 60_000,
        capacity: 7 * 24 * 60,
        file: "recent.bin",
        every: Duration::from_secs(15 * 60),
    },
    Spec {
        width: 30 * 60_000,
        capacity: 30 * 24 * 2,
        file: "long.bin",
        every: Duration::from_secs(60 * 60),
    },
];

/// 一个读数量化成 16 位：占用率和温度是定点数（精度 0.01），速率取对数（相对误差不到 0.1%）。
/// 速率从几字节到几 GB 差了九个数量级，定点数装不下。
fn quantize(channel: Channel, value: f64) -> u16 {
    if !value.is_finite() {
        return MISSING;
    }
    let q = match channel.scale() {
        Scale::Percent => value.clamp(0., 100.) * 100.,
        Scale::Celsius => (value.clamp(-50., 200.) + 50.) * 100.,
        Scale::Rate => (1. + value.max(0.)).log2() * 1000.,
    };
    q.round().clamp(0., 64_000.) as u16
}

fn dequantize(channel: Channel, q: u16) -> Option<f64> {
    if q == MISSING {
        return None;
    }
    let q = f64::from(q);
    Some(match channel.scale() {
        Scale::Percent => (q / 100.).min(100.),
        Scale::Celsius => (q / 100. - 50.).min(200.),
        Scale::Rate => (q / 1000.).exp2() - 1.,
    })
}

/// 一格：这一段时间里每条曲线的平均值和峰值。
#[derive(Clone, Copy)]
struct Cell {
    avg: [u16; N],
    peak: [u16; N],
}

const EMPTY: Cell = Cell {
    avg: [MISSING; N],
    peak: [MISSING; N],
};

/// 还在积累的一格。
struct Open {
    /// 第几格：时间戳除以格宽。
    slot: u64,
    sum: [f64; N],
    peak: [f64; N],
    count: [u32; N],
}

impl Open {
    fn new(slot: u64) -> Self {
        Self {
            slot,
            sum: [0.; N],
            peak: [f64::NEG_INFINITY; N],
            count: [0; N],
        }
    }

    fn add(&mut self, values: &[Option<f64>; N]) {
        for (i, value) in values.iter().enumerate() {
            if let Some(v) = value.filter(|v| v.is_finite()) {
                self.sum[i] += v;
                self.peak[i] = self.peak[i].max(v);
                self.count[i] += 1;
            }
        }
    }

    fn cell(&self) -> Cell {
        let mut cell = EMPTY;
        for (i, channel) in Channel::ALL.into_iter().enumerate() {
            if self.count[i] > 0 {
                cell.avg[i] = quantize(channel, self.sum[i] / f64::from(self.count[i]));
                cell.peak[i] = quantize(channel, self.peak[i]);
            }
        }
        cell
    }
}

/// 定长的环：连续的一串格子，`first` 是第一格的编号，写满就挤掉最旧的。
#[derive(Clone)]
struct Ring {
    first: u64,
    cells: VecDeque<Cell>,
    capacity: usize,
}

impl Ring {
    fn new(capacity: usize) -> Self {
        Self {
            first: 0,
            cells: VecDeque::new(),
            capacity,
        }
    }

    /// 下一个空位的编号。
    fn end(&self) -> u64 {
        self.first + self.cells.len() as u64
    }

    fn get(&self, slot: u64) -> Option<&Cell> {
        let index = usize::try_from(slot.checked_sub(self.first)?).ok()?;
        self.cells.get(index)
    }

    /// 把一格放到它该在的位置：中间隔着的时间补空格，写满了挤掉最旧的。
    fn put(&mut self, slot: u64, cell: Cell) {
        if self.cells.is_empty() {
            self.first = slot;
        } else if slot < self.first {
            // 比保留的最旧一格还早：已经过期。
            return;
        } else if slot < self.end() {
            // 同一格写第二次（重启后接着上次没记完的那一格）：以新的为准。
            self.cells[(slot - self.first) as usize] = cell;
            return;
        } else {
            let gap = slot - self.end();
            if gap >= self.capacity as u64 {
                // 隔得比整个窗口还久（很长时间没开机）：旧的全都过期了。
                self.cells.clear();
                self.first = slot;
            } else {
                self.cells.extend(std::iter::repeat_n(EMPTY, gap as usize));
            }
        }
        self.cells.push_back(cell);
        self.trim();
    }

    fn trim(&mut self) {
        while self.cells.len() > self.capacity {
            self.cells.pop_front();
            self.first += 1;
        }
    }

    /// 丢掉编号 `slot` 及之后的格子。
    fn cut(&mut self, slot: u64) {
        let keep = usize::try_from(slot.saturating_sub(self.first)).unwrap_or(usize::MAX);
        self.cells.truncate(keep);
    }

    /// 丢掉编号小于 `slot` 的格子。
    fn expire(&mut self, slot: u64) {
        while self.first < slot && self.cells.pop_front().is_some() {
            self.first += 1;
        }
    }
}

struct Tier {
    spec: &'static Spec,
    ring: Ring,
    open: Option<Open>,
    /// 有没有还没落盘的新格子。
    dirty: bool,
    saved: Instant,
}

impl Tier {
    fn new(spec: &'static Spec) -> Self {
        Self {
            spec,
            ring: Ring::new(spec.capacity),
            open: None,
            dirty: false,
            saved: Instant::now(),
        }
    }

    fn add(&mut self, at: u64, values: &[Option<f64>; N]) {
        let slot = at / self.spec.width;
        match self.open.as_ref().map(|open| open.slot) {
            Some(open) if slot == open => {}
            // 校时让时钟抖了一下：并进当前这一格，别去冲掉刚封好的那一格。
            Some(open) if slot < open && open - slot <= 1 => {}
            // 时钟被拨回去很多：「未来」的格子作废，从这里重新记。
            Some(open) if slot < open => {
                self.ring.cut(slot);
                self.dirty = true;
                self.open = Some(Open::new(slot));
            }
            Some(_) => {
                self.seal();
                self.open = Some(Open::new(slot));
            }
            None => self.open = Some(Open::new(slot)),
        }
        if let Some(open) = &mut self.open {
            open.add(values);
        }
    }

    fn seal(&mut self) {
        if let Some(open) = self.open.take() {
            self.ring.put(open.slot, open.cell());
            self.dirty = true;
        }
    }

    fn cell(&self, slot: u64) -> Option<Cell> {
        match &self.open {
            Some(open) if open.slot == slot => Some(open.cell()),
            _ => self.ring.get(slot).copied(),
        }
    }

    /// 要落盘的内容：封好的格子，加上正在积累的这一格（退出前别丢了最后一分钟）。
    fn snapshot(&self) -> Ring {
        let mut ring = self.ring.clone();
        if let Some(open) = &self.open {
            ring.put(open.slot, open.cell());
        }
        ring
    }
}

/// 一条曲线在一段时间里的样子：给定数量的柱子，加上整段的平均和峰值。
#[derive(Clone, Debug, Default)]
pub struct Series {
    /// 每根柱子的平均值，`None` 表示那段时间没有记录（没开机、没有这个传感器）。
    pub columns: Vec<Option<f64>>,
    pub mean: Option<f64>,
    pub peak: Option<f64>,
}

pub struct Archive {
    tiers: [Tier; 2],
    persist: bool,
    dir: Option<PathBuf>,
    writer: Option<mpsc::Sender<Job>>,
    /// 每档的文件现在多大，后台线程写完更新。
    sizes: Arc<[AtomicU64; 2]>,
}

enum Job {
    Save {
        tier: usize,
        path: PathBuf,
        width: u64,
        ring: Ring,
    },
    Remove(Vec<PathBuf>),
    /// 排在它前面的活都干完了。
    Done(mpsc::Sender<()>),
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn history_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join("pc-specs").join("history"))
}

impl Archive {
    /// 打开长期历史：读回上次落盘的数据，顺手把目录里不该有的东西清掉。
    /// `persist` 为假时只在内存里记，磁盘上的旧数据也一并删除。
    pub fn open(persist: bool) -> Self {
        Self::open_in(history_dir(), persist, now_ms())
    }

    fn open_in(dir: Option<PathBuf>, persist: bool, now: u64) -> Self {
        let mut archive = Self::in_memory();
        archive.persist = persist;
        archive.dir = dir;
        if let Some(dir) = archive.dir.clone() {
            sweep(&dir, persist);
            if persist {
                archive.load(&dir, now);
                archive.ensure_writer();
            }
        }
        archive
    }

    fn in_memory() -> Self {
        Self {
            tiers: TIERS.each_ref().map(Tier::new),
            persist: false,
            dir: None,
            writer: None,
            sizes: Arc::new([AtomicU64::new(0), AtomicU64::new(0)]),
        }
    }

    fn load(&mut self, dir: &Path, now: u64) {
        for (index, tier) in self.tiers.iter_mut().enumerate() {
            let path = dir.join(tier.spec.file);
            let Some((ring, size)) = read(&path, tier.spec) else {
                continue;
            };
            tier.ring = ring;
            // 时钟拨回过：比现在还新的格子是假的；隔了太久的格子早就出了窗口。
            let now_slot = now / tier.spec.width;
            tier.ring.cut(now_slot + 1);
            tier.ring
                .expire((now_slot + 1).saturating_sub(tier.spec.capacity as u64));
            self.sizes[index].store(size, Ordering::Relaxed);
        }
    }

    fn ensure_writer(&mut self) {
        if self.writer.is_some() {
            return;
        }
        let (jobs, work) = mpsc::channel();
        let sizes = self.sizes.clone();
        let started = std::thread::Builder::new()
            .name("pc-specs-history".into())
            .spawn(move || run_writer(work, sizes));
        match started {
            Ok(_) => self.writer = Some(jobs),
            Err(e) => tracing::error!("failed to start the history thread: {e}"),
        }
    }

    /// 记下一个采样。`at` 是采样时刻（Unix 毫秒）。
    pub fn record(&mut self, at: u64, value: impl Fn(Channel) -> Option<f64>) {
        let values = Channel::ALL.map(value);
        for tier in &mut self.tiers {
            tier.add(at, &values);
        }
        self.flush(false);
    }

    /// 把到期的档位交给后台线程落盘。`force` 用于退出和刚打开开关：有数据就写，不等时间。
    fn flush(&mut self, force: bool) {
        if !self.persist {
            return;
        }
        let (Some(dir), Some(writer)) = (&self.dir, &self.writer) else {
            return;
        };
        for (index, tier) in self.tiers.iter_mut().enumerate() {
            let pending = tier.dirty || (force && tier.open.is_some());
            if !pending || !(force || tier.saved.elapsed() >= tier.spec.every) {
                continue;
            }
            tier.dirty = false;
            tier.saved = Instant::now();
            let _ = writer.send(Job::Save {
                tier: index,
                path: dir.join(tier.spec.file),
                width: tier.spec.width,
                ring: tier.snapshot(),
            });
        }
    }

    /// 取一段时间的走势。`now` 是图的右边缘（Unix 毫秒），`columns` 是柱子数。
    /// `Live` 不在长期历史里，返回空。
    pub fn series(&self, range: TimeRange, channel: Channel, now: u64, columns: usize) -> Series {
        let Some(span) = range.span_ms().filter(|_| columns > 0) else {
            return Series::default();
        };
        // 能装下这段时间的、最细的一档。
        let tier = self
            .tiers
            .iter()
            .find(|tier| span <= tier.spec.width * tier.spec.capacity as u64)
            .unwrap_or(&self.tiers[TIERS.len() - 1]);
        let width = tier.spec.width;
        let start = now.saturating_sub(span);
        let column_of = |slot: u64| {
            let middle = slot * width + width / 2;
            let offset = middle.saturating_sub(start) as f64 / span as f64;
            ((offset * columns as f64) as usize).min(columns - 1)
        };

        let index = channel.index();
        let mut sums = vec![(0., 0u32); columns];
        let (mut total, mut count) = (0., 0u32);
        let mut peak: Option<f64> = None;
        for slot in start / width..=now / width {
            let Some(cell) = tier.cell(slot) else {
                continue;
            };
            let Some(avg) = dequantize(channel, cell.avg[index]) else {
                continue;
            };
            let top = dequantize(channel, cell.peak[index]).unwrap_or(avg);
            let sum = &mut sums[column_of(slot)];
            sum.0 += avg;
            sum.1 += 1;
            total += avg;
            count += 1;
            peak = Some(peak.map_or(top, |p| p.max(top)));
        }
        Series {
            columns: sums
                .into_iter()
                .map(|(sum, n)| (n > 0).then(|| sum / f64::from(n)))
                .collect(),
            mean: (count > 0).then(|| total / f64::from(count)),
            peak,
        }
    }

    /// 开关「保存到磁盘」。关掉时删文件（内存里的照常记）；打开时马上存一份。
    pub fn set_persist(&mut self, on: bool) {
        if self.persist == on {
            return;
        }
        self.persist = on;
        if on {
            self.ensure_writer();
            // 内存里已有的历史还没有对应的文件，都算「还没落盘」。
            for tier in &mut self.tiers {
                tier.dirty |= !tier.ring.cells.is_empty();
            }
            self.flush(true);
        } else {
            self.remove_files();
        }
    }

    /// 清空全部历史，内存和磁盘都清。
    pub fn clear(&mut self) {
        self.tiers = TIERS.each_ref().map(Tier::new);
        self.remove_files();
    }

    fn remove_files(&mut self) {
        let Some(dir) = &self.dir else { return };
        let paths: Vec<PathBuf> = TIERS.iter().map(|spec| dir.join(spec.file)).collect();
        match &self.writer {
            Some(writer) => {
                let _ = writer.send(Job::Remove(paths));
            }
            None => {
                for path in &paths {
                    remove(path);
                }
                for size in self.sizes.iter() {
                    size.store(0, Ordering::Relaxed);
                }
            }
        }
    }

    /// 历史文件现在在磁盘上占多少字节。
    pub fn disk_bytes(&self) -> u64 {
        self.sizes
            .iter()
            .map(|size| size.load(Ordering::Relaxed))
            .sum()
    }

    /// 退出前：把还没落盘的（包括正在积累的这一格）写下去，并等它写完。
    pub fn shutdown(&mut self) {
        self.flush(true);
        let Some(writer) = &self.writer else { return };
        let (ack, done) = mpsc::channel();
        if writer.send(Job::Done(ack)).is_ok() {
            let _ = done.recv_timeout(SHUTDOWN_WAIT);
        }
    }
}

fn run_writer(jobs: mpsc::Receiver<Job>, sizes: Arc<[AtomicU64; 2]>) {
    while let Ok(job) = jobs.recv() {
        guarded("history writer", || match job {
            Job::Save {
                tier,
                path,
                width,
                ring,
            } => match encode(width, &ring).and_then(|bytes| write(&path, &bytes)) {
                Ok(size) => sizes[tier].store(size, Ordering::Relaxed),
                Err(e) => tracing::warn!("failed to save history to {}: {e}", path.display()),
            },
            Job::Remove(paths) => {
                for path in &paths {
                    remove(path);
                }
                for size in sizes.iter() {
                    size.store(0, Ordering::Relaxed);
                }
            }
            Job::Done(ack) => {
                let _ = ack.send(());
            }
        });
    }
}

// ---- 文件格式 -----------------------------------------------------------
//
// 头部 24 字节：魔数 `PCSH`、版本、列数、2 字节保留、格宽（秒，u32）、第一格的编号（u64）、
// 格数（u32），之后是 zlib 压缩的正文。正文按列排：每条曲线先平均后峰值，每列 `格数` 个 u16，
// 存的是和前一格的差（环绕减法）：读数平稳时全是 0，压缩得很狠。

const MAGIC: [u8; 4] = *b"PCSH";
const VERSION: u8 = 1;
const HEADER: usize = 24;

fn encode(width: u64, ring: &Ring) -> io::Result<Vec<u8>> {
    let count = ring.cells.len();
    let mut raw = Vec::with_capacity(count * N * 4);
    for i in 0..N {
        push_column(&mut raw, &ring.cells, |cell| cell.avg[i]);
        push_column(&mut raw, &ring.cells, |cell| cell.peak[i]);
    }
    let mut out = Vec::with_capacity(HEADER + raw.len() / 4);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.push(N as u8);
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&((width / 1000) as u32).to_le_bytes());
    out.extend_from_slice(&ring.first.to_le_bytes());
    out.extend_from_slice(&(count as u32).to_le_bytes());
    let mut zlib = ZlibEncoder::new(out, Compression::default());
    zlib.write_all(&raw)?;
    zlib.finish()
}

fn push_column(raw: &mut Vec<u8>, cells: &VecDeque<Cell>, pick: impl Fn(&Cell) -> u16) {
    let mut previous = 0u16;
    for cell in cells {
        let value = pick(cell);
        raw.extend_from_slice(&value.wrapping_sub(previous).to_le_bytes());
        previous = value;
    }
}

fn decode(bytes: &[u8], spec: &Spec) -> Option<Ring> {
    let header = bytes.get(..HEADER)?;
    if header[..4] != MAGIC || header[4] != VERSION || usize::from(header[5]) != N {
        return None;
    }
    let width = u64::from(u32::from_le_bytes(header[8..12].try_into().ok()?)) * 1000;
    let first = u64::from_le_bytes(header[12..20].try_into().ok()?);
    let count = u32::from_le_bytes(header[20..24].try_into().ok()?) as usize;
    if width != spec.width || count == 0 || count > spec.capacity {
        return None;
    }
    first.checked_add(count as u64)?;

    // 正文的长度由格数决定：多一个字节都不要，防止被构造的压缩包撑爆内存。
    let expected = count * N * 4;
    let mut raw = Vec::with_capacity(expected);
    ZlibDecoder::new(&bytes[HEADER..])
        .take(expected as u64 + 1)
        .read_to_end(&mut raw)
        .ok()?;
    if raw.len() != expected {
        return None;
    }

    let mut cells = vec![EMPTY; count];
    let column = count * 2;
    for i in 0..N {
        let avg = &raw[i * 2 * column..][..column];
        let peak = &raw[(i * 2 + 1) * column..][..column];
        let mut previous = 0u16;
        for (cell, pair) in cells.iter_mut().zip(avg.as_chunks::<2>().0) {
            previous = previous.wrapping_add(u16::from_le_bytes(*pair));
            cell.avg[i] = previous;
        }
        previous = 0;
        for (cell, pair) in cells.iter_mut().zip(peak.as_chunks::<2>().0) {
            previous = previous.wrapping_add(u16::from_le_bytes(*pair));
            cell.peak[i] = previous;
        }
    }
    Some(Ring {
        first,
        cells: cells.into(),
        capacity: spec.capacity,
    })
}

/// 读一个档位的文件，返回数据和文件大小。文件不存在、损坏、过大都返回 `None`。
fn read(path: &Path, spec: &Spec) -> Option<(Ring, u64)> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > MAX_FILE {
        return None;
    }
    let ring = decode(&std::fs::read(path).ok()?, spec)?;
    Some((ring, size))
}

/// 先写临时文件再改名：任何时刻被杀都不会留下写了一半的文件。
fn write(path: &Path, bytes: &[u8]) -> io::Result<u64> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(bytes.len() as u64)
}

fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("tmp"));
}

/// 清理历史目录：这个目录只有我们自己用，除了当前版本的档位文件，其余都是残留
/// （没写完的临时文件、旧版本的档位）。不保存历史时，档位文件也一并删掉；
/// 超大的文件不可能是正常数据，同样删掉。
fn sweep(dir: &Path, keep: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
            continue;
        }
        let name = entry.file_name();
        let known = TIERS.iter().any(|spec| name == spec.file);
        let oversized = entry.metadata().is_ok_and(|meta| meta.len() > MAX_FILE);
        if !known || !keep || oversized {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

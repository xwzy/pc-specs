//! macOS 的 CPU 使用率：直接读 `host_processor_info` 的逐核 tick 计数。
//!
//! 不沿用 sysinfo 的算法，是因为它按每核的「忙 / (忙 + 闲)」算：Apple 芯片空闲时会把核心停放，
//! 停放期间既不计忙也不计闲，一个核心在整个采样窗口里可能只留下零星几个 tick，其中一个恰好是忙的，
//! 这个核心就被算成几十个百分点（社区里 Mole 的 #1237 就是这个现象）。
//! 这里每核的分母不低于这段时间的挂钟 tick 数，没计到的时间当作空闲；计数是 32 位的，按回绕处理。

use super::macos_vm::host;
use libc::{
    CPU_STATE_IDLE, CPU_STATE_MAX, CPU_STATE_NICE, CPU_STATE_SYSTEM, CPU_STATE_USER, KERN_SUCCESS,
    PROCESSOR_CPU_LOAD_INFO, host_processor_info, vm_deallocate,
};
use std::time::Instant;

unsafe extern "C" {
    /// 当前进程的 task 端口。`libc` 里的同名函数已标为弃用，这里直接声明。
    static mach_task_self_: u32;
}

/// 内核的计时频率（`sysconf(_SC_CLK_TCK)`），macOS 上固定是 100，一个 tick 十毫秒。
const TICKS_PER_SECOND: f64 = 100.0;
/// 窗口比这还短的话，十毫秒一格的 tick 会让读数抖得没法看，等下一次再算（和 sysinfo 的下限一致）。
const MIN_WINDOW_SECS: f64 = 0.2;

/// 一个核心的累计 tick。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct CoreTicks {
    user: u32,
    system: u32,
    idle: u32,
    nice: u32,
}

impl CoreTicks {
    /// 相对更早一次读数多出来的（忙，闲）tick 数。
    fn since(self, earlier: Self) -> (u64, u64) {
        let busy = u64::from(self.user.wrapping_sub(earlier.user))
            + u64::from(self.system.wrapping_sub(earlier.system))
            + u64::from(self.nice.wrapping_sub(earlier.nice));
        (busy, u64::from(self.idle.wrapping_sub(earlier.idle)))
    }
}

/// 两次读数之间的使用率（%）。
#[derive(Clone, Debug, PartialEq)]
pub struct Usage {
    pub per_core: Vec<f32>,
    pub overall: f32,
}

/// 记着上一次的读数，每次调用算出这段时间里的使用率。
#[derive(Default)]
pub struct CpuLoad {
    previous: Option<(Instant, Vec<CoreTicks>)>,
}

impl CpuLoad {
    /// 距上一次读数的使用率。第一次调用只建立基线；窗口太短或读取失败时返回 `None`，
    /// 基线保持不动，下一次的窗口更长，调用方先沿用别处的读数。
    pub fn sample(&mut self) -> Option<Usage> {
        let now = Instant::now();
        let current = read_ticks()?;
        let usage = match self.previous.as_ref() {
            None => None,
            Some((at, previous)) => {
                let window = now.saturating_duration_since(*at).as_secs_f64();
                if window < MIN_WINDOW_SECS {
                    return None;
                }
                usage_between(previous, &current, window)
            }
        };
        self.previous = Some((now, current));
        usage
    }
}

/// 由两次读数算出各核和整体的使用率。整体是各核忙 tick 之和除以各核分母之和。
fn usage_between(previous: &[CoreTicks], current: &[CoreTicks], window_secs: f64) -> Option<Usage> {
    if previous.len() != current.len() || current.is_empty() || window_secs <= 0.0 {
        return None;
    }
    let wall_ticks = window_secs * TICKS_PER_SECOND;
    let (mut busy_sum, mut total_sum) = (0.0_f64, 0.0_f64);
    let per_core = current
        .iter()
        .zip(previous)
        .map(|(now, before)| {
            let (busy, idle) = now.since(*before);
            let busy = busy as f64;
            // 停放的核心没计到的时间当作空闲：分母不低于挂钟时间。
            let total = (busy + idle as f64).max(wall_ticks);
            busy_sum += busy;
            total_sum += total;
            percent(busy, total)
        })
        .collect();
    Some(Usage {
        per_core,
        overall: percent(busy_sum, total_sum),
    })
}

fn percent(part: f64, whole: f64) -> f32 {
    (part / whole * 100.0).clamp(0.0, 100.0) as f32
}

/// 每个逻辑核心的累计 tick，顺序和 sysinfo 一致（Apple 芯片上能效核在前、性能核在后）。
fn read_ticks() -> Option<Vec<CoreTicks>> {
    let mut cores = 0_u32;
    let mut info: *mut i32 = std::ptr::null_mut();
    let mut len = 0_u32;
    // SAFETY: 三个输出参数都是本地变量；成功时内核分配一块 `len` 个整数的内存交给调用方。
    let status = unsafe {
        host_processor_info(
            host(),
            PROCESSOR_CPU_LOAD_INFO,
            &mut cores,
            &mut info,
            &mut len,
        )
    };
    if status != KERN_SUCCESS || info.is_null() {
        return None;
    }
    let stride = CPU_STATE_MAX as usize;
    let cores = cores as usize;
    let ticks = if cores * stride <= len as usize {
        // SAFETY: 内核交来的这块内存有 `len` 个整数，在下面释放之前一直有效。
        let raw = unsafe { std::slice::from_raw_parts(info, len as usize) };
        Some(
            raw.chunks_exact(stride)
                .take(cores)
                .map(|state| CoreTicks {
                    // 内核的计数是无符号的，接口把它们声明成了有符号整数。
                    user: state[CPU_STATE_USER as usize] as u32,
                    system: state[CPU_STATE_SYSTEM as usize] as u32,
                    idle: state[CPU_STATE_IDLE as usize] as u32,
                    nice: state[CPU_STATE_NICE as usize] as u32,
                })
                .collect(),
        )
    } else {
        None
    };
    // SAFETY: 这块内存是内核替我们分配的，由我们负责释放，此后不再访问。
    unsafe {
        vm_deallocate(
            mach_task_self_,
            info as usize,
            len as usize * std::mem::size_of::<i32>(),
        );
    }
    ticks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks(busy: u32, idle: u32) -> CoreTicks {
        CoreTicks {
            user: busy,
            idle,
            ..Default::default()
        }
    }

    fn usage(before: &[CoreTicks], after: &[CoreTicks]) -> Usage {
        usage_between(before, after, 1.0).expect("usable window")
    }

    #[test]
    fn a_busy_core_and_an_idle_core_average_to_half() {
        let u = usage(&[ticks(0, 0), ticks(0, 0)], &[ticks(100, 0), ticks(0, 100)]);
        assert_eq!(u.per_core, vec![100.0, 0.0]);
        assert_eq!(u.overall, 50.0);
    }

    #[test]
    fn user_system_and_nice_all_count_as_busy() {
        let after = CoreTicks {
            user: 10,
            system: 20,
            nice: 5,
            idle: 65,
        };
        let u = usage(&[CoreTicks::default()], &[after]);
        assert_eq!(u.per_core, vec![35.0]);
    }

    #[test]
    fn a_parked_core_with_one_stray_busy_tick_stays_near_zero() {
        // 窗口一秒（100 tick），这个核心只留下了一个忙 tick。sysinfo 的算法会给出 100%。
        let u = usage(&[ticks(0, 0)], &[ticks(1, 0)]);
        assert_eq!(u.per_core, vec![1.0]);
        assert_eq!(u.overall, 1.0);
    }

    #[test]
    fn a_fully_parked_core_is_idle_not_missing() {
        let u = usage(&[ticks(0, 0), ticks(0, 0)], &[ticks(50, 50), ticks(0, 0)]);
        assert_eq!(u.per_core, vec![50.0, 0.0]);
        assert_eq!(u.overall, 25.0);
    }

    #[test]
    fn ticks_that_run_ahead_of_the_wall_clock_are_taken_as_they_are() {
        // 计数走得比挂钟快一点（采样时刻在 tick 边界上），不能把分母压回挂钟时间，否则会超过 100%。
        let u = usage(&[ticks(0, 0)], &[ticks(60, 55)]);
        assert!((u.per_core[0] - 60.0 / 115.0 * 100.0).abs() < 1e-3);
        let u = usage(&[ticks(0, 0)], &[ticks(100, 8)]);
        assert!(u.per_core[0] <= 100.0);
    }

    #[test]
    fn the_window_scales_the_wall_clock_floor() {
        // 两秒的窗口是 200 个 tick，核心只计到 100 个忙 tick：一半。
        let u = usage_between(&[ticks(0, 0)], &[ticks(100, 0)], 2.0).unwrap();
        assert_eq!(u.per_core, vec![50.0]);
    }

    #[test]
    fn counters_wrap_around_at_32_bits() {
        let before = ticks(u32::MAX - 9, u32::MAX - 49);
        let after = ticks(40, 50);
        // 忙 50 个 tick，闲 100 个 tick（都回绕过了）。
        assert_eq!(after.since(before), (50, 100));
        let u = usage(&[before], &[after]);
        assert!((u.per_core[0] - 50.0 / 150.0 * 100.0).abs() < 1e-3);
    }

    #[test]
    fn nothing_can_be_computed_from_mismatched_or_empty_readings() {
        assert!(usage_between(&[], &[], 1.0).is_none());
        assert!(usage_between(&[ticks(0, 0)], &[ticks(1, 1), ticks(1, 1)], 1.0).is_none());
        assert!(usage_between(&[ticks(0, 0)], &[ticks(1, 1)], 0.0).is_none());
    }

    #[test]
    fn the_first_sample_only_sets_the_baseline_and_a_short_window_waits() {
        let mut load = CpuLoad::default();
        assert!(load.sample().is_none());
        // 刚建完基线，窗口远小于下限：不算，也不丢掉基线。
        assert!(load.sample().is_none());
        assert!(load.previous.is_some());
    }

    #[test]
    fn the_real_counters_give_a_share_for_every_logical_core() {
        let cores = std::thread::available_parallelism().unwrap().get();
        assert_eq!(read_ticks().expect("host_processor_info").len(), cores);

        let mut load = CpuLoad::default();
        assert!(load.sample().is_none());
        std::thread::sleep(std::time::Duration::from_millis(300));
        let u = load.sample().expect("a long enough window");
        assert_eq!(u.per_core.len(), cores);
        assert!(u.per_core.iter().all(|p| (0.0..=100.0).contains(p)));
        assert!((0.0..=100.0).contains(&u.overall));
    }
}

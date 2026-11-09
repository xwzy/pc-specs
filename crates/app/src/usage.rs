//! 流量累计：每天一份下载 / 上传字节数，存在磁盘上，重启不丢。
//!
//! 数据来自实时采样（速率 × 采样间隔），所以统计的是程序运行期间的流量；
//! 口径和速率一样（不含回环和虚拟网卡），数字和菜单栏里看到的一致。

use chrono::{Datelike as _, Duration as Days, Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 逐日记录最多保留多少天；更早的只留在累计总量里，文件不会越长越大。
const KEEP_DAYS: usize = 400;
/// 累加的结果隔多久写一次盘。
const SAVE_EVERY: Duration = Duration::from_secs(60);

/// 一段时间里的下载 / 上传字节数。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Traffic {
    pub rx: u64,
    pub tx: u64,
}

impl Traffic {
    pub fn total(self) -> u64 {
        self.rx.saturating_add(self.tx)
    }

    fn add(&mut self, other: Traffic) {
        self.rx = self.rx.saturating_add(other.rx);
        self.tx = self.tx.saturating_add(other.tx);
    }

    pub fn sum(parts: impl IntoIterator<Item = Traffic>) -> Traffic {
        let mut sum = Traffic::default();
        for part in parts {
            sum.add(part);
        }
        sum
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    /// 每天的流量，键是本地日期 `YYYY-MM-DD`。
    days: BTreeMap<String, Traffic>,
    /// 从开始累计到现在的总量（不受 `KEEP_DAYS` 影响）。
    total: Traffic,
    /// 开始累计的日期。
    since: Option<String>,
}

pub struct Usage {
    stored: Stored,
    /// 不足一字节的零头：速率乘间隔是小数，逐次取整会越丢越多。
    carry: (f64, f64),
    dirty: bool,
    saved_at: Instant,
}

/// 本地时区的今天。
pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn key(day: NaiveDate) -> String {
    day.format("%Y-%m-%d").to_string()
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("pc-specs").join("usage.json"))
}

impl Usage {
    /// 读取上次保存的累计；文件不存在或损坏就当从头开始。
    pub fn load() -> Self {
        let stored = path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self::from(stored)
    }

    fn from(stored: Stored) -> Self {
        Self {
            stored,
            carry: (0.0, 0.0),
            dirty: false,
            saved_at: Instant::now(),
        }
    }

    /// 记下 `day` 这一天又走了多少字节。
    pub fn record(&mut self, day: NaiveDate, rx_bytes: f64, tx_bytes: f64) {
        let whole = |carry: &mut f64, bytes: f64| -> u64 {
            let sum = *carry
                + if bytes.is_finite() {
                    bytes.max(0.0)
                } else {
                    0.0
                };
            let whole = sum.floor();
            *carry = sum - whole;
            whole as u64
        };
        let added = Traffic {
            rx: whole(&mut self.carry.0, rx_bytes),
            tx: whole(&mut self.carry.1, tx_bytes),
        };
        if added == Traffic::default() {
            return;
        }
        let key = key(day);
        self.stored.days.entry(key.clone()).or_default().add(added);
        self.stored.total.add(added);
        self.stored.since.get_or_insert(key);
        while self.stored.days.len() > KEEP_DAYS {
            self.stored.days.pop_first();
        }
        self.dirty = true;
    }

    pub fn day(&self, day: NaiveDate) -> Traffic {
        self.stored.days.get(&key(day)).copied().unwrap_or_default()
    }

    /// 截至 `today`（含）的最近 `count` 天，从旧到新；没有记录的天是 0。
    pub fn last_days(&self, today: NaiveDate, count: i64) -> Vec<(NaiveDate, Traffic)> {
        (0..count)
            .rev()
            .map(|back| today - Days::days(back))
            .map(|day| (day, self.day(day)))
            .collect()
    }

    /// `today` 所在这个月到目前为止的总量。
    pub fn month(&self, today: NaiveDate) -> Traffic {
        let prefix = format!("{:04}-{:02}-", today.year(), today.month());
        Traffic::sum(
            self.stored
                .days
                .iter()
                .filter(|(day, _)| day.starts_with(&prefix))
                .map(|(_, traffic)| *traffic),
        )
    }

    pub fn total(&self) -> Traffic {
        self.stored.total
    }

    /// 开始累计的日期；一点流量都没记过时是 `None`。
    pub fn since(&self) -> Option<NaiveDate> {
        let since = self.stored.since.as_deref()?;
        NaiveDate::parse_from_str(since, "%Y-%m-%d").ok()
    }

    /// 清零重新开始。
    pub fn reset(&mut self) {
        self.stored = Stored::default();
        self.carry = (0.0, 0.0);
        self.dirty = true;
        self.save();
    }

    /// 有改动、并且离上次写盘够久了才写：每秒都写没有必要。
    pub fn save_if_due(&mut self) {
        if self.dirty && self.saved_at.elapsed() >= SAVE_EVERY {
            self.save();
        }
    }

    /// 写入失败只记日志：丢一段统计不该打断使用。
    pub fn save(&mut self) {
        self.saved_at = Instant::now();
        if !self.dirty {
            return;
        }
        let Some(path) = path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        let result = serde_json::to_string(&self.stored)
            .map_err(std::io::Error::other)
            .and_then(|text| std::fs::write(&tmp, text))
            .and_then(|_| std::fs::rename(&tmp, &path));
        match result {
            Ok(()) => self.dirty = false,
            Err(e) => tracing::warn!("failed to save data usage: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn empty() -> Usage {
        Usage::from(Stored::default())
    }

    #[test]
    fn fractions_of_a_byte_are_carried_over_instead_of_lost() {
        let mut usage = empty();
        for _ in 0..10 {
            usage.record(day("2026-11-09"), 0.4, 0.0);
        }
        // 10 × 0.4 = 4 字节，逐次取整的话一个字节都不会记下。
        assert_eq!(usage.day(day("2026-11-09")).rx, 4);
        assert_eq!(usage.total().rx, 4);
    }

    #[test]
    fn days_months_and_the_total_add_up() {
        let mut usage = empty();
        usage.record(day("2026-10-31"), 100.0, 10.0);
        usage.record(day("2026-11-01"), 200.0, 20.0);
        usage.record(day("2026-11-09"), 300.0, 30.0);
        let today = day("2026-11-09");
        assert_eq!(usage.day(today), Traffic { rx: 300, tx: 30 });
        assert_eq!(usage.month(today), Traffic { rx: 500, tx: 50 });
        assert_eq!(usage.total(), Traffic { rx: 600, tx: 60 });
        assert_eq!(usage.since(), Some(day("2026-10-31")));
        // 最近 3 天：从旧到新，没记录的天是 0。
        let days = usage.last_days(today, 3);
        assert_eq!(
            days.iter().map(|(d, _)| *d).collect::<Vec<_>>(),
            [day("2026-11-07"), day("2026-11-08"), day("2026-11-09")]
        );
        assert_eq!(days[0].1, Traffic::default());
        assert_eq!(days[2].1.total(), 330);
    }

    #[test]
    fn nonsense_readings_add_nothing_and_old_days_age_out_of_the_log_only() {
        let mut usage = empty();
        usage.record(day("2026-11-09"), f64::NAN, -5.0);
        assert_eq!(usage.total(), Traffic::default());
        assert_eq!(usage.since(), None);

        let first = day("2025-01-01");
        for offset in 0..(KEEP_DAYS as i64 + 10) {
            usage.record(first + Days::days(offset), 1.0, 0.0);
        }
        assert_eq!(usage.stored.days.len(), KEEP_DAYS);
        // 逐日记录丢了最早的十天，累计总量没丢。
        assert_eq!(usage.total().rx, KEEP_DAYS as u64 + 10);
        assert_eq!(usage.day(first), Traffic::default());
    }

    #[test]
    fn old_or_partial_files_still_load() {
        let stored: Stored = serde_json::from_str("{}").unwrap();
        assert_eq!(Usage::from(stored).total(), Traffic::default());
        let stored: Stored =
            serde_json::from_str(r#"{"total":{"rx":5,"tx":6},"days":{"2026-11-09":{"rx":5}}}"#)
                .unwrap();
        let usage = Usage::from(stored);
        assert_eq!(usage.total(), Traffic { rx: 5, tx: 6 });
        assert_eq!(usage.day(day("2026-11-09")), Traffic { rx: 5, tx: 0 });
    }
}

use crate::model::BatteryInfo;
use battery::units::{
    energy::watt_hour, power::watt, ratio::percent, thermodynamic_temperature::degree_celsius,
    time::second,
};
use battery::{Manager, State};

/// 第一块电池的状态；台式机 / 虚拟机没有电池时返回 `None`。
pub fn collect() -> Option<BatteryInfo> {
    let manager = Manager::new().ok()?;
    let bat = manager.batteries().ok()?.find_map(|b| b.ok())?;

    let state = match bat.state() {
        State::Charging => "charging",
        State::Discharging => "discharging",
        State::Empty => "empty",
        State::Full => "full",
        _ => "unknown",
    };

    let to_mwh = |wh: f32| -> Option<u64> {
        (wh.is_finite() && wh > 0.0).then(|| (wh * 1000.0).round() as u64)
    };

    let power_w = bat.energy_rate().get::<watt>();
    // 放电记负、充电记正，界面直接显示符号。
    let power_now_mw = (power_w.is_finite() && power_w.abs() > 0.001).then(|| {
        let mw = (power_w * 1000.0).round() as i64;
        if matches!(bat.state(), State::Discharging) {
            -mw
        } else {
            mw
        }
    });

    #[allow(unused_mut)]
    let mut info = BatteryInfo {
        vendor: bat.vendor().map(str::to_string),
        model: bat.model().map(str::to_string),
        state: state.to_string(),
        percentage: bat.state_of_charge().get::<percent>(),
        cycle_count: bat.cycle_count(),
        design_capacity_mwh: to_mwh(bat.energy_full_design().get::<watt_hour>()),
        full_capacity_mwh: to_mwh(bat.energy_full().get::<watt_hour>()),
        current_capacity_mwh: to_mwh(bat.energy().get::<watt_hour>()),
        temperature_c: bat.temperature().map(|t| t.get::<degree_celsius>()),
        time_to_empty_secs: bat
            .time_to_empty()
            .map(|t| t.get::<second>().round() as u64),
        time_to_full_secs: bat.time_to_full().map(|t| t.get::<second>().round() as u64),
        power_now_mw,
        health_percent: None,
    };
    #[cfg(target_os = "macos")]
    apply_ioreg(&mut info);
    info.health_percent = capacity_ratio(&info);
    // Apple 的「最大容量」是校准过的，和「系统设置」里的数字一致，比自己拿原始容量相除准。
    #[cfg(target_os = "macos")]
    if let Some(reported) = crate::platform::macos::battery_health_percent() {
        info.health_percent = Some(reported);
    }
    Some(info)
}

/// 满电容量占设计容量的百分比。
fn capacity_ratio(info: &BatteryInfo) -> Option<f32> {
    match (info.full_capacity_mwh, info.design_capacity_mwh) {
        (Some(full), Some(design)) if design > 0 => Some(full as f32 / design as f32 * 100.0),
        _ => None,
    }
}

/// `battery` 库在 macOS 上把 `MaxCapacity`（Apple Silicon 上是百分比 0–100）当 mAh 用，
/// 满电容量会被算成几百 mWh，健康度因此严重偏低。这里改读 IORegistry 里的真实 mAh 值，
/// 再按当前电压换算成 mWh。
#[cfg(target_os = "macos")]
fn apply_ioreg(info: &mut BatteryInfo) {
    let Some(text) = crate::util::run_stdout_timeout(
        "/usr/sbin/ioreg",
        &["-rn", "AppleSmartBattery"],
        std::time::Duration::from_secs(3),
    ) else {
        return;
    };
    // 顶层键各占一行：`    "DesignCapacity" = 6075`。负的电流以无符号补码形式出现，所以先按 u64 解析再回转。
    let num = |key: &str| -> Option<i64> {
        let prefix = format!("\"{key}\" = ");
        text.lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix(prefix.as_str())?
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .map(|v| v as i64)
    };
    let Some(volts) = num("Voltage")
        .map(|mv| mv as f64 / 1000.0)
        .filter(|v| *v > 0.0)
    else {
        return;
    };
    let to_mwh = |mah: i64| (mah > 0).then(|| (mah as f64 * volts).round() as u64);
    if let Some(v) = num("DesignCapacity").and_then(to_mwh) {
        info.design_capacity_mwh = Some(v);
    }
    if let Some(v) = num("AppleRawMaxCapacity").and_then(to_mwh) {
        info.full_capacity_mwh = Some(v);
    }
    if let Some(v) = num("AppleRawCurrentCapacity").and_then(to_mwh) {
        info.current_capacity_mwh = Some(v);
    }
    if let Some(ma) = num("InstantAmperage") {
        let mw = (ma as f64 * volts).round() as i64;
        info.power_now_mw = (mw != 0).then_some(mw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_is_the_full_capacity_over_the_design_capacity() {
        let info = BatteryInfo {
            design_capacity_mwh: Some(50_000),
            full_capacity_mwh: Some(45_000),
            ..Default::default()
        };
        assert_eq!(capacity_ratio(&info), Some(90.0));
    }

    #[test]
    fn health_needs_both_capacities_and_a_real_design_capacity() {
        let only_full = BatteryInfo {
            full_capacity_mwh: Some(45_000),
            ..Default::default()
        };
        assert_eq!(capacity_ratio(&only_full), None);
        let zero_design = BatteryInfo {
            design_capacity_mwh: Some(0),
            full_capacity_mwh: Some(45_000),
            ..Default::default()
        };
        assert_eq!(capacity_ratio(&zero_design), None);
    }
}

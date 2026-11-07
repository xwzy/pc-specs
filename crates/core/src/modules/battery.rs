use crate::model::BatteryInfo;
use battery::units::{electric_potential::volt, energy::watt_hour, power::watt, ratio::percent, time::second};
use battery::{Manager, State};

pub fn collect() -> Option<BatteryInfo> {
    let manager = Manager::new().ok()?;
    let mut iter = manager.batteries().ok()?;
    let bat = iter.find_map(|b| b.ok())?;

    let state = match bat.state() {
        State::Unknown => "unknown",
        State::Charging => "charging",
        State::Discharging => "discharging",
        State::Empty => "empty",
        State::Full => "full",
        _ => "unknown",
    };

    let percentage = bat.state_of_charge().get::<percent>();
    let cycle_count = bat.cycle_count();

    let design_wh = bat.energy_full_design().get::<watt_hour>();
    let full_wh = bat.energy_full().get::<watt_hour>();
    let now_wh = bat.energy().get::<watt_hour>();
    let to_mwh = |wh: f32| -> Option<u64> {
        if wh.is_finite() && wh > 0.0 {
            Some((wh * 1000.0).round() as u64)
        } else {
            None
        }
    };

    let temperature_c = bat.temperature().map(|t| t.get::<battery::units::thermodynamic_temperature::degree_celsius>());

    let time_to_empty_secs = bat.time_to_empty().map(|t| t.get::<second>().round() as u64);
    let time_to_full_secs = bat.time_to_full().map(|t| t.get::<second>().round() as u64);

    let power = bat.energy_rate().get::<watt>();
    let voltage = bat.voltage().get::<volt>();
    let _ = voltage;
    let signed_power_mw: i64 = match bat.state() {
        State::Discharging => -((power * 1000.0) as i64),
        _ => (power * 1000.0) as i64,
    };

    Some(BatteryInfo {
        vendor: bat.vendor().map(|s| s.to_string()),
        model: bat.model().map(|s| s.to_string()),
        state: state.to_string(),
        percentage,
        cycle_count,
        design_capacity_mwh: to_mwh(design_wh),
        full_capacity_mwh: to_mwh(full_wh),
        current_capacity_mwh: to_mwh(now_wh),
        temperature_c,
        time_to_empty_secs,
        time_to_full_secs,
        power_now_mw: if power.is_finite() && power.abs() > 0.001 {
            Some(signed_power_mw)
        } else {
            None
        },
    })
}

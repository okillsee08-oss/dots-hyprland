//! Pure UPower battery-state projection matching the upstream Quickshell model.
//!
//! This module deliberately performs no D-Bus I/O, notifications, sound playback,
//! or power actions. A future shared-bus adapter can feed it UPower properties;
//! the display/event loop only needs to consume the resulting snapshot.

/// UPower's `org.freedesktop.UPower.Device.Type` value for a battery.
pub const UPOWER_DEVICE_TYPE_BATTERY: u32 = 2;

/// UPower device state values from `org.freedesktop.UPower.Device.State`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum DeviceState {
    #[default]
    Unknown = 0,
    Charging = 1,
    Discharging = 2,
    Empty = 3,
    FullyCharged = 4,
    PendingCharge = 5,
    PendingDischarge = 6,
}

impl From<u32> for DeviceState {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::FullyCharged,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            _ => Self::Unknown,
        }
    }
}

/// Battery thresholds from the upstream `Config.options.battery` object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatteryThresholds {
    pub low_percent: u8,
    pub critical_percent: u8,
    pub suspend_percent: u8,
    pub full_percent: u8,
    pub automatic_suspend: bool,
}

impl Default for BatteryThresholds {
    fn default() -> Self {
        Self {
            low_percent: 20,
            critical_percent: 5,
            suspend_percent: 3,
            // Upstream deliberately defaults this above 100%, disabling the
            // full-charge notification until the user chooses a lower value.
            full_percent: 101,
            automatic_suspend: true,
        }
    }
}

/// The subset of UPower device properties needed by the native battery model.
///
/// `percentage_percent` and `energy_rate_w` use UPower's D-Bus units:
/// percentage is 0..=100, while energy rate is a non-negative magnitude in W.
/// UPower reports charging direction separately through `state`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UPowerBatteryProperties {
    pub device_type: u32,
    pub power_supply: bool,
    pub state: u32,
    pub percentage_percent: f64,
    pub energy_rate_w: f64,
    pub time_to_empty_seconds: i64,
    pub time_to_full_seconds: i64,
    pub energy_full: Option<f64>,
    pub energy_full_design: Option<f64>,
}

/// A compact immutable view for the panel and battery popup.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatterySnapshot {
    pub available: bool,
    pub state: DeviceState,
    /// Normalized fraction in the inclusive range 0.0..=1.0, matching
    /// Quickshell's `UPowerDevice.percentage` property.
    pub percentage: f64,
    /// Signed like Quickshell's `changeRate`: positive while charging,
    /// negative while discharging.
    pub change_rate_w: f64,
    pub time_to_empty_seconds: Option<u64>,
    pub time_to_full_seconds: Option<u64>,
    /// Battery health as a percentage of design capacity, when supported.
    pub health_percent: Option<f64>,
    pub is_charging: bool,
    pub is_plugged_in: bool,
    pub is_low: bool,
    pub is_critical: bool,
    pub is_suspending: bool,
    pub is_full: bool,
    pub is_low_and_not_charging: bool,
    pub is_critical_and_not_charging: bool,
    pub should_automatic_suspend: bool,
    pub is_full_and_charging: bool,
}

impl BatterySnapshot {
    pub fn from_upower(properties: UPowerBatteryProperties, thresholds: BatteryThresholds) -> Self {
        let state = DeviceState::from(properties.state);
        let available =
            properties.device_type == UPOWER_DEVICE_TYPE_BATTERY && properties.power_supply;

        let percentage = finite_clamp(properties.percentage_percent, 0.0, 100.0) / 100.0;
        let rate = finite_clamp(properties.energy_rate_w.abs(), 0.0, f64::MAX);
        let change_rate_w = match state {
            DeviceState::Charging => rate,
            DeviceState::Discharging => -rate,
            _ => 0.0,
        };

        let health_percent =
            battery_health_percent(properties.energy_full, properties.energy_full_design);
        let time_to_empty_seconds = positive_seconds(properties.time_to_empty_seconds);
        let time_to_full_seconds = positive_seconds(properties.time_to_full_seconds);

        let is_charging = state == DeviceState::Charging;
        let is_plugged_in = is_charging || state == DeviceState::PendingCharge;
        let percent = percentage * 100.0;

        let is_low = available && percent <= f64::from(thresholds.low_percent);
        let is_critical = available && percent <= f64::from(thresholds.critical_percent);
        let is_suspending = available && percent <= f64::from(thresholds.suspend_percent);
        let is_full = available && percent >= f64::from(thresholds.full_percent);

        Self {
            available,
            state,
            percentage,
            change_rate_w,
            time_to_empty_seconds,
            time_to_full_seconds,
            health_percent,
            is_charging,
            is_plugged_in,
            is_low,
            is_critical,
            is_suspending,
            is_full,
            is_low_and_not_charging: is_low && !is_charging,
            is_critical_and_not_charging: is_critical && !is_charging,
            should_automatic_suspend: available
                && thresholds.automatic_suspend
                && is_suspending
                && !is_charging,
            is_full_and_charging: is_full && is_charging,
        }
    }

    /// The estimate selected by the upstream popup: time-to-full only while
    /// the state is exactly Charging; all other states use time-to-empty.
    pub fn estimated_time_seconds(self) -> Option<u64> {
        if self.is_charging {
            self.time_to_full_seconds
        } else {
            self.time_to_empty_seconds
        }
    }

    /// Selects the same eight icon levels as upstream `Icons.qml`.
    pub fn icon_level(self) -> BatteryIconLevel {
        let percent = self.percentage * 100.0;
        if percent >= 93.0 {
            BatteryIconLevel::Full
        } else if percent >= 78.0 {
            BatteryIconLevel::Six
        } else if percent >= 64.0 {
            BatteryIconLevel::Five
        } else if percent >= 50.0 {
            BatteryIconLevel::Four
        } else if percent >= 35.0 {
            BatteryIconLevel::Three
        } else if percent >= 21.0 {
            BatteryIconLevel::Two
        } else if percent >= 7.0 {
            BatteryIconLevel::One
        } else {
            BatteryIconLevel::Empty
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatteryIconLevel {
    Empty,
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
    Full,
}

fn finite_clamp(value: f64, min: f64, max: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        min
    }
}

fn positive_seconds(value: i64) -> Option<u64> {
    u64::try_from(value).ok().filter(|seconds| *seconds > 0)
}

fn battery_health_percent(
    energy_full: Option<f64>,
    energy_full_design: Option<f64>,
) -> Option<f64> {
    let full = energy_full.filter(|value| value.is_finite() && *value >= 0.0)?;
    let design = energy_full_design.filter(|value| value.is_finite() && *value > 0.0)?;
    let health = full / design * 100.0;
    Some(if health == 0.0 { 0.01 } else { health })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn properties() -> UPowerBatteryProperties {
        UPowerBatteryProperties {
            device_type: UPOWER_DEVICE_TYPE_BATTERY,
            power_supply: true,
            state: DeviceState::Discharging as u32,
            percentage_percent: 48.5,
            energy_rate_w: 12.0,
            time_to_empty_seconds: 7_200,
            time_to_full_seconds: 0,
            energy_full: Some(45.0),
            energy_full_design: Some(50.0),
        }
    }

    #[test]
    fn maps_all_known_upower_states_and_unknown_values() {
        assert_eq!(DeviceState::from(0), DeviceState::Unknown);
        assert_eq!(DeviceState::from(1), DeviceState::Charging);
        assert_eq!(DeviceState::from(2), DeviceState::Discharging);
        assert_eq!(DeviceState::from(3), DeviceState::Empty);
        assert_eq!(DeviceState::from(4), DeviceState::FullyCharged);
        assert_eq!(DeviceState::from(5), DeviceState::PendingCharge);
        assert_eq!(DeviceState::from(6), DeviceState::PendingDischarge);
        assert_eq!(DeviceState::from(99), DeviceState::Unknown);
    }

    #[test]
    fn normalizes_upower_percent_and_discharge_rate() {
        let snapshot = BatterySnapshot::from_upower(properties(), BatteryThresholds::default());

        assert!(snapshot.available);
        assert!((snapshot.percentage - 0.485).abs() < f64::EPSILON);
        assert_eq!(snapshot.change_rate_w, -12.0);
        assert_eq!(snapshot.time_to_empty_seconds, Some(7_200));
        assert_eq!(snapshot.health_percent, Some(90.0));
    }

    #[test]
    fn charging_and_pending_charge_match_upstream_flags() {
        let mut input = properties();
        input.state = DeviceState::Charging as u32;
        input.energy_rate_w = 18.0;
        input.time_to_full_seconds = 1_800;
        let charging = BatterySnapshot::from_upower(input, BatteryThresholds::default());

        assert!(charging.is_charging);
        assert!(charging.is_plugged_in);
        assert_eq!(charging.change_rate_w, 18.0);
        assert_eq!(charging.estimated_time_seconds(), Some(1_800));

        input.state = DeviceState::PendingCharge as u32;
        let pending = BatterySnapshot::from_upower(input, BatteryThresholds::default());
        assert!(!pending.is_charging);
        assert!(pending.is_plugged_in);
        assert_eq!(pending.change_rate_w, 0.0);
        // Matches Battery.qml: PendingCharge is plugged in, but isCharging is
        // false, so the popup still selects time-to-empty.
        assert_eq!(pending.estimated_time_seconds(), Some(7_200));
    }

    #[test]
    fn pending_discharge_is_not_reported_as_plugged_in() {
        let mut input = properties();
        input.state = DeviceState::PendingDischarge as u32;
        let snapshot = BatterySnapshot::from_upower(input, BatteryThresholds::default());

        assert!(!snapshot.is_charging);
        assert!(!snapshot.is_plugged_in);
        assert_eq!(snapshot.change_rate_w, 0.0);
    }

    #[test]
    fn applies_upstream_low_critical_suspend_and_full_defaults() {
        let mut input = properties();
        input.percentage_percent = 4.0;
        let snapshot = BatterySnapshot::from_upower(input, BatteryThresholds::default());

        assert!(snapshot.is_low);
        assert!(snapshot.is_critical);
        assert!(!snapshot.is_suspending);
        assert!(!snapshot.is_full);
        assert!(snapshot.is_low_and_not_charging);
        assert!(snapshot.is_critical_and_not_charging);
        assert!(!snapshot.should_automatic_suspend);
    }

    #[test]
    fn automatic_suspend_is_only_a_state_flag_and_respects_config() {
        let mut input = properties();
        input.percentage_percent = 2.0;
        let enabled = BatterySnapshot::from_upower(input, BatteryThresholds::default());
        assert!(enabled.is_suspending);
        assert!(enabled.should_automatic_suspend);

        let disabled = BatterySnapshot::from_upower(
            input,
            BatteryThresholds {
                automatic_suspend: false,
                ..BatteryThresholds::default()
            },
        );
        assert!(disabled.is_suspending);
        assert!(!disabled.should_automatic_suspend);
    }

    #[test]
    fn charging_suppresses_low_and_automatic_suspend_actions() {
        let mut input = properties();
        input.state = DeviceState::Charging as u32;
        input.percentage_percent = 2.0;
        let snapshot = BatterySnapshot::from_upower(input, BatteryThresholds::default());

        assert!(snapshot.is_low);
        assert!(!snapshot.is_low_and_not_charging);
        assert!(!snapshot.should_automatic_suspend);
    }

    #[test]
    fn full_flag_requires_configured_threshold_and_charging() {
        let mut input = properties();
        input.state = DeviceState::Charging as u32;
        input.percentage_percent = 99.0;
        let thresholds = BatteryThresholds {
            full_percent: 95,
            ..BatteryThresholds::default()
        };
        let snapshot = BatterySnapshot::from_upower(input, thresholds);

        assert!(snapshot.is_full);
        assert!(snapshot.is_full_and_charging);
    }

    #[test]
    fn ignores_non_battery_and_non_power_supply_devices() {
        let mut input = properties();
        input.device_type = 1;
        let non_battery = BatterySnapshot::from_upower(input, BatteryThresholds::default());
        assert!(!non_battery.available);
        assert!(!non_battery.is_low);
        assert!(!non_battery.should_automatic_suspend);

        input.device_type = UPOWER_DEVICE_TYPE_BATTERY;
        input.power_supply = false;
        let peripheral = BatterySnapshot::from_upower(input, BatteryThresholds::default());
        assert!(!peripheral.available);
    }

    #[test]
    fn clamps_invalid_percentages_and_sanitizes_non_finite_rates() {
        let mut input = properties();
        input.percentage_percent = f64::NAN;
        input.energy_rate_w = f64::INFINITY;
        let snapshot = BatterySnapshot::from_upower(input, BatteryThresholds::default());

        assert_eq!(snapshot.percentage, 0.0);
        assert_eq!(snapshot.change_rate_w, 0.0);

        input.percentage_percent = 150.0;
        input.energy_rate_w = -8.0;
        let snapshot = BatterySnapshot::from_upower(input, BatteryThresholds::default());
        assert_eq!(snapshot.percentage, 1.0);
        // EnergyRate is a magnitude in UPower; the direction comes from State.
        assert_eq!(snapshot.change_rate_w, -8.0);
    }

    #[test]
    fn missing_or_invalid_health_and_time_estimates_are_not_exposed() {
        let mut input = properties();
        input.energy_full = Some(f64::NAN);
        input.energy_full_design = Some(0.0);
        input.time_to_empty_seconds = -1;
        input.time_to_full_seconds = 0;
        let snapshot = BatterySnapshot::from_upower(input, BatteryThresholds::default());

        assert_eq!(snapshot.health_percent, None);
        assert_eq!(snapshot.time_to_empty_seconds, None);
        assert_eq!(snapshot.time_to_full_seconds, None);
        assert_eq!(snapshot.estimated_time_seconds(), None);
    }

    #[test]
    fn zero_health_uses_upstream_minimum_visible_value() {
        let mut input = properties();
        input.energy_full = Some(0.0);
        input.energy_full_design = Some(50.0);
        let snapshot = BatterySnapshot::from_upower(input, BatteryThresholds::default());
        assert_eq!(snapshot.health_percent, Some(0.01));
    }

    #[test]
    fn selects_upstream_battery_icon_thresholds() {
        let mut input = properties();
        for (percent, expected) in [
            (0.0, BatteryIconLevel::Empty),
            (6.9, BatteryIconLevel::Empty),
            (7.0, BatteryIconLevel::One),
            (21.0, BatteryIconLevel::Two),
            (35.0, BatteryIconLevel::Three),
            (50.0, BatteryIconLevel::Four),
            (64.0, BatteryIconLevel::Five),
            (78.0, BatteryIconLevel::Six),
            (93.0, BatteryIconLevel::Full),
            (100.0, BatteryIconLevel::Full),
        ] {
            input.percentage_percent = percent;
            let snapshot = BatterySnapshot::from_upower(input, BatteryThresholds::default());
            assert_eq!(snapshot.icon_level(), expected, "{percent}%");
        }
    }
}

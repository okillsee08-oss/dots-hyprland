//! On-demand battery popup projection matching upstream BatteryPopup.qml.
//!
//! This module contains only the immutable content model. It performs no D-Bus
//! I/O, rendering, notifications, sound playback, or power actions. A future
//! Wayland popup surface can consume the rows without consulting UPower again.

use crate::battery::{BatterySnapshot, DeviceState};

/// The direction label used by the upstream popup's time-estimate row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeDirection {
    ToEmpty,
    ToFull,
}

/// A compact time estimate. Seconds are reduced to the hour/minute format
/// used by BatteryPopup.qml; remaining seconds are intentionally discarded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeEstimate {
    pub direction: TimeDirection,
    pub hours: u64,
    pub minutes: u8,
}

impl TimeEstimate {
    pub fn from_seconds(direction: TimeDirection, seconds: u64) -> Self {
        Self {
            direction,
            hours: seconds / 3_600,
            minutes: ((seconds % 3_600) / 60) as u8,
        }
    }

    pub fn label(self) -> &'static str {
        match self.direction {
            TimeDirection::ToFull => "Time to full:",
            TimeDirection::ToEmpty => "Time to empty:",
        }
    }

    /// Matches the upstream popup: omit hours when the estimate is under one
    /// hour, and keep the minutes even when they round down to zero.
    pub fn value_text(self) -> String {
        if self.hours > 0 {
            format!("{}h, {}m", self.hours, self.minutes)
        } else {
            format!("{}m", self.minutes)
        }
    }
}

/// The status represented by the popup's bolt row. Watt values retain
/// Quickshell's signed `changeRate` convention: positive while charging and
/// negative while discharging.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PowerStatus {
    FullyCharged,
    Charging { watts: f64 },
    Discharging { watts: f64 },
}

impl PowerStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::FullyCharged => "Fully charged",
            Self::Charging { .. } => "Charging:",
            Self::Discharging { .. } => "Discharging:",
        }
    }

    /// Fully charged is a status-only row in the upstream popup.
    pub fn value_text(self) -> String {
        match self {
            Self::FullyCharged => String::new(),
            Self::Charging { watts } | Self::Discharging { watts } => {
                format!("{watts:.2}W")
            }
        }
    }
}

/// Data for the three value rows in upstream BatteryPopup.qml.
///
/// The popup is only available for a laptop battery. Its time row is omitted
/// when the device is fully charged, the estimate is unavailable, or the
/// absolute power rate is at or below 0.01 W. The health row remains visible;
/// upstream displays 0.0% when health is unsupported.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatteryPopupModel {
    pub time: Option<TimeEstimate>,
    pub power: Option<PowerStatus>,
    pub health_percent: f64,
}

impl BatteryPopupModel {
    /// Build popup content from the latest immutable battery snapshot.
    /// Returns None when UPower does not report an available laptop battery.
    pub fn from_snapshot(snapshot: BatterySnapshot) -> Option<Self> {
        if !snapshot.available {
            return None;
        }

        let rate = snapshot.change_rate_w.abs();
        let has_rate = rate.is_finite() && rate > 0.0;
        let has_time_rate = rate.is_finite() && rate > 0.01;

        let selected_time = if snapshot.is_charging {
            snapshot.time_to_full_seconds
        } else {
            snapshot.time_to_empty_seconds
        };

        let time = if snapshot.state != DeviceState::FullyCharged && has_time_rate {
            selected_time
                .filter(|seconds| *seconds > 0)
                .map(|seconds| {
                    let direction = if snapshot.is_charging {
                        TimeDirection::ToFull
                    } else {
                        TimeDirection::ToEmpty
                    };
                    TimeEstimate::from_seconds(direction, seconds)
                })
        } else {
            None
        };

        let power = if snapshot.state == DeviceState::FullyCharged {
            Some(PowerStatus::FullyCharged)
        } else if has_rate {
            // Preserve the sign exposed by Quickshell's changeRate property.
            let watts = snapshot.change_rate_w;
            Some(if snapshot.is_charging {
                PowerStatus::Charging { watts }
            } else {
                PowerStatus::Discharging { watts }
            })
        } else {
            None
        };

        let health_percent = snapshot
            .health_percent
            .filter(|percent| percent.is_finite())
            .unwrap_or(0.0);

        Some(Self {
            time,
            power,
            health_percent,
        })
    }

    pub fn health_value_text(self) -> String {
        format!("{:.1}%", self.health_percent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::{BatteryThresholds, UPowerBatteryProperties};

    fn snapshot(
        state: DeviceState,
        rate_w: f64,
        time_to_empty: i64,
        time_to_full: i64,
        health: Option<f64>,
        design: Option<f64>,
        is_battery: bool,
    ) -> BatterySnapshot {
        BatterySnapshot::from_upower(
            UPowerBatteryProperties {
                device_type: if is_battery { 2 } else { 1 },
                power_supply: true,
                state: state as u32,
                percentage_percent: 52.0,
                energy_rate_w: rate_w,
                time_to_empty_seconds: time_to_empty,
                time_to_full_seconds: time_to_full,
                energy_full: health,
                energy_full_design: design,
            },
            BatteryThresholds::default(),
        )
    }

    #[test]
    fn discharging_popup_uses_time_to_empty_rate_and_health() {
        let model = BatteryPopupModel::from_snapshot(snapshot(
            DeviceState::Discharging,
            12.0,
            7_500,
            0,
            Some(45.0),
            Some(50.0),
            true,
        ))
        .unwrap();

        let time = model.time.unwrap();
        assert_eq!(time.direction, TimeDirection::ToEmpty);
        assert_eq!(time.label(), "Time to empty:");
        assert_eq!(time.value_text(), "2h, 5m");
        let power = model.power.unwrap();
        assert_eq!(power.label(), "Discharging:");
        assert_eq!(power, PowerStatus::Discharging { watts: -12.0 });
        assert_eq!(power.value_text(), "-12.00W");
        assert_eq!(model.health_value_text(), "90.0%");
    }

    #[test]
    fn charging_popup_uses_time_to_full_and_charging_rate() {
        let model = BatteryPopupModel::from_snapshot(snapshot(
            DeviceState::Charging,
            4.25,
            0,
            3_660,
            Some(40.0),
            Some(50.0),
            true,
        ))
        .unwrap();

        let time = model.time.unwrap();
        assert_eq!(time.direction, TimeDirection::ToFull);
        assert_eq!(time.label(), "Time to full:");
        assert_eq!(time.value_text(), "1h, 1m");
        let power = model.power.unwrap();
        assert_eq!(power.label(), "Charging:");
        assert_eq!(power.value_text(), "4.25W");
    }

    #[test]
    fn fully_charged_hides_time_and_shows_status_only_power_row() {
        let model = BatteryPopupModel::from_snapshot(snapshot(
            DeviceState::FullyCharged,
            0.0,
            0,
            0,
            Some(48.0),
            Some(50.0),
            true,
        ))
        .unwrap();

        assert_eq!(model.time, None);
        let power = model.power.unwrap();
        assert_eq!(power, PowerStatus::FullyCharged);
        assert_eq!(power.label(), "Fully charged");
        assert_eq!(power.value_text(), "");
    }

    #[test]
    fn time_row_requires_a_positive_estimate_and_rate_above_point_zero_one_w() {
        let low_rate = BatteryPopupModel::from_snapshot(snapshot(
            DeviceState::Discharging,
            0.01,
            3_600,
            0,
            None,
            None,
            true,
        ))
        .unwrap();
        assert_eq!(low_rate.time, None);
        assert_eq!(low_rate.power, Some(PowerStatus::Discharging { watts: -0.01 }));

        let positive_rate = BatteryPopupModel::from_snapshot(snapshot(
            DeviceState::Discharging,
            0.0101,
            3_600,
            0,
            None,
            None,
            true,
        ))
        .unwrap();
        assert_eq!(
            positive_rate.time.map(TimeEstimate::value_text).as_deref(),
            Some("1h, 0m")
        );

        let missing_estimate = BatteryPopupModel::from_snapshot(snapshot(
            DeviceState::Discharging,
            1.0,
            0,
            0,
            None,
            None,
            true,
        ))
        .unwrap();
        assert_eq!(missing_estimate.time, None);
        assert!(missing_estimate.power.is_some());
    }

    #[test]
    fn sub_hour_estimates_show_minutes_only_and_floor_seconds() {
        assert_eq!(
            TimeEstimate::from_seconds(TimeDirection::ToEmpty, 3_599).value_text(),
            "59m"
        );
        assert_eq!(
            TimeEstimate::from_seconds(TimeDirection::ToEmpty, 59).value_text(),
            "0m"
        );
    }

    #[test]
    fn unsupported_or_invalid_health_matches_upstream_zero_default() {
        let mut battery = snapshot(
            DeviceState::Discharging,
            1.0,
            1_200,
            0,
            None,
            None,
            true,
        );
        battery.health_percent = None;
        let unsupported = BatteryPopupModel::from_snapshot(battery).unwrap();
        assert_eq!(unsupported.health_percent, 0.0);
        assert_eq!(unsupported.health_value_text(), "0.0%");

        battery.health_percent = Some(f64::NAN);
        let invalid = BatteryPopupModel::from_snapshot(battery).unwrap();
        assert_eq!(invalid.health_percent, 0.0);
    }

    #[test]
    fn unavailable_non_battery_device_has_no_popup() {
        let battery = snapshot(
            DeviceState::Discharging,
            10.0,
            1_200,
            0,
            Some(45.0),
            Some(50.0),
            false,
        );
        assert!(!battery.available);
        assert_eq!(BatteryPopupModel::from_snapshot(battery), None);
    }

    #[test]
    fn non_finite_power_rate_does_not_create_time_or_power_rows() {
        let mut battery = snapshot(
            DeviceState::Discharging,
            1.0,
            1_200,
            0,
            None,
            None,
            true,
        );
        battery.change_rate_w = f64::INFINITY;
        let model = BatteryPopupModel::from_snapshot(battery).unwrap();
        assert_eq!(model.time, None);
        assert_eq!(model.power, None);
    }
}

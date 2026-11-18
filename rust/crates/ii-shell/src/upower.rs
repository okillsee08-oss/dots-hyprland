//! Event-driven adapter for the composite UPower display device.
//!
//! UPower is provided on the system bus. The caller supplies the shared system
//! bus connection; this adapter never opens a connection or starts a thread.
//! A filtered PropertiesChanged stream is installed before the initial
//! GetAll snapshot so changes during startup cannot be lost.

use std::collections::HashMap;

use futures_lite::StreamExt;
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedObjectPath, OwnedValue},
};

use crate::battery::{BatterySnapshot, BatteryThresholds, UPowerBatteryProperties};

pub const UPOWER_DESTINATION: &str = "org.freedesktop.UPower";
pub const UPOWER_MANAGER_PATH: &str = "/org/freedesktop/UPower";
pub const UPOWER_MANAGER_INTERFACE: &str = "org.freedesktop.UPower";
pub const UPOWER_DEVICE_INTERFACE: &str = "org.freedesktop.UPower.Device";
const DBUS_PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";

#[derive(Debug, thiserror::Error)]
pub enum UPowerError {
    #[error("UPower D-Bus operation failed: {0}")]
    Bus(#[from] zbus::Error),
    #[error("UPower D-Bus value could not be decoded: {0}")]
    Value(#[from] zbus::zvariant::Error),
    #[error("UPower omitted required property {0}")]
    MissingProperty(&'static str),
    #[error("UPower property {0} had an unexpected D-Bus type")]
    InvalidProperty(&'static str),
    #[error("UPower PropertiesChanged stream closed")]
    SignalStreamClosed,
}

/// Reads and monitors UPower's aggregate desktop battery device.
#[derive(Clone, Debug)]
pub struct UPowerAdapter {
    connection: Connection,
    display_device: OwnedObjectPath,
}

impl UPowerAdapter {
    /// Creates an adapter from the process-wide system-bus connection.
    ///
    /// UPower exposes GetDisplayDevice on its manager object. Passing the
    /// connection in keeps connection ownership at the application/runtime
    /// level and lets other system-service adapters share it.
    pub async fn new(connection: &Connection) -> Result<Self, UPowerError> {
        let manager = Proxy::new(
            connection,
            UPOWER_DESTINATION,
            UPOWER_MANAGER_PATH,
            UPOWER_MANAGER_INTERFACE,
        )
        .await?;
        let display_device: OwnedObjectPath = manager.call("GetDisplayDevice", &()).await?;

        Ok(Self {
            connection: connection.clone(),
            display_device,
        })
    }

    pub fn display_device_path(&self) -> &str {
        self.display_device.as_str()
    }

    /// Reads all device properties in one Properties.GetAll round trip.
    pub async fn snapshot(
        &self,
        thresholds: BatteryThresholds,
    ) -> Result<BatterySnapshot, UPowerError> {
        let properties = Proxy::new(
            &self.connection,
            UPOWER_DESTINATION,
            self.display_device.as_str(),
            DBUS_PROPERTIES_INTERFACE,
        )
        .await?;

        self.snapshot_with_proxy(&properties, thresholds).await
    }

    /// Emits an initial snapshot and then a fresh snapshot after each device
    /// PropertiesChanged signal. The match rule filters on the UPower Device
    /// interface at the bus, so unrelated property traffic is not delivered.
    ///
    /// The callback runs synchronously on the caller's executor and must be
    /// quick and non-blocking. A shell event-loop bridge can use it to publish
    /// the latest immutable snapshot to the UI thread.
    ///
    /// This future ends with an error if the D-Bus signal stream closes; the
    /// owner should reconnect the shared system-bus connection and adapter.
    pub async fn run_monitor<F>(
        &self,
        thresholds: BatteryThresholds,
        mut on_update: F,
    ) -> Result<(), UPowerError>
    where
        F: FnMut(BatterySnapshot),
    {
        let properties = Proxy::new(
            &self.connection,
            UPOWER_DESTINATION,
            self.display_device.as_str(),
            DBUS_PROPERTIES_INTERFACE,
        )
        .await?;
        let mut signals = properties
            .receive_signal_with_args("PropertiesChanged", &[(0, UPOWER_DEVICE_INTERFACE)])
            .await?;

        // Subscribe first, then read the initial snapshot. Any change racing
        // with GetAll remains queued and is followed by another full snapshot.
        on_update(self.snapshot_with_proxy(&properties, thresholds).await?);

        while let Some(message) = signals.next().await {
            let message = message?;
            let (interface, _changed, _invalidated): (
                String,
                HashMap<String, OwnedValue>,
                Vec<String>,
            ) = message.body().deserialize()?;

            if interface == UPOWER_DEVICE_INTERFACE {
                on_update(self.snapshot_with_proxy(&properties, thresholds).await?);
            }
        }

        Err(UPowerError::SignalStreamClosed)
    }

    async fn snapshot_with_proxy(
        &self,
        properties: &Proxy<'_>,
        thresholds: BatteryThresholds,
    ) -> Result<BatterySnapshot, UPowerError> {
        let values: HashMap<String, OwnedValue> = properties
            .call("GetAll", &(UPOWER_DEVICE_INTERFACE,))
            .await?;
        let raw = decode_device_properties(&values)?;
        Ok(BatterySnapshot::from_upower(raw, thresholds))
    }
}

/// Converts the standard UPower Device Properties.GetAll dictionary into the
/// compact model input. Unknown properties are ignored for forward
/// compatibility; missing optional estimates/health values use safe defaults.
pub fn decode_device_properties(
    properties: &HashMap<String, OwnedValue>,
) -> Result<UPowerBatteryProperties, UPowerError> {
    Ok(UPowerBatteryProperties {
        device_type: required(properties, "Type")?,
        power_supply: required(properties, "PowerSupply")?,
        state: required(properties, "State")?,
        percentage_percent: required(properties, "Percentage")?,
        energy_rate_w: optional(properties, "EnergyRate")?.unwrap_or_default(),
        time_to_empty_seconds: optional(properties, "TimeToEmpty")?.unwrap_or_default(),
        time_to_full_seconds: optional(properties, "TimeToFull")?.unwrap_or_default(),
        energy_full: optional(properties, "EnergyFull")?,
        energy_full_design: optional(properties, "EnergyFullDesign")?,
    })
}

fn required<T>(
    properties: &HashMap<String, OwnedValue>,
    name: &'static str,
) -> Result<T, UPowerError>
where
    for<'a> T: TryFrom<&'a OwnedValue>,
{
    let value = properties
        .get(name)
        .ok_or(UPowerError::MissingProperty(name))?;
    T::try_from(value).map_err(|_| UPowerError::InvalidProperty(name))
}

fn optional<T>(
    properties: &HashMap<String, OwnedValue>,
    name: &'static str,
) -> Result<Option<T>, UPowerError>
where
    for<'a> T: TryFrom<&'a OwnedValue>,
{
    properties
        .get(name)
        .map(|value| T::try_from(value).map_err(|_| UPowerError::InvalidProperty(name)))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::{BatterySnapshot, DeviceState};

    fn fixture() -> HashMap<String, OwnedValue> {
        HashMap::from([
            ("Type".to_owned(), OwnedValue::from(2_u32)),
            ("PowerSupply".to_owned(), OwnedValue::from(true)),
            (
                "State".to_owned(),
                OwnedValue::from(DeviceState::Discharging as u32),
            ),
            ("Percentage".to_owned(), OwnedValue::from(72.5_f64)),
            ("EnergyRate".to_owned(), OwnedValue::from(11.25_f64)),
            ("TimeToEmpty".to_owned(), OwnedValue::from(7_200_i64)),
            ("TimeToFull".to_owned(), OwnedValue::from(0_i64)),
            ("EnergyFull".to_owned(), OwnedValue::from(45.0_f64)),
            ("EnergyFullDesign".to_owned(), OwnedValue::from(50.0_f64)),
        ])
    }

    #[test]
    fn decodes_the_composite_display_device_properties() {
        let decoded = decode_device_properties(&fixture()).unwrap();

        assert_eq!(decoded.device_type, 2);
        assert!(decoded.power_supply);
        assert_eq!(decoded.state, DeviceState::Discharging as u32);
        assert_eq!(decoded.percentage_percent, 72.5);
        assert_eq!(decoded.energy_rate_w, 11.25);
        assert_eq!(decoded.time_to_empty_seconds, 7_200);
        assert_eq!(decoded.time_to_full_seconds, 0);
        assert_eq!(decoded.energy_full, Some(45.0));
        assert_eq!(decoded.energy_full_design, Some(50.0));
    }

    #[test]
    fn preserves_upower_rate_sign_for_quickshell_projection() {
        let mut values = fixture();
        values.insert(
            "State".to_owned(),
            OwnedValue::from(DeviceState::Charging as u32),
        );
        // UPower uses negative for charging; Quickshell exposes positive.
        values.insert("EnergyRate".to_owned(), OwnedValue::from(-8.5_f64));

        let raw = decode_device_properties(&values).unwrap();
        let snapshot = BatterySnapshot::from_upower(raw, BatteryThresholds::default());

        assert_eq!(raw.energy_rate_w, -8.5);
        assert_eq!(snapshot.change_rate_w, 8.5);
    }

    #[test]
    fn absent_optional_estimates_and_health_are_safe() {
        let mut values = fixture();
        for key in [
            "EnergyRate",
            "TimeToEmpty",
            "TimeToFull",
            "EnergyFull",
            "EnergyFullDesign",
        ] {
            values.remove(key);
        }

        let decoded = decode_device_properties(&values).unwrap();
        assert_eq!(decoded.energy_rate_w, 0.0);
        assert_eq!(decoded.time_to_empty_seconds, 0);
        assert_eq!(decoded.time_to_full_seconds, 0);
        assert_eq!(decoded.energy_full, None);
        assert_eq!(decoded.energy_full_design, None);
    }

    #[test]
    fn missing_required_properties_are_reported() {
        let mut values = fixture();
        values.remove("Percentage");

        assert!(matches!(
            decode_device_properties(&values),
            Err(UPowerError::MissingProperty("Percentage"))
        ));
    }

    #[test]
    fn invalid_required_property_types_are_rejected() {
        let mut values = fixture();
        values.insert("Type".to_owned(), OwnedValue::from(true));

        assert!(matches!(
            decode_device_properties(&values),
            Err(UPowerError::InvalidProperty("Type"))
        ));
    }

    #[test]
    fn unknown_properties_are_ignored() {
        let mut values = fixture();
        values.insert("FutureProperty".to_owned(), OwnedValue::from(42_u32));

        assert!(decode_device_properties(&values).is_ok());
    }
}

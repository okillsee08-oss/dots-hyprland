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

#[cfg(all(test, target_os = "linux"))]
mod dbus_integration_tests {
    use super::*;
    use crate::battery::DeviceState;
    use futures_lite::future;
    use std::{
        io::{self, BufRead, BufReader},
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex, mpsc},
        thread,
        time::Duration,
    };
    use zbus::connection::Builder;

    const DISPLAY_DEVICE_PATH: &str = "/org/freedesktop/UPower/devices/DisplayDevice";

    struct PrivateBus {
        child: Child,
        address: String,
    }

    impl PrivateBus {
        fn start() -> io::Result<Self> {
            let mut child = Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?;
            let stdout = child.stdout.take().ok_or_else(|| {
                io::Error::other("dbus-daemon stdout was not piped")
            })?;
            let mut reader = BufReader::new(stdout);
            let mut address = String::new();
            match reader.read_line(&mut address) {
                Ok(0) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "dbus-daemon exited before publishing its address",
                    ))
                }
                Ok(_) if !address.trim().is_empty() => Ok(Self {
                    child,
                    address: address.trim().to_owned(),
                }),
                Ok(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "dbus-daemon published an empty address",
                    ))
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Err(error)
                }
            }
        }
    }

    impl Drop for PrivateBus {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[derive(Clone)]
    struct MockDeviceState {
        state: u32,
        percentage: f64,
        energy_rate: f64,
    }

    #[derive(Clone)]
    struct MockDevice(Arc<Mutex<MockDeviceState>>);

    #[zbus::interface(name = "org.freedesktop.UPower.Device")]
    impl MockDevice {
        #[zbus(property, name = "Type")]
        fn device_type(&self) -> u32 {
            2
        }

        #[zbus(property, name = "PowerSupply")]
        fn power_supply(&self) -> bool {
            true
        }

        #[zbus(property, name = "State")]
        fn state(&self) -> u32 {
            self.0.lock().unwrap().state
        }

        #[zbus(property, name = "Percentage")]
        fn percentage(&self) -> f64 {
            self.0.lock().unwrap().percentage
        }

        #[zbus(property, name = "EnergyRate")]
        fn energy_rate(&self) -> f64 {
            self.0.lock().unwrap().energy_rate
        }

        #[zbus(property, name = "TimeToEmpty")]
        fn time_to_empty(&self) -> i64 {
            3_600
        }

        #[zbus(property, name = "TimeToFull")]
        fn time_to_full(&self) -> i64 {
            1_800
        }

        #[zbus(property, name = "EnergyFull")]
        fn energy_full(&self) -> f64 {
            45.0
        }

        #[zbus(property, name = "EnergyFullDesign")]
        fn energy_full_design(&self) -> f64 {
            50.0
        }
    }

    struct MockManager;

    #[zbus::interface(name = "org.freedesktop.UPower")]
    impl MockManager {
        fn get_display_device(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from(DISPLAY_DEVICE_PATH).unwrap()
        }
    }

    #[test]
    fn monitor_reads_initial_snapshot_and_refreshes_changed_and_invalidated_properties() {
        future::block_on(async {
            let bus = PrivateBus::start().expect("private D-Bus daemon should start");
            let device_state = Arc::new(Mutex::new(MockDeviceState {
                state: DeviceState::Discharging as u32,
                percentage: 72.5,
                energy_rate: 11.25,
            }));

            let service = Builder::address(bus.address.as_str())
                .unwrap()
                .name(UPOWER_DESTINATION)
                .unwrap()
                .serve_at(UPOWER_MANAGER_PATH, MockManager)
                .unwrap()
                .serve_at(DISPLAY_DEVICE_PATH, MockDevice(Arc::clone(&device_state)))
                .unwrap()
                .build()
                .await
                .expect("mock UPower service should connect");

            let client = Builder::address(bus.address.as_str())
                .unwrap()
                .build()
                .await
                .expect("UPower client should connect");
            let adapter = UPowerAdapter::new(&client)
                .await
                .expect("adapter should resolve GetDisplayDevice");

            let (updates_tx, updates_rx) = mpsc::channel();
            let (stop_tx, stop_rx) = async_channel::bounded::<()>(1);
            let monitor_adapter = adapter.clone();
            let monitor = thread::spawn(move || {
                let run =
                    monitor_adapter.run_monitor(BatteryThresholds::default(), move |snapshot| {
                        let _ = updates_tx.send(snapshot);
                    });
                let stop = async move {
                    let _ = stop_rx.recv().await;
                    Ok::<(), UPowerError>(())
                };
                future::block_on(future::or(run, stop))
            });

            let initial = updates_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("monitor should publish its initial GetAll snapshot");
            assert!(initial.available);
            assert_eq!(initial.percentage, 0.725);
            assert_eq!(initial.state, DeviceState::Discharging);
            assert_eq!(initial.change_rate_w, -11.25);

            {
                let mut state = device_state.lock().unwrap();
                state.state = DeviceState::Charging as u32;
                state.percentage = 41.0;
                state.energy_rate = -8.5;
            }
            let changed = HashMap::from([
                (
                    "State".to_owned(),
                    OwnedValue::from(DeviceState::Charging as u32),
                ),
                ("Percentage".to_owned(), OwnedValue::from(41.0_f64)),
                ("EnergyRate".to_owned(), OwnedValue::from(-8.5_f64)),
            ]);
            service
                .emit_signal(
                    None::<&str>,
                    DISPLAY_DEVICE_PATH,
                    DBUS_PROPERTIES_INTERFACE,
                    "PropertiesChanged",
                    &(UPOWER_DEVICE_INTERFACE, changed, Vec::<String>::new()),
                )
                .await
                .expect("mock service should emit PropertiesChanged");

            let updated = updates_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("monitor should refresh after a changed-property signal");
            assert_eq!(updated.percentage, 0.41);
            assert_eq!(updated.state, DeviceState::Charging);
            assert_eq!(updated.change_rate_w, 8.5);
            assert_eq!(updated.time_to_full_seconds, Some(1_800));

            {
                let mut state = device_state.lock().unwrap();
                state.percentage = 38.0;
            }
            let invalidated = Vec::from(["Percentage".to_owned()]);
            service
                .emit_signal(
                    None::<&str>,
                    DISPLAY_DEVICE_PATH,
                    DBUS_PROPERTIES_INTERFACE,
                    "PropertiesChanged",
                    &(
                        UPOWER_DEVICE_INTERFACE,
                        HashMap::<String, OwnedValue>::new(),
                        invalidated,
                    ),
                )
                .await
                .expect("mock service should emit an invalidation signal");

            let refreshed = updates_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("monitor should refresh invalidated properties with GetAll");
            assert_eq!(refreshed.percentage, 0.38);
            assert_eq!(refreshed.state, DeviceState::Charging);

            stop_tx
                .try_send(())
                .expect("monitor stop signal should queue");
            assert!(monitor.join().expect("monitor thread should join").is_ok());

            drop(adapter);
            drop(client);
            drop(service);
            drop(bus);
        });
    }
}

//! Bounded bridge from the asynchronous UPower monitor to the blocking shell loop.
//!
//! One process-wide worker owns the shared system-bus connection. Updates are
//! coalesced into a single latest-value slot and signalled with eventfd, so the
//! Wayland thread never waits for D-Bus and cannot accumulate a stale queue.

use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use futures_lite::future;
use crate::{
    battery::{BatterySnapshot, BatteryThresholds},
    bus,
    upower::UPowerAdapter,
};

const INITIAL_RETRY: Duration = Duration::from_millis(250);
const MAX_RETRY: Duration = Duration::from_secs(30);
const RETRY_CHECK_INTERVAL: Duration = Duration::from_millis(50);

struct SnapshotMailbox {
    latest: Mutex<Option<BatterySnapshot>>,
    wake_fd: OwnedFd,
}

impl SnapshotMailbox {
    fn new() -> io::Result<Self> {
        let raw_fd = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        if raw_fd < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            latest: Mutex::new(None),
            wake_fd: unsafe { OwnedFd::from_raw_fd(raw_fd) },
        })
    }

    fn as_raw_fd(&self) -> RawFd {
        self.wake_fd.as_raw_fd()
    }

    /// Replace any older pending snapshot and wake the shell at most once
    /// until it drains the eventfd. EAGAIN means the eventfd is already
    /// readable, which is exactly the desired coalescing behavior.
    fn publish(&self, snapshot: BatterySnapshot) {
        {
            let mut latest = self
                .latest
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *latest = Some(snapshot);
        }

        let value = 1_u64;
        loop {
            let written = unsafe {
                libc::write(
                    self.wake_fd.as_raw_fd(),
                    (&value as *const u64).cast(),
                    std::mem::size_of::<u64>(),
                )
            };
            if written == std::mem::size_of::<u64>() as isize {
                return;
            }
            if written < 0 {
                let error = io::Error::last_os_error();
                match error.kind() {
                    io::ErrorKind::Interrupted => continue,
                    io::ErrorKind::WouldBlock => return,
                    _ => {
                        eprintln!("ii-shell: battery eventfd write failed: {error}");
                        return;
                    }
                }
            }
            eprintln!("ii-shell: battery eventfd write returned {written} bytes");
            return;
        }
    }

    /// Drain all accumulated wakeups, then take the newest immutable snapshot.
    /// A concurrent publication may leave one harmless extra eventfd wakeup,
    /// but it cannot be lost: the latest slot is protected independently.
    fn take_latest(&self) -> io::Result<Option<BatterySnapshot>> {
        loop {
            let mut value = 0_u64;
            let read = unsafe {
                libc::read(
                    self.wake_fd.as_raw_fd(),
                    (&mut value as *mut u64).cast(),
                    std::mem::size_of::<u64>(),
                )
            };
            if read == std::mem::size_of::<u64>() as isize {
                continue;
            }
            if read < 0 {
                let error = io::Error::last_os_error();
                match error.kind() {
                    io::ErrorKind::Interrupted => continue,
                    io::ErrorKind::WouldBlock => break,
                    _ => return Err(error),
                }
            }
            if read == 0 {
                break;
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short read from battery eventfd",
            ));
        }

        let mut latest = self
            .latest
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(latest.take())
    }
}

enum WorkerExit {
    Retry,
    Stop,
}

/// Owns the single UPower monitor worker and its eventfd mailbox.
///
/// The shell polls `as_raw_fd()` beside Wayland and Hyprland. Calling
/// `take_latest()` drains accumulated notifications and returns only the
/// newest state, keeping memory and UI work bounded when several D-Bus
/// property changes arrive before the next display-loop iteration.
pub struct BatteryMonitorWorker {
    mailbox: Arc<SnapshotMailbox>,
    stop: async_channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl BatteryMonitorWorker {
    pub fn spawn(thresholds: BatteryThresholds) -> io::Result<Self> {
        let mailbox = Arc::new(SnapshotMailbox::new()?);
        let worker_mailbox = Arc::clone(&mailbox);
        let (stop, stop_receiver) = async_channel::bounded::<()>(1);
        let worker = thread::Builder::new()
            .name("ii-upower-monitor".into())
            .stack_size(256 * 1024)
            .spawn(move || run_worker(worker_mailbox, stop_receiver, thresholds))?;

        Ok(Self {
            mailbox,
            stop,
            thread: Some(worker),
        })
    }

    pub fn as_raw_fd(&self) -> RawFd {
        self.mailbox.as_raw_fd()
    }

    pub fn take_latest(&self) -> io::Result<Option<BatterySnapshot>> {
        self.mailbox.take_latest()
    }
}

impl Drop for BatteryMonitorWorker {
    fn drop(&mut self) {
        let _ = self.stop.try_send(());
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}

fn run_worker(
    mailbox: Arc<SnapshotMailbox>,
    stop_receiver: async_channel::Receiver<()>,
    thresholds: BatteryThresholds,
) {
    let mut retry_delay = INITIAL_RETRY;

    loop {
        let worker_mailbox = Arc::clone(&mailbox);
        let monitor = async move {
            let connection = match bus::connect_system().await {
                Ok(connection) => connection,
                Err(error) => {
                    eprintln!("ii-shell: system D-Bus unavailable for UPower: {error}");
                    return WorkerExit::Retry;
                }
            };
            let adapter = match UPowerAdapter::new(&connection).await {
                Ok(adapter) => adapter,
                Err(error) => {
                    eprintln!("ii-shell: UPower display device unavailable: {error}");
                    return WorkerExit::Retry;
                }
            };

            if let Err(error) = adapter
                .run_monitor(thresholds, move |snapshot| worker_mailbox.publish(snapshot))
                .await
            {
                eprintln!("ii-shell: UPower monitor disconnected: {error}");
            }
            WorkerExit::Retry
        };
        let stop = async {
            let _ = stop_receiver.recv().await;
            WorkerExit::Stop
        };

        match future::block_on(future::or(monitor, stop)) {
            WorkerExit::Stop => return,
            WorkerExit::Retry => {
                if sleep_or_stop(&stop_receiver, retry_delay) {
                    return;
                }
                retry_delay = retry_delay.saturating_mul(2).min(MAX_RETRY);
            }
        }
    }
}

fn sleep_or_stop(receiver: &async_channel::Receiver<()>, delay: Duration) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        match receiver.try_recv() {
            Ok(()) | Err(async_channel::TryRecvError::Closed) => return true,
            Err(async_channel::TryRecvError::Empty) => {}
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        thread::sleep(remaining.min(RETRY_CHECK_INTERVAL));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::{DeviceState, UPowerBatteryProperties};

    fn snapshot(percent: f64) -> BatterySnapshot {
        BatterySnapshot::from_upower(
            UPowerBatteryProperties {
                device_type: 2,
                power_supply: true,
                state: DeviceState::Discharging as u32,
                percentage_percent: percent,
                energy_rate_w: 8.0,
                time_to_empty_seconds: 3_600,
                time_to_full_seconds: 0,
                energy_full: Some(40.0),
                energy_full_design: Some(50.0),
            },
            BatteryThresholds::default(),
        )
    }

    fn is_readable(fd: RawFd) -> bool {
        let mut descriptor = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut descriptor, 1, 0) } > 0
            && descriptor.revents & libc::POLLIN != 0
    }

    #[test]
    fn mailbox_keeps_only_the_newest_snapshot_and_coalesces_wakeups() {
        let mailbox = SnapshotMailbox::new().unwrap();
        assert_eq!(mailbox.take_latest().unwrap(), None);
        assert!(!is_readable(mailbox.as_raw_fd()));

        mailbox.publish(snapshot(25.0));
        mailbox.publish(snapshot(72.0));
        mailbox.publish(snapshot(88.0));
        assert!(is_readable(mailbox.as_raw_fd()));

        let latest = mailbox.take_latest().unwrap().unwrap();
        assert_eq!(latest.percentage, 0.88);
        assert_eq!(latest.change_rate_w, -8.0);
        assert!(!is_readable(mailbox.as_raw_fd()));
        assert_eq!(mailbox.take_latest().unwrap(), None);
    }

    #[test]
    fn a_publication_after_a_drain_generates_a_new_wakeup() {
        let mailbox = SnapshotMailbox::new().unwrap();

        mailbox.publish(snapshot(40.0));
        assert_eq!(
            mailbox.take_latest().unwrap().unwrap().percentage,
            0.40
        );
        assert!(!is_readable(mailbox.as_raw_fd()));

        mailbox.publish(snapshot(41.0));
        assert!(is_readable(mailbox.as_raw_fd()));
        assert_eq!(
            mailbox.take_latest().unwrap().unwrap().percentage,
            0.41
        );
        assert!(!is_readable(mailbox.as_raw_fd()));
    }
}

//! Event-driven Hyprland state coordinator.
//!
//! The coordinator joins the non-blocking broadcast socket, the pure event
//! reducer, and the command socket. The owner calls `drain_ready_events` only
//! after the event fd is readable, then `refresh_pending` once per batch.
//! Refresh flags are coalesced, and failed queries remain pending for retry.
//! This module deliberately does not own or poll the Wayland event loop yet.

use std::{
    io,
    os::fd::RawFd,
};

use crate::{
    hyprland_events::{EventParseError, HyprlandEventStream},
    hyprland_ipc::{IpcError, HyprlandIpc, Snapshot},
    hyprland_state::{HyprlandState, StateChange},
};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RefreshPlan {
    pub monitors: bool,
    pub workspaces: bool,
    pub active_window: bool,
}

impl RefreshPlan {
    fn include(&mut self, change: StateChange) {
        self.monitors |= change.refresh_monitors;
        self.workspaces |= change.refresh_workspaces;
        self.active_window |= change.refresh_active_window;
    }

    pub fn is_empty(self) -> bool {
        !self.monitors && !self.workspaces && !self.active_window
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct EventDrain {
    pub applied: usize,
    pub state_changed: bool,
    pub parse_errors: Vec<EventParseError>,
    pub stream_closed: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RefreshReport {
    pub monitors_refreshed: bool,
    pub workspaces_refreshed: bool,
    pub active_window_refreshed: bool,
    pub state_changed: bool,
}

pub struct HyprlandRuntime {
    ipc: HyprlandIpc,
    events: HyprlandEventStream,
    state: HyprlandState,
    pending: RefreshPlan,
}

impl HyprlandRuntime {
    /// Connect the event socket first, then take an initial command-socket
    /// snapshot. Events arriving during the snapshot remain queued for the
    /// first readable notification.
    pub fn connect_from_environment() -> Result<Self, IpcError> {
        let events = HyprlandEventStream::connect_from_environment()?;
        let ipc = HyprlandIpc::connect_from_environment()?;
        let state = HyprlandState::from(ipc.snapshot()?);
        Ok(Self::from_parts(ipc, events, state))
    }

    /// Inject connected transports and a known initial state. This also keeps
    /// the coordinator testable without requiring a running Hyprland session.
    pub fn from_parts(
        ipc: HyprlandIpc,
        events: HyprlandEventStream,
        state: HyprlandState,
    ) -> Self {
        Self {
            ipc,
            events,
            state,
            pending: RefreshPlan::default(),
        }
    }

    pub fn event_fd(&self) -> RawFd {
        self.events.raw_fd()
    }

    pub fn event_stream_closed(&self) -> bool {
        self.events.is_closed()
    }

    pub fn state(&self) -> &HyprlandState {
        &self.state
    }

    pub fn pending_refresh(&self) -> RefreshPlan {
        self.pending
    }

    /// Drain all bytes currently available on a readable event socket.
    /// Parse failures are reported but do not prevent later lines being applied.
    pub fn drain_ready_events(&mut self) -> Result<EventDrain, io::Error> {
        let decoded = self.events.read_available()?;
        let mut report = EventDrain::default();

        for result in decoded {
            match result {
                Ok(event) => {
                    let change = self.state.apply(&event);
                    self.pending.include(change);
                    report.applied += 1;
                    report.state_changed |= change.changed;
                }
                Err(error) => report.parse_errors.push(error),
            }
        }

        report.stream_closed = self.events.is_closed();
        Ok(report)
    }

    /// Run at most one query per requested subsystem. A flag is cleared only
    /// after its query succeeds, so transient IPC errors are safely retryable.
    pub fn refresh_pending(&mut self) -> Result<RefreshReport, IpcError> {
        let mut report = RefreshReport::default();

        if self.pending.monitors {
            let monitors = self.ipc.monitors()?;
            report.state_changed |= self.state.monitors != monitors;
            self.state.monitors = monitors;
            self.pending.monitors = false;
            report.monitors_refreshed = true;
        }

        if self.pending.workspaces {
            let workspaces = self.ipc.workspaces()?;
            report.state_changed |= self.state.workspaces != workspaces;
            self.state.workspaces = workspaces;
            self.pending.workspaces = false;
            report.workspaces_refreshed = true;
        }

        if self.pending.active_window {
            let active_window = self.ipc.active_window()?;
            report.state_changed |= self.state.active_window != active_window;
            self.state.active_window = active_window;
            self.pending.active_window = false;
            report.active_window_refreshed = true;
        }

        Ok(report)
    }

    /// Replace the cached state with a full authoritative snapshot, clearing
    /// any refresh flags covered by that snapshot.
    pub fn apply_snapshot(&mut self, snapshot: Snapshot) -> bool {
        let next = HyprlandState::from(snapshot);
        let changed = self.state != next;
        self.state = next;
        self.pending = RefreshPlan::default();
        changed
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        os::unix::net::{UnixListener, UnixStream},
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        thread,
    };

    use super::*;
    use crate::{
        hyprland_events::HyprlandEventStream,
        hyprland_ipc::{ActiveWindow, Monitor, Workspace},
    };

    static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);

    fn socket_path() -> PathBuf {
        let id = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ii-shell-hyprland-runtime-{}-{id}.sock",
            std::process::id()
        ))
    }

    fn empty_state() -> HyprlandState {
        HyprlandState {
            monitors: Vec::new(),
            workspaces: Vec::new(),
            active_window: ActiveWindow {
                address: String::new(),
                app_class: String::new(),
                title: String::new(),
            },
        }
    }

    fn runtime_with_event_pair(path: PathBuf) -> (HyprlandRuntime, UnixStream) {
        let (writer, reader) = UnixStream::pair().unwrap();
        let events = HyprlandEventStream::from_stream(reader).unwrap();
        let ipc = HyprlandIpc::from_socket_path(path);
        (HyprlandRuntime::from_parts(ipc, events, empty_state()), writer)
    }

    #[test]
    fn event_bursts_coalesce_targeted_queries_and_update_cached_state() {
        let path = socket_path();
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                stream.read_to_string(&mut request).unwrap();
                requests.push(request.clone());
                let response = match request.as_str() {
                    "j/monitors" => {
                        r#"[{"id":0,"name":"eDP-1","width":1920,"height":1080,"focused":true,"activeWorkspace":{"id":1,"name":"1"}}]"#
                    }
                    "j/workspaces" => {
                        r#"[{"id":1,"name":"1","monitor":"eDP-1"}]"#
                    }
                    "j/activewindow" => {
                        r#"{"address":"0x123","class":"firefox","title":"Docs"}"#
                    }
                    other => panic!("unexpected IPC request: {other}"),
                };
                stream.write_all(response.as_bytes()).unwrap();
            }
            requests
        });

        let (mut runtime, mut event_writer) = runtime_with_event_pair(path.clone());
        event_writer
            .write_all(
                b"monitoradded>>DP-1\nmonitoradded>>DP-2\nactivewindowv2>>0x123\n",
            )
            .unwrap();

        let drained = runtime.drain_ready_events().unwrap();
        assert_eq!(drained.applied, 3);
        assert!(drained.parse_errors.is_empty());
        assert!(drained.state_changed);
        assert_eq!(
            runtime.pending_refresh(),
            RefreshPlan {
                monitors: true,
                workspaces: true,
                active_window: true,
            }
        );

        let refreshed = runtime.refresh_pending().unwrap();
        assert!(refreshed.monitors_refreshed);
        assert!(refreshed.workspaces_refreshed);
        assert!(refreshed.active_window_refreshed);
        assert!(refreshed.state_changed);
        assert!(runtime.pending_refresh().is_empty());
        assert_eq!(runtime.state().monitors.len(), 1);
        assert_eq!(runtime.state().workspaces.len(), 1);
        assert_eq!(runtime.state().active_window.app_class, "firefox");

        let requests = server.join().unwrap();
        assert_eq!(
            requests,
            vec!["j/monitors", "j/workspaces", "j/activewindow"]
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn malformed_line_is_reported_without_discarding_valid_neighbors() {
        let path = socket_path();
        let (mut runtime, mut event_writer) = runtime_with_event_pair(path);
        event_writer
            .write_all(b"workspacev2>>bad-id,code\nactivewindowv2>>0xabc\n")
            .unwrap();

        let report = runtime.drain_ready_events().unwrap();
        assert_eq!(report.applied, 1);
        assert_eq!(report.parse_errors.len(), 1);
        assert!(report.state_changed);
        assert_eq!(runtime.state().active_window.address, "0xabc");
        assert!(runtime.pending_refresh().active_window);
    }

    #[test]
    fn failed_refresh_keeps_the_unresolved_request_pending() {
        let path = socket_path();
        let (mut runtime, mut event_writer) = runtime_with_event_pair(path);
        event_writer.write_all(b"monitoradded>>DP-1\n").unwrap();
        runtime.drain_ready_events().unwrap();

        assert!(runtime.refresh_pending().is_err());
        assert!(runtime.pending_refresh().monitors);
        assert!(runtime.pending_refresh().workspaces);
    }

    #[test]
    fn identical_authoritative_snapshot_does_not_report_a_change() {
        let path = socket_path();
        let (mut runtime, _writer) = runtime_with_event_pair(path);
        let snapshot = Snapshot {
            monitors: Vec::<Monitor>::new(),
            workspaces: Vec::<Workspace>::new(),
            active_window: ActiveWindow {
                address: String::new(),
                app_class: String::new(),
                title: String::new(),
            },
        };
        assert!(!runtime.apply_snapshot(snapshot));
        assert!(runtime.pending_refresh().is_empty());
    }
}

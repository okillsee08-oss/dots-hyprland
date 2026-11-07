//! Non-blocking reader and bounded decoder for Hyprland's socket2 event stream.
//!
//! The decoder is independent of the Wayland loop and accepts arbitrarily
//! chunked reads. The stream exposes a non-blocking drain operation so the
//! caller can register its fd with the shell's event loop; it never spawns a
//! listener thread or polls in a busy loop.

use std::{
    io::{self, Read},
    os::unix::net::UnixStream,
};

use crate::hyprland_ipc::{event_socket_path, IpcError};

pub const MAX_EVENT_LINE_BYTES: usize = 4096;
const READ_CHUNK_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyprlandEvent {
    Workspace { name: String },
    WorkspaceV2 { id: i64, name: String },
    FocusedMonitor { monitor: String, workspace: String },
    FocusedMonitorV2 { monitor: String, workspace_id: i64 },
    ActiveWindow { app_class: String, title: String },
    ActiveWindowV2 { address: String },
    Fullscreen { active: bool },
    MonitorAdded { name: String },
    MonitorRemoved { name: String },
    MonitorAddedV2 { id: i64, name: String, description: String },
    MonitorRemovedV2 { id: i64, name: String, description: String },
    OpenWindow { address: String, workspace: String, app_class: String, title: String },
    CloseWindow { address: String },
    MoveWindow { address: String, workspace: String },
    MoveWindowV2 { address: String, workspace_id: i64, workspace_name: String },
    CreateWorkspace { name: String },
    CreateWorkspaceV2 { id: i64, name: String },
    DestroyWorkspace { name: String },
    DestroyWorkspaceV2 { id: i64, name: String },
    MoveWorkspace { name: String, monitor: String },
    MoveWorkspaceV2 { id: i64, name: String, monitor: String },
    RenameWorkspace { id: i64, name: String },
    ActiveSpecial { workspace: String, monitor: String },
    WindowTitleV2 { address: String, title: String },
    Submap { name: String },
    ConfigReloaded,
    /// Preserve events unknown to this version instead of silently dropping them.
    Unknown { name: String, payload: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventParseError {
    #[error("Hyprland event line is missing the >> separator")]
    MissingSeparator,
    #[error("Hyprland event name is empty")]
    EmptyName,
    #[error("invalid payload for Hyprland event {event}: expected {expected}")]
    InvalidPayload { event: String, expected: &'static str },
    #[error("Hyprland event line is not valid UTF-8")]
    InvalidUtf8,
    #[error("Hyprland event line exceeds 4096 bytes")]
    LineTooLong,
}

fn invalid(event: &str, expected: &'static str) -> EventParseError {
    EventParseError::InvalidPayload { event: event.to_owned(), expected }
}

fn parse_id(event: &str, value: &str) -> Result<i64, EventParseError> {
    value.parse().map_err(|_| invalid(event, "a signed integer id"))
}

fn pair<'a>(event: &str, payload: &'a str) -> Result<(&'a str, &'a str), EventParseError> {
    payload.split_once(',').ok_or_else(|| invalid(event, "two comma-separated fields"))
}

/// Parse one complete EVENT>>DATA line (without its line terminator).
pub fn parse_event_line(line: &str) -> Result<HyprlandEvent, EventParseError> {
    let (name, payload) = line.split_once(">>").ok_or(EventParseError::MissingSeparator)?;
    if name.is_empty() {
        return Err(EventParseError::EmptyName);
    }

    let event = match name {
        "workspace" => HyprlandEvent::Workspace { name: payload.to_owned() },
        "workspacev2" => {
            let (id, workspace_name) = pair(name, payload)?;
            HyprlandEvent::WorkspaceV2 {
                id: parse_id(name, id)?,
                name: workspace_name.to_owned(),
            }
        }
        "focusedmon" => {
            let (monitor, workspace) = pair(name, payload)?;
            HyprlandEvent::FocusedMonitor {
                monitor: monitor.to_owned(),
                workspace: workspace.to_owned(),
            }
        }
        "focusedmonv2" => {
            let (monitor, workspace_id) = pair(name, payload)?;
            HyprlandEvent::FocusedMonitorV2 {
                monitor: monitor.to_owned(),
                workspace_id: parse_id(name, workspace_id)?,
            }
        }
        "activewindow" => {
            let (app_class, title) = pair(name, payload)?;
            HyprlandEvent::ActiveWindow {
                app_class: app_class.to_owned(),
                title: title.to_owned(),
            }
        }
        "activewindowv2" => HyprlandEvent::ActiveWindowV2 { address: payload.to_owned() },
        "fullscreen" => match payload {
            "0" => HyprlandEvent::Fullscreen { active: false },
            "1" => HyprlandEvent::Fullscreen { active: true },
            _ => return Err(invalid(name, "0 or 1")),
        },
        "monitoradded" => HyprlandEvent::MonitorAdded { name: payload.to_owned() },
        "monitorremoved" => HyprlandEvent::MonitorRemoved { name: payload.to_owned() },
        "monitoraddedv2" | "monitorremovedv2" => {
            let mut fields = payload.splitn(3, ',');
            let id = parse_id(name, fields.next().unwrap_or_default())?;
            let monitor_name = fields.next().ok_or_else(|| invalid(name, "id,name,description"))?;
            let description = fields.next().ok_or_else(|| invalid(name, "id,name,description"))?;
            if name == "monitoraddedv2" {
                HyprlandEvent::MonitorAddedV2 {
                    id,
                    name: monitor_name.to_owned(),
                    description: description.to_owned(),
                }
            } else {
                HyprlandEvent::MonitorRemovedV2 {
                    id,
                    name: monitor_name.to_owned(),
                    description: description.to_owned(),
                }
            }
        }
        "openwindow" => {
            let fields: Vec<_> = payload.splitn(4, ',').collect();
            if fields.len() != 4 {
                return Err(invalid(name, "address,workspace,class,title"));
            }
            HyprlandEvent::OpenWindow {
                address: fields[0].to_owned(),
                workspace: fields[1].to_owned(),
                app_class: fields[2].to_owned(),
                title: fields[3].to_owned(),
            }
        }
        "closewindow" => HyprlandEvent::CloseWindow { address: payload.to_owned() },
        "movewindow" => {
            let (address, workspace) = pair(name, payload)?;
            HyprlandEvent::MoveWindow {
                address: address.to_owned(),
                workspace: workspace.to_owned(),
            }
        }
        "movewindowv2" => {
            let mut fields = payload.splitn(3, ',');
            let address = fields.next().unwrap_or_default();
            let workspace_id = parse_id(
                name,
                fields.next().ok_or_else(|| invalid(name, "address,workspace_id,workspace_name"))?,
            )?;
            let workspace_name = fields.next().ok_or_else(|| invalid(name, "address,workspace_id,workspace_name"))?;
            HyprlandEvent::MoveWindowV2 {
                address: address.to_owned(),
                workspace_id,
                workspace_name: workspace_name.to_owned(),
            }
        }
        "createworkspace" => HyprlandEvent::CreateWorkspace { name: payload.to_owned() },
        "createworkspacev2" | "destroyworkspacev2" => {
            let (id, workspace_name) = pair(name, payload)?;
            let id = parse_id(name, id)?;
            if name == "createworkspacev2" {
                HyprlandEvent::CreateWorkspaceV2 { id, name: workspace_name.to_owned() }
            } else {
                HyprlandEvent::DestroyWorkspaceV2 { id, name: workspace_name.to_owned() }
            }
        }
        "destroyworkspace" => HyprlandEvent::DestroyWorkspace { name: payload.to_owned() },
        "moveworkspace" => {
            let (workspace, monitor) = pair(name, payload)?;
            HyprlandEvent::MoveWorkspace {
                name: workspace.to_owned(),
                monitor: monitor.to_owned(),
            }
        }
        "moveworkspacev2" => {
            let mut fields = payload.splitn(3, ',');
            let id = parse_id(name, fields.next().unwrap_or_default())?;
            let workspace_name = fields.next().ok_or_else(|| invalid(name, "id,name,monitor"))?;
            let monitor = fields.next().ok_or_else(|| invalid(name, "id,name,monitor"))?;
            HyprlandEvent::MoveWorkspaceV2 {
                id,
                name: workspace_name.to_owned(),
                monitor: monitor.to_owned(),
            }
        }
        "renameworkspace" => {
            let (id, workspace_name) = pair(name, payload)?;
            HyprlandEvent::RenameWorkspace {
                id: parse_id(name, id)?,
                name: workspace_name.to_owned(),
            }
        }
        "activespecial" => {
            let (workspace, monitor) = pair(name, payload)?;
            HyprlandEvent::ActiveSpecial {
                workspace: workspace.to_owned(),
                monitor: monitor.to_owned(),
            }
        }
        "windowtitlev2" => {
            let (address, title) = pair(name, payload)?;
            HyprlandEvent::WindowTitleV2 {
                address: address.to_owned(),
                title: title.to_owned(),
            }
        }
        "submap" => HyprlandEvent::Submap { name: payload.to_owned() },
        "configreloaded" => HyprlandEvent::ConfigReloaded,
        _ => HyprlandEvent::Unknown { name: name.to_owned(), payload: payload.to_owned() },
    };
    Ok(event)
}

/// Incremental, bounded line decoder. Overlong lines are discarded through
/// their next newline, after which decoding resumes at the next event.
#[derive(Debug, Default)]
pub struct EventDecoder {
    pending: Vec<u8>,
    discarding_overlong_line: bool,
}

impl EventDecoder {
    pub fn new() -> Self { Self::default() }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Result<HyprlandEvent, EventParseError>> {
        let mut events = Vec::new();
        for part in bytes.split_inclusive(|byte| *byte == b'\n') {
            let terminated = part.last() == Some(&b'\n');
            if self.discarding_overlong_line {
                if terminated {
                    self.discarding_overlong_line = false;
                }
                continue;
            }

            let content = if terminated { &part[..part.len() - 1] } else { part };
            self.pending.extend_from_slice(content);
            if self.pending.len() > MAX_EVENT_LINE_BYTES {
                self.pending.clear();
                self.discarding_overlong_line = !terminated;
                events.push(Err(EventParseError::LineTooLong));
                continue;
            }

            if terminated {
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                if !self.pending.is_empty() {
                    let parsed = std::str::from_utf8(&self.pending)
                        .map_err(|_| EventParseError::InvalidUtf8)
                        .and_then(parse_event_line);
                    events.push(parsed);
                }
                self.pending.clear();
            }
        }
        events
    }
}

/// A connected, non-blocking event socket. Register its fd with the main event
/// loop and call read_available only when it becomes readable.
pub struct HyprlandEventStream {
    socket: UnixStream,
    decoder: EventDecoder,
    closed: bool,
}

impl HyprlandEventStream {
    pub fn connect_from_environment() -> Result<Self, IpcError> {
        let socket = UnixStream::connect(event_socket_path()?)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, decoder: EventDecoder::new(), closed: false })
    }

    /// Construct from an already-connected socket (also useful for integration tests).
    pub fn from_stream(socket: UnixStream) -> Result<Self, io::Error> {
        socket.set_nonblocking(true)?;
        Ok(Self { socket, decoder: EventDecoder::new(), closed: false })
    }

    pub fn raw_fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;
        self.socket.as_raw_fd()
    }

    pub fn is_closed(&self) -> bool { self.closed }

    /// Drain all currently available bytes without blocking.
    /// WouldBlock means the fd has been fully drained for this readiness edge.
    pub fn read_available(&mut self) -> Result<Vec<Result<HyprlandEvent, EventParseError>>, io::Error> {
        let mut decoded = Vec::new();
        let mut buffer = [0_u8; READ_CHUNK_BYTES];
        loop {
            match self.socket.read(&mut buffer) {
                Ok(0) => {
                    self.closed = true;
                    return Ok(decoded);
                }
                Ok(count) => decoded.extend(self.decoder.feed(&buffer[..count])),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(decoded),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_core_workspace_window_and_monitor_events() {
        assert_eq!(
            parse_event_line("workspacev2>>4,code").unwrap(),
            HyprlandEvent::WorkspaceV2 { id: 4, name: "code".into() }
        );
        assert_eq!(
            parse_event_line("focusedmon>>DP-1,4").unwrap(),
            HyprlandEvent::FocusedMonitor { monitor: "DP-1".into(), workspace: "4".into() }
        );
        assert_eq!(
            parse_event_line("activewindow>>firefox,Docs, notes").unwrap(),
            HyprlandEvent::ActiveWindow { app_class: "firefox".into(), title: "Docs, notes".into() }
        );
        assert_eq!(
            parse_event_line("openwindow>>0x123,4,firefox,Docs, notes").unwrap(),
            HyprlandEvent::OpenWindow {
                address: "0x123".into(),
                workspace: "4".into(),
                app_class: "firefox".into(),
                title: "Docs, notes".into(),
            }
        );
        assert_eq!(
            parse_event_line("monitoradded>>DP-2").unwrap(),
            HyprlandEvent::MonitorAdded { name: "DP-2".into() }
        );
    }

    #[test]
    fn parses_v2_monitor_workspace_and_window_events() {
        assert_eq!(
            parse_event_line("focusedmonv2>>DP-1,12").unwrap(),
            HyprlandEvent::FocusedMonitorV2 { monitor: "DP-1".into(), workspace_id: 12 }
        );
        assert_eq!(
            parse_event_line("monitoraddedv2>>2,DP-2,External Display").unwrap(),
            HyprlandEvent::MonitorAddedV2 {
                id: 2,
                name: "DP-2".into(),
                description: "External Display".into(),
            }
        );
        assert_eq!(
            parse_event_line("movewindowv2>>0xabc,3,three").unwrap(),
            HyprlandEvent::MoveWindowV2 {
                address: "0xabc".into(),
                workspace_id: 3,
                workspace_name: "three".into(),
            }
        );
    }

    #[test]
    fn preserves_unknown_events_for_forward_compatibility() {
        assert_eq!(
            parse_event_line("futureevent>>a,b").unwrap(),
            HyprlandEvent::Unknown { name: "futureevent".into(), payload: "a,b".into() }
        );
    }

    #[test]
    fn rejects_malformed_known_events() {
        assert!(matches!(
            parse_event_line("workspacev2>>not-an-id,code"),
            Err(EventParseError::InvalidPayload { .. })
        ));
        assert!(matches!(
            parse_event_line("activewindow>>class-only"),
            Err(EventParseError::InvalidPayload { .. })
        ));
        assert_eq!(parse_event_line("not-an-event"), Err(EventParseError::MissingSeparator));
    }

    #[test]
    fn decoder_handles_split_and_multiple_lines_and_crlf() {
        let mut decoder = EventDecoder::new();
        assert!(decoder.feed(b"workspacev2>>2,co").is_empty());
        let events = decoder.feed(b"de\r\nactivewindowv2>>0x1\n");
        assert_eq!(
            events,
            vec![
                Ok(HyprlandEvent::WorkspaceV2 { id: 2, name: "code".into() }),
                Ok(HyprlandEvent::ActiveWindowV2 { address: "0x1".into() }),
            ]
        );
    }

    #[test]
    fn decoder_discards_an_overlong_line_then_recovers() {
        let mut decoder = EventDecoder::new();
        let oversized = vec![b'x'; MAX_EVENT_LINE_BYTES + 1];
        assert_eq!(decoder.feed(&oversized), vec![Err(EventParseError::LineTooLong)]);
        assert_eq!(
            decoder.feed(b"\nworkspace>>2\n"),
            vec![Ok(HyprlandEvent::Workspace { name: "2".into() })]
        );
    }
}

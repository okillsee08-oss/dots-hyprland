//! Bounded adapter for Hyprland's per-instance command IPC socket.
//!
//! Queries are bounded and synchronous. Call them from startup or a worker,
//! never from a Wayland dispatch callback. Event-stream integration and
//! reconnect/backoff are coordinated by `hyprland_runtime` and the shell loop.

use std::{
    env,
    io::{Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    time::Duration,
};

use serde::Deserialize;

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_COMMAND_RESPONSE_BYTES: usize = 16 * 1024;
const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);
const COMMAND_TIMEOUT: Duration = Duration::from_millis(500);
const BATCH_REPLY_DELIMITER: &str = "\n\n\n";

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WorkspaceRef {
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SpecialWorkspace {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub focused: bool,
    #[serde(default, rename = "activeWorkspace")]
    pub active_workspace: Option<WorkspaceRef>,
    #[serde(default, rename = "specialWorkspace")]
    pub special_workspace: Option<SpecialWorkspace>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Workspace {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub monitor: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ActiveWindow {
    #[serde(default)]
    pub address: String,
    #[serde(default, rename = "class")]
    pub app_class: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    pub active_window: ActiveWindow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceDirection {
    Next,
    Previous,
}

/// A narrowly-scoped command emitted by the panel input layer.
///
/// Monitor names and workspace ids are validated before becoming IPC text;
/// this deliberately does not expose an arbitrary command-string API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceCommand {
    Focus {
        monitor_name: String,
        workspace_id: i64,
    },
    Relative {
        monitor_name: String,
        direction: WorkspaceDirection,
    },
    ToggleSpecial {
        monitor_name: String,
    },
}

impl WorkspaceCommand {
    fn request(&self) -> Result<String, IpcError> {
        let monitor_name = match self {
            Self::Focus { monitor_name, .. }
            | Self::Relative { monitor_name, .. }
            | Self::ToggleSpecial { monitor_name } => monitor_name,
        };
        validate_monitor_name(monitor_name)?;

        let action = match self {
            Self::Focus { workspace_id, .. } => {
                if *workspace_id <= 0 {
                    return Err(IpcError::InvalidWorkspaceId(*workspace_id));
                }
                format!("dispatch workspace {workspace_id}")
            }
            Self::Relative { direction, .. } => match direction {
                WorkspaceDirection::Next => "dispatch workspace r+1".to_owned(),
                WorkspaceDirection::Previous => "dispatch workspace r-1".to_owned(),
            },
            Self::ToggleSpecial { .. } => "dispatch togglespecialworkspace special".to_owned(),
        };

        // Focus the panel's output before applying the action. This preserves
        // per-monitor workspace semantics even when a pointer enters a
        // non-focused output's layer surface.
        Ok(format!(
            "[[BATCH]]dispatch focusmonitor {monitor_name};{action}"
        ))
    }
}

fn validate_monitor_name(name: &str) -> Result<(), IpcError> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(IpcError::InvalidMonitorName);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("HYPRLAND_INSTANCE_SIGNATURE is not set")]
    MissingInstance,
    #[error("XDG_RUNTIME_DIR is not set")]
    MissingRuntimeDirectory,
    #[error("invalid Hyprland instance signature")]
    InvalidInstance,
    #[error("Hyprland IPC response exceeds 1 MiB")]
    ResponseTooLarge,
    #[error("Hyprland IPC returned invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Hyprland IPC I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid Hyprland monitor name")]
    InvalidMonitorName,
    #[error("invalid normal workspace id: {0}")]
    InvalidWorkspaceId(i64),
    #[error("Hyprland command response exceeds 16 KiB")]
    CommandResponseTooLarge,
    #[error("Hyprland rejected command: {0}")]
    CommandRejected(String),
}

fn default_scale() -> f64 {
    1.0
}

/// Resolve the per-instance command socket without spawning hyprctl.
pub fn command_socket_path() -> Result<PathBuf, IpcError> {
    instance_socket_path(".socket.sock")
}

/// Resolve Hyprland's broadcast event socket for this compositor instance.
pub fn event_socket_path() -> Result<PathBuf, IpcError> {
    instance_socket_path(".socket2.sock")
}

fn instance_socket_path(socket_name: &str) -> Result<PathBuf, IpcError> {
    let signature =
        env::var("HYPRLAND_INSTANCE_SIGNATURE").map_err(|_| IpcError::MissingInstance)?;
    validate_instance_signature(&signature)?;

    let runtime = env::var_os("XDG_RUNTIME_DIR").ok_or(IpcError::MissingRuntimeDirectory)?;
    Ok(PathBuf::from(runtime)
        .join("hypr")
        .join(signature)
        .join(socket_name))
}

fn validate_instance_signature(signature: &str) -> Result<(), IpcError> {
    if signature.is_empty()
        || signature == "."
        || signature == ".."
        || signature.contains('/')
        || signature.contains('\\')
    {
        return Err(IpcError::InvalidInstance);
    }

    Ok(())
}

pub struct HyprlandIpc {
    socket_path: PathBuf,
}

impl HyprlandIpc {
    pub fn connect_from_environment() -> Result<Self, IpcError> {
        Ok(Self {
            socket_path: command_socket_path()?,
        })
    }

    pub fn from_socket_path(path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: path.into(),
        }
    }

    pub fn monitors(&self) -> Result<Vec<Monitor>, IpcError> {
        self.query_json("j/monitors")
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>, IpcError> {
        self.query_json("j/workspaces")
    }

    pub fn active_window(&self) -> Result<ActiveWindow, IpcError> {
        self.query_json("j/activewindow")
    }

    pub fn snapshot(&self) -> Result<Snapshot, IpcError> {
        Ok(Snapshot {
            monitors: self.monitors()?,
            workspaces: self.workspaces()?,
            active_window: self.active_window()?,
        })
    }

    /// Dispatch one validated workspace action using a fresh, bounded socket.
    /// Hyprland evaluates command-socket connections synchronously, so callers
    /// should use the shell's single command worker rather than a Wayland callback.
    pub fn dispatch_workspace(&self, command: &WorkspaceCommand) -> Result<(), IpcError> {
        let request = command.request()?;
        let mut socket = UnixStream::connect(&self.socket_path)?;
        socket.set_read_timeout(Some(COMMAND_TIMEOUT))?;
        socket.set_write_timeout(Some(COMMAND_TIMEOUT))?;
        socket.write_all(request.as_bytes())?;
        socket.shutdown(std::net::Shutdown::Write)?;

        let mut response = Vec::with_capacity(128);
        socket
            .take((MAX_COMMAND_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut response)?;
        if response.len() > MAX_COMMAND_RESPONSE_BYTES {
            return Err(IpcError::CommandResponseTooLarge);
        }

        let message = String::from_utf8_lossy(&response);
        let replies = message
            .split(BATCH_REPLY_DELIMITER)
            .map(str::trim)
            .filter(|reply| !reply.is_empty());
        let mut saw_response = false;
        for reply in replies {
            saw_response = true;
            if reply != "ok" {
                return Err(IpcError::CommandRejected(reply.to_owned()));
            }
        }
        if !saw_response {
            return Err(IpcError::CommandRejected("empty response".to_owned()));
        }
        Ok(())
    }

    fn query_json<T: for<'de> Deserialize<'de>>(&self, request: &str) -> Result<T, IpcError> {
        let mut socket = UnixStream::connect(&self.socket_path)?;
        socket.set_read_timeout(Some(SOCKET_TIMEOUT))?;
        socket.set_write_timeout(Some(SOCKET_TIMEOUT))?;
        socket.write_all(request.as_bytes())?;
        socket.shutdown(std::net::Shutdown::Write)?;

        let mut response = Vec::with_capacity(4096);
        socket
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut response)?;

        if response.len() > MAX_RESPONSE_BYTES {
            return Err(IpcError::ResponseTooLarge);
        }

        Ok(serde_json::from_slice(&response)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_monitor_workspace_and_active_window_payloads() {
        let monitor: Monitor = serde_json::from_str(
            r#"{
                "id": 0,
                "name": "eDP-1",
                "width": 1920,
                "height": 1080,
                "scale": 1.25,
                "focused": true,
                "activeWorkspace": {"id": 3, "name": "3"},
                "specialWorkspace": {"id": -99, "name": "special:chat"}
            }"#,
        )
        .unwrap();

        assert_eq!(monitor.name, "eDP-1");
        assert_eq!(monitor.scale, 1.25);
        assert!(monitor.focused);
        assert_eq!(monitor.active_workspace.as_ref().unwrap().id, 3);
        assert_eq!(
            monitor.special_workspace.as_ref().unwrap().name,
            "special:chat"
        );

        let workspace: Workspace =
            serde_json::from_str(r#"{"id":3,"name":"3","monitor":"eDP-1"}"#).unwrap();
        assert_eq!(workspace.monitor, "eDP-1");

        let window: ActiveWindow =
            serde_json::from_str(r#"{"address":"0x123","class":"firefox","title":"Docs"}"#)
                .unwrap();
        assert_eq!(window.app_class, "firefox");
        assert_eq!(window.title, "Docs");
    }

    #[test]
    fn tolerates_optional_fields_added_by_newer_hyprland_versions() {
        let monitor: Monitor = serde_json::from_str(
            r#"{"id":1,"name":"DP-1","width":2560,"height":1440,"futureField":true}"#,
        )
        .unwrap();

        assert_eq!(monitor.scale, 1.0);
        assert!(!monitor.focused);
        assert!(monitor.active_workspace.is_none());
        assert!(monitor.special_workspace.is_none());
    }

    #[test]
    fn rejects_invalid_instance_path_segments() {
        assert!(matches!(
            validate_instance_signature("../other"),
            Err(IpcError::InvalidInstance)
        ));
        assert!(matches!(
            validate_instance_signature(""),
            Err(IpcError::InvalidInstance)
        ));
        assert!(validate_instance_signature("valid-signature").is_ok());
    }

    #[test]
    fn renders_monitor_scoped_workspace_commands_as_a_single_batch() {
        let focus = WorkspaceCommand::Focus {
            monitor_name: "eDP-1".into(),
            workspace_id: 4,
        };
        assert_eq!(
            focus.request().unwrap(),
            "[[BATCH]]dispatch focusmonitor eDP-1;dispatch workspace 4"
        );

        let next = WorkspaceCommand::Relative {
            monitor_name: "DP-2".into(),
            direction: WorkspaceDirection::Next,
        };
        assert_eq!(
            next.request().unwrap(),
            "[[BATCH]]dispatch focusmonitor DP-2;dispatch workspace r+1"
        );

        let special = WorkspaceCommand::ToggleSpecial {
            monitor_name: "HDMI-A-1".into(),
        };
        assert_eq!(
            special.request().unwrap(),
            "[[BATCH]]dispatch focusmonitor HDMI-A-1;dispatch togglespecialworkspace special"
        );
    }

    #[test]
    fn validates_monitor_names_and_workspace_ids_before_building_commands() {
        let invalid_monitor = WorkspaceCommand::ToggleSpecial {
            monitor_name: "eDP-1;dispatch exit".into(),
        };
        assert!(matches!(
            invalid_monitor.request(),
            Err(IpcError::InvalidMonitorName)
        ));

        let invalid_workspace = WorkspaceCommand::Focus {
            monitor_name: "eDP-1".into(),
            workspace_id: 0,
        };
        assert!(matches!(
            invalid_workspace.request(),
            Err(IpcError::InvalidWorkspaceId(0))
        ));
    }

    #[test]
    fn sends_one_bounded_command_request_and_accepts_batch_ok_responses() {
        use std::{
            fs,
            os::unix::net::UnixListener,
            sync::atomic::{AtomicU64, Ordering},
            thread,
        };

        static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ii-shell-hyprland-command-{}-{id}.sock",
            std::process::id()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            stream.read_to_string(&mut request).unwrap();
            assert_eq!(
                request,
                "[[BATCH]]dispatch focusmonitor eDP-1;dispatch workspace 2"
            );
            stream.write_all(b"ok\n\n\nok").unwrap();
        });

        let ipc = HyprlandIpc::from_socket_path(&path);
        let command = WorkspaceCommand::Focus {
            monitor_name: "eDP-1".into(),
            workspace_id: 2,
        };
        assert!(ipc.dispatch_workspace(&command).is_ok());
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reports_hyprland_command_rejections() {
        use std::{
            fs,
            os::unix::net::UnixListener,
            sync::atomic::{AtomicU64, Ordering},
            thread,
        };

        static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);
        let id = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ii-shell-hyprland-reject-{}-{id}.sock",
            std::process::id()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            stream.read_to_string(&mut request).unwrap();
            stream.write_all(b"Invalid dispatcher").unwrap();
        });

        let ipc = HyprlandIpc::from_socket_path(&path);
        let command = WorkspaceCommand::ToggleSpecial {
            monitor_name: "eDP-1".into(),
        };
        assert!(matches!(
            ipc.dispatch_workspace(&command),
            Err(IpcError::CommandRejected(message)) if message == "Invalid dispatcher"
        ));
        server.join().unwrap();
        fs::remove_file(path).unwrap();
    }
}

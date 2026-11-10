//! Read-only adapter for Hyprland's per-instance command IPC socket.
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
const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);

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
}

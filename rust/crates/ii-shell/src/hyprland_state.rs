//! Pure reducer from Hyprland broadcast events into the shell's cached state.
//!
//! The reducer performs no socket I/O and never invokes hyprctl. Events that do
//! not carry enough information request a targeted command-socket refresh; the
//! caller can coalesce those requests before querying the compositor.

use crate::{
    hyprland_events::HyprlandEvent,
    hyprland_ipc::{
        ActiveWindow, Monitor, Snapshot, SpecialWorkspace, Workspace, WorkspaceRef,
    },
};

const SPECIAL_WORKSPACE_ID: i64 = -99;

#[derive(Debug, Clone, PartialEq)]
pub struct HyprlandState {
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    pub active_window: ActiveWindow,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StateChange {
    /// A visible or cached value changed, or a refresh was requested.
    pub changed: bool,
    pub refresh_monitors: bool,
    pub refresh_workspaces: bool,
    pub refresh_active_window: bool,
}

impl From<Snapshot> for HyprlandState {
    fn from(snapshot: Snapshot) -> Self {
        Self {
            monitors: snapshot.monitors,
            workspaces: snapshot.workspaces,
            active_window: snapshot.active_window,
        }
    }
}

impl HyprlandState {
    /// Apply one decoded event. The returned flags describe the minimum
    /// command-socket refreshes needed to complete events with partial payloads.
    pub fn apply(&mut self, event: &HyprlandEvent) -> StateChange {
        let mut change = StateChange::default();

        match event {
            HyprlandEvent::Workspace { name } => {
                if let Ok(id) = name.parse::<i64>() {
                    self.activate_on_focused_monitor(id, name, &mut change);
                } else {
                    request_workspaces(&mut change);
                }
            }
            HyprlandEvent::WorkspaceV2 { id, name } => {
                self.activate_on_focused_monitor(*id, name, &mut change);
            }
            HyprlandEvent::FocusedMonitor { monitor, workspace } => {
                let active = workspace.parse::<i64>().ok().map(|id| WorkspaceRef {
                    id,
                    name: workspace.clone(),
                });
                if active.is_none() {
                    request_workspaces(&mut change);
                }
                if !self.focus_monitor(monitor, active) {
                    request_monitors(&mut change);
                } else {
                    change.changed = true;
                }
            }
            HyprlandEvent::FocusedMonitorV2 {
                monitor,
                workspace_id,
            } => {
                let name = self
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == *workspace_id)
                    .map_or_else(|| workspace_id.to_string(), |workspace| workspace.name.clone());
                let active = Some(WorkspaceRef {
                    id: *workspace_id,
                    name,
                });
                if !self.focus_monitor(monitor, active) {
                    request_monitors(&mut change);
                } else {
                    change.changed = true;
                }
            }
            HyprlandEvent::ActiveWindow { app_class, title } => {
                if self.active_window.app_class != *app_class || self.active_window.title != *title {
                    self.active_window.app_class.clone_from(app_class);
                    self.active_window.title.clone_from(title);
                    change.changed = true;
                }
            }
            HyprlandEvent::ActiveWindowV2 { address } => {
                if self.active_window.address != *address {
                    self.active_window.address.clone_from(address);
                    self.active_window.app_class.clear();
                    self.active_window.title.clear();
                    change.changed = true;
                    if !address.is_empty() {
                        change.refresh_active_window = true;
                    }
                }
            }
            HyprlandEvent::WindowTitleV2 { address, title } => {
                if self.active_window.address == *address && self.active_window.title != *title {
                    self.active_window.title.clone_from(title);
                    change.changed = true;
                }
            }
            HyprlandEvent::CloseWindow { address } => {
                if self.active_window.address == *address {
                    self.clear_active_window();
                    change.changed = true;
                }
            }
            HyprlandEvent::MonitorAdded { .. } | HyprlandEvent::MonitorAddedV2 { .. } => {
                request_monitors(&mut change);
                request_workspaces(&mut change);
            }
            HyprlandEvent::MonitorRemoved { name }
            | HyprlandEvent::MonitorRemovedV2 { name, .. } => {
                let before = self.monitors.len();
                self.monitors.retain(|monitor| monitor.name != *name);
                self.workspaces.retain(|workspace| workspace.monitor != *name);
                change.changed = self.monitors.len() != before;
                request_monitors(&mut change);
                request_workspaces(&mut change);
            }
            HyprlandEvent::CreateWorkspace { name } => {
                if let Ok(id) = name.parse::<i64>() {
                    self.create_workspace(id, name, &mut change);
                } else {
                    request_workspaces(&mut change);
                }
            }
            HyprlandEvent::CreateWorkspaceV2 { id, name } => {
                self.create_workspace(*id, name, &mut change);
            }
            HyprlandEvent::DestroyWorkspace { name } => {
                if let Ok(id) = name.parse::<i64>() {
                    change.changed |= self.remove_workspace(id);
                } else {
                    let before = self.workspaces.len();
                    self.workspaces.retain(|workspace| workspace.name != *name);
                    change.changed |= self.workspaces.len() != before;
                }
            }
            HyprlandEvent::DestroyWorkspaceV2 { id, .. } => {
                change.changed |= self.remove_workspace(*id);
            }
            HyprlandEvent::MoveWorkspace { name, monitor } => {
                if let Some(workspace) = self.workspaces.iter_mut().find(|ws| ws.name == *name) {
                    if workspace.monitor != *monitor {
                        workspace.monitor.clone_from(monitor);
                        change.changed = true;
                    }
                } else {
                    request_workspaces(&mut change);
                }
            }
            HyprlandEvent::MoveWorkspaceV2 { id, monitor, .. } => {
                if let Some(workspace) = self.workspaces.iter_mut().find(|ws| ws.id == *id) {
                    if workspace.monitor != *monitor {
                        workspace.monitor.clone_from(monitor);
                        change.changed = true;
                    }
                } else {
                    request_workspaces(&mut change);
                }
            }
            HyprlandEvent::RenameWorkspace { id, name } => {
                if let Some(workspace) = self.workspaces.iter_mut().find(|ws| ws.id == *id) {
                    if workspace.name != *name {
                        workspace.name.clone_from(name);
                        change.changed = true;
                    }
                } else {
                    request_workspaces(&mut change);
                }
                for monitor in &mut self.monitors {
                    if monitor.active_workspace.as_ref().is_some_and(|active| active.id == *id)
                        && monitor.active_workspace.as_ref().is_some_and(|active| active.name != *name)
                    {
                        if let Some(active) = monitor.active_workspace.as_mut() {
                            active.name.clone_from(name);
                        }
                        change.changed = true;
                    }
                }
            }
            HyprlandEvent::ActiveSpecial { workspace, monitor } => {
                if let Some(target) = self.monitors.iter_mut().find(|item| item.name == *monitor) {
                    let next = if workspace.is_empty() {
                        None
                    } else {
                        let name = if workspace.starts_with("special:") {
                            workspace.clone()
                        } else {
                            format!("special:{workspace}")
                        };
                        Some(SpecialWorkspace {
                            id: target
                                .special_workspace
                                .as_ref()
                                .map_or(SPECIAL_WORKSPACE_ID, |current| current.id),
                            name,
                        })
                    };
                    if target.special_workspace != next {
                        target.special_workspace = next;
                        change.changed = true;
                    }
                } else {
                    request_monitors(&mut change);
                }
            }
            HyprlandEvent::ConfigReloaded => {
                request_monitors(&mut change);
                request_workspaces(&mut change);
                change.refresh_active_window = true;
                change.changed = true;
            }
            // These events are either not represented in the current cached
            // model or are informational. Window inventory and layer state are
            // deliberately separate follow-up subsystems.
            HyprlandEvent::Fullscreen { .. }
            | HyprlandEvent::OpenWindow { .. }
            | HyprlandEvent::MoveWindow { .. }
            | HyprlandEvent::MoveWindowV2 { .. }
            | HyprlandEvent::ActiveSpecial { .. }
            | HyprlandEvent::Submap { .. }
            | HyprlandEvent::Unknown { .. } => {}
        }

        change
    }

    fn activate_on_focused_monitor(
        &mut self,
        id: i64,
        name: &str,
        change: &mut StateChange,
    ) {
        let Some(monitor_name) = self
            .monitors
            .iter()
            .find(|monitor| monitor.focused)
            .map(|monitor| monitor.name.clone())
        else {
            request_monitors(change);
            return;
        };

        if self.set_active_workspace(&monitor_name, id, name) {
            change.changed = true;
        }
        change.changed |= self.upsert_workspace(id, name, &monitor_name);
    }

    fn focus_monitor(&mut self, name: &str, active: Option<WorkspaceRef>) -> bool {
        let mut found = false;
        let mut changed = false;
        for monitor in &mut self.monitors {
            let focused = monitor.name == name;
            found |= focused;
            if monitor.focused != focused {
                monitor.focused = focused;
                changed = true;
            }
            if focused {
                if let Some(next) = active.as_ref() {
                    if monitor.active_workspace.as_ref() != Some(next) {
                        monitor.active_workspace = Some(next.clone());
                        changed = true;
                    }
                }
            }
        }
        found && changed || found
    }

    fn set_active_workspace(&mut self, monitor_name: &str, id: i64, name: &str) -> bool {
        let Some(monitor) = self.monitors.iter_mut().find(|item| item.name == monitor_name) else {
            return false;
        };
        let next = WorkspaceRef {
            id,
            name: name.to_owned(),
        };
        if monitor.active_workspace.as_ref() == Some(&next) {
            return false;
        }
        monitor.active_workspace = Some(next);
        true
    }

    fn upsert_workspace(&mut self, id: i64, name: &str, monitor: &str) -> bool {
        if let Some(existing) = self.workspaces.iter_mut().find(|workspace| workspace.id == id) {
            let mut changed = false;
            if existing.name != name {
                existing.name = name.to_owned();
                changed = true;
            }
            if existing.monitor != monitor {
                existing.monitor = monitor.to_owned();
                changed = true;
            }
            changed
        } else {
            self.workspaces.push(Workspace {
                id,
                name: name.to_owned(),
                monitor: monitor.to_owned(),
            });
            true
        }
    }

    fn create_workspace(&mut self, id: i64, name: &str, change: &mut StateChange) {
        let monitor = self
            .monitors
            .iter()
            .find(|item| item.focused)
            .map(|item| item.name.clone());
        if let Some(monitor) = monitor {
            change.changed |= self.upsert_workspace(id, name, &monitor);
        } else {
            request_workspaces(change);
        }
    }

    fn remove_workspace(&mut self, id: i64) -> bool {
        let before = self.workspaces.len();
        self.workspaces.retain(|workspace| workspace.id != id);
        self.workspaces.len() != before
    }

    fn clear_active_window(&mut self) {
        self.active_window.address.clear();
        self.active_window.app_class.clear();
        self.active_window.title.clear();
    }
}

fn request_monitors(change: &mut StateChange) {
    change.changed = true;
    change.refresh_monitors = true;
}

fn request_workspaces(change: &mut StateChange) {
    change.changed = true;
    change.refresh_workspaces = true;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyprland_ipc::{WorkspaceRef, SpecialWorkspace};

    fn monitor(id: i64, name: &str, focused: bool, active: i64) -> Monitor {
        Monitor {
            id,
            name: name.into(),
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused,
            active_workspace: Some(WorkspaceRef {
                id: active,
                name: active.to_string(),
            }),
            special_workspace: None,
        }
    }

    fn state() -> HyprlandState {
        HyprlandState {
            monitors: vec![
                monitor(0, "eDP-1", true, 2),
                monitor(1, "DP-1", false, 7),
            ],
            workspaces: vec![
                Workspace { id: 1, name: "1".into(), monitor: "eDP-1".into() },
                Workspace { id: 2, name: "2".into(), monitor: "eDP-1".into() },
                Workspace { id: 7, name: "7".into(), monitor: "DP-1".into() },
            ],
            active_window: ActiveWindow {
                address: "0xabc".into(),
                app_class: "firefox".into(),
                title: "Docs".into(),
            },
        }
    }

    #[test]
    fn workspace_event_updates_only_the_focused_monitor() {
        let mut state = state();
        let change = state.apply(&HyprlandEvent::WorkspaceV2 { id: 3, name: "3".into() });
        assert!(change.changed);
        assert_eq!(state.monitors[0].active_workspace.as_ref().unwrap().id, 3);
        assert_eq!(state.monitors[1].active_workspace.as_ref().unwrap().id, 7);
        assert!(state.workspaces.iter().any(|workspace| workspace.id == 3 && workspace.monitor == "eDP-1"));
    }

    #[test]
    fn focused_monitor_event_updates_focus_and_active_workspace() {
        let mut state = state();
        let change = state.apply(&HyprlandEvent::FocusedMonitorV2 {
            monitor: "DP-1".into(),
            workspace_id: 8,
        });
        assert!(change.changed);
        assert!(!state.monitors[0].focused);
        assert!(state.monitors[1].focused);
        assert_eq!(state.monitors[1].active_workspace.as_ref().unwrap().id, 8);
    }

    #[test]
    fn create_move_rename_and_destroy_workspace_events_are_applied() {
        let mut state = state();
        state.apply(&HyprlandEvent::CreateWorkspaceV2 { id: 3, name: "three".into() });
        assert!(state.workspaces.iter().any(|workspace| workspace.id == 3));
        state.apply(&HyprlandEvent::MoveWorkspaceV2 {
            id: 3,
            name: "three".into(),
            monitor: "DP-1".into(),
        });
        assert_eq!(state.workspaces.iter().find(|workspace| workspace.id == 3).unwrap().monitor, "DP-1");
        state.apply(&HyprlandEvent::RenameWorkspace { id: 3, name: "code".into() });
        assert_eq!(state.workspaces.iter().find(|workspace| workspace.id == 3).unwrap().name, "code");
        let change = state.apply(&HyprlandEvent::DestroyWorkspaceV2 { id: 3, name: "code".into() });
        assert!(change.changed);
        assert!(!state.workspaces.iter().any(|workspace| workspace.id == 3));
    }

    #[test]
    fn active_window_events_update_only_matching_window_metadata() {
        let mut state = state();
        let title = state.apply(&HyprlandEvent::WindowTitleV2 {
            address: "0xabc".into(),
            title: "New title".into(),
        });
        assert!(title.changed);
        assert_eq!(state.active_window.title, "New title");

        let stale = state.apply(&HyprlandEvent::WindowTitleV2 {
            address: "0xdef".into(),
            title: "Other window".into(),
        });
        assert!(!stale.changed);
        assert_eq!(state.active_window.title, "New title");

        let focus = state.apply(&HyprlandEvent::ActiveWindowV2 { address: "0xdef".into() });
        assert!(focus.refresh_active_window);
        assert!(state.active_window.title.is_empty());
    }

    #[test]
    fn closing_the_active_window_clears_its_cached_metadata() {
        let mut state = state();
        let change = state.apply(&HyprlandEvent::CloseWindow { address: "0xabc".into() });
        assert!(change.changed);
        assert!(state.active_window.address.is_empty());
        assert!(state.active_window.app_class.is_empty());
    }

    #[test]
    fn special_workspace_events_are_monitor_scoped() {
        let mut state = state();
        let change = state.apply(&HyprlandEvent::ActiveSpecial {
            workspace: "chat".into(),
            monitor: "DP-1".into(),
        });
        assert!(change.changed);
        assert_eq!(
            state.monitors[1].special_workspace.as_ref().unwrap().name,
            "special:chat"
        );
        state.apply(&HyprlandEvent::ActiveSpecial {
            workspace: String::new(),
            monitor: "DP-1".into(),
        });
        assert!(state.monitors[1].special_workspace.is_none());
    }

    #[test]
    fn monitor_hotplug_requests_authoritative_refresh_and_removal_is_immediate() {
        let mut state = state();
        let added = state.apply(&HyprlandEvent::MonitorAdded { name: "HDMI-A-1".into() });
        assert!(added.refresh_monitors);
        assert!(added.refresh_workspaces);

        let removed = state.apply(&HyprlandEvent::MonitorRemoved { name: "DP-1".into() });
        assert!(removed.refresh_monitors);
        assert!(!state.monitors.iter().any(|monitor| monitor.name == "DP-1"));
        assert!(!state.workspaces.iter().any(|workspace| workspace.monitor == "DP-1"));
    }

    #[test]
    fn events_outside_the_current_state_model_do_not_trigger_redraws() {
        let mut state = state();
        let change = state.apply(&HyprlandEvent::Unknown {
            name: "future".into(),
            payload: "payload".into(),
        });
        assert_eq!(change, StateChange::default());
    }

    #[test]
    fn config_reload_requests_a_coalescible_refresh_of_all_cached_domains() {
        let mut state = state();
        let change = state.apply(&HyprlandEvent::ConfigReloaded);
        assert!(change.refresh_monitors);
        assert!(change.refresh_workspaces);
        assert!(change.refresh_active_window);
    }
}

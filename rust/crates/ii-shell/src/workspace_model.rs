//! Pure workspace projection used by each monitor's bar.
//!
//! This mirrors the data rules in upstream WorkspaceModel.qml while keeping
//! rendering and Hyprland socket I/O outside the model.

use crate::hyprland_ipc::{Monitor, Workspace};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSlot {
    pub id: i64,
    pub occupied: bool,
    pub is_fake: bool,
}

impl WorkspaceSlot {
    /// Upstream suppresses the occupied pill for the fake empty active workspace.
    pub fn visibly_occupied(&self) -> bool {
        self.occupied && !self.is_fake
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceModel {
    pub monitor_name: String,
    pub active_workspace_id: i64,
    pub group: i64,
    pub active_index: usize,
    pub fake_workspace_id: i64,
    pub slots: Vec<WorkspaceSlot>,
    pub special_workspace_name: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkspaceModelError {
    #[error("shown workspace count must be greater than zero")]
    ZeroShownCount,
    #[error("shown workspace count is too large")]
    ShownCountTooLarge,
    #[error("workspace id arithmetic overflowed")]
    WorkspaceIdOverflow,
}

/// Build one monitor-specific group of workspace slots.
///
/// current_workspace_not_fake corresponds to the upstream active Wayland
/// toplevel's activated property. It is explicit because that property is
/// supplied by the Wayland toplevel model, not Hyprland's activewindow JSON.
impl WorkspaceModel {
    pub fn from_monitor(
        monitor: &Monitor,
        workspaces: &[Workspace],
        shown_count: usize,
        current_workspace_not_fake: bool,
    ) -> Result<Self, WorkspaceModelError> {
        if shown_count == 0 {
            return Err(WorkspaceModelError::ZeroShownCount);
        }

        let shown_count_i64 =
            i64::try_from(shown_count).map_err(|_| WorkspaceModelError::ShownCountTooLarge)?;
        let active_workspace_id = monitor
            .active_workspace
            .as_ref()
            .map_or(1, |workspace| workspace.id);
        let zero_based_active = active_workspace_id
            .checked_sub(1)
            .ok_or(WorkspaceModelError::WorkspaceIdOverflow)?;
        let group = zero_based_active.div_euclid(shown_count_i64);
        let first_workspace_id = group
            .checked_mul(shown_count_i64)
            .and_then(|first| first.checked_add(1))
            .ok_or(WorkspaceModelError::WorkspaceIdOverflow)?;
        let active_index = usize::try_from(
            active_workspace_id
                .checked_sub(first_workspace_id)
                .ok_or(WorkspaceModelError::WorkspaceIdOverflow)?,
        )
        .map_err(|_| WorkspaceModelError::WorkspaceIdOverflow)?;
        let fake_workspace_id = if current_workspace_not_fake {
            -9999
        } else {
            active_workspace_id
        };

        let mut slots = Vec::with_capacity(shown_count);
        for index in 0..shown_count {
            let offset =
                i64::try_from(index).map_err(|_| WorkspaceModelError::ShownCountTooLarge)?;
            let id = first_workspace_id
                .checked_add(offset)
                .ok_or(WorkspaceModelError::WorkspaceIdOverflow)?;
            slots.push(WorkspaceSlot {
                id,
                occupied: workspaces.iter().any(|workspace| workspace.id == id),
                is_fake: id == fake_workspace_id,
            });
        }

        let special_workspace_name = monitor
            .special_workspace
            .as_ref()
            .map(|workspace| {
                workspace
                    .name
                    .strip_prefix("special:")
                    .unwrap_or(&workspace.name)
                    .to_owned()
            })
            .filter(|name| !name.is_empty());

        Ok(Self {
            monitor_name: monitor.name.clone(),
            active_workspace_id,
            group,
            active_index,
            fake_workspace_id,
            slots,
            special_workspace_name,
        })
    }

    pub fn workspace_id_at(&self, index: usize) -> Option<i64> {
        self.slots.get(index).map(|slot| slot.id)
    }

    pub fn special_workspace_active(&self) -> bool {
        self.special_workspace_name.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyprland_ipc::{SpecialWorkspace, WorkspaceRef};

    fn monitor(active_workspace_id: i64) -> Monitor {
        Monitor {
            id: 0,
            name: "eDP-1".into(),
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused: true,
            active_workspace: Some(WorkspaceRef {
                id: active_workspace_id,
                name: active_workspace_id.to_string(),
            }),
            special_workspace: Some(SpecialWorkspace {
                id: -99,
                name: "special:chat".into(),
            }),
        }
    }

    fn workspace(id: i64) -> Workspace {
        Workspace {
            id,
            name: id.to_string(),
            monitor: "eDP-1".into(),
        }
    }

    #[test]
    fn groups_workspaces_around_the_active_workspace() {
        let model = WorkspaceModel::from_monitor(
            &monitor(5),
            &[workspace(5), workspace(7), workspace(12)],
            4,
            true,
        )
        .unwrap();

        assert_eq!(model.group, 1);
        assert_eq!(model.active_index, 0);
        assert_eq!(
            model.slots.iter().map(|slot| slot.id).collect::<Vec<_>>(),
            vec![5, 6, 7, 8]
        );
        assert_eq!(model.workspace_id_at(2), Some(7));
        assert_eq!(model.workspace_id_at(4), None);
    }

    #[test]
    fn derives_occupancy_and_fake_active_workspace_state() {
        let model =
            WorkspaceModel::from_monitor(&monitor(5), &[workspace(5), workspace(7)], 4, false)
                .unwrap();

        assert!(model.slots[0].occupied);
        assert!(model.slots[0].is_fake);
        assert!(!model.slots[0].visibly_occupied());
        assert!(model.slots[2].visibly_occupied());
        assert!(!model.slots[1].occupied);
        assert_eq!(model.fake_workspace_id, 5);
    }

    #[test]
    fn active_toplevel_disables_fake_workspace_suppression() {
        let model = WorkspaceModel::from_monitor(&monitor(5), &[workspace(5)], 4, true).unwrap();

        assert!(!model.slots[0].is_fake);
        assert!(model.slots[0].visibly_occupied());
        assert_eq!(model.fake_workspace_id, -9999);
    }

    #[test]
    fn exposes_the_monitor_special_workspace_name() {
        let model = WorkspaceModel::from_monitor(&monitor(1), &[], 5, false).unwrap();

        assert_eq!(model.monitor_name, "eDP-1");
        assert_eq!(model.special_workspace_name.as_deref(), Some("chat"));
        assert!(model.special_workspace_active());
    }

    #[test]
    fn rejects_a_zero_workspace_count() {
        assert_eq!(
            WorkspaceModel::from_monitor(&monitor(1), &[], 0, false),
            Err(WorkspaceModelError::ZeroShownCount)
        );
    }
}

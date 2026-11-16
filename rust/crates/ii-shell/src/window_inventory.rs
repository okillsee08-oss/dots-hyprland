//! Largest-client selection matching upstream HyprlandData.qml.
//!
//! The QML reference filters clients by workspace id and keeps the first
//! largest-area client. This projection is deliberately pure: it performs no
//! IPC and does not retain a second copy of the client list.

use crate::hyprland_ipc::Client;

/// Return the largest-area client on a workspace.
///
/// A strict greater-than comparison preserves the first client on equal-area
/// ties, matching JavaScript's reduce implementation in HyprlandData.qml.
/// Missing or non-positive geometry has zero area and cannot win.
pub fn biggest_window_for_workspace(clients: &[Client], workspace_id: i64) -> Option<&Client> {
    let mut biggest = None;
    let mut biggest_area = 0_u128;

    for client in clients {
        if !client
            .workspace
            .as_ref()
            .is_some_and(|workspace| workspace.id == workspace_id)
        {
            continue;
        }

        let area = client_area(client);
        if area > biggest_area {
            biggest = Some(client);
            biggest_area = area;
        }
    }

    biggest
}

fn client_area(client: &Client) -> u128 {
    let width = u128::try_from(client.size[0]).unwrap_or_default();
    let height = u128::try_from(client.size[1]).unwrap_or_default();
    width.saturating_mul(height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyprland_ipc::WorkspaceRef;

    fn client(address: &str, workspace_id: Option<i64>, size: [i64; 2]) -> Client {
        Client {
            address: address.into(),
            app_class: "app".into(),
            title: "window".into(),
            size,
            workspace: workspace_id.map(|id| WorkspaceRef {
                id,
                name: id.to_string(),
            }),
        }
    }

    #[test]
    fn selects_the_largest_window_only_within_the_requested_workspace() {
        let clients = vec![
            client("0x1", Some(1), [800, 600]),
            client("0x2", Some(1), [1200, 900]),
            client("0x3", Some(2), [1920, 1080]),
        ];

        assert_eq!(
            biggest_window_for_workspace(&clients, 1).map(|window| window.address.as_str()),
            Some("0x2")
        );
        assert_eq!(
            biggest_window_for_workspace(&clients, 2).map(|window| window.address.as_str()),
            Some("0x3")
        );
        assert!(biggest_window_for_workspace(&clients, 3).is_none());
    }

    #[test]
    fn equal_area_ties_keep_the_first_client_like_upstream_reduce() {
        let clients = vec![
            client("0x1", Some(4), [1000, 800]),
            client("0x2", Some(4), [800, 1000]),
        ];

        assert_eq!(
            biggest_window_for_workspace(&clients, 4).map(|window| window.address.as_str()),
            Some("0x1")
        );
    }

    #[test]
    fn missing_or_non_positive_geometry_never_becomes_the_biggest_window() {
        let clients = vec![
            client("0x1", Some(5), [0, 800]),
            client("0x2", Some(5), [-100, 900]),
            client("0x3", None, [1920, 1080]),
        ];

        assert!(biggest_window_for_workspace(&clients, 5).is_none());
    }

    #[test]
    fn very_large_geometry_uses_wide_area_arithmetic() {
        let clients = vec![client("0x1", Some(8), [i64::MAX, i64::MAX])];

        assert!(biggest_window_for_workspace(&clients, 8).is_some());
    }
}

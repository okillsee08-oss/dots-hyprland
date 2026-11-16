//! Per-output active-window and largest-workspace-window label projection.
//!
//! Upstream ActiveWindow.qml uses the active toplevel on the focused monitor
//! and the largest client in each other monitor's active workspace.

use std::borrow::Cow;

use crate::hyprland_ipc::{ActiveWindow, Client};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPresentation<'a> {
    pub app_name: Cow<'a, str>,
    pub title: Cow<'a, str>,
}

pub fn for_monitor<'a>(
    active_window: &'a ActiveWindow,
    biggest_window: Option<&'a Client>,
    monitor_focused: bool,
    workspace_id: i64,
) -> WindowPresentation<'a> {
    let workspace_title = || Cow::Owned(format!("Workspace {workspace_id}"));

    if monitor_focused && biggest_window.is_some() && !active_window.app_class.trim().is_empty() {
        WindowPresentation {
            app_name: Cow::Borrowed(&active_window.app_class),
            title: if active_window.title.trim().is_empty() {
                workspace_title()
            } else {
                Cow::Borrowed(&active_window.title)
            },
        }
    } else if let Some(window) = biggest_window {
        WindowPresentation {
            app_name: if window.app_class.trim().is_empty() {
                Cow::Borrowed("Desktop")
            } else {
                Cow::Borrowed(&window.app_class)
            },
            title: if window.title.trim().is_empty() {
                workspace_title()
            } else {
                Cow::Borrowed(&window.title)
            },
        }
    } else {
        WindowPresentation {
            app_name: Cow::Borrowed("Desktop"),
            title: workspace_title(),
        }
    }
}

/// Limit labels by Unicode scalar values and use one ellipsis when truncated.
/// This avoids slicing UTF-8 strings at arbitrary byte offsets.
pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();

    if chars.next().is_some() && max_chars > 0 {
        let mut truncated: String = prefix.chars().take(max_chars - 1).collect();
        truncated.push('…');
        truncated
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyprland_ipc::WorkspaceRef;

    fn active_window(app_class: &str, title: &str) -> ActiveWindow {
        ActiveWindow {
            address: "0x123".into(),
            app_class: app_class.into(),
            title: title.into(),
        }
    }

    fn client(app_class: &str, title: &str) -> Client {
        Client {
            address: "0x123".into(),
            app_class: app_class.into(),
            title: title.into(),
            size: [1200, 800],
            workspace: Some(WorkspaceRef {
                id: 4,
                name: "4".into(),
            }),
        }
    }

    #[test]
    fn focused_monitor_shows_the_active_window_when_its_workspace_has_a_client() {
        let active = active_window("firefox", "Project notes");
        let biggest = client("terminal", "Shell");
        let presentation = for_monitor(&active, Some(&biggest), true, 4);

        assert_eq!(presentation.app_name, "firefox");
        assert_eq!(presentation.title, "Project notes");
    }

    #[test]
    fn unfocused_monitor_uses_its_largest_workspace_window() {
        let active = active_window("firefox", "Focused monitor");
        let biggest = client("code", "src/main.rs");
        let presentation = for_monitor(&active, Some(&biggest), false, 4);

        assert_eq!(presentation.app_name, "code");
        assert_eq!(presentation.title, "src/main.rs");
    }

    #[test]
    fn focused_monitor_falls_back_to_the_biggest_client_if_active_metadata_is_missing() {
        let active = active_window("  ", "");
        let biggest = client("terminal", "Shell");
        let presentation = for_monitor(&active, Some(&biggest), true, 4);

        assert_eq!(presentation.app_name, "terminal");
        assert_eq!(presentation.title, "Shell");
    }

    #[test]
    fn missing_client_inventory_uses_desktop_and_workspace_fallback() {
        let active = active_window("firefox", "Project notes");
        let presentation = for_monitor(&active, None, true, 3);

        assert_eq!(presentation.app_name, "Desktop");
        assert_eq!(presentation.title, "Workspace 3");
    }

    #[test]
    fn missing_largest_window_title_uses_the_workspace_label() {
        let active = active_window("terminal", "");
        let biggest = client("terminal", "");
        let presentation = for_monitor(&active, Some(&biggest), true, 2);

        assert_eq!(presentation.app_name, "terminal");
        assert_eq!(presentation.title, "Workspace 2");
    }

    #[test]
    fn truncation_is_utf8_safe_and_reserves_one_character_for_the_ellipsis() {
        assert_eq!(truncate_chars("Firefox", 7), "Firefox");
        assert_eq!(truncate_chars("Firefox", 5), "Fire…");
        assert_eq!(truncate_chars("Firefox", 1), "…");
        assert_eq!(truncate_chars("Firefox", 0), "");
        assert_eq!(truncate_chars("日本語とEnglish", 4), "日本語…");
    }
}

//! Small per-output active-window label projection for the native top bar.
//!
//! Upstream ActiveWindow.qml shows the active toplevel's app id and title on the
//! focused monitor. Other monitors fall back to the largest window in their
//! active workspace; until the Rust client inventory is ported, those outputs
//! intentionally use the existing Desktop/Workspace fallback.

use std::borrow::Cow;

use crate::hyprland_ipc::ActiveWindow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPresentation<'a> {
    pub app_name: Cow<'a, str>,
    pub title: Cow<'a, str>,
}

pub fn for_monitor(
    active_window: &ActiveWindow,
    monitor_focused: bool,
    workspace_id: i64,
) -> WindowPresentation<'_> {
    let workspace_title = || Cow::Owned(format!("Workspace {workspace_id}"));

    if monitor_focused && !active_window.app_class.trim().is_empty() {
        WindowPresentation {
            app_name: Cow::Borrowed(&active_window.app_class),
            title: if active_window.title.trim().is_empty() {
                workspace_title()
            } else {
                Cow::Borrowed(&active_window.title)
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

    fn active_window(app_class: &str, title: &str) -> ActiveWindow {
        ActiveWindow {
            address: "0x123".into(),
            app_class: app_class.into(),
            title: title.into(),
        }
    }

    #[test]
    fn focused_monitor_shows_the_active_application_and_title() {
        let presentation = for_monitor(&active_window("firefox", "Project notes"), true, 4);

        assert_eq!(presentation.app_name, "firefox");
        assert_eq!(presentation.title, "Project notes");
    }

    #[test]
    fn unfocused_monitor_uses_desktop_and_its_own_workspace_fallback() {
        let presentation = for_monitor(&active_window("firefox", "Project notes"), false, 12);

        assert_eq!(presentation.app_name, "Desktop");
        assert_eq!(presentation.title, "Workspace 12");
    }

    #[test]
    fn missing_active_metadata_falls_back_without_an_empty_label() {
        let presentation = for_monitor(&active_window("  ", ""), true, 3);

        assert_eq!(presentation.app_name, "Desktop");
        assert_eq!(presentation.title, "Workspace 3");
    }

    #[test]
    fn title_falls_back_when_the_active_class_is_present_but_title_is_empty() {
        let presentation = for_monitor(&active_window("terminal", ""), true, 2);

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

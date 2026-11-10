use std::{env, fs, path::PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    pub panel_height: u32,
    pub panel_background: String,
    pub accent: String,
    pub font_family: String,
    pub font_size: f32,
    pub panel_visible: bool,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            panel_height: 36,
            panel_background: "#141313".into(),
            accent: "#B2535D".into(),
            font_family: "Google Sans Flex".into(),
            font_size: 14.0,
            panel_visible: true,
        }
    }
}

impl ShellConfig {
    pub fn from_json(input: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(input)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}


/// The workspace-strip settings read from the existing upstream Quickshell
/// configuration at `~/.config/illogical-impulse/config.json`.
///
/// This type intentionally reads only the relevant field and leaves all
/// other Quickshell settings, comments, scripts and user-owned config alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceBarConfig {
    pub shown: usize,
}

impl Default for WorkspaceBarConfig {
    fn default() -> Self {
        Self { shown: 10 }
    }
}

impl WorkspaceBarConfig {
    pub const MAX_SHOWN: usize = 100;

    /// Parse the same nested JSON key exposed by upstream `Config.qml`:
    /// `options.bar.workspaces.shown`.
    pub fn from_upstream_json(input: &str) -> Self {
        let Ok(root) = serde_json::from_str::<serde_json::Value>(input) else {
            return Self::default();
        };
        let Some(shown) = root
            .pointer("/bar/workspaces/shown")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (1..=Self::MAX_SHOWN).contains(value))
        else {
            return Self::default();
        };

        Self { shown }
    }

    /// Load workspace settings from the existing upstream XDG config path.
    /// Missing, unreadable or malformed files fall back to the upstream
    /// default of ten workspaces; this function never writes a file.
    pub fn load() -> Self {
        let config_home = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
        let Some(config_home) = config_home else {
            return Self::default();
        };

        Self::load_from_path(&config_home.join("illogical-impulse/config.json"))
    }

    pub fn load_from_path(path: &std::path::Path) -> Self {
        fs::read_to_string(path)
            .map(|contents| Self::from_upstream_json(&contents))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_fields_use_low_overhead_defaults() {
        let config = ShellConfig::from_json("{}").expect("empty config is valid");
        assert_eq!(config.panel_height, 36);
        assert!(config.panel_visible);
    }

    #[test]
    fn reads_the_existing_upstream_workspace_count_setting() {
        let config = WorkspaceBarConfig::from_upstream_json(
            r#"{"bar":{"workspaces":{"shown":7,"showAppIcons":true}}}"#,
        );
        assert_eq!(config.shown, 7);
    }

    #[test]
    fn missing_or_invalid_workspace_counts_use_the_upstream_default() {
        for input in [
            "{}",
            r#"{"bar":{}}"#,
            r#"{"bar":{"workspaces":{"shown":0}}}"#,
            r#"{"bar":{"workspaces":{"shown":101}}}"#,
            r#"{"bar":{"workspaces":{"shown":"8"}}}"#,
            "not-json",
        ] {
            assert_eq!(WorkspaceBarConfig::from_upstream_json(input).shown, 10);
        }
    }

    #[test]
    fn config_round_trips() {
        let config = ShellConfig::default();
        let encoded = config.to_json().expect("serialize config");
        let decoded = ShellConfig::from_json(&encoded).expect("deserialize config");
        assert_eq!(decoded.panel_height, config.panel_height);
        assert_eq!(decoded.accent, config.accent);
    }
}

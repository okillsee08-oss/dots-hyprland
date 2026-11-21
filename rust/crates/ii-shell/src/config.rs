use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::battery::BatteryThresholds;

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
    pub show_app_icons: bool,
}

impl Default for WorkspaceBarConfig {
    fn default() -> Self {
        Self {
            shown: 10,
            show_app_icons: true,
        }
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
        let shown = root
            .pointer("/bar/workspaces/shown")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| (1..=Self::MAX_SHOWN).contains(value))
            .unwrap_or(10);
        let show_app_icons = root
            .pointer("/bar/workspaces/showAppIcons")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);

        Self {
            shown,
            show_app_icons,
        }
    }

    /// Load workspace settings from the existing upstream XDG config path.
    /// Missing, unreadable or malformed files fall back to the upstream
    /// default of ten workspaces; this function never writes a file.
    pub fn load() -> Self {
        let Some(path) = upstream_config_path() else {
            return Self::default();
        };
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &std::path::Path) -> Self {
        fs::read_to_string(path)
            .map(|contents| Self::from_upstream_json(&contents))
            .unwrap_or_default()
    }
}

/// Battery options read from the existing upstream Illogical Impulse JSON.
/// Invalid or out-of-range values fall back independently to upstream defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatteryConfig {
    pub thresholds: BatteryThresholds,
}

impl BatteryConfig {
    /// Parse the root-level battery object used by upstream Config.qml.
    pub fn from_upstream_json(input: &str) -> Self {
        let Ok(root) = serde_json::from_str::<serde_json::Value>(input) else {
            return Self::default();
        };
        let defaults = BatteryThresholds::default();
        Self {
            thresholds: BatteryThresholds {
                low_percent: battery_percent(&root, "/battery/low", defaults.low_percent, 100),
                critical_percent: battery_percent(
                    &root,
                    "/battery/critical",
                    defaults.critical_percent,
                    100,
                ),
                suspend_percent: battery_percent(
                    &root,
                    "/battery/suspend",
                    defaults.suspend_percent,
                    100,
                ),
                full_percent: battery_percent(&root, "/battery/full", defaults.full_percent, 101),
                automatic_suspend: root
                    .pointer("/battery/automaticSuspend")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(defaults.automatic_suspend),
            },
        }
    }

    /// Load battery options from the existing upstream config path.
    /// This function never creates or modifies user-owned configuration.
    pub fn load() -> Self {
        let Some(path) = upstream_config_path() else {
            return Self::default();
        };
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> Self {
        fs::read_to_string(path)
            .map(|contents| Self::from_upstream_json(&contents))
            .unwrap_or_default()
    }
}

fn battery_percent(root: &serde_json::Value, pointer: &str, fallback: u8, maximum: u8) -> u8 {
    root.pointer(pointer)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .filter(|value| *value <= maximum)
        .unwrap_or(fallback)
}

fn upstream_config_path() -> Option<PathBuf> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config_home.join("illogical-impulse/config.json"))
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
        assert!(config.show_app_icons);
    }

    #[test]
    fn reads_the_upstream_app_icon_visibility_toggle() {
        let enabled = WorkspaceBarConfig::from_upstream_json(
            r#"{"bar":{"workspaces":{"shown":6,"showAppIcons":true}}}"#,
        );
        let disabled = WorkspaceBarConfig::from_upstream_json(
            r#"{"bar":{"workspaces":{"shown":6,"showAppIcons":false}}}"#,
        );
        assert!(enabled.show_app_icons);
        assert!(!disabled.show_app_icons);
        assert_eq!(disabled.shown, 6);
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
    fn reads_all_upstream_battery_thresholds_and_suspend_toggle() {
        let config = BatteryConfig::from_upstream_json(
            r#"{"battery":{"low":25,"critical":10,"suspend":7,"full":95,"automaticSuspend":false}}"#,
        );

        assert_eq!(
            config.thresholds,
            BatteryThresholds {
                low_percent: 25,
                critical_percent: 10,
                suspend_percent: 7,
                full_percent: 95,
                automatic_suspend: false,
            }
        );
    }

    #[test]
    fn accepts_upstream_battery_boundary_values() {
        let config = BatteryConfig::from_upstream_json(
            r#"{"battery":{"low":0,"critical":100,"suspend":0,"full":101,"automaticSuspend":false}}"#,
        );
        assert_eq!(config.thresholds.low_percent, 0);
        assert_eq!(config.thresholds.critical_percent, 100);
        assert_eq!(config.thresholds.suspend_percent, 0);
        assert_eq!(config.thresholds.full_percent, 101);
        assert!(!config.thresholds.automatic_suspend);
    }

    #[test]
    fn invalid_battery_fields_fall_back_to_upstream_defaults() {
        let config = BatteryConfig::from_upstream_json(
            r#"{"battery":{"low":101,"critical":-1,"suspend":"3","full":102,"automaticSuspend":"false"}}"#,
        );
        assert_eq!(config, BatteryConfig::default());
    }

    #[test]
    fn partial_or_malformed_battery_config_uses_safe_defaults() {
        let partial = BatteryConfig::from_upstream_json(r#"{"battery":{"low":15}}"#);
        assert_eq!(partial.thresholds.low_percent, 15);
        assert_eq!(partial.thresholds.critical_percent, 5);
        assert_eq!(partial.thresholds.suspend_percent, 3);
        assert_eq!(partial.thresholds.full_percent, 101);
        assert!(partial.thresholds.automatic_suspend);
        assert_eq!(
            BatteryConfig::from_upstream_json("not-json"),
            BatteryConfig::default()
        );
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

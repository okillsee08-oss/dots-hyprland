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
    fn config_round_trips() {
        let config = ShellConfig::default();
        let encoded = config.to_json().expect("serialize config");
        let decoded = ShellConfig::from_json(&encoded).expect("deserialize config");
        assert_eq!(decoded.panel_height, config.panel_height);
        assert_eq!(decoded.accent, config.accent);
    }
}

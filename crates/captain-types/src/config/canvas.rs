use serde::{Deserialize, Serialize};

/// Canvas (Agent-to-UI) configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CanvasConfig {
    /// Enable canvas tool. Default: false.
    pub enabled: bool,
    /// Max HTML size in bytes. Default: 512KB.
    pub max_html_bytes: usize,
    /// Allowed HTML tags (empty = all safe tags allowed).
    #[serde(default)]
    pub allowed_tags: Vec<String>,
}

impl Default for CanvasConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_html_bytes: 512 * 1024,
            allowed_tags: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CanvasConfig;
    use crate::config::KernelConfig;

    #[test]
    fn canvas_defaults_keep_tool_disabled_with_size_limit() {
        let config = CanvasConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.max_html_bytes, 512 * 1024);
        assert!(config.allowed_tags.is_empty());
    }

    #[test]
    fn canvas_config_roundtrips_allowed_tags() {
        let config = CanvasConfig {
            enabled: true,
            max_html_bytes: 4096,
            allowed_tags: vec!["section".to_string(), "table".to_string()],
        };

        let encoded = toml::to_string(&config).unwrap();
        let decoded: CanvasConfig = toml::from_str(&encoded).unwrap();

        assert!(decoded.enabled);
        assert_eq!(decoded.max_html_bytes, 4096);
        assert_eq!(decoded.allowed_tags, vec!["section", "table"]);
    }

    #[test]
    fn canvas_section_deserializes_from_kernel_toml() {
        let config: KernelConfig = toml::from_str(
            r#"
            [canvas]
            enabled = true
            max_html_bytes = 8192
            allowed_tags = ["main", "article"]
            "#,
        )
        .unwrap();

        assert!(config.canvas.enabled);
        assert_eq!(config.canvas.max_html_bytes, 8192);
        assert_eq!(config.canvas.allowed_tags, vec!["main", "article"]);
    }
}

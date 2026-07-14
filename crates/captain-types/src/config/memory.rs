use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Which backend handles `memory_store` / `memory_recall` tool calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryBackend {
    /// Built-in graph memory (hora-graph-core + SQLite KV).
    Graph,
    /// External MemPalace MCP server (structured KG, taxonomy, diary).
    #[default]
    Mempalace,
}

/// Memory substrate configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    /// Which backend to use for memory_store/recall. Default: mempalace.
    #[serde(default)]
    pub backend: MemoryBackend,
    /// Path to SQLite database file.
    pub sqlite_path: Option<PathBuf>,
    /// Embedding model for semantic search.
    pub embedding_model: String,
    /// Maximum memories before consolidation is triggered.
    pub consolidation_threshold: u64,
    /// Memory decay rate (0.0 = no decay, 1.0 = aggressive decay).
    pub decay_rate: f32,
    /// Embedding provider (e.g., "openai", "ollama"). None = auto-detect.
    #[serde(default)]
    pub embedding_provider: Option<String>,
    /// Environment variable name for the embedding API key.
    #[serde(default)]
    pub embedding_api_key_env: Option<String>,
    /// How often to run memory consolidation (hours). 0 = disabled.
    #[serde(default = "default_consolidation_interval")]
    pub consolidation_interval_hours: u64,
}

fn default_consolidation_interval() -> u64 {
    24
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: MemoryBackend::default(),
            sqlite_path: None,
            embedding_model: "all-MiniLM-L6-v2".to_string(),
            consolidation_threshold: 10_000,
            decay_rate: 0.1,
            embedding_provider: None,
            embedding_api_key_env: None,
            consolidation_interval_hours: default_consolidation_interval(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_config_defaults_keep_mempalace_and_local_embedding() {
        let config = MemoryConfig::default();

        assert_eq!(config.backend, MemoryBackend::Mempalace);
        assert!(config.sqlite_path.is_none());
        assert_eq!(config.embedding_model, "all-MiniLM-L6-v2");
        assert_eq!(config.consolidation_threshold, 10_000);
        assert_eq!(config.decay_rate, 0.1);
        assert!(config.embedding_provider.is_none());
        assert!(config.embedding_api_key_env.is_none());
        assert_eq!(config.consolidation_interval_hours, 24);
    }

    #[test]
    fn memory_backend_serde_accepts_lowercase_values() {
        let graph: MemoryBackend = toml::from_str("backend = \"graph\"")
            .map(|wrapper: BackendWrapper| wrapper.backend)
            .unwrap();
        let mempalace: MemoryBackend = toml::from_str("backend = \"mempalace\"")
            .map(|wrapper: BackendWrapper| wrapper.backend)
            .unwrap();

        assert_eq!(graph, MemoryBackend::Graph);
        assert_eq!(mempalace, MemoryBackend::Mempalace);
    }

    #[test]
    fn memory_config_deserializes_partial_toml_with_defaults() {
        let config: MemoryConfig = toml::from_str(
            r#"
            backend = "graph"
            embedding_provider = "openai"
            embedding_api_key_env = "OPENAI_API_KEY"
            "#,
        )
        .unwrap();

        assert_eq!(config.backend, MemoryBackend::Graph);
        assert!(config.sqlite_path.is_none());
        assert_eq!(config.embedding_model, "all-MiniLM-L6-v2");
        assert_eq!(config.consolidation_threshold, 10_000);
        assert_eq!(config.decay_rate, 0.1);
        assert_eq!(config.embedding_provider.as_deref(), Some("openai"));
        assert_eq!(
            config.embedding_api_key_env.as_deref(),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(config.consolidation_interval_hours, 24);
    }

    #[test]
    fn memory_config_roundtrips_sqlite_and_consolidation_fields() {
        let config = MemoryConfig {
            backend: MemoryBackend::Graph,
            sqlite_path: Some(PathBuf::from("/tmp/captain-memory.sqlite")),
            embedding_model: "text-embedding-3-small".to_string(),
            consolidation_threshold: 5_000,
            decay_rate: 0.25,
            embedding_provider: Some("openai".to_string()),
            embedding_api_key_env: Some("OPENAI_API_KEY".to_string()),
            consolidation_interval_hours: 6,
        };

        let encoded = toml::to_string(&config).unwrap();
        let decoded: MemoryConfig = toml::from_str(&encoded).unwrap();

        assert_eq!(decoded.backend, MemoryBackend::Graph);
        assert_eq!(
            decoded.sqlite_path.as_deref(),
            Some(std::path::Path::new("/tmp/captain-memory.sqlite"))
        );
        assert_eq!(decoded.embedding_model, "text-embedding-3-small");
        assert_eq!(decoded.consolidation_threshold, 5_000);
        assert_eq!(decoded.decay_rate, 0.25);
        assert_eq!(decoded.embedding_provider.as_deref(), Some("openai"));
        assert_eq!(
            decoded.embedding_api_key_env.as_deref(),
            Some("OPENAI_API_KEY")
        );
        assert_eq!(decoded.consolidation_interval_hours, 6);
    }

    #[derive(Deserialize)]
    struct BackendWrapper {
        backend: MemoryBackend,
    }
}

//! Core type definitions — IDs, configuration, property values, and query options.

use std::collections::HashMap;
use std::fmt;

/// Unique identifier for an entity in the knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub u64);

/// Unique identifier for an edge (fact) in the knowledge graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdgeId(pub u64);

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "entity:{}", self.0)
    }
}

impl fmt::Display for EdgeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "edge:{}", self.0)
    }
}

/// A property value attached to an entity.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PropertyValue {
    /// UTF-8 string value.
    String(std::string::String),
    /// 64-bit signed integer value.
    Int(i64),
    /// 64-bit floating-point value.
    Float(f64),
    /// Boolean value.
    Bool(bool),
}

impl From<&str> for PropertyValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_string())
    }
}

impl From<std::string::String> for PropertyValue {
    fn from(s: std::string::String) -> Self {
        Self::String(s)
    }
}

impl From<i64> for PropertyValue {
    fn from(v: i64) -> Self {
        Self::Int(v)
    }
}

impl From<i32> for PropertyValue {
    fn from(v: i32) -> Self {
        Self::Int(v as i64)
    }
}

impl From<f64> for PropertyValue {
    fn from(v: f64) -> Self {
        Self::Float(v)
    }
}

impl From<bool> for PropertyValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

/// Convenience type alias for entity properties.
pub type Properties = HashMap<std::string::String, PropertyValue>;

/// Convenience macro for building property maps.
///
/// ```
/// use hora_graph_core::{props, PropertyValue};
/// let p = props! { "language" => "Rust", "stars" => 42 };
/// assert_eq!(p.get("language"), Some(&PropertyValue::String("Rust".into())));
/// ```
#[macro_export]
macro_rules! props {
    ($($key:expr => $val:expr),* $(,)?) => {{
        let mut map = $crate::Properties::new();
        $(map.insert($key.to_string(), $crate::PropertyValue::from($val));)*
        map
    }};
}

/// Partial update for an entity. Only `Some` fields are applied.
#[derive(Debug, Clone, Default)]
pub struct EntityUpdate {
    /// New name to apply, if any.
    pub name: Option<std::string::String>,
    /// New entity type to apply, if any.
    pub entity_type: Option<std::string::String>,
    /// New property map to apply, if any.
    pub properties: Option<Properties>,
    /// New embedding vector to apply, if any.
    pub embedding: Option<Vec<f32>>,
}

/// Partial update for a fact. Only `Some` fields are applied.
#[derive(Debug, Clone, Default)]
pub struct FactUpdate {
    /// New confidence score to apply, if any.
    pub confidence: Option<f32>,
    /// New description to apply, if any.
    pub description: Option<std::string::String>,
}

/// Options for graph traversal (BFS).
#[derive(Debug, Clone)]
pub struct TraverseOpts {
    /// Maximum depth of the BFS. 0 = start node only.
    pub depth: u32,
}

impl Default for TraverseOpts {
    fn default() -> Self {
        Self { depth: 3 }
    }
}

/// Result of a graph traversal.
#[derive(Debug, Clone)]
#[must_use]
pub struct TraverseResult {
    /// All entity IDs discovered during traversal, starting with the start node.
    pub entity_ids: Vec<EntityId>,
    /// All edge IDs traversed.
    pub edge_ids: Vec<EdgeId>,
}

/// Source of an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EpisodeSource {
    /// Episode originated from a chat / dialogue session.
    Conversation,
    /// Episode extracted from a document or file.
    Document,
    /// Episode injected via an external API call.
    Api,
}

/// Configuration for entity deduplication.
#[derive(Debug, Clone)]
pub struct DedupConfig {
    /// Enable deduplication on add_entity.
    pub enabled: bool,
    /// Detect exact name match after normalisation (lowercase, trim, collapse separators).
    pub name_exact: bool,
    /// Jaccard token overlap threshold (0.0–1.0). 0.0 = disabled.
    pub jaccard_threshold: f32,
    /// Cosine embedding similarity threshold (0.0–1.0). 0.0 = disabled.
    pub cosine_threshold: f32,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            name_exact: true,
            jaccard_threshold: 0.85,
            cosine_threshold: 0.92,
        }
    }
}

impl DedupConfig {
    /// Create a config with deduplication disabled.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }
}

/// Configuration for a HoraCore instance.
#[derive(Debug, Clone, Default)]
pub struct HoraConfig {
    /// Embedding vector dimensions. 0 = no vector search (text-only mode).
    pub embedding_dims: u16,
    /// Deduplication settings. Active by default.
    pub dedup: DedupConfig,
}

/// Summary statistics for the knowledge graph.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct StorageStats {
    /// Total number of entities in the graph.
    pub entities: u64,
    /// Total number of edges (facts) in the graph.
    pub edges: u64,
    /// Total number of episodes stored.
    pub episodes: u64,
}

/// Current time in epoch milliseconds.
pub(crate) fn now_millis() -> i64 {
    #[cfg(not(feature = "wasm"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_millis() as i64
    }
    #[cfg(feature = "wasm")]
    {
        js_sys::Date::now() as i64
    }
}

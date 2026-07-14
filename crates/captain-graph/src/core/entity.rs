//! Entity type — a node in the knowledge graph with optional properties and embedding.

use crate::core::types::{EntityId, Properties};

/// A node in the knowledge graph.
#[derive(Debug, Clone)]
pub struct Entity {
    /// Unique identifier of this entity.
    pub id: EntityId,
    /// Semantic type label (e.g. `"Person"`, `"Concept"`).
    pub entity_type: String,
    /// Human-readable name of the entity.
    pub name: String,
    /// Arbitrary key-value metadata attached to this entity.
    pub properties: Properties,
    /// Optional dense vector representation used for similarity search.
    pub embedding: Option<Vec<f32>>,
    /// Unix timestamp (ms) when this entity was stored.
    pub created_at: i64,
}

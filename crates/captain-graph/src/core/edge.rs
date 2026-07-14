//! Edge (fact) type — a directed, bi-temporal relation between two entities.

use crate::core::types::{EdgeId, EntityId};

/// A directed edge (fact) between two entities in the knowledge graph.
///
/// Edges are bi-temporal: `valid_at` / `invalid_at` represent when the fact
/// is true in the real world, while `created_at` records when it was stored.
#[derive(Debug, Clone)]
pub struct Edge {
    /// Unique identifier of this edge.
    pub id: EdgeId,
    /// ID of the entity this edge originates from.
    pub source: EntityId,
    /// ID of the entity this edge points to.
    pub target: EntityId,
    /// Label describing the relationship (e.g. `"knows"`, `"part_of"`).
    pub relation_type: String,
    /// Human-readable description of the fact.
    pub description: String,
    /// Confidence score in `[0.0, 1.0]` for this fact.
    pub confidence: f32,
    /// Unix timestamp (ms) when this fact became true in the world.
    pub valid_at: i64,
    /// Unix timestamp (ms) when this fact ceased to be true; `0` means still valid.
    pub invalid_at: i64,
    /// Unix timestamp (ms) when this edge was stored.
    pub created_at: i64,
}

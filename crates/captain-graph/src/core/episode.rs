//! Episode type — an episodic memory record capturing interaction snapshots.

use crate::core::types::{EdgeId, EntityId, EpisodeSource};

/// An episodic memory record — a snapshot of an interaction or event.
///
/// Episodes are the raw, fast-write store (analogous to the hippocampus).
/// They are progressively transformed into semantic facts via the dream cycle.
#[derive(Debug, Clone)]
pub struct Episode {
    /// Unique identifier of this episode.
    pub id: u64,
    /// Origin of the episode (conversation, document, API call, …).
    pub source: EpisodeSource,
    /// Identifier of the session that produced this episode.
    pub session_id: String,
    /// IDs of entities referenced in this episode.
    pub entity_ids: Vec<EntityId>,
    /// IDs of edges (facts) referenced in this episode.
    pub fact_ids: Vec<EdgeId>,
    /// Unix timestamp (ms) when this episode was stored.
    pub created_at: i64,
    /// Number of times this episode has been replayed during consolidation.
    pub consolidation_count: u32,
}

//! Storage trait — the interface every backend must implement.

use crate::core::edge::Edge;
use crate::core::entity::Entity;
use crate::core::episode::Episode;
use crate::core::types::{EdgeId, EntityId, StorageStats};
use crate::error::Result;

/// Abstraction over storage backends (memory, embedded, sqlite, postgres).
///
/// All backends implement the same trait so the engine behaves identically
/// regardless of the underlying storage.
pub trait StorageOps: Send {
    // --- Entities ---
    /// Insert or overwrite an entity in storage.
    fn put_entity(&mut self, entity: Entity) -> Result<()>;
    /// Retrieve an entity by ID, returning `None` if not found.
    fn get_entity(&self, id: EntityId) -> Result<Option<Entity>>;
    /// Delete an entity by ID; returns `true` if it existed.
    fn delete_entity(&mut self, id: EntityId) -> Result<bool>;

    // --- Edges ---
    /// Insert or overwrite an edge in storage.
    fn put_edge(&mut self, edge: Edge) -> Result<()>;
    /// Retrieve an edge by ID, returning `None` if not found.
    fn get_edge(&self, id: EdgeId) -> Result<Option<Edge>>;
    /// Return all edges incident to the given entity (as source or target).
    fn get_entity_edges(&self, entity_id: EntityId) -> Result<Vec<Edge>>;
    /// Return the IDs of all edges incident to the given entity.
    fn get_entity_edge_ids(&self, entity_id: EntityId) -> Result<Vec<EdgeId>>;
    /// Delete an edge by ID; returns `true` if it existed.
    fn delete_edge(&mut self, id: EdgeId) -> Result<bool>;

    // --- Episodes ---
    /// Persist a new episode.
    fn put_episode(&mut self, episode: Episode) -> Result<()>;
    /// Retrieve an episode by ID, returning `None` if not found.
    fn get_episode(&self, id: u64) -> Result<Option<Episode>>;
    /// Update the consolidation count of an episode; returns `true` if the episode was found.
    fn update_episode_consolidation(&mut self, id: u64, count: u32) -> Result<bool>;

    // --- Scan ---
    /// Return all entities in storage.
    fn scan_all_entities(&self) -> Result<Vec<Entity>>;
    /// Return all edges in storage.
    fn scan_all_edges(&self) -> Result<Vec<Edge>>;
    /// Return all episodes in storage.
    fn scan_all_episodes(&self) -> Result<Vec<Episode>>;

    // --- Stats ---
    /// Return aggregate counts of entities, edges, and episodes.
    fn stats(&self) -> StorageStats;
}

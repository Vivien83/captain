//! In-memory storage backend — `Vec`-indexed, zero persistence.

use std::collections::HashMap;

use crate::core::edge::Edge;
use crate::core::entity::Entity;
use crate::core::episode::Episode;
use crate::core::types::{EdgeId, EntityId, StorageStats};
use crate::error::Result;
use crate::storage::traits::StorageOps;

/// In-memory storage backend. Used for tests and ephemeral instances.
///
/// Entities and edges are stored in dense `Vec`s indexed by their sequential
/// IDs, giving O(1) lookup without hashing. Deletions leave `None` holes.
pub struct MemoryStorage {
    entities: Vec<Option<Entity>>,
    edges: Vec<Option<Edge>>,
    /// Maps an entity ID to the IDs of all edges where it appears as source or target.
    entity_edges: HashMap<EntityId, Vec<EdgeId>>,
    episodes: Vec<Episode>,
    entity_count: u64,
    edge_count: u64,
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryStorage {
    /// Create a new empty in-memory storage instance.
    pub fn new() -> Self {
        Self {
            entities: Vec::new(),
            edges: Vec::new(),
            entity_edges: HashMap::new(),
            episodes: Vec::new(),
            entity_count: 0,
            edge_count: 0,
        }
    }

    fn entity_slot(&self, id: EntityId) -> Option<&Entity> {
        self.entities.get(id.0 as usize).and_then(|s| s.as_ref())
    }

    fn edge_slot(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(id.0 as usize).and_then(|s| s.as_ref())
    }

    fn ensure_entity_capacity(&mut self, id: EntityId) {
        let idx = id.0 as usize;
        if idx >= self.entities.len() {
            self.entities.resize_with(idx + 1, || None);
        }
    }

    fn ensure_edge_capacity(&mut self, id: EdgeId) {
        let idx = id.0 as usize;
        if idx >= self.edges.len() {
            self.edges.resize_with(idx + 1, || None);
        }
    }
}

impl StorageOps for MemoryStorage {
    fn put_entity(&mut self, entity: Entity) -> Result<()> {
        let id = entity.id;
        self.ensure_entity_capacity(id);
        let idx = id.0 as usize;
        if self.entities[idx].is_none() {
            self.entity_count += 1;
        }
        self.entities[idx] = Some(entity);
        Ok(())
    }

    fn get_entity(&self, id: EntityId) -> Result<Option<Entity>> {
        Ok(self.entity_slot(id).cloned())
    }

    fn delete_entity(&mut self, id: EntityId) -> Result<bool> {
        let idx = id.0 as usize;
        if idx < self.entities.len() && self.entities[idx].take().is_some() {
            self.entity_count -= 1;
            self.entity_edges.remove(&id);
            return Ok(true);
        }
        Ok(false)
    }

    fn put_edge(&mut self, edge: Edge) -> Result<()> {
        let id = edge.id;
        let source = edge.source;
        let target = edge.target;

        self.ensure_edge_capacity(id);
        let idx = id.0 as usize;
        if self.edges[idx].is_none() {
            self.edge_count += 1;
        }
        self.edges[idx] = Some(edge);
        self.entity_edges.entry(source).or_default().push(id);
        if source != target {
            self.entity_edges.entry(target).or_default().push(id);
        }
        Ok(())
    }

    fn get_edge(&self, id: EdgeId) -> Result<Option<Edge>> {
        Ok(self.edge_slot(id).cloned())
    }

    fn get_entity_edges(&self, entity_id: EntityId) -> Result<Vec<Edge>> {
        let edge_ids = match self.entity_edges.get(&entity_id) {
            Some(ids) => ids,
            None => return Ok(Vec::new()),
        };

        let edges = edge_ids
            .iter()
            .filter_map(|id| self.edge_slot(*id).cloned())
            .collect();
        Ok(edges)
    }

    fn get_entity_edge_ids(&self, entity_id: EntityId) -> Result<Vec<EdgeId>> {
        Ok(self
            .entity_edges
            .get(&entity_id)
            .cloned()
            .unwrap_or_default())
    }

    fn delete_edge(&mut self, id: EdgeId) -> Result<bool> {
        let idx = id.0 as usize;
        let edge = if idx < self.edges.len() {
            self.edges[idx].take()
        } else {
            None
        };
        let edge = match edge {
            Some(e) => {
                self.edge_count -= 1;
                e
            }
            None => return Ok(false),
        };

        // Clean up entity_edges index for both sides
        if let Some(ids) = self.entity_edges.get_mut(&edge.source) {
            ids.retain(|eid| *eid != id);
        }
        if edge.source != edge.target {
            if let Some(ids) = self.entity_edges.get_mut(&edge.target) {
                ids.retain(|eid| *eid != id);
            }
        }
        Ok(true)
    }

    fn scan_all_entities(&self) -> Result<Vec<Entity>> {
        Ok(self.entities.iter().flatten().cloned().collect())
    }

    fn scan_all_edges(&self) -> Result<Vec<Edge>> {
        Ok(self.edges.iter().flatten().cloned().collect())
    }

    fn scan_all_episodes(&self) -> Result<Vec<Episode>> {
        Ok(self.episodes.clone())
    }

    fn put_episode(&mut self, episode: Episode) -> Result<()> {
        self.episodes.push(episode);
        Ok(())
    }

    fn get_episode(&self, id: u64) -> Result<Option<Episode>> {
        Ok(self.episodes.iter().find(|e| e.id == id).cloned())
    }

    fn update_episode_consolidation(&mut self, id: u64, count: u32) -> Result<bool> {
        if let Some(ep) = self.episodes.iter_mut().find(|e| e.id == id) {
            ep.consolidation_count = count;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn stats(&self) -> StorageStats {
        StorageStats {
            entities: self.entity_count,
            edges: self.edge_count,
            episodes: self.episodes.len() as u64,
        }
    }
}

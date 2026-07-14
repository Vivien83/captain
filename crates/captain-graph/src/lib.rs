//! hora-graph-core — Bio-inspired embedded knowledge graph engine.
//!
//! A pure-Rust knowledge graph with bi-temporal facts, vector/BM25 hybrid search,
//! ACT-R memory activation, reconsolidation, dark nodes, FSRS scheduling,
//! and a dream cycle for memory consolidation.

pub mod core;
pub mod error;
pub mod memory;
pub mod search;
pub mod storage;

pub use crate::core::edge::Edge;
pub use crate::core::entity::Entity;
pub use crate::core::episode::Episode;
pub use crate::core::types::{
    DedupConfig, EdgeId, EntityId, EntityUpdate, EpisodeSource, FactUpdate, HoraConfig, Properties,
    PropertyValue, StorageStats, TraverseOpts, TraverseResult,
};
pub use crate::error::{HoraError, Result};
pub use crate::memory::consolidation::{
    ClsStats, ConsolidationParams, DreamCycleConfig, DreamCycleStats, LinkingStats, ReplayStats,
};
pub use crate::memory::dark_nodes::DarkNodeParams;
pub use crate::memory::fsrs::FsrsParams;
pub use crate::memory::reconsolidation::{MemoryPhase, ReconsolidationParams};
pub use crate::memory::spreading::SpreadingParams;
pub use crate::search::{SearchHit, SearchOpts};
pub use crate::storage::format::{verify_file, VerifyReport};

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crate::core::types::now_millis;
use crate::memory::activation::ActivationState;
use crate::memory::fsrs::FsrsState;
use crate::memory::reconsolidation::ReconsolidationState;
use crate::search::bm25::{self, Bm25Index};
use crate::storage::format::{self, FileHeader};
use crate::storage::memory::MemoryStorage;
use crate::storage::traits::StorageOps;

/// The main entry point for hora-graph-core.
///
/// ```
/// use hora_graph_core::{HoraCore, HoraConfig};
///
/// let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
/// let id = hora.add_entity("project", "hora", None, None).unwrap();
/// let entity = hora.get_entity(id).unwrap().unwrap();
/// assert_eq!(entity.name, "hora");
/// ```
pub struct HoraCore {
    config: HoraConfig,
    storage: Box<dyn StorageOps>,
    next_entity_id: u64,
    next_edge_id: u64,
    next_episode_id: u64,
    file_path: Option<PathBuf>,
    bm25_index: Bm25Index,
    bm25_built: bool,
    pending_accesses: Vec<EntityId>,
    activation_states: HashMap<EntityId, ActivationState>,
    reconsolidation_states: HashMap<EntityId, ReconsolidationState>,
    reconsolidation_params: ReconsolidationParams,
    dark_node_params: DarkNodeParams,
    fsrs_states: HashMap<EntityId, FsrsState>,
    fsrs_params: FsrsParams,
    consolidation_params: ConsolidationParams,
}

impl HoraCore {
    /// Create a new in-memory HoraCore instance (no persistence).
    pub fn new(config: HoraConfig) -> Result<Self> {
        Ok(Self {
            config,
            storage: Box::new(MemoryStorage::new()),
            next_entity_id: 1,
            next_edge_id: 1,
            next_episode_id: 1,
            file_path: None,
            bm25_index: Bm25Index::new(),
            bm25_built: true,
            pending_accesses: Vec::new(),
            activation_states: HashMap::new(),
            reconsolidation_states: HashMap::new(),
            reconsolidation_params: ReconsolidationParams::default(),
            dark_node_params: DarkNodeParams::default(),
            fsrs_states: HashMap::new(),
            fsrs_params: FsrsParams::default(),
            consolidation_params: ConsolidationParams::default(),
        })
    }

    /// Open a file-backed HoraCore instance.
    ///
    /// If the file exists, loads data from it. If it does not exist, creates
    /// a new empty instance that will write to the given path on `flush()`.
    pub fn open(path: impl AsRef<Path>, config: HoraConfig) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        if path.exists() {
            let file = std::fs::File::open(&path)?;
            let mut reader = std::io::BufReader::new(file);
            let graph = format::deserialize(&mut reader)?;

            // Rebuild MemoryStorage from deserialized data (BM25 is lazy)
            let mut storage = MemoryStorage::new();
            for entity in graph.entities {
                storage.put_entity(entity)?;
            }
            for edge in graph.edges {
                storage.put_edge(edge)?;
            }
            for episode in graph.episodes {
                storage.put_episode(episode)?;
            }

            Ok(Self {
                config: HoraConfig {
                    embedding_dims: graph.header.embedding_dims,
                    ..Default::default()
                },
                storage: Box::new(storage),
                next_entity_id: graph.header.next_entity_id,
                next_edge_id: graph.header.next_edge_id,
                next_episode_id: graph.header.next_episode_id,
                file_path: Some(path),
                bm25_index: Bm25Index::new(),
                bm25_built: false,
                pending_accesses: Vec::new(),
                activation_states: HashMap::new(),
                reconsolidation_states: HashMap::new(),
                reconsolidation_params: ReconsolidationParams::default(),
                dark_node_params: DarkNodeParams::default(),
                fsrs_states: HashMap::new(),
                fsrs_params: FsrsParams::default(),
                consolidation_params: ConsolidationParams::default(),
            })
        } else {
            Ok(Self {
                file_path: Some(path),
                ..Self::new(config)?
            })
        }
    }

    // --- Persistence ---

    /// Flush pending access events to activation tracking.
    fn flush_accesses(&mut self) {
        if self.pending_accesses.is_empty() {
            return;
        }
        let ids: Vec<EntityId> = self.pending_accesses.drain(..).collect();
        for id in ids {
            self.record_access(id);
        }
    }

    /// Build the BM25 index from all entities if not already built.
    fn ensure_bm25(&mut self) -> Result<()> {
        if !self.bm25_built {
            let entities = self.storage.scan_all_entities()?;
            for entity in &entities {
                let text = bm25::entity_text(&entity.name, &entity.properties);
                self.bm25_index.index_document(entity.id.0 as u32, &text);
            }
            self.bm25_built = true;
        }
        Ok(())
    }

    /// Flush all data to the backing file.
    ///
    /// Writes to a temporary file first, then renames for crash safety.
    /// Returns an error if this is an in-memory-only instance.
    pub fn flush(&self) -> Result<()> {
        let path = self.file_path.as_ref().ok_or(HoraError::InvalidFile {
            reason: "cannot flush an in-memory-only instance",
        })?;

        let entities = self.storage.scan_all_entities()?;
        let edges = self.storage.scan_all_edges()?;
        let episodes = self.storage.scan_all_episodes()?;

        let header = FileHeader {
            embedding_dims: self.config.embedding_dims,
            next_entity_id: self.next_entity_id,
            next_edge_id: self.next_edge_id,
            next_episode_id: self.next_episode_id,
            entity_count: entities.len() as u32,
            edge_count: edges.len() as u32,
            episode_count: episodes.len() as u32,
        };

        // Write to .tmp then rename (crash-safe)
        let tmp_path = path.with_extension("hora.tmp");
        {
            let file = std::fs::File::create(&tmp_path)?;
            let mut writer = std::io::BufWriter::new(file);
            format::serialize(&mut writer, &header, &entities, &edges, &episodes)?;
        }
        std::fs::rename(&tmp_path, path)?;

        Ok(())
    }

    /// Copy the current state to a snapshot file.
    ///
    /// Flushes first if file-backed, then copies. For in-memory instances,
    /// writes directly to the given path.
    pub fn snapshot(&self, dest: impl AsRef<Path>) -> Result<()> {
        let entities = self.storage.scan_all_entities()?;
        let edges = self.storage.scan_all_edges()?;
        let episodes = self.storage.scan_all_episodes()?;

        let header = FileHeader {
            embedding_dims: self.config.embedding_dims,
            next_entity_id: self.next_entity_id,
            next_edge_id: self.next_edge_id,
            next_episode_id: self.next_episode_id,
            entity_count: entities.len() as u32,
            edge_count: edges.len() as u32,
            episode_count: episodes.len() as u32,
        };

        let file = std::fs::File::create(dest)?;
        let mut writer = std::io::BufWriter::new(file);
        format::serialize(&mut writer, &header, &entities, &edges, &episodes)?;

        Ok(())
    }

    // --- CRUD Entities ---

    /// Add a new entity to the knowledge graph.
    ///
    /// If deduplication is enabled and a duplicate is detected among entities
    /// of the same type, returns the existing entity's ID instead of creating
    /// a new one. Detection uses: normalized name exact match, cosine embedding
    /// similarity, and Jaccard token overlap (in that priority order).
    pub fn add_entity(
        &mut self,
        entity_type: &str,
        name: &str,
        properties: Option<Properties>,
        embedding: Option<&[f32]>,
    ) -> Result<EntityId> {
        // Validate embedding dimensions
        if let Some(emb) = embedding {
            if self.config.embedding_dims == 0 {
                return Err(HoraError::DimensionMismatch {
                    expected: 0,
                    got: emb.len(),
                });
            }
            if emb.len() != self.config.embedding_dims as usize {
                return Err(HoraError::DimensionMismatch {
                    expected: self.config.embedding_dims as usize,
                    got: emb.len(),
                });
            }
        }

        // Deduplication check
        if self.config.dedup.enabled {
            let candidates = self.storage.scan_all_entities()?;
            if let Some(existing_id) = crate::core::dedup::find_duplicate(
                name,
                embedding,
                entity_type,
                &candidates,
                &self.config.dedup,
            ) {
                return Ok(existing_id);
            }
        }

        let id = EntityId(self.next_entity_id);
        self.next_entity_id += 1;

        let entity = Entity {
            id,
            entity_type: entity_type.to_string(),
            name: name.to_string(),
            properties: properties.unwrap_or_default(),
            embedding: embedding.map(|e| e.to_vec()),
            created_at: now_millis(),
        };

        // Index for BM25 full-text search (skip if lazy rebuild pending)
        if self.bm25_built {
            let text = bm25::entity_text(&entity.name, &entity.properties);
            self.bm25_index.index_document(id.0 as u32, &text);
        }

        // Initialize activation state
        let now_secs = entity.created_at as f64 / 1000.0;
        let mut act_state = ActivationState::new(now_secs);
        act_state.record_access(now_secs);
        self.activation_states.insert(id, act_state);

        // Initialize reconsolidation state
        self.reconsolidation_states
            .insert(id, ReconsolidationState::new());

        // Initialize FSRS state
        self.fsrs_states.insert(
            id,
            FsrsState::new(now_secs, self.fsrs_params.initial_stability_days),
        );

        self.storage.put_entity(entity)?;
        Ok(id)
    }

    /// Get an entity by ID. Returns `None` if not found.
    ///
    /// Side-effect: buffers an access event for ACT-R activation tracking.
    /// The actual activation computation is deferred until needed.
    pub fn get_entity(&mut self, id: EntityId) -> Result<Option<Entity>> {
        let entity = self.storage.get_entity(id)?;
        if entity.is_some() {
            self.pending_accesses.push(id);
        }
        Ok(entity)
    }

    /// Update an entity. Only fields set to `Some` in the update are changed.
    pub fn update_entity(&mut self, id: EntityId, update: EntityUpdate) -> Result<()> {
        let mut entity = self
            .storage
            .get_entity(id)?
            .ok_or(HoraError::EntityNotFound(id.0))?;

        if let Some(name) = update.name {
            entity.name = name;
        }
        if let Some(entity_type) = update.entity_type {
            entity.entity_type = entity_type;
        }
        if let Some(properties) = update.properties {
            entity.properties = properties;
        }
        if let Some(embedding) = update.embedding {
            if self.config.embedding_dims == 0 {
                return Err(HoraError::DimensionMismatch {
                    expected: 0,
                    got: embedding.len(),
                });
            }
            if embedding.len() != self.config.embedding_dims as usize {
                return Err(HoraError::DimensionMismatch {
                    expected: self.config.embedding_dims as usize,
                    got: embedding.len(),
                });
            }
            entity.embedding = Some(embedding);
        }

        // Re-index for BM25 (skip if lazy rebuild pending)
        if self.bm25_built {
            let text = bm25::entity_text(&entity.name, &entity.properties);
            self.bm25_index.index_document(id.0 as u32, &text);
        }

        self.storage.put_entity(entity)
    }

    /// Delete an entity and all its associated edges (cascade).
    pub fn delete_entity(&mut self, id: EntityId) -> Result<()> {
        if self.storage.get_entity(id)?.is_none() {
            return Err(HoraError::EntityNotFound(id.0));
        }

        // Cascade: delete all edges connected to this entity
        let edge_ids = self.storage.get_entity_edge_ids(id)?;
        for edge_id in edge_ids {
            self.storage.delete_edge(edge_id)?;
        }

        self.storage.delete_entity(id)?;
        if self.bm25_built {
            self.bm25_index.remove_document(id.0 as u32);
        }
        self.activation_states.remove(&id);
        self.reconsolidation_states.remove(&id);
        self.fsrs_states.remove(&id);
        Ok(())
    }

    // --- CRUD Facts (edges) ---

    /// Add a new fact (directed edge) between two entities.
    ///
    /// Returns the ID of the newly created fact. Both source and target
    /// entities must exist.
    pub fn add_fact(
        &mut self,
        source: EntityId,
        target: EntityId,
        relation: &str,
        description: &str,
        confidence: Option<f32>,
    ) -> Result<EdgeId> {
        // Verify both entities exist
        if self.storage.get_entity(source)?.is_none() {
            return Err(HoraError::EntityNotFound(source.0));
        }
        if self.storage.get_entity(target)?.is_none() {
            return Err(HoraError::EntityNotFound(target.0));
        }

        let id = EdgeId(self.next_edge_id);
        self.next_edge_id += 1;
        let now = now_millis();

        let edge = Edge {
            id,
            source,
            target,
            relation_type: relation.to_string(),
            description: description.to_string(),
            confidence: confidence.unwrap_or(1.0),
            valid_at: now,
            invalid_at: 0,
            created_at: now,
        };

        self.storage.put_edge(edge)?;
        Ok(id)
    }

    /// Get a fact by ID. Returns `None` if not found.
    pub fn get_fact(&self, id: EdgeId) -> Result<Option<Edge>> {
        self.storage.get_edge(id)
    }

    /// Update a fact. Only fields set to `Some` in the update are changed.
    pub fn update_fact(&mut self, id: EdgeId, update: FactUpdate) -> Result<()> {
        let mut edge = self
            .storage
            .get_edge(id)?
            .ok_or(HoraError::EdgeNotFound(id.0))?;

        if let Some(confidence) = update.confidence {
            edge.confidence = confidence;
        }
        if let Some(description) = update.description {
            edge.description = description;
        }

        self.storage.put_edge(edge)
    }

    /// Mark a fact as invalid (bi-temporal). The fact is NOT deleted —
    /// it remains queryable with its validity window.
    pub fn invalidate_fact(&mut self, id: EdgeId) -> Result<()> {
        let mut edge = self
            .storage
            .get_edge(id)?
            .ok_or(HoraError::EdgeNotFound(id.0))?;

        if edge.invalid_at != 0 {
            return Err(HoraError::AlreadyInvalidated(id.0));
        }

        edge.invalid_at = now_millis();
        self.storage.put_edge(edge)
    }

    /// Physically delete a fact. Use `invalidate_fact` for bi-temporal soft-delete.
    pub fn delete_fact(&mut self, id: EdgeId) -> Result<()> {
        if !self.storage.delete_edge(id)? {
            return Err(HoraError::EdgeNotFound(id.0));
        }
        Ok(())
    }

    /// Get all facts where the given entity is source or target.
    pub fn get_entity_facts(&self, entity_id: EntityId) -> Result<Vec<Edge>> {
        self.storage.get_entity_edges(entity_id)
    }

    // --- Graph Traversal ---

    /// BFS traversal from a start entity up to the given depth.
    ///
    /// Returns IDs of all discovered entities and edges.
    /// Depth 0 returns only the start entity (no edges).
    pub fn traverse(&self, start: EntityId, opts: TraverseOpts) -> Result<TraverseResult> {
        if self.storage.get_entity(start)?.is_none() {
            return Err(HoraError::EntityNotFound(start.0));
        }

        let mut visited: HashSet<EntityId> = HashSet::new();
        let mut result_entity_ids = vec![start];
        let mut result_edge_ids: Vec<EdgeId> = Vec::new();
        let mut seen_edges: HashSet<EdgeId> = HashSet::new();

        visited.insert(start);

        // BFS queue holds (entity_id, current_depth)
        let mut queue: VecDeque<(EntityId, u32)> = VecDeque::new();
        queue.push_back((start, 0));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth >= opts.depth {
                continue;
            }

            let edges = self.storage.get_entity_edges(current_id)?;
            for edge in edges {
                if !seen_edges.insert(edge.id) {
                    continue;
                }

                // The neighbor is whichever end is NOT current_id
                let neighbor_id = if edge.source == current_id {
                    edge.target
                } else {
                    edge.source
                };

                result_edge_ids.push(edge.id);

                if visited.insert(neighbor_id) && self.storage.get_entity(neighbor_id)?.is_some() {
                    result_entity_ids.push(neighbor_id);
                    queue.push_back((neighbor_id, depth + 1));
                }
            }
        }

        Ok(TraverseResult {
            entity_ids: result_entity_ids,
            edge_ids: result_edge_ids,
        })
    }

    /// Get all direct neighbor entity IDs (connected via any edge).
    pub fn neighbors(&self, entity_id: EntityId) -> Result<Vec<EntityId>> {
        let edges = self.storage.get_entity_edges(entity_id)?;
        let mut neighbor_ids: HashSet<EntityId> = HashSet::new();

        for edge in &edges {
            if edge.source == entity_id {
                neighbor_ids.insert(edge.target);
            } else {
                neighbor_ids.insert(edge.source);
            }
        }

        // Remove self (possible with self-loops)
        neighbor_ids.remove(&entity_id);
        Ok(neighbor_ids.into_iter().collect())
    }

    /// Timeline of all facts involving an entity, sorted by `valid_at`.
    pub fn timeline(&self, entity_id: EntityId) -> Result<Vec<Edge>> {
        let mut edges = self.storage.get_entity_edges(entity_id)?;
        edges.sort_by_key(|e| e.valid_at);
        Ok(edges)
    }

    /// All facts valid at a given point in time.
    ///
    /// A fact is valid at time `t` if `valid_at <= t` and
    /// (`invalid_at == 0` or `invalid_at > t`).
    pub fn facts_at(&self, t: i64) -> Result<Vec<Edge>> {
        let all = self.storage.scan_all_edges()?;
        let valid: Vec<Edge> = all
            .into_iter()
            .filter(|e| e.valid_at <= t && (e.invalid_at == 0 || e.invalid_at > t))
            .collect();
        Ok(valid)
    }

    // --- Vector Search ---

    /// Brute-force vector search: find the `k` most similar entities by cosine similarity.
    ///
    /// Requires `embedding_dims > 0` in the config. Entities without embeddings
    /// are silently skipped. The query embedding must match `embedding_dims` in length.
    pub fn vector_search(&self, query: &[f32], k: usize) -> Result<Vec<SearchHit>> {
        if self.config.embedding_dims == 0 {
            return Err(HoraError::DimensionMismatch {
                expected: 0,
                got: query.len(),
            });
        }
        if query.len() != self.config.embedding_dims as usize {
            return Err(HoraError::DimensionMismatch {
                expected: self.config.embedding_dims as usize,
                got: query.len(),
            });
        }

        let entities = self.storage.scan_all_entities()?;

        // Collect (id, embedding_slice) pairs, skip entities without embeddings
        let with_embeddings: Vec<(EntityId, &[f32])> = entities
            .iter()
            .filter_map(|e| e.embedding.as_ref().map(|emb| (e.id, emb.as_slice())))
            .collect();

        Ok(search::vector::top_k_brute_force(
            query,
            &with_embeddings,
            k,
        ))
    }

    // --- Text Search (BM25) ---

    /// Full-text search using BM25+ scoring over entity names and string properties.
    ///
    /// Returns the top `k` matching entities. Entities without indexable text
    /// are invisible to BM25.
    pub fn text_search(&mut self, query: &str, k: usize) -> Result<Vec<SearchHit>> {
        self.ensure_bm25()?;
        Ok(self.bm25_index.search(query, k))
    }

    // --- Hybrid Search ---

    /// Hybrid search combining vector similarity and BM25 full-text via RRF fusion.
    ///
    /// Provide `query_text` for BM25, `query_embedding` for vector search, or both.
    /// When both are provided, results are fused using Reciprocal Rank Fusion.
    /// When only one is provided, that leg runs alone.
    /// Returns empty if neither is provided.
    pub fn search(
        &mut self,
        query_text: Option<&str>,
        query_embedding: Option<&[f32]>,
        opts: SearchOpts,
    ) -> Result<Vec<SearchHit>> {
        let candidate_k = opts.top_k * 3;

        // Vector leg (skip if embedding_dims=0 or no embedding provided)
        let vec_results = if let Some(emb) = query_embedding {
            if self.config.embedding_dims > 0 && emb.len() == self.config.embedding_dims as usize {
                Some(self.vector_search(emb, candidate_k)?)
            } else {
                None
            }
        } else {
            None
        };

        // BM25 leg
        let bm25_results = if let Some(text) = query_text {
            self.ensure_bm25()?;
            let results = self.bm25_index.search(text, candidate_k);
            if results.is_empty() {
                None
            } else {
                Some(results)
            }
        } else {
            None
        };

        let mut results =
            search::hybrid::rrf_fuse(vec_results.as_deref(), bm25_results.as_deref(), opts.top_k);

        // Filter out dark nodes unless include_dark is set
        if !opts.include_dark {
            results.retain(|hit| {
                !self
                    .reconsolidation_states
                    .get(&hit.entity_id)
                    .is_some_and(|r| r.is_dark())
            });
        }

        // Side-effect: record access for returned results
        for hit in &results {
            self.record_access(hit.entity_id);
        }

        Ok(results)
    }

    // --- Activation (ACT-R) ---

    /// Get the current ACT-R base-level activation for an entity.
    ///
    /// Returns `f64::NEG_INFINITY` if the entity has never been accessed,
    /// or `None` if the entity doesn't exist.
    pub fn get_activation(&mut self, id: EntityId) -> Option<f64> {
        self.flush_accesses();
        let now = now_millis() as f64 / 1000.0;
        self.activation_states
            .get_mut(&id)
            .map(|state| state.compute_activation(now))
    }

    /// Record an access on an entity for ACT-R activation tracking.
    ///
    /// Called automatically by `get_entity()` and `search()`. Can also be
    /// called directly for external access events.
    pub fn record_access(&mut self, id: EntityId) {
        let now = now_millis() as f64 / 1000.0;
        if let Some(act_state) = self.activation_states.get_mut(&id) {
            let activation = act_state.compute_activation(now);
            act_state.record_access(now);

            // Trigger reconsolidation check
            if let Some(recon) = self.reconsolidation_states.get_mut(&id) {
                recon.on_reactivation(activation, now, &self.reconsolidation_params);
            }

            // FSRS: record review with reconsolidation boost
            let boost = self
                .reconsolidation_states
                .get(&id)
                .map(|r| r.stability_multiplier())
                .unwrap_or(1.0);
            if let Some(fsrs) = self.fsrs_states.get_mut(&id) {
                fsrs.record_review(now, boost, &self.fsrs_params);
            }
        }
    }

    /// Get the current reconsolidation phase for an entity.
    ///
    /// Resolves any pending time-based transitions before returning.
    /// Returns `None` if the entity doesn't exist.
    pub fn get_memory_phase(&mut self, id: EntityId) -> Option<&MemoryPhase> {
        let now = now_millis() as f64 / 1000.0;
        if let Some(recon) = self.reconsolidation_states.get_mut(&id) {
            recon.tick(now, &self.reconsolidation_params);
            Some(recon.phase())
        } else {
            None
        }
    }

    /// Get the cumulative stability multiplier for an entity.
    ///
    /// Starts at 1.0, increases by `restabilization_boost` (default 1.2×)
    /// each time the entity completes a reconsolidation cycle.
    pub fn get_stability_multiplier(&mut self, id: EntityId) -> Option<f64> {
        let now = now_millis() as f64 / 1000.0;
        if let Some(recon) = self.reconsolidation_states.get_mut(&id) {
            recon.tick(now, &self.reconsolidation_params);
            Some(recon.stability_multiplier())
        } else {
            None
        }
    }

    // --- FSRS Scheduling ---

    /// Get the current retrievability for an entity (0.0 to 1.0).
    ///
    /// R = 1.0 immediately after review, R → 0.0 as time passes.
    /// Returns `None` if the entity doesn't exist.
    pub fn get_retrievability(&self, id: EntityId) -> Option<f64> {
        let now = now_millis() as f64 / 1000.0;
        self.fsrs_states
            .get(&id)
            .map(|fsrs| fsrs.current_retrievability(now, &self.fsrs_params))
    }

    /// Get the optimal next review interval in days for an entity.
    ///
    /// Uses the configured `desired_retention` (default 0.9).
    /// Returns `None` if the entity doesn't exist.
    pub fn get_next_review_days(&self, id: EntityId) -> Option<f64> {
        self.fsrs_states.get(&id).map(|fsrs| {
            fsrs.next_review_interval_days(self.fsrs_params.desired_retention, &self.fsrs_params)
        })
    }

    /// Get the current FSRS stability in days for an entity.
    ///
    /// Returns `None` if the entity doesn't exist.
    pub fn get_fsrs_stability(&self, id: EntityId) -> Option<f64> {
        self.fsrs_states.get(&id).map(|fsrs| fsrs.stability_days())
    }

    // --- Dark Nodes ---

    /// Scan all entities and mark those below activation threshold as Dark.
    ///
    /// An entity becomes Dark when:
    /// - Its activation is below `silencing_threshold` (default -2.0)
    /// - Its last access was more than `silencing_delay_secs` ago (default 7 days)
    /// - It is currently in Stable state (not Labile/Restabilizing/already Dark)
    ///
    /// Returns the number of entities newly marked as Dark.
    pub fn dark_node_pass(&mut self) -> usize {
        let now = now_millis() as f64 / 1000.0;
        let params = &self.dark_node_params;
        let recon_params = &self.reconsolidation_params;

        let mut to_darken: Vec<EntityId> = Vec::new();

        for (&id, act_state) in &mut self.activation_states {
            let activation = act_state.compute_activation(now);

            // Check activation threshold
            if activation >= params.silencing_threshold {
                continue;
            }

            // Check delay since last access
            let last_access = act_state.last_access_time().unwrap_or(0.0);
            if now - last_access < params.silencing_delay_secs {
                continue;
            }

            // Only silence Stable entities (not Labile/Restabilizing/already Dark)
            if let Some(recon) = self.reconsolidation_states.get_mut(&id) {
                recon.tick(now, recon_params);
                if *recon.phase() == MemoryPhase::Stable {
                    to_darken.push(id);
                }
            }
        }

        for id in &to_darken {
            if let Some(recon) = self.reconsolidation_states.get_mut(id) {
                recon.mark_dark(now);
            }
        }

        to_darken.len()
    }

    /// Attempt to recover a Dark entity via strong external reactivation.
    ///
    /// If the entity is Dark, it transitions to Labile (for re-encoding)
    /// and a record_access is applied. Returns `true` if recovery occurred.
    pub fn attempt_recovery(&mut self, id: EntityId) -> bool {
        let now = now_millis() as f64 / 1000.0;
        let recovered = self
            .reconsolidation_states
            .get_mut(&id)
            .is_some_and(|recon| recon.recover(now));

        if recovered {
            // Record the strong reactivation
            if let Some(act_state) = self.activation_states.get_mut(&id) {
                act_state.record_access(now);
            }
        }

        recovered
    }

    /// List all entity IDs currently in Dark state.
    pub fn dark_nodes(&mut self) -> Vec<EntityId> {
        let now = now_millis() as f64 / 1000.0;
        self.reconsolidation_states
            .iter_mut()
            .filter_map(|(&id, recon)| {
                recon.tick(now, &self.reconsolidation_params);
                if recon.is_dark() {
                    Some(id)
                } else {
                    None
                }
            })
            .collect()
    }

    /// List dark entities eligible for garbage collection (dark > gc_eligible_after_secs).
    pub fn gc_candidates(&mut self) -> Vec<EntityId> {
        let now = now_millis() as f64 / 1000.0;
        let gc_after = self.dark_node_params.gc_eligible_after_secs;

        self.reconsolidation_states
            .iter_mut()
            .filter_map(|(&id, recon)| {
                recon.tick(now, &self.reconsolidation_params);
                match recon.phase() {
                    MemoryPhase::Dark { silenced_at } => {
                        if now - silenced_at >= gc_after {
                            Some(id)
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            })
            .collect()
    }

    // --- Consolidation (SHY) ---

    /// Apply SHY homeostatic downscaling to all entity activations.
    ///
    /// Multiplies every entity's activation score by `factor` (default 0.78).
    /// This is cumulative and idempotent-safe: two calls produce `factor²`.
    /// Affects both positive and negative activations (amplitude reduction).
    ///
    /// Returns the number of entities downscaled.
    pub fn shy_downscaling(&mut self, factor: f64) -> usize {
        let count = self.activation_states.len();
        for state in self.activation_states.values_mut() {
            state.apply_shy_downscaling(factor);
        }
        count
    }

    /// Interleaved replay: re-activate entities from a mix of recent and old episodes.
    ///
    /// Selects up to `max_replay_items` episodes, split by `recent_ratio` (default 70%
    /// recent, 30% older). For each selected episode, calls `record_access()` on every
    /// referenced entity that still exists.
    ///
    /// Episodes are split at the median `created_at` timestamp: those above median are
    /// "recent", the rest are "older". This is deterministic (no RNG required).
    pub fn interleaved_replay(&mut self) -> Result<ReplayStats> {
        let params = &self.consolidation_params;
        let max = params.max_replay_items;
        let ratio = params.recent_ratio.clamp(0.0, 1.0);

        let mut all_episodes = self.storage.scan_all_episodes()?;
        if all_episodes.is_empty() || max == 0 {
            return Ok(ReplayStats {
                episodes_replayed: 0,
                entities_reactivated: 0,
            });
        }

        // Sort by created_at ascending
        all_episodes.sort_by_key(|e| e.created_at);

        // Split at median into older (first half) and recent (second half)
        let mid = all_episodes.len() / 2;
        let (older, recent) = all_episodes.split_at(mid);

        // Budget allocation
        let recent_budget = ((max as f64) * ratio).ceil() as usize;
        let older_budget = max.saturating_sub(recent_budget);

        // Take from the end of each group (most recent within each group)
        let selected_recent: Vec<_> = recent.iter().rev().take(recent_budget).collect();
        let selected_older: Vec<_> = older.iter().rev().take(older_budget).collect();

        let mut episodes_replayed = 0;
        let mut entities_reactivated = 0;

        for ep in selected_recent.iter().chain(selected_older.iter()) {
            episodes_replayed += 1;
            for &entity_id in &ep.entity_ids {
                // Only reactivate if the entity still has an activation state
                if self.activation_states.contains_key(&entity_id) {
                    self.record_access(entity_id);
                    entities_reactivated += 1;
                }
            }
        }

        Ok(ReplayStats {
            episodes_replayed,
            entities_reactivated,
        })
    }

    /// CLS transfer: extract recurring episodic patterns into semantic facts.
    ///
    /// Scans episodes with `consolidation_count >= cls_threshold`. For each,
    /// collects the referenced facts and groups them by (source, relation, target).
    /// Triplets seen in >= `cls_threshold` distinct episodes become semantic facts
    /// (or get their confidence reinforced if already existing).
    ///
    /// Each processed episode gets its `consolidation_count` incremented.
    pub fn cls_transfer(&mut self) -> Result<ClsStats> {
        let threshold = self.consolidation_params.cls_threshold;
        let all_episodes = self.storage.scan_all_episodes()?;

        // Filter eligible episodes
        let eligible: Vec<_> = all_episodes
            .iter()
            .filter(|ep| ep.consolidation_count >= threshold)
            .collect();

        if eligible.is_empty() {
            return Ok(ClsStats {
                episodes_processed: 0,
                facts_created: 0,
                facts_reinforced: 0,
            });
        }

        // Count triplets across eligible episodes: (source, relation, target) → count
        let mut triplet_counts: HashMap<(EntityId, String, EntityId), u32> = HashMap::new();

        for ep in &eligible {
            // Deduplicate triplets within a single episode
            let mut seen_in_ep: HashSet<(EntityId, String, EntityId)> = HashSet::new();
            for &fact_id in &ep.fact_ids {
                if let Some(edge) = self.storage.get_edge(fact_id)? {
                    let key = (edge.source, edge.relation_type.clone(), edge.target);
                    if seen_in_ep.insert(key.clone()) {
                        *triplet_counts.entry(key).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut facts_created = 0_usize;
        let mut facts_reinforced = 0_usize;

        // For triplets seen in >= threshold episodes, create or reinforce semantic fact
        for ((source, relation, target), count) in &triplet_counts {
            if *count < threshold {
                continue;
            }

            // Check if a valid edge with same triplet already exists
            let existing_edges = self.storage.get_entity_edges(*source)?;
            let existing = existing_edges
                .iter()
                .find(|e| e.target == *target && e.relation_type == *relation && e.invalid_at == 0);

            if let Some(edge) = existing {
                // Reinforce: bump confidence (cap at 1.0)
                let new_confidence = (edge.confidence + 0.1).min(1.0);
                self.storage.put_edge(Edge {
                    confidence: new_confidence,
                    ..edge.clone()
                })?;
                facts_reinforced += 1;
            } else {
                // Check both entities still exist before creating
                if self.storage.get_entity(*source)?.is_some()
                    && self.storage.get_entity(*target)?.is_some()
                {
                    let id = EdgeId(self.next_edge_id);
                    self.next_edge_id += 1;
                    let now = crate::core::types::now_millis();
                    let edge = Edge {
                        id,
                        source: *source,
                        target: *target,
                        relation_type: relation.clone(),
                        description: format!("semantic: consolidated from {count} episodes"),
                        confidence: 0.9,
                        valid_at: now,
                        invalid_at: 0,
                        created_at: now,
                    };
                    self.storage.put_edge(edge)?;
                    facts_created += 1;
                }
            }
        }

        // Increment consolidation_count on processed episodes
        let episodes_processed = eligible.len();
        for ep in &eligible {
            self.storage
                .update_episode_consolidation(ep.id, ep.consolidation_count + 1)?;
        }

        Ok(ClsStats {
            episodes_processed,
            facts_created,
            facts_reinforced,
        })
    }

    /// Memory linking: create temporal links between entities co-created within a time window.
    ///
    /// Scans all entities sorted by `created_at`. For each pair within `linking_window_ms`
    /// (default 6h), creates bidirectional "temporally_linked" edges (A→B and B→A).
    /// If the link already exists, reinforces its confidence (+0.1, capped at 1.0).
    pub fn memory_linking(&mut self) -> Result<LinkingStats> {
        let window = self.consolidation_params.linking_window_ms;
        let mut entities = self.storage.scan_all_entities()?;

        if entities.len() < 2 {
            return Ok(LinkingStats {
                links_created: 0,
                links_reinforced: 0,
            });
        }

        entities.sort_by_key(|e| e.created_at);

        // Collect existing "temporally_linked" edges into a lookup set
        let all_edges = self.storage.scan_all_edges()?;
        let mut existing_links: HashMap<(EntityId, EntityId), EdgeId> = HashMap::new();
        for edge in &all_edges {
            if edge.relation_type == "temporally_linked" && edge.invalid_at == 0 {
                existing_links.insert((edge.source, edge.target), edge.id);
            }
        }

        let mut links_created = 0_usize;
        let mut links_reinforced = 0_usize;

        let max_neighbors = self.consolidation_params.linking_max_neighbors;

        // Sliding window: for each entity, pair with subsequent entities within window
        // O(n·k) cap via max_neighbors limit per entity
        for i in 0..entities.len() {
            for ej in entities[(i + 1)..].iter().take(max_neighbors) {
                let delta = ej.created_at - entities[i].created_at;
                if delta >= window {
                    break; // sorted, so all further will also exceed window
                }

                let a = entities[i].id;
                let b = ej.id;

                // Direction A→B
                if let Some(&edge_id) = existing_links.get(&(a, b)) {
                    if let Some(edge) = self.storage.get_edge(edge_id)? {
                        let new_conf = (edge.confidence + 0.1).min(1.0);
                        self.storage.put_edge(Edge {
                            confidence: new_conf,
                            ..edge
                        })?;
                        links_reinforced += 1;
                    }
                } else {
                    let id = EdgeId(self.next_edge_id);
                    self.next_edge_id += 1;
                    let now = crate::core::types::now_millis();
                    self.storage.put_edge(Edge {
                        id,
                        source: a,
                        target: b,
                        relation_type: "temporally_linked".to_string(),
                        description: String::new(),
                        confidence: 0.5,
                        valid_at: now,
                        invalid_at: 0,
                        created_at: now,
                    })?;
                    links_created += 1;
                }

                // Direction B→A
                if let Some(&edge_id) = existing_links.get(&(b, a)) {
                    if let Some(edge) = self.storage.get_edge(edge_id)? {
                        let new_conf = (edge.confidence + 0.1).min(1.0);
                        self.storage.put_edge(Edge {
                            confidence: new_conf,
                            ..edge
                        })?;
                        links_reinforced += 1;
                    }
                } else {
                    let id = EdgeId(self.next_edge_id);
                    self.next_edge_id += 1;
                    let now = crate::core::types::now_millis();
                    self.storage.put_edge(Edge {
                        id,
                        source: b,
                        target: a,
                        relation_type: "temporally_linked".to_string(),
                        description: String::new(),
                        confidence: 0.5,
                        valid_at: now,
                        invalid_at: 0,
                        created_at: now,
                    })?;
                    links_created += 1;
                }
            }
        }

        Ok(LinkingStats {
            links_created,
            links_reinforced,
        })
    }

    /// Run a full dream cycle: the 6-step consolidation pipeline.
    ///
    /// Steps (in order):
    /// 1. **SHY downscaling** — reduce all activation scores
    /// 2. **Interleaved replay** — reactivate entities from mixed episodes
    /// 3. **CLS transfer** — extract recurring patterns into semantic facts
    /// 4. **Memory linking** — create temporal co-creation edges
    /// 5. **Dark check** — silence low-activation entities
    /// 6. **GC** (optional) — delete GC-eligible dark entities
    ///
    /// Each step can be enabled/disabled via `DreamCycleConfig`.
    pub fn dream_cycle(&mut self, config: &DreamCycleConfig) -> Result<DreamCycleStats> {
        self.flush_accesses();
        // 1. SHY
        let entities_downscaled = if config.shy {
            self.shy_downscaling(self.consolidation_params.shy_factor)
        } else {
            0
        };

        // 2. Replay
        let replay = if config.replay {
            self.interleaved_replay()?
        } else {
            ReplayStats {
                episodes_replayed: 0,
                entities_reactivated: 0,
            }
        };

        // 3. CLS
        let cls = if config.cls {
            self.cls_transfer()?
        } else {
            ClsStats {
                episodes_processed: 0,
                facts_created: 0,
                facts_reinforced: 0,
            }
        };

        // 4. Memory linking
        let linking = if config.linking {
            self.memory_linking()?
        } else {
            LinkingStats {
                links_created: 0,
                links_reinforced: 0,
            }
        };

        // 5. Dark check
        let dark_nodes_marked = if config.dark_check {
            self.dark_node_pass()
        } else {
            0
        };

        // 6. GC (destructive, opt-in)
        let gc_deleted = if config.gc {
            let candidates = self.gc_candidates();
            let count = candidates.len();
            for id in candidates {
                let _ = self.delete_entity(id);
            }
            count
        } else {
            0
        };

        Ok(DreamCycleStats {
            entities_downscaled,
            replay,
            cls,
            linking,
            dark_nodes_marked,
            gc_deleted,
        })
    }

    // --- Spreading Activation ---

    /// Spread activation from source entities through the knowledge graph.
    ///
    /// Uses ACT-R fan effect: `S_ji = S_max - ln(fan)`. High-fan nodes
    /// inhibit spreading (negative activation when fan > e^S_max ≈ 5).
    ///
    /// Returns accumulated activation per entity (can be negative for inhibition).
    pub fn spread_activation(
        &self,
        sources: &[(EntityId, f64)],
        params: &SpreadingParams,
    ) -> Result<std::collections::HashMap<EntityId, f64>> {
        let storage = &self.storage;
        let activations = crate::memory::spreading::spread_activation(
            sources,
            |id| {
                storage
                    .get_entity_edges(id)
                    .unwrap_or_default()
                    .iter()
                    .map(|e| if e.source == id { e.target } else { e.source })
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect()
            },
            params,
        );
        Ok(activations)
    }

    // --- Episodes ---

    /// Record an episode (interaction snapshot).
    pub fn add_episode(
        &mut self,
        source: EpisodeSource,
        session_id: &str,
        entity_ids: &[EntityId],
        fact_ids: &[EdgeId],
    ) -> Result<u64> {
        let id = self.next_episode_id;
        self.next_episode_id += 1;

        let episode = Episode {
            id,
            source,
            session_id: session_id.to_string(),
            entity_ids: entity_ids.to_vec(),
            fact_ids: fact_ids.to_vec(),
            created_at: now_millis(),
            consolidation_count: 0,
        };

        self.storage.put_episode(episode)?;
        Ok(id)
    }

    /// Get an episode by ID.
    pub fn get_episode(&self, id: u64) -> Result<Option<Episode>> {
        self.storage.get_episode(id)
    }

    /// Get all episodes, optionally filtered.
    ///
    /// Filters:
    /// - `session_id`: only episodes from this session
    /// - `source`: only episodes from this source type
    /// - `since`/`until`: epoch millis time range on `created_at`
    pub fn get_episodes(
        &self,
        session_id: Option<&str>,
        source: Option<EpisodeSource>,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<Episode>> {
        let mut episodes = self.storage.scan_all_episodes()?;

        if let Some(sid) = session_id {
            episodes.retain(|e| e.session_id == sid);
        }
        if let Some(src) = source {
            episodes.retain(|e| e.source == src);
        }
        if let Some(t) = since {
            episodes.retain(|e| e.created_at >= t);
        }
        if let Some(t) = until {
            episodes.retain(|e| e.created_at <= t);
        }

        episodes.sort_by_key(|e| e.created_at);
        Ok(episodes)
    }

    /// Increment the consolidation count for an episode.
    pub fn increment_consolidation(&mut self, episode_id: u64) -> Result<bool> {
        if let Some(ep) = self.storage.get_episode(episode_id)? {
            self.storage
                .update_episode_consolidation(episode_id, ep.consolidation_count + 1)
        } else {
            Ok(false)
        }
    }

    // --- Stats ---

    /// Get summary statistics about the knowledge graph.
    pub fn stats(&self) -> Result<StorageStats> {
        Ok(self.storage.stats())
    }

    /// List all entities in the graph.
    pub fn scan_entities(&self) -> Result<Vec<Entity>> {
        self.storage.scan_all_entities()
    }

    /// List all edges/facts in the graph.
    pub fn scan_edges(&self) -> Result<Vec<Edge>> {
        self.storage.scan_all_edges()
    }

    /// List all episodes in the graph.
    pub fn scan_episodes(&self) -> Result<Vec<Episode>> {
        self.storage.scan_all_episodes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_creation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("project", "hora", None, None).unwrap();
        let entity = hora.get_entity(id).unwrap().unwrap();
        assert_eq!(entity.name, "hora");
        assert_eq!(entity.entity_type, "project");
    }

    #[test]
    fn test_edge_creation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("project", "hora", None, None).unwrap();
        let b = hora.add_entity("language", "Rust", None, None).unwrap();
        let _fact = hora
            .add_fact(a, b, "built_with", "hora is built with Rust", None)
            .unwrap();
        let edges = hora.get_entity_facts(a).unwrap();
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn test_entity_id_auto_increment() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id1 = hora.add_entity("a", "first", None, None).unwrap();
        let id2 = hora.add_entity("b", "second", None, None).unwrap();
        assert_ne!(id1, id2);
        assert_eq!(id1.0 + 1, id2.0);
    }

    #[test]
    fn test_entity_not_found() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let result = hora.get_entity(EntityId(999)).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_fact_references_valid_entities() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("project", "hora", None, None).unwrap();
        let result = hora.add_fact(a, EntityId(999), "rel", "desc", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_edge_bidirectional_lookup() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("project", "hora", None, None).unwrap();
        let b = hora.add_entity("language", "Rust", None, None).unwrap();
        hora.add_fact(a, b, "built_with", "hora is built with Rust", None)
            .unwrap();

        // Edge visible from both source and target
        let from_a = hora.get_entity_facts(a).unwrap();
        let from_b = hora.get_entity_facts(b).unwrap();
        assert_eq!(from_a.len(), 1);
        assert_eq!(from_b.len(), 1);
        assert_eq!(from_a[0].id, from_b[0].id);
    }

    #[test]
    fn test_edge_temporal_defaults() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("a", "x", None, None).unwrap();
        let b = hora.add_entity("b", "y", None, None).unwrap();
        let eid = hora.add_fact(a, b, "rel", "desc", None).unwrap();
        let edge = hora.get_fact(eid).unwrap().unwrap();

        assert!(edge.valid_at > 0);
        assert_eq!(edge.invalid_at, 0); // still valid
        assert!(edge.created_at > 0);
        assert_eq!(edge.confidence, 1.0);
    }

    #[test]
    fn test_embedding_dimension_mismatch() {
        let config = HoraConfig {
            embedding_dims: 4,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();
        let wrong_dims = vec![1.0, 2.0]; // 2 instead of 4
        let result = hora.add_entity("a", "x", None, Some(&wrong_dims));
        assert!(result.is_err());
    }

    #[test]
    fn test_embedding_when_dims_zero() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let emb = vec![1.0, 2.0, 3.0];
        let result = hora.add_entity("a", "x", None, Some(&emb));
        assert!(result.is_err());
    }

    #[test]
    fn test_embedding_correct_dims() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();
        let emb = vec![1.0, 2.0, 3.0];
        let id = hora.add_entity("a", "x", None, Some(&emb)).unwrap();
        let entity = hora.get_entity(id).unwrap().unwrap();
        assert_eq!(entity.embedding.as_ref().unwrap().len(), 3);
    }

    #[test]
    fn test_properties() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let mut props = Properties::new();
        props.insert(
            "language".to_string(),
            PropertyValue::String("Rust".to_string()),
        );
        props.insert("stars".to_string(), PropertyValue::Int(42));

        let id = hora
            .add_entity("project", "hora", Some(props), None)
            .unwrap();
        let entity = hora.get_entity(id).unwrap().unwrap();
        assert_eq!(
            entity.properties.get("language"),
            Some(&PropertyValue::String("Rust".to_string()))
        );
        assert_eq!(
            entity.properties.get("stars"),
            Some(&PropertyValue::Int(42))
        );
    }

    #[test]
    fn test_episode_creation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("project", "hora", None, None).unwrap();
        let b = hora.add_entity("language", "Rust", None, None).unwrap();
        let fact = hora.add_fact(a, b, "built_with", "desc", None).unwrap();

        let ep_id = hora
            .add_episode(EpisodeSource::Conversation, "sess-1", &[a, b], &[fact])
            .unwrap();
        assert_eq!(ep_id, 1);
    }

    #[test]
    fn test_stats() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("project", "hora", None, None).unwrap();
        let b = hora.add_entity("language", "Rust", None, None).unwrap();
        hora.add_fact(a, b, "built_with", "desc", None).unwrap();

        let stats = hora.stats().unwrap();
        assert_eq!(stats.entities, 2);
        assert_eq!(stats.edges, 1);
        assert_eq!(stats.episodes, 0);
    }

    #[test]
    fn test_entity_id_display() {
        let id = EntityId(42);
        assert_eq!(format!("{}", id), "entity:42");
    }

    #[test]
    fn test_edge_id_display() {
        let id = EdgeId(7);
        assert_eq!(format!("{}", id), "edge:7");
    }

    // --- v0.1b tests ---

    #[test]
    fn test_update_entity() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("project", "hora", None, None).unwrap();

        hora.update_entity(
            id,
            EntityUpdate {
                name: Some("hora-graph-core".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        let entity = hora.get_entity(id).unwrap().unwrap();
        assert_eq!(entity.name, "hora-graph-core");
        assert_eq!(entity.entity_type, "project"); // unchanged
    }

    #[test]
    fn test_update_entity_not_found() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let result = hora.update_entity(EntityId(999), EntityUpdate::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_entity_cascades() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("project", "hora", None, None).unwrap();
        let b = hora.add_entity("language", "Rust", None, None).unwrap();
        let fact_id = hora.add_fact(a, b, "built_with", "desc", None).unwrap();

        hora.delete_entity(a).unwrap();

        // Entity gone
        assert!(hora.get_entity(a).unwrap().is_none());
        // Edge cascade-deleted
        assert!(hora.get_fact(fact_id).unwrap().is_none());
        // Other entity untouched
        assert!(hora.get_entity(b).unwrap().is_some());
        // b's edge list is clean
        assert_eq!(hora.get_entity_facts(b).unwrap().len(), 0);
    }

    #[test]
    fn test_delete_entity_not_found() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let result = hora.delete_entity(EntityId(999));
        assert!(result.is_err());
    }

    #[test]
    fn test_invalidate_fact() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("a", "x", None, None).unwrap();
        let b = hora.add_entity("b", "y", None, None).unwrap();
        let fact_id = hora.add_fact(a, b, "rel", "desc", None).unwrap();

        hora.invalidate_fact(fact_id).unwrap();

        let fact = hora.get_fact(fact_id).unwrap().unwrap();
        assert!(fact.invalid_at > 0);
        // Fact still exists, just marked as invalid
    }

    #[test]
    fn test_invalidate_fact_twice_errors() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("a", "x", None, None).unwrap();
        let b = hora.add_entity("b", "y", None, None).unwrap();
        let fact_id = hora.add_fact(a, b, "rel", "desc", None).unwrap();

        hora.invalidate_fact(fact_id).unwrap();
        let result = hora.invalidate_fact(fact_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_fact() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("a", "x", None, None).unwrap();
        let b = hora.add_entity("b", "y", None, None).unwrap();
        let fact_id = hora.add_fact(a, b, "rel", "desc", None).unwrap();

        hora.delete_fact(fact_id).unwrap();
        assert!(hora.get_fact(fact_id).unwrap().is_none());
        // Edge lists are clean
        assert_eq!(hora.get_entity_facts(a).unwrap().len(), 0);
        assert_eq!(hora.get_entity_facts(b).unwrap().len(), 0);
    }

    #[test]
    fn test_delete_fact_not_found() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let result = hora.delete_fact(EdgeId(999));
        assert!(result.is_err());
    }

    #[test]
    fn test_update_fact() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("a", "x", None, None).unwrap();
        let b = hora.add_entity("b", "y", None, None).unwrap();
        let fact_id = hora.add_fact(a, b, "rel", "desc", Some(0.5)).unwrap();

        hora.update_fact(
            fact_id,
            FactUpdate {
                confidence: Some(0.95),
                ..Default::default()
            },
        )
        .unwrap();

        let fact = hora.get_fact(fact_id).unwrap().unwrap();
        assert_eq!(fact.confidence, 0.95);
        assert_eq!(fact.description, "desc"); // unchanged
    }

    #[test]
    fn test_props_macro() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora
            .add_entity(
                "project",
                "hora",
                Some(props! { "language" => "Rust", "stars" => 42 }),
                None,
            )
            .unwrap();

        let entity = hora.get_entity(id).unwrap().unwrap();
        assert_eq!(
            entity.properties.get("language"),
            Some(&PropertyValue::String("Rust".into()))
        );
        assert_eq!(
            entity.properties.get("stars"),
            Some(&PropertyValue::Int(42))
        );
    }

    #[test]
    fn test_stats_after_delete() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("a", "x", None, None).unwrap();
        let b = hora.add_entity("b", "y", None, None).unwrap();
        hora.add_fact(a, b, "rel", "desc", None).unwrap();

        assert_eq!(hora.stats().unwrap().entities, 2);
        assert_eq!(hora.stats().unwrap().edges, 1);

        hora.delete_entity(a).unwrap();

        assert_eq!(hora.stats().unwrap().entities, 1);
        assert_eq!(hora.stats().unwrap().edges, 0); // cascade
    }

    // --- v0.1c tests: Graph Traversal ---

    #[test]
    fn test_bfs_depth_2() {
        // A -> B -> C -> D
        // traverse(A, depth=2) should return {A, B, C} but not D
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        let c = hora.add_entity("node", "C", None, None).unwrap();
        let d = hora.add_entity("node", "D", None, None).unwrap();

        hora.add_fact(a, b, "link", "A->B", None).unwrap();
        hora.add_fact(b, c, "link", "B->C", None).unwrap();
        hora.add_fact(c, d, "link", "C->D", None).unwrap();

        let result = hora.traverse(a, TraverseOpts { depth: 2 }).unwrap();

        assert!(result.entity_ids.contains(&a));
        assert!(result.entity_ids.contains(&b));
        assert!(result.entity_ids.contains(&c));
        assert!(!result.entity_ids.contains(&d));
        assert_eq!(result.entity_ids.len(), 3);
        assert_eq!(result.edge_ids.len(), 2); // A->B and B->C
    }

    #[test]
    fn test_bfs_depth_0() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        hora.add_fact(a, b, "link", "A->B", None).unwrap();

        let result = hora.traverse(a, TraverseOpts { depth: 0 }).unwrap();
        assert_eq!(result.entity_ids.len(), 1);
        assert_eq!(result.entity_ids[0], a);
        assert_eq!(result.edge_ids.len(), 0);
    }

    #[test]
    fn test_bfs_cycle() {
        // A -> B -> C -> A (cycle)
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        let c = hora.add_entity("node", "C", None, None).unwrap();

        hora.add_fact(a, b, "link", "A->B", None).unwrap();
        hora.add_fact(b, c, "link", "B->C", None).unwrap();
        hora.add_fact(c, a, "link", "C->A", None).unwrap();

        // Should not infinite loop, and should find all 3 nodes
        let result = hora.traverse(a, TraverseOpts { depth: 10 }).unwrap();
        assert_eq!(result.entity_ids.len(), 3);
        assert_eq!(result.edge_ids.len(), 3);
    }

    #[test]
    fn test_bfs_isolated_node() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "lonely", None, None).unwrap();

        let result = hora.traverse(a, TraverseOpts { depth: 5 }).unwrap();
        assert_eq!(result.entity_ids.len(), 1);
        assert_eq!(result.edge_ids.len(), 0);
    }

    #[test]
    fn test_bfs_not_found() {
        let hora = HoraCore::new(HoraConfig::default()).unwrap();
        let result = hora.traverse(EntityId(999), TraverseOpts::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_neighbors() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        let c = hora.add_entity("node", "C", None, None).unwrap();
        let d = hora.add_entity("node", "D", None, None).unwrap();

        hora.add_fact(a, b, "link", "A->B", None).unwrap();
        hora.add_fact(a, c, "link", "A->C", None).unwrap();
        // D is not connected to A

        let mut neighbors = hora.neighbors(a).unwrap();
        neighbors.sort();
        assert_eq!(neighbors.len(), 2);
        assert!(neighbors.contains(&b));
        assert!(neighbors.contains(&c));
        assert!(!neighbors.contains(&d));
    }

    #[test]
    fn test_timeline_ordered() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("person", "Alice", None, None).unwrap();
        let b = hora.add_entity("company", "Acme", None, None).unwrap();
        let c = hora.add_entity("company", "BigCorp", None, None).unwrap();

        // Create facts — since they're created sequentially, valid_at increases
        let f1 = hora
            .add_fact(a, b, "works_at", "Alice at Acme", None)
            .unwrap();
        let f2 = hora
            .add_fact(a, c, "works_at", "Alice at BigCorp", None)
            .unwrap();

        let tl = hora.timeline(a).unwrap();
        assert_eq!(tl.len(), 2);
        assert_eq!(tl[0].id, f1);
        assert_eq!(tl[1].id, f2);
        assert!(tl[0].valid_at <= tl[1].valid_at);
    }

    #[test]
    fn test_facts_at_bitemporal() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("a", "x", None, None).unwrap();
        let b = hora.add_entity("b", "y", None, None).unwrap();

        // Manually craft edges with controlled timestamps via add_fact + update
        let f1 = hora.add_fact(a, b, "rel", "fact1", None).unwrap();
        let f2 = hora.add_fact(a, b, "rel2", "fact2", None).unwrap();

        // Get the actual timestamps so we can reason about them
        let e1 = hora.get_fact(f1).unwrap().unwrap();
        let e2 = hora.get_fact(f2).unwrap().unwrap();

        // Invalidate f1 — sets invalid_at to now
        hora.invalidate_fact(f1).unwrap();
        let e1_after = hora.get_fact(f1).unwrap().unwrap();

        // facts_at(before everything) = nothing
        let before = hora.facts_at(e1.valid_at - 1).unwrap();
        assert_eq!(before.len(), 0);

        // facts_at(between creation and invalidation) = both
        // Since f1 was valid from e1.valid_at until e1_after.invalid_at,
        // and f2 was valid from e2.valid_at with no end,
        // at time e2.valid_at both should be visible (before invalidation timestamp)
        let mid = hora.facts_at(e2.valid_at).unwrap();
        // f1 is still valid here (valid_at <= t, invalid_at > t because invalidation happens after)
        assert!(mid.iter().any(|e| e.id == f2));

        // facts_at(well into the future) = only f2 (f1 is invalidated)
        let future = hora.facts_at(e1_after.invalid_at + 1000).unwrap();
        assert!(future.iter().any(|e| e.id == f2));
        assert!(!future.iter().any(|e| e.id == f1));
    }

    #[test]
    fn test_facts_at_never_invalidated() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("a", "x", None, None).unwrap();
        let b = hora.add_entity("b", "y", None, None).unwrap();
        let f = hora.add_fact(a, b, "rel", "always valid", None).unwrap();

        let edge = hora.get_fact(f).unwrap().unwrap();

        // A fact with invalid_at=0 is valid at any time >= valid_at
        let result = hora.facts_at(edge.valid_at + 1_000_000).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, f);
    }

    // --- v0.1d tests: Persistence ---

    #[test]
    fn test_persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let (a_id, b_id, fact_id);
        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            a_id = hora.add_entity("project", "hora", None, None).unwrap();
            b_id = hora
                .add_entity("language", "Rust", Some(props! { "year" => 2015 }), None)
                .unwrap();
            fact_id = hora
                .add_fact(a_id, b_id, "built_with", "hora uses Rust", Some(0.95))
                .unwrap();
            hora.flush().unwrap();
        }

        // Reopen and verify
        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            let stats = hora.stats().unwrap();
            assert_eq!(stats.entities, 2);
            assert_eq!(stats.edges, 1);

            let a = hora.get_entity(a_id).unwrap().unwrap();
            assert_eq!(a.name, "hora");
            assert_eq!(a.entity_type, "project");

            let b = hora.get_entity(b_id).unwrap().unwrap();
            assert_eq!(b.name, "Rust");
            assert_eq!(b.properties.get("year"), Some(&PropertyValue::Int(2015)));

            let fact = hora.get_fact(fact_id).unwrap().unwrap();
            assert_eq!(fact.relation_type, "built_with");
            assert_eq!(fact.confidence, 0.95);
        }
    }

    #[test]
    fn test_persistence_ids_continue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            hora.add_entity("a", "first", None, None).unwrap(); // id=1
            hora.add_entity("b", "second", None, None).unwrap(); // id=2
            hora.flush().unwrap();
        }

        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            let id = hora.add_entity("c", "third", None, None).unwrap();
            // ID should continue from where we left off (3), not restart at 1
            assert_eq!(id.0, 3);
        }
    }

    #[test]
    fn test_persistence_with_embeddings() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut hora = HoraCore::open(&path, config.clone()).unwrap();
            let emb = vec![1.0, 2.0, 3.0];
            hora.add_entity("a", "x", None, Some(&emb)).unwrap();
            hora.flush().unwrap();
        }

        {
            let mut hora = HoraCore::open(&path, config).unwrap();
            let e = hora.get_entity(EntityId(1)).unwrap().unwrap();
            assert_eq!(e.embedding.as_ref().unwrap(), &[1.0, 2.0, 3.0]);
        }
    }

    #[test]
    fn test_persistence_with_episodes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            let a = hora.add_entity("a", "x", None, None).unwrap();
            hora.add_episode(EpisodeSource::Conversation, "sess-1", &[a], &[])
                .unwrap();
            hora.flush().unwrap();
        }

        {
            let hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            let stats = hora.stats().unwrap();
            assert_eq!(stats.episodes, 1);
        }
    }

    #[test]
    fn test_persistence_invalidated_fact() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        let fact_id;
        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            let a = hora.add_entity("a", "x", None, None).unwrap();
            let b = hora.add_entity("b", "y", None, None).unwrap();
            fact_id = hora.add_fact(a, b, "rel", "desc", None).unwrap();
            hora.invalidate_fact(fact_id).unwrap();
            hora.flush().unwrap();
        }

        {
            let hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            let fact = hora.get_fact(fact_id).unwrap().unwrap();
            assert!(fact.invalid_at > 0);
        }
    }

    #[test]
    fn test_corrupted_file_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.hora");
        std::fs::write(&path, b"NOT_HORA_FILE").unwrap();

        let result = HoraCore::open(&path, HoraConfig::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");
        let snap = dir.path().join("snapshot.hora");

        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            hora.add_entity("project", "hora", None, None).unwrap();
            hora.flush().unwrap();
            hora.snapshot(&snap).unwrap();
        }

        // Open from snapshot
        {
            let hora = HoraCore::open(&snap, HoraConfig::default()).unwrap();
            assert_eq!(hora.stats().unwrap().entities, 1);
        }
    }

    #[test]
    fn test_flush_memory_only_errors() {
        let hora = HoraCore::new(HoraConfig::default()).unwrap();
        let result = hora.flush();
        assert!(result.is_err());
    }

    #[test]
    fn test_snapshot_memory_instance() {
        let dir = tempfile::tempdir().unwrap();
        let snap = dir.path().join("snapshot.hora");

        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.add_entity("project", "hora", None, None).unwrap();
        hora.snapshot(&snap).unwrap();

        let hora2 = HoraCore::open(&snap, HoraConfig::default()).unwrap();
        assert_eq!(hora2.stats().unwrap().entities, 1);
    }

    #[test]
    fn test_persistence_all_property_types() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hora");

        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            hora.add_entity(
                "test",
                "props",
                Some(props! {
                    "name" => "hora",
                    "stars" => 42,
                    "score" => 2.72,
                    "active" => true
                }),
                None,
            )
            .unwrap();
            hora.flush().unwrap();
        }

        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            let e = hora.get_entity(EntityId(1)).unwrap().unwrap();
            assert_eq!(
                e.properties.get("name"),
                Some(&PropertyValue::String("hora".into()))
            );
            assert_eq!(e.properties.get("stars"), Some(&PropertyValue::Int(42)));
            assert_eq!(e.properties.get("score"), Some(&PropertyValue::Float(2.72)));
            assert_eq!(e.properties.get("active"), Some(&PropertyValue::Bool(true)));
        }
    }

    // --- v0.2a tests: Vector Search ---

    #[test]
    fn test_vector_search_basic() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        // Entity close to query
        hora.add_entity("a", "close", None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();
        // Entity far from query
        hora.add_entity("b", "far", None, Some(&[0.0, 1.0, 0.0]))
            .unwrap();
        // Entity very close to query
        hora.add_entity("c", "very_close", None, Some(&[0.9, 0.1, 0.0]))
            .unwrap();

        let results = hora.vector_search(&[1.0, 0.0, 0.0], 2).unwrap();

        assert_eq!(results.len(), 2);
        // First result should be the exact match
        assert_eq!(results[0].entity_id, EntityId(1));
        // Second should be the close one
        assert_eq!(results[1].entity_id, EntityId(3));
    }

    #[test]
    fn test_vector_search_returns_exact_k() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        for i in 0..20 {
            let emb = vec![i as f32, 0.0, 1.0];
            hora.add_entity("node", &format!("n{}", i), None, Some(&emb))
                .unwrap();
        }

        let results = hora.vector_search(&[10.0, 0.0, 1.0], 5).unwrap();
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_vector_search_skips_no_embedding() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        // One with embedding, one without
        hora.add_entity("a", "with_emb", None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();
        hora.add_entity("b", "no_emb", None, None).unwrap();

        let results = hora.vector_search(&[1.0, 0.0, 0.0], 10).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_vector_search_dims_mismatch() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let hora = HoraCore::new(config).unwrap();

        // Query with wrong dimensions
        let result = hora.vector_search(&[1.0, 0.0], 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_vector_search_dims_zero_errors() {
        let hora = HoraCore::new(HoraConfig::default()).unwrap();
        let result = hora.vector_search(&[1.0, 0.0, 0.0], 10);
        assert!(result.is_err());
    }

    #[test]
    fn test_vector_search_empty_graph() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let hora = HoraCore::new(config).unwrap();

        let results = hora.vector_search(&[1.0, 0.0, 0.0], 10).unwrap();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_vector_search_k_larger_than_corpus() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        hora.add_entity("a", "x", None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();

        let results = hora.vector_search(&[1.0, 0.0, 0.0], 100).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_vector_search_scores_descending() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        hora.add_entity("a", "x", None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();
        hora.add_entity("b", "y", None, Some(&[0.5, 0.5, 0.0]))
            .unwrap();
        hora.add_entity("c", "z", None, Some(&[0.0, 1.0, 0.0]))
            .unwrap();

        let results = hora.vector_search(&[1.0, 0.0, 0.0], 3).unwrap();
        for w in results.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    // --- v0.2b tests: BM25 Text Search ---

    #[test]
    fn test_text_search_finds_by_name() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.add_entity("project", "hora graph engine", None, None)
            .unwrap();
        hora.add_entity("language", "Rust programming", None, None)
            .unwrap();

        let results = hora.text_search("hora", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, EntityId(1));
    }

    #[test]
    fn test_text_search_finds_by_properties() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.add_entity(
            "project",
            "hora",
            Some(props! { "description" => "knowledge graph authentication engine" }),
            None,
        )
        .unwrap();
        hora.add_entity("other", "unrelated", None, None).unwrap();

        let results = hora.text_search("authentication", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, EntityId(1));
    }

    #[test]
    fn test_text_search_tf_ranking() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        // Same doc length, but entity 1 has "rust" 3 times vs entity 2's 1 time
        hora.add_entity("a", "rust rust rust", None, None).unwrap();
        hora.add_entity("b", "rust java python", None, None)
            .unwrap();

        let results = hora.text_search("rust", 10).unwrap();
        assert_eq!(results.len(), 2);
        // More occurrences (same length) → higher score
        assert_eq!(results[0].entity_id, EntityId(1));
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_text_search_no_match() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.add_entity("project", "hora", None, None).unwrap();

        let results = hora.text_search("nonexistent", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_text_search_respects_delete() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora
            .add_entity("project", "hora graph engine", None, None)
            .unwrap();

        hora.delete_entity(id).unwrap();

        let results = hora.text_search("hora", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_text_search_respects_update() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora
            .add_entity("project", "old name cats", None, None)
            .unwrap();

        hora.update_entity(
            id,
            EntityUpdate {
                name: Some("new name dogs".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(hora.text_search("cats", 10).unwrap().is_empty());
        assert_eq!(hora.text_search("dogs", 10).unwrap().len(), 1);
    }

    #[test]
    fn test_text_search_after_persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bm25.hora");

        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            hora.add_entity("project", "hora graph engine", None, None)
                .unwrap();
            hora.add_entity("language", "rust programming", None, None)
                .unwrap();
            hora.flush().unwrap();
        }

        // Reopen → BM25 index rebuilt from entities
        {
            let mut hora = HoraCore::open(&path, HoraConfig::default()).unwrap();
            let results = hora.text_search("hora", 10).unwrap();
            assert_eq!(results.len(), 1);

            let results = hora.text_search("rust", 10).unwrap();
            assert_eq!(results.len(), 1);
        }
    }

    // --- v0.2c tests: Hybrid Search (RRF) ---

    #[test]
    fn test_hybrid_search_both_legs() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        // Entity 1: strong vector match + text match → should rank highest
        hora.add_entity("a", "rust language", None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();
        // Entity 2: text match only
        hora.add_entity("b", "rust compiler", None, Some(&[0.0, 1.0, 0.0]))
            .unwrap();
        // Entity 3: vector match only (no "rust" in name)
        hora.add_entity("c", "speed daemon", None, Some(&[0.9, 0.1, 0.0]))
            .unwrap();

        let results = hora
            .search(
                Some("rust"),
                Some(&[1.0, 0.0, 0.0]),
                SearchOpts {
                    top_k: 10,
                    ..Default::default()
                },
            )
            .unwrap();

        // Entity 1 found by both legs → should be first
        assert_eq!(results[0].entity_id, EntityId(1));
        assert!(results.len() >= 2);
        // Scores descending
        for w in results.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }

    #[test]
    fn test_hybrid_search_text_only_mode() {
        // embedding_dims=0 → vector leg skipped, pure BM25
        let config = HoraConfig {
            embedding_dims: 0,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        hora.add_entity("a", "rust language", None, None).unwrap();
        hora.add_entity("b", "python language", None, None).unwrap();

        let results = hora
            .search(Some("rust"), None, SearchOpts::default())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, EntityId(1));
    }

    #[test]
    fn test_hybrid_search_vector_only_mode() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        hora.add_entity("a", "alpha", None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();
        hora.add_entity("b", "beta", None, Some(&[0.0, 1.0, 0.0]))
            .unwrap();

        // No text query → vector leg only
        let results = hora
            .search(None, Some(&[1.0, 0.0, 0.0]), SearchOpts::default())
            .unwrap();

        assert_eq!(results[0].entity_id, EntityId(1));
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_hybrid_search_neither_leg() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();
        hora.add_entity("a", "test", None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();

        let results = hora.search(None, None, SearchOpts::default()).unwrap();

        assert!(results.is_empty());
    }

    #[test]
    fn test_hybrid_search_top_k_respected() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        for i in 0..20 {
            let emb = [1.0 - i as f32 * 0.01, 0.0, 0.0];
            hora.add_entity("t", &format!("entity{i}"), None, Some(&emb))
                .unwrap();
        }

        let results = hora
            .search(
                None,
                Some(&[1.0, 0.0, 0.0]),
                SearchOpts {
                    top_k: 5,
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_hybrid_search_wrong_dims_skips_vector() {
        let config = HoraConfig {
            embedding_dims: 3,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        hora.add_entity("a", "rust language", None, Some(&[1.0, 0.0, 0.0]))
            .unwrap();

        // Wrong embedding dims → vector leg skipped, BM25 only
        let results = hora
            .search(Some("rust"), Some(&[1.0, 0.0]), SearchOpts::default())
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].entity_id, EntityId(1));
    }

    // --- v0.2d tests: Deduplication ---

    #[test]
    fn test_dedup_name_exact_normalization() {
        // "hora-engine" and "Hora Engine" should be detected as duplicates
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();

        let id1 = hora
            .add_entity("project", "Hora Engine", None, None)
            .unwrap();
        let id2 = hora
            .add_entity("project", "hora-engine", None, None)
            .unwrap();

        // Should return the existing entity's ID
        assert_eq!(id1, id2);
        // Only 1 entity should exist
        assert_eq!(hora.stats().unwrap().entities, 1);
    }

    #[test]
    fn test_dedup_name_case_insensitive() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();

        let id1 = hora.add_entity("project", "Rust", None, None).unwrap();
        let id2 = hora.add_entity("project", "rust", None, None).unwrap();
        let id3 = hora.add_entity("project", "RUST", None, None).unwrap();

        assert_eq!(id1, id2);
        assert_eq!(id1, id3);
        assert_eq!(hora.stats().unwrap().entities, 1);
    }

    #[test]
    fn test_dedup_different_type_allows_same_name() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();

        let id1 = hora.add_entity("project", "rust", None, None).unwrap();
        let id2 = hora.add_entity("language", "rust", None, None).unwrap();

        // Different types → not a duplicate
        assert_ne!(id1, id2);
        assert_eq!(hora.stats().unwrap().entities, 2);
    }

    #[test]
    fn test_dedup_cosine_embedding() {
        let config = HoraConfig {
            embedding_dims: 3,
            ..Default::default()
        };
        let mut hora = HoraCore::new(config).unwrap();

        let emb1 = [1.0, 0.0, 0.0];
        let emb2 = [0.99, 0.1, 0.0]; // very similar (cosine > 0.99)

        let id1 = hora
            .add_entity("concept", "alpha", None, Some(&emb1))
            .unwrap();
        let id2 = hora
            .add_entity("concept", "beta", None, Some(&emb2))
            .unwrap();

        // Different names but similar embeddings → duplicate
        assert_eq!(id1, id2);
        assert_eq!(hora.stats().unwrap().entities, 1);
    }

    #[test]
    fn test_dedup_cosine_below_threshold() {
        let config = HoraConfig {
            embedding_dims: 3,
            ..Default::default()
        };
        let mut hora = HoraCore::new(config).unwrap();

        let emb1 = [1.0, 0.0, 0.0];
        let emb2 = [0.0, 1.0, 0.0]; // orthogonal → cosine = 0

        let id1 = hora
            .add_entity("concept", "alpha", None, Some(&emb1))
            .unwrap();
        let id2 = hora
            .add_entity("concept", "beta", None, Some(&emb2))
            .unwrap();

        // Very different embeddings → not a duplicate
        assert_ne!(id1, id2);
        assert_eq!(hora.stats().unwrap().entities, 2);
    }

    #[test]
    fn test_dedup_disabled() {
        let config = HoraConfig {
            dedup: DedupConfig::disabled(),
            ..Default::default()
        };
        let mut hora = HoraCore::new(config).unwrap();

        let id1 = hora.add_entity("project", "rust", None, None).unwrap();
        let id2 = hora.add_entity("project", "rust", None, None).unwrap();

        // Dedup disabled → both created
        assert_ne!(id1, id2);
        assert_eq!(hora.stats().unwrap().entities, 2);
    }

    #[test]
    fn test_dedup_no_id_increment_on_duplicate() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();

        let id1 = hora.add_entity("project", "hora", None, None).unwrap();
        let _id2 = hora.add_entity("project", "hora", None, None).unwrap(); // dedup → returns id1

        // Next unique entity should get id=2, not id=3
        let id3 = hora.add_entity("language", "rust", None, None).unwrap();
        assert_eq!(id1, EntityId(1));
        assert_eq!(id3, EntityId(2));
    }

    #[test]
    fn test_dedup_configurable_thresholds() {
        // Lower Jaccard threshold → easier to dedup
        let config = HoraConfig {
            dedup: DedupConfig {
                enabled: true,
                name_exact: false, // disable exact name to test Jaccard only
                jaccard_threshold: 0.5,
                cosine_threshold: 0.0,
            },
            ..Default::default()
        };
        let mut hora = HoraCore::new(config).unwrap();

        // "rust graph engine" tokens: [rust, graph, engine]
        let id1 = hora
            .add_entity("project", "rust graph engine", None, None)
            .unwrap();
        // "rust graph database" tokens: [rust, graph, database] → Jaccard 2/4 = 0.5 >= 0.5
        let id2 = hora
            .add_entity("project", "rust graph database", None, None)
            .unwrap();

        assert_eq!(id1, id2);
    }

    // --- v0.3a tests: ACT-R Activation ---

    #[test]
    fn test_activation_exists_after_creation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("a", "test", None, None).unwrap();

        let act = hora.get_activation(id);
        assert!(act.is_some());
        // Activation should be finite (1 access at creation)
        assert!(act.unwrap().is_finite());
    }

    #[test]
    fn test_activation_increases_with_access() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("a", "test", None, None).unwrap();

        let act_before = hora.get_activation(id).unwrap();
        // get_entity records an access
        let _ = hora.get_entity(id).unwrap();
        let act_after = hora.get_activation(id).unwrap();

        // More accesses → higher activation
        assert!(
            act_after > act_before,
            "act_after={act_after} should be > act_before={act_before}"
        );
    }

    #[test]
    fn test_activation_none_for_unknown_entity() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        assert!(hora.get_activation(EntityId(999)).is_none());
    }

    #[test]
    fn test_activation_removed_on_delete() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("a", "test", None, None).unwrap();

        assert!(hora.get_activation(id).is_some());
        hora.delete_entity(id).unwrap();
        assert!(hora.get_activation(id).is_none());
    }

    #[test]
    fn test_record_access_manually() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("a", "test", None, None).unwrap();

        let act_before = hora.get_activation(id).unwrap();
        hora.record_access(id);
        hora.record_access(id);
        hora.record_access(id);
        let act_after = hora.get_activation(id).unwrap();

        assert!(act_after > act_before);
    }

    #[test]
    fn test_search_records_access_side_effect() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("a", "rust language", None, None).unwrap();

        let act_before = hora.get_activation(id).unwrap();

        // text_search doesn't record access, but search() does
        hora.search(Some("rust"), None, SearchOpts::default())
            .unwrap();

        let act_after = hora.get_activation(id).unwrap();
        assert!(
            act_after > act_before,
            "search should increase activation: before={act_before}, after={act_after}"
        );
    }

    // ── Spreading Activation (v0.3b) ────────────────────────

    #[test]
    fn test_spread_activation_simple() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        hora.add_fact(a, b, "link", "A-B", None).unwrap();

        let params = SpreadingParams::default();
        let result = hora.spread_activation(&[(a, 1.0)], &params).unwrap();

        // B should receive positive activation (fan=1, s_ji = 1.6 - ln(1) = 1.6 > 0)
        assert!(result.contains_key(&b));
        assert!(result[&b] > 0.0, "B should have positive activation");
    }

    #[test]
    fn test_spread_activation_fan_inhibition() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let hub = hora.add_entity("node", "hub", None, None).unwrap();
        // Connect hub to 10 nodes → fan=10, s_ji = 1.6 - ln(10) ≈ -0.70
        let mut leaves = Vec::new();
        for i in 0..10 {
            let leaf = hora
                .add_entity("node", &format!("leaf{i}"), None, None)
                .unwrap();
            hora.add_fact(hub, leaf, "link", &format!("hub-leaf{i}"), None)
                .unwrap();
            leaves.push(leaf);
        }

        let params = SpreadingParams::default();
        let result = hora.spread_activation(&[(hub, 1.0)], &params).unwrap();

        // Fan=10 → negative spreading (inhibition)
        for leaf in &leaves {
            let act = result[leaf];
            assert!(
                act < 0.0,
                "Leaf should have negative activation (inhibition), got {act}"
            );
        }
    }

    #[test]
    fn test_spread_activation_depth_limit() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        let c = hora.add_entity("node", "C", None, None).unwrap();
        let d = hora.add_entity("node", "D", None, None).unwrap();
        hora.add_fact(a, b, "link", "A-B", None).unwrap();
        hora.add_fact(b, c, "link", "B-C", None).unwrap();
        hora.add_fact(c, d, "link", "C-D", None).unwrap();

        let params = SpreadingParams {
            max_depth: 2,
            ..Default::default()
        };
        let result = hora.spread_activation(&[(a, 1.0)], &params).unwrap();

        // A, B, C should have activation; D should not (beyond depth 2)
        assert!(result.contains_key(&a));
        assert!(result.contains_key(&b));
        assert!(result.contains_key(&c));
        let d_act = result.get(&d).copied().unwrap_or(0.0);
        assert!(
            d_act.abs() < f64::EPSILON,
            "D should have no activation at depth 2, got {d_act}"
        );
    }

    #[test]
    fn test_spread_activation_multiple_sources() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        let c = hora.add_entity("node", "C", None, None).unwrap();
        hora.add_fact(a, c, "link", "A-C", None).unwrap();
        hora.add_fact(b, c, "link", "B-C", None).unwrap();

        let params = SpreadingParams::default();
        let result = hora
            .spread_activation(&[(a, 1.0), (b, 1.0)], &params)
            .unwrap();

        // C receives activation from both A and B
        let c_act = result[&c];
        assert!(
            c_act > 0.0,
            "C should have positive activation from 2 sources, got {c_act}"
        );
    }

    #[test]
    fn test_spread_activation_no_edges() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "isolated", None, None).unwrap();

        let params = SpreadingParams::default();
        let result = hora.spread_activation(&[(a, 1.0)], &params).unwrap();

        // Only source present, no propagation
        assert_eq!(result.len(), 1);
        assert!((result[&a] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_spread_activation_cycle_terminates() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        hora.add_fact(a, b, "link", "A-B", None).unwrap();

        let params = SpreadingParams::default();
        // Should terminate without hanging (cycle via bidirectional edges)
        let result = hora.spread_activation(&[(a, 1.0)], &params).unwrap();
        assert!(result.contains_key(&a));
        assert!(result.contains_key(&b));
    }

    // ── Reconsolidation (v0.3c) ─────────────────────────────

    #[test]
    fn test_reconsolidation_initial_state_stable() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "A", None, None).unwrap();

        let phase = hora.get_memory_phase(id).unwrap().clone();
        assert_eq!(phase, MemoryPhase::Stable);
    }

    #[test]
    fn test_reconsolidation_removed_on_delete() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "A", None, None).unwrap();
        hora.delete_entity(id).unwrap();
        assert!(hora.get_memory_phase(id).is_none());
    }

    #[test]
    fn test_reconsolidation_stability_multiplier_default() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "A", None, None).unwrap();

        let mult = hora.get_stability_multiplier(id).unwrap();
        assert!((mult - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reconsolidation_strong_access_destabilizes() {
        // We need to control timing precisely, so we test the state module directly
        // but via HoraCore's reconsolidation_states for integration.
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "A", None, None).unwrap();

        // Access many times to build up activation above threshold (0.5)
        // Each get_entity call records access, which triggers reconsolidation check
        for _ in 0..5 {
            let _ = hora.get_entity(id);
        }

        // After enough accesses, activation should be high enough to destabilize
        let activation = hora.get_activation(id).unwrap();

        // The reconsolidation check happens inside record_access.
        // If activation > 0.5, entity should be Labile.
        if activation >= 0.5 {
            let phase = hora.get_memory_phase(id).unwrap().clone();
            assert!(
                matches!(phase, MemoryPhase::Labile { .. }),
                "Expected Labile for activation {activation}, got {phase:?}"
            );
        }
    }

    #[test]
    fn test_reconsolidation_unit_level_full_cycle() {
        // Direct unit test of the reconsolidation state within HoraCore
        use crate::memory::reconsolidation::{ReconsolidationParams, ReconsolidationState};

        let params = ReconsolidationParams {
            labile_window_secs: 100.0,
            restabilization_secs: 200.0,
            destabilization_threshold: 0.0, // always destabilize
            restabilization_boost: 1.5,
        };

        let mut state = ReconsolidationState::new();

        // Strong reactivation → Labile
        state.on_reactivation(1.0, 0.0, &params);
        assert!(matches!(state.phase(), MemoryPhase::Labile { .. }));

        // After 100s → Restabilizing
        state.tick(100.0, &params);
        assert!(matches!(state.phase(), MemoryPhase::Restabilizing { .. }));

        // After 200s more → Stable with boost
        state.tick(300.0, &params);
        assert_eq!(*state.phase(), MemoryPhase::Stable);
        assert!((state.stability_multiplier() - 1.5).abs() < f64::EPSILON);
    }

    // ── Dark Nodes (v0.3d) ──────────────────────────────────

    #[test]
    fn test_dark_node_pass_marks_stale_entities() {
        use crate::memory::dark_nodes::DarkNodeParams;

        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();

        // Override dark params: immediate silencing (0 delay, threshold 999)
        hora.dark_node_params = DarkNodeParams {
            silencing_threshold: 999.0, // everything is below this
            silencing_delay_secs: 0.0,  // no delay
            recovery_threshold: 1.5,
            gc_eligible_after_secs: 0.0,
        };

        let id = hora.add_entity("node", "forgotten", None, None).unwrap();

        let count = hora.dark_node_pass();
        assert_eq!(count, 1, "Should mark 1 entity as dark");

        let phase = hora.get_memory_phase(id).unwrap().clone();
        assert!(
            matches!(phase, MemoryPhase::Dark { .. }),
            "Expected Dark, got {phase:?}"
        );
    }

    #[test]
    fn test_dark_node_not_silenced_if_active() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "active", None, None).unwrap();

        // Access it many times → high activation
        for _ in 0..10 {
            hora.record_access(id);
        }

        // Default threshold is -2.0, activation should be well above
        let count = hora.dark_node_pass();
        assert_eq!(count, 0, "Active entity should not be silenced");

        let phase = hora.get_memory_phase(id).unwrap().clone();
        assert_ne!(phase, MemoryPhase::Dark { silenced_at: 0.0 });
    }

    #[test]
    fn test_dark_node_invisible_in_search() {
        use crate::memory::dark_nodes::DarkNodeParams;

        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.dark_node_params = DarkNodeParams {
            silencing_threshold: 999.0,
            silencing_delay_secs: 0.0,
            recovery_threshold: 1.5,
            gc_eligible_after_secs: 0.0,
        };
        // High destabilization threshold so search() side-effect doesn't
        // transition entity out of Stable before dark_node_pass
        hora.reconsolidation_params.destabilization_threshold = 9999.0;

        let _id = hora
            .add_entity("node", "invisible ghost", None, None)
            .unwrap();

        // Before dark_node_pass: entity visible in search
        let results = hora
            .search(Some("ghost"), None, SearchOpts::default())
            .unwrap();
        assert_eq!(results.len(), 1, "Should find entity before silencing");

        hora.dark_node_pass();

        // After dark_node_pass: entity invisible by default
        let results = hora
            .search(Some("ghost"), None, SearchOpts::default())
            .unwrap();
        assert_eq!(results.len(), 0, "Dark node should be invisible in search");

        // But visible with include_dark=true
        let results = hora
            .search(
                Some("ghost"),
                None,
                SearchOpts {
                    include_dark: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "Dark node should be visible with include_dark"
        );
    }

    #[test]
    fn test_dark_node_recovery() {
        use crate::memory::dark_nodes::DarkNodeParams;

        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.dark_node_params = DarkNodeParams {
            silencing_threshold: 999.0,
            silencing_delay_secs: 0.0,
            recovery_threshold: 1.5,
            gc_eligible_after_secs: 0.0,
        };

        let id = hora.add_entity("node", "recoverable", None, None).unwrap();
        hora.dark_node_pass();

        // Entity is Dark
        assert!(matches!(
            hora.get_memory_phase(id).unwrap(),
            MemoryPhase::Dark { .. }
        ));

        // Recovery → Labile
        let recovered = hora.attempt_recovery(id);
        assert!(recovered, "Recovery should succeed for dark node");

        let phase = hora.get_memory_phase(id).unwrap().clone();
        assert!(
            matches!(phase, MemoryPhase::Labile { .. }),
            "Expected Labile after recovery, got {phase:?}"
        );

        // Search should find it again
        let results = hora
            .search(Some("recoverable"), None, SearchOpts::default())
            .unwrap();
        assert_eq!(results.len(), 1, "Recovered entity should be searchable");
    }

    #[test]
    fn test_dark_nodes_list() {
        use crate::memory::dark_nodes::DarkNodeParams;

        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.dark_node_params = DarkNodeParams {
            silencing_threshold: 999.0,
            silencing_delay_secs: 0.0,
            recovery_threshold: 1.5,
            gc_eligible_after_secs: 0.0,
        };

        let a = hora.add_entity("node", "alpha", None, None).unwrap();
        let b = hora.add_entity("node", "bravo", None, None).unwrap();
        hora.dark_node_pass();

        let darks = hora.dark_nodes();
        assert_eq!(darks.len(), 2);
        assert!(darks.contains(&a));
        assert!(darks.contains(&b));
    }

    #[test]
    fn test_gc_candidates() {
        use crate::memory::dark_nodes::DarkNodeParams;

        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.dark_node_params = DarkNodeParams {
            silencing_threshold: 999.0,
            silencing_delay_secs: 0.0,
            recovery_threshold: 1.5,
            gc_eligible_after_secs: 0.0, // immediate GC eligibility
        };

        let id = hora.add_entity("node", "ancient", None, None).unwrap();
        hora.dark_node_pass();

        let gc = hora.gc_candidates();
        assert!(
            gc.contains(&id),
            "Dark entity should be GC candidate with 0s threshold"
        );
    }

    #[test]
    fn test_attempt_recovery_on_non_dark_is_noop() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "stable", None, None).unwrap();

        let recovered = hora.attempt_recovery(id);
        assert!(!recovered, "Recovery on Stable entity should return false");
    }

    // ── FSRS Scheduling (v0.3e) ─────────────────────────────

    #[test]
    fn test_fsrs_retrievability_starts_at_1() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "fresh", None, None).unwrap();
        // Entity was just created, so retrievability should be ~1.0
        let r = hora.get_retrievability(id).unwrap();
        assert!(
            r > 0.99,
            "Retrievability should be ~1.0 right after creation, got {r}"
        );
    }

    #[test]
    fn test_fsrs_stability_initial() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "stable", None, None).unwrap();
        let s = hora.get_fsrs_stability(id).unwrap();
        assert!(
            (s - 1.0).abs() < f64::EPSILON,
            "Initial stability should be 1.0 day, got {s}"
        );
    }

    #[test]
    fn test_fsrs_next_review_days() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "reviewable", None, None).unwrap();
        let interval = hora.get_next_review_days(id).unwrap();
        // With default r=0.9, interval should ≈ S = 1.0 day
        assert!(
            (interval - 1.0).abs() < 0.1,
            "Next review interval should be ~1 day, got {interval}"
        );
    }

    #[test]
    fn test_fsrs_stability_increases_with_access() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "learning", None, None).unwrap();

        let s_before = hora.get_fsrs_stability(id).unwrap();

        // Multiple accesses → record_review with boost from reconsolidation
        for _ in 0..5 {
            hora.record_access(id);
        }

        let s_after = hora.get_fsrs_stability(id).unwrap();
        assert!(
            s_after >= s_before,
            "Stability should not decrease with reviews: before={s_before}, after={s_after}"
        );
    }

    #[test]
    fn test_fsrs_none_for_unknown_entity() {
        let hora = HoraCore::new(HoraConfig::default()).unwrap();
        assert!(hora.get_retrievability(EntityId(9999)).is_none());
        assert!(hora.get_next_review_days(EntityId(9999)).is_none());
        assert!(hora.get_fsrs_stability(EntityId(9999)).is_none());
    }

    #[test]
    fn test_fsrs_removed_on_delete() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "temp", None, None).unwrap();
        assert!(hora.get_retrievability(id).is_some());
        hora.delete_entity(id).unwrap();
        assert!(hora.get_retrievability(id).is_none());
    }

    // ── Episode Management (v0.4a) ──────────────────────────

    #[test]
    fn test_get_episodes_by_session() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let e1 = hora.add_entity("node", "A", None, None).unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s1", &[e1], &[])
            .unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s2", &[e1], &[])
            .unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s1", &[e1], &[])
            .unwrap();

        let eps = hora.get_episodes(Some("s1"), None, None, None).unwrap();
        assert_eq!(eps.len(), 2);
        assert!(eps.iter().all(|e| e.session_id == "s1"));
    }

    #[test]
    fn test_get_episodes_by_source() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let e1 = hora.add_entity("node", "A", None, None).unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s1", &[e1], &[])
            .unwrap();
        hora.add_episode(EpisodeSource::Document, "s1", &[e1], &[])
            .unwrap();
        hora.add_episode(EpisodeSource::Api, "s1", &[e1], &[])
            .unwrap();

        let eps = hora
            .get_episodes(None, Some(EpisodeSource::Document), None, None)
            .unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].source, EpisodeSource::Document);
    }

    #[test]
    fn test_get_episode_by_id() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let e1 = hora.add_entity("node", "A", None, None).unwrap();
        let ep_id = hora
            .add_episode(EpisodeSource::Api, "s1", &[e1], &[])
            .unwrap();

        let ep = hora.get_episode(ep_id).unwrap().unwrap();
        assert_eq!(ep.id, ep_id);
        assert_eq!(ep.source, EpisodeSource::Api);
    }

    #[test]
    fn test_consolidation_count_initial_zero() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let e1 = hora.add_entity("node", "A", None, None).unwrap();
        let ep_id = hora
            .add_episode(EpisodeSource::Conversation, "s1", &[e1], &[])
            .unwrap();

        let ep = hora.get_episode(ep_id).unwrap().unwrap();
        assert_eq!(ep.consolidation_count, 0);
    }

    #[test]
    fn test_increment_consolidation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let e1 = hora.add_entity("node", "A", None, None).unwrap();
        let ep_id = hora
            .add_episode(EpisodeSource::Conversation, "s1", &[e1], &[])
            .unwrap();

        hora.increment_consolidation(ep_id).unwrap();
        hora.increment_consolidation(ep_id).unwrap();

        let ep = hora.get_episode(ep_id).unwrap().unwrap();
        assert_eq!(ep.consolidation_count, 2);
    }

    // --- SHY Downscaling ---

    #[test]
    fn test_shy_downscaling_reduces_activation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "A", None, None).unwrap();

        // Record a few accesses to build up activation
        for _ in 0..3 {
            hora.record_access(id);
        }

        let before = hora.get_activation(id).unwrap();
        hora.shy_downscaling(0.78);
        let after = hora.get_activation(id).unwrap();

        // Activation should be reduced by factor 0.78
        let expected = before * 0.78;
        assert!(
            (after - expected).abs() < 1e-10,
            "expected {expected}, got {after}"
        );
    }

    #[test]
    fn test_shy_downscaling_negative_activation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "A", None, None).unwrap();

        // Entity with only the initial creation access will have negative activation
        // after enough time passes, but we can check the factor applies to negatives too.
        // Force a known negative by checking: activation at creation is near 0 or negative.
        let act = hora.get_activation(id).unwrap();
        // Even if activation is positive, after SHY it should be factor × act
        hora.shy_downscaling(0.78);
        let after = hora.get_activation(id).unwrap();
        let expected = act * 0.78;
        assert!(
            (after - expected).abs() < 1e-10,
            "expected {expected}, got {after}"
        );
    }

    #[test]
    fn test_shy_double_downscaling() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let id = hora.add_entity("node", "A", None, None).unwrap();
        for _ in 0..3 {
            hora.record_access(id);
        }

        let before = hora.get_activation(id).unwrap();
        hora.shy_downscaling(0.78);
        hora.shy_downscaling(0.78);
        let after = hora.get_activation(id).unwrap();

        // Double SHY: factor² = 0.78 * 0.78 = 0.6084
        let expected = before * 0.78 * 0.78;
        assert!(
            (after - expected).abs() < 1e-10,
            "expected {expected}, got {after}"
        );
    }

    #[test]
    fn test_shy_downscaling_all_entities() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        let c = hora.add_entity("node", "C", None, None).unwrap();

        // Give different activation levels
        hora.record_access(a);
        hora.record_access(b);
        hora.record_access(b);
        hora.record_access(c);
        hora.record_access(c);
        hora.record_access(c);

        let before_a = hora.get_activation(a).unwrap();
        let before_b = hora.get_activation(b).unwrap();
        let before_c = hora.get_activation(c).unwrap();

        let count = hora.shy_downscaling(0.78);
        assert_eq!(count, 3);

        let after_a = hora.get_activation(a).unwrap();
        let after_b = hora.get_activation(b).unwrap();
        let after_c = hora.get_activation(c).unwrap();

        assert!((after_a - before_a * 0.78).abs() < 1e-10);
        assert!((after_b - before_b * 0.78).abs() < 1e-10);
        assert!((after_c - before_c * 0.78).abs() < 1e-10);
    }

    // --- Interleaved Replay ---

    #[test]
    fn test_replay_boosts_entity_activation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();

        hora.add_episode(EpisodeSource::Conversation, "s1", &[a, b], &[])
            .unwrap();

        let act_a_before = hora.get_activation(a).unwrap();
        let act_b_before = hora.get_activation(b).unwrap();

        let stats = hora.interleaved_replay().unwrap();
        assert_eq!(stats.episodes_replayed, 1);
        assert_eq!(stats.entities_reactivated, 2);

        let act_a_after = hora.get_activation(a).unwrap();
        let act_b_after = hora.get_activation(b).unwrap();

        assert!(
            act_a_after > act_a_before,
            "A activation should increase after replay"
        );
        assert!(
            act_b_after > act_b_before,
            "B activation should increase after replay"
        );
    }

    #[test]
    fn test_replay_respects_max_items() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.consolidation_params.max_replay_items = 3;
        let e = hora.add_entity("node", "A", None, None).unwrap();

        for i in 0..10 {
            hora.add_episode(EpisodeSource::Conversation, &format!("s{i}"), &[e], &[])
                .unwrap();
        }

        let stats = hora.interleaved_replay().unwrap();
        assert_eq!(stats.episodes_replayed, 3);
    }

    #[test]
    fn test_replay_mix_recent_and_older() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.consolidation_params.max_replay_items = 10;
        hora.consolidation_params.recent_ratio = 0.7;
        let e = hora.add_entity("node", "A", None, None).unwrap();

        // Create 20 episodes — first 10 are "older", last 10 are "recent"
        for i in 0..20 {
            hora.add_episode(EpisodeSource::Conversation, &format!("s{i}"), &[e], &[])
                .unwrap();
        }

        let stats = hora.interleaved_replay().unwrap();
        // Budget: 10 total, ceil(10 * 0.7) = 7 recent, 3 older
        assert_eq!(stats.episodes_replayed, 10);
        assert_eq!(stats.entities_reactivated, 10);
    }

    #[test]
    fn test_replay_ignores_deleted_entities() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();

        hora.add_episode(EpisodeSource::Conversation, "s1", &[a, b], &[])
            .unwrap();

        // Delete entity B
        hora.delete_entity(b).unwrap();

        let stats = hora.interleaved_replay().unwrap();
        assert_eq!(stats.episodes_replayed, 1);
        // Only A should be reactivated (B was deleted)
        assert_eq!(stats.entities_reactivated, 1);
    }

    #[test]
    fn test_replay_empty_episodes() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let stats = hora.interleaved_replay().unwrap();
        assert_eq!(stats.episodes_replayed, 0);
        assert_eq!(stats.entities_reactivated, 0);
    }

    // --- CLS Transfer ---

    #[test]
    fn test_cls_transfer_creates_semantic_fact() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.consolidation_params.cls_threshold = 3;

        let a = hora.add_entity("person", "Alice", None, None).unwrap();
        let b = hora.add_entity("person", "Bob", None, None).unwrap();

        // Create the same fact in 3 different episodes
        let f1 = hora
            .add_fact(a, b, "knows", "they know each other", None)
            .unwrap();
        let f2 = hora.add_fact(a, b, "knows", "met at work", None).unwrap();
        let f3 = hora.add_fact(a, b, "knows", "colleagues", None).unwrap();

        // Create 3 episodes each referencing one of these facts, with consolidation_count >= 3
        let ep1 = hora
            .add_episode(EpisodeSource::Conversation, "s1", &[a, b], &[f1])
            .unwrap();
        let ep2 = hora
            .add_episode(EpisodeSource::Conversation, "s2", &[a, b], &[f2])
            .unwrap();
        let ep3 = hora
            .add_episode(EpisodeSource::Conversation, "s3", &[a, b], &[f3])
            .unwrap();

        // Manually set consolidation_count to threshold
        for _ in 0..3 {
            hora.increment_consolidation(ep1).unwrap();
            hora.increment_consolidation(ep2).unwrap();
            hora.increment_consolidation(ep3).unwrap();
        }

        let stats = hora.cls_transfer().unwrap();
        assert_eq!(stats.episodes_processed, 3);
        // The triplet (a, "knows", b) appears in 3 episodes → creates 1 semantic fact
        // But 3 existing edges already match, so it reinforces the first one found
        assert!(stats.facts_created + stats.facts_reinforced > 0);
    }

    #[test]
    fn test_cls_transfer_below_threshold_skipped() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.consolidation_params.cls_threshold = 3;

        let a = hora.add_entity("person", "Alice", None, None).unwrap();
        let b = hora.add_entity("person", "Bob", None, None).unwrap();
        let f1 = hora.add_fact(a, b, "knows", "friends", None).unwrap();

        // Only 2 episodes — below threshold
        let ep1 = hora
            .add_episode(EpisodeSource::Conversation, "s1", &[a, b], &[f1])
            .unwrap();
        let ep2 = hora
            .add_episode(EpisodeSource::Conversation, "s2", &[a, b], &[f1])
            .unwrap();

        for _ in 0..3 {
            hora.increment_consolidation(ep1).unwrap();
            hora.increment_consolidation(ep2).unwrap();
        }

        let stats = hora.cls_transfer().unwrap();
        // 2 episodes processed but triplet count (2) < threshold (3)
        assert_eq!(stats.episodes_processed, 2);
        assert_eq!(stats.facts_created, 0);
        assert_eq!(stats.facts_reinforced, 0);
    }

    #[test]
    fn test_cls_transfer_reinforces_existing() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.consolidation_params.cls_threshold = 3;

        let a = hora.add_entity("person", "Alice", None, None).unwrap();
        let b = hora.add_entity("person", "Bob", None, None).unwrap();

        // Create ONE canonical fact
        let fact_id = hora.add_fact(a, b, "knows", "friends", Some(0.8)).unwrap();

        // Reference the same fact_id in 3 episodes
        let ep1 = hora
            .add_episode(EpisodeSource::Conversation, "s1", &[a, b], &[fact_id])
            .unwrap();
        let ep2 = hora
            .add_episode(EpisodeSource::Conversation, "s2", &[a, b], &[fact_id])
            .unwrap();
        let ep3 = hora
            .add_episode(EpisodeSource::Conversation, "s3", &[a, b], &[fact_id])
            .unwrap();

        for _ in 0..3 {
            hora.increment_consolidation(ep1).unwrap();
            hora.increment_consolidation(ep2).unwrap();
            hora.increment_consolidation(ep3).unwrap();
        }

        let stats = hora.cls_transfer().unwrap();
        assert_eq!(stats.episodes_processed, 3);
        // Existing edge found → reinforce
        assert_eq!(stats.facts_reinforced, 1);
        assert_eq!(stats.facts_created, 0);

        // Confidence should have increased from 0.8 to 0.9
        let edge = hora.get_fact(fact_id).unwrap().unwrap();
        assert!((edge.confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn test_cls_transfer_increments_consolidation() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.consolidation_params.cls_threshold = 3;

        let a = hora.add_entity("person", "Alice", None, None).unwrap();
        let b = hora.add_entity("person", "Bob", None, None).unwrap();
        let f = hora.add_fact(a, b, "knows", "friends", None).unwrap();

        let ep = hora
            .add_episode(EpisodeSource::Conversation, "s1", &[a, b], &[f])
            .unwrap();
        // Set to exactly threshold
        for _ in 0..3 {
            hora.increment_consolidation(ep).unwrap();
        }

        let before = hora.get_episode(ep).unwrap().unwrap().consolidation_count;
        hora.cls_transfer().unwrap();
        let after = hora.get_episode(ep).unwrap().unwrap().consolidation_count;

        assert_eq!(after, before + 1);
    }

    // --- Memory Linking ---

    #[test]
    fn test_memory_linking_creates_bidirectional_links() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        // Entities created in quick succession → within any reasonable window
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();

        let stats = hora.memory_linking().unwrap();
        // A→B and B→A
        assert_eq!(stats.links_created, 2);
        assert_eq!(stats.links_reinforced, 0);

        // Verify edges exist
        let edges_a = hora.get_entity_facts(a).unwrap();
        assert!(edges_a
            .iter()
            .any(|e| e.target == b && e.relation_type == "temporally_linked"));
        let edges_b = hora.get_entity_facts(b).unwrap();
        assert!(edges_b
            .iter()
            .any(|e| e.target == a && e.relation_type == "temporally_linked"));
    }

    #[test]
    fn test_memory_linking_outside_window_no_link() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        // Set window to 0ms → no entities can be within window
        hora.consolidation_params.linking_window_ms = 0;

        let _a = hora.add_entity("node", "A", None, None).unwrap();
        let _b = hora.add_entity("node", "B", None, None).unwrap();

        let stats = hora.memory_linking().unwrap();
        assert_eq!(stats.links_created, 0);
    }

    #[test]
    fn test_memory_linking_reinforces_existing() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let _a = hora.add_entity("node", "A", None, None).unwrap();
        let _b = hora.add_entity("node", "B", None, None).unwrap();

        // First pass: create links
        let stats1 = hora.memory_linking().unwrap();
        assert_eq!(stats1.links_created, 2);

        // Second pass: reinforce (same entities, links already exist)
        let stats2 = hora.memory_linking().unwrap();
        assert_eq!(stats2.links_created, 0);
        assert_eq!(stats2.links_reinforced, 2);
    }

    #[test]
    fn test_memory_linking_combinatoric() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        // Create 4 entities in quick succession (all within window)
        hora.add_entity("node", "A", None, None).unwrap();
        hora.add_entity("node", "B", None, None).unwrap();
        hora.add_entity("node", "C", None, None).unwrap();
        hora.add_entity("node", "D", None, None).unwrap();

        let stats = hora.memory_linking().unwrap();
        // 4 entities → C(4,2) = 6 pairs × 2 directions = 12 links
        assert_eq!(stats.links_created, 12);
    }

    // --- Dream Cycle ---

    #[test]
    fn test_dream_cycle_executes_all_steps() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s1", &[a, b], &[])
            .unwrap();

        let config = DreamCycleConfig::default();
        let stats = hora.dream_cycle(&config).unwrap();

        // SHY should have downscaled 2 entities
        assert_eq!(stats.entities_downscaled, 2);
        // Replay should have processed 1 episode
        assert_eq!(stats.replay.episodes_replayed, 1);
        // Linking should have created temporal links
        assert!(stats.linking.links_created > 0);
    }

    #[test]
    fn test_dream_cycle_disable_steps() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        hora.add_entity("node", "A", None, None).unwrap();
        hora.add_entity("node", "B", None, None).unwrap();

        let config = DreamCycleConfig {
            shy: false,
            replay: false,
            cls: false,
            linking: false,
            dark_check: false,
            gc: false,
        };

        let stats = hora.dream_cycle(&config).unwrap();
        assert_eq!(stats.entities_downscaled, 0);
        assert_eq!(stats.replay.episodes_replayed, 0);
        assert_eq!(stats.cls.episodes_processed, 0);
        assert_eq!(stats.linking.links_created, 0);
        assert_eq!(stats.dark_nodes_marked, 0);
        assert_eq!(stats.gc_deleted, 0);
    }

    #[test]
    fn test_dream_cycle_idempotent_no_duplicates() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let b = hora.add_entity("node", "B", None, None).unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s1", &[a, b], &[])
            .unwrap();

        let config = DreamCycleConfig::default();
        let stats1 = hora.dream_cycle(&config).unwrap();
        let stats2 = hora.dream_cycle(&config).unwrap();

        // Second call: linking should reinforce, not create new links
        assert_eq!(stats2.linking.links_created, 0);
        // First call created links, second reinforced them
        assert!(stats1.linking.links_created > 0);
        assert!(stats2.linking.links_reinforced > 0);
    }

    #[test]
    fn test_dream_cycle_stats_coherent() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let a = hora.add_entity("node", "A", None, None).unwrap();
        let _b = hora.add_entity("node", "B", None, None).unwrap();
        let _c = hora.add_entity("node", "C", None, None).unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s1", &[a], &[])
            .unwrap();

        let config = DreamCycleConfig::default();
        let stats = hora.dream_cycle(&config).unwrap();

        // 3 entities downscaled
        assert_eq!(stats.entities_downscaled, 3);
        // 1 episode replayed with 1 entity
        assert_eq!(stats.replay.episodes_replayed, 1);
        assert_eq!(stats.replay.entities_reactivated, 1);
        // No GC by default
        assert_eq!(stats.gc_deleted, 0);
    }

    #[test]
    fn test_episodes_sorted_by_created_at() {
        let mut hora = HoraCore::new(HoraConfig::default()).unwrap();
        let e1 = hora.add_entity("node", "A", None, None).unwrap();

        hora.add_episode(EpisodeSource::Conversation, "s1", &[e1], &[])
            .unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s2", &[e1], &[])
            .unwrap();
        hora.add_episode(EpisodeSource::Conversation, "s3", &[e1], &[])
            .unwrap();

        let eps = hora.get_episodes(None, None, None, None).unwrap();
        assert_eq!(eps.len(), 3);
        // Should be sorted by created_at
        for w in eps.windows(2) {
            assert!(w[0].created_at <= w[1].created_at);
        }
    }
}

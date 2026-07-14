#![deny(clippy::all)]

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;

use hora_graph_core::{
    Edge, EdgeId, EntityId, EntityUpdate as CoreEntityUpdate, EpisodeSource,
    FactUpdate as CoreFactUpdate, HoraConfig, HoraCore as CoreHoraCore, MemoryPhase, Properties,
    PropertyValue, SpreadingParams as CoreSpreadingParams, TraverseOpts as CoreTraverseOpts,
};

// ── Error conversion macro ─────────────────────────────────

macro_rules! h {
    ($expr:expr) => {
        $expr.map_err(|e| Error::from_reason(e.to_string()))
    };
}

// ── JS DTO structs (#[napi(object)] = plain JS objects) ────

#[napi(object)]
pub struct JsHoraConfig {
    pub embedding_dims: Option<u32>,
}

#[napi(object)]
pub struct JsEntity {
    pub id: u32,
    pub entity_type: String,
    pub name: String,
    pub properties: HashMap<String, String>,
    pub created_at: f64,
}

#[napi(object)]
pub struct JsFact {
    pub id: u32,
    pub source: u32,
    pub target: u32,
    pub relation_type: String,
    pub description: String,
    pub confidence: f64,
    pub valid_at: f64,
    pub invalid_at: f64,
    pub created_at: f64,
}

#[napi(object)]
pub struct JsEntityUpdate {
    pub name: Option<String>,
    pub entity_type: Option<String>,
    pub properties: Option<HashMap<String, String>>,
}

#[napi(object)]
pub struct JsFactUpdate {
    pub confidence: Option<f64>,
    pub description: Option<String>,
}

#[napi(object)]
pub struct JsTraverseOpts {
    pub depth: Option<u32>,
}

#[napi(object)]
pub struct JsTraverseResult {
    pub entity_ids: Vec<u32>,
    pub edge_ids: Vec<u32>,
}

#[napi(object)]
pub struct JsStats {
    pub entities: u32,
    pub edges: u32,
    pub episodes: u32,
}

#[napi(object)]
pub struct JsSpreadingParams {
    /// Maximum associative strength (default 1.6).
    pub s_max: Option<f64>,
    /// Total activation weight distributed across sources (default 1.0).
    pub w_total: Option<f64>,
    /// Maximum propagation depth (default 3).
    pub max_depth: Option<u32>,
    /// Minimum absolute activation to continue propagating (default 0.01).
    pub cutoff: Option<f64>,
}

#[napi(object)]
pub struct JsSpreadingSource {
    pub entity_id: u32,
    pub weight: f64,
}

#[napi(object)]
pub struct JsSpreadingResult {
    pub entity_id: u32,
    pub activation: f64,
}

// ── Conversion helpers ─────────────────────────────────────

fn props_to_js(props: &Properties) -> HashMap<String, String> {
    props
        .iter()
        .map(|(k, v)| {
            let s = match v {
                PropertyValue::String(s) => s.clone(),
                PropertyValue::Int(i) => i.to_string(),
                PropertyValue::Float(f) => f.to_string(),
                PropertyValue::Bool(b) => b.to_string(),
                _ => format!("{:?}", v),
            };
            (k.clone(), s)
        })
        .collect()
}

fn js_to_props(map: Option<HashMap<String, String>>) -> Option<Properties> {
    map.map(|m| {
        m.into_iter()
            .map(|(k, v)| (k, PropertyValue::String(v)))
            .collect()
    })
}

fn edge_to_js(e: Edge) -> JsFact {
    JsFact {
        id: e.id.0 as u32,
        source: e.source.0 as u32,
        target: e.target.0 as u32,
        relation_type: e.relation_type,
        description: e.description,
        confidence: e.confidence as f64,
        valid_at: e.valid_at as f64,
        invalid_at: e.invalid_at as f64,
        created_at: e.created_at as f64,
    }
}

fn make_config(config: Option<JsHoraConfig>) -> HoraConfig {
    HoraConfig {
        embedding_dims: config.and_then(|c| c.embedding_dims).unwrap_or(0) as u16,
        ..Default::default()
    }
}

// ── HoraCore class ─────────────────────────────────────────

#[napi(js_name = "HoraCore")]
pub struct JsHoraCore {
    inner: CoreHoraCore,
}

#[napi]
impl JsHoraCore {
    /// Create a new in-memory HoraCore instance (no persistence).
    #[napi(factory)]
    pub fn new_memory(config: Option<JsHoraConfig>) -> Result<Self> {
        let inner = h!(CoreHoraCore::new(make_config(config)))?;
        Ok(Self { inner })
    }

    /// Open a file-backed HoraCore instance.
    /// If the file exists, loads data from it. Otherwise creates a new empty instance.
    #[napi(factory)]
    pub fn open(path: String, config: Option<JsHoraConfig>) -> Result<Self> {
        let inner = h!(CoreHoraCore::open(&path, make_config(config)))?;
        Ok(Self { inner })
    }

    // ── CRUD Entities ──────────────────────────────────────

    /// Add a new entity. Returns its ID.
    #[napi]
    pub fn add_entity(
        &mut self,
        entity_type: String,
        name: String,
        properties: Option<HashMap<String, String>>,
    ) -> Result<u32> {
        let props = js_to_props(properties);
        let id = h!(self.inner.add_entity(&entity_type, &name, props, None))?;
        Ok(id.0 as u32)
    }

    /// Get an entity by ID. Returns null if not found.
    #[napi]
    pub fn get_entity(&mut self, id: u32) -> Result<Option<JsEntity>> {
        let entity = h!(self.inner.get_entity(EntityId(id as u64)))?;
        Ok(entity.map(|e| JsEntity {
            id: e.id.0 as u32,
            entity_type: e.entity_type,
            name: e.name,
            properties: props_to_js(&e.properties),
            created_at: e.created_at as f64,
        }))
    }

    /// Update an entity. Only fields present in the update object are changed.
    #[napi]
    pub fn update_entity(&mut self, id: u32, update: JsEntityUpdate) -> Result<()> {
        let core_update = CoreEntityUpdate {
            name: update.name,
            entity_type: update.entity_type,
            properties: js_to_props(update.properties),
            embedding: None,
        };
        h!(self.inner.update_entity(EntityId(id as u64), core_update))
    }

    /// Delete an entity and all its connected edges (cascade).
    #[napi]
    pub fn delete_entity(&mut self, id: u32) -> Result<()> {
        h!(self.inner.delete_entity(EntityId(id as u64)))
    }

    // ── CRUD Facts ─────────────────────────────────────────

    /// Add a new fact (directed edge) between two entities. Returns its ID.
    #[napi]
    pub fn add_fact(
        &mut self,
        source: u32,
        target: u32,
        relation: String,
        description: String,
        confidence: Option<f64>,
    ) -> Result<u32> {
        let id = h!(self.inner.add_fact(
            EntityId(source as u64),
            EntityId(target as u64),
            &relation,
            &description,
            confidence.map(|c| c as f32),
        ))?;
        Ok(id.0 as u32)
    }

    /// Get a fact by ID. Returns null if not found.
    #[napi]
    pub fn get_fact(&self, id: u32) -> Result<Option<JsFact>> {
        let edge = h!(self.inner.get_fact(EdgeId(id as u64)))?;
        Ok(edge.map(edge_to_js))
    }

    /// Update a fact. Only fields present in the update object are changed.
    #[napi]
    pub fn update_fact(&mut self, id: u32, update: JsFactUpdate) -> Result<()> {
        let core_update = CoreFactUpdate {
            confidence: update.confidence.map(|c| c as f32),
            description: update.description,
        };
        h!(self.inner.update_fact(EdgeId(id as u64), core_update))
    }

    /// Mark a fact as invalid (bi-temporal soft-delete).
    #[napi]
    pub fn invalidate_fact(&mut self, id: u32) -> Result<()> {
        h!(self.inner.invalidate_fact(EdgeId(id as u64)))
    }

    /// Physically delete a fact.
    #[napi]
    pub fn delete_fact(&mut self, id: u32) -> Result<()> {
        h!(self.inner.delete_fact(EdgeId(id as u64)))
    }

    /// Get all facts where the given entity is source or target.
    #[napi]
    pub fn get_entity_facts(&self, entity_id: u32) -> Result<Vec<JsFact>> {
        let edges = h!(self.inner.get_entity_facts(EntityId(entity_id as u64)))?;
        Ok(edges.into_iter().map(edge_to_js).collect())
    }

    // ── Graph Traversal ────────────────────────────────────

    /// BFS traversal from a start entity up to the given depth.
    #[napi]
    pub fn traverse(
        &self,
        start_id: u32,
        opts: Option<JsTraverseOpts>,
    ) -> Result<JsTraverseResult> {
        let core_opts = CoreTraverseOpts {
            depth: opts.and_then(|o| o.depth).unwrap_or(3),
        };
        let result = h!(self.inner.traverse(EntityId(start_id as u64), core_opts))?;
        Ok(JsTraverseResult {
            entity_ids: result.entity_ids.into_iter().map(|id| id.0 as u32).collect(),
            edge_ids: result.edge_ids.into_iter().map(|id| id.0 as u32).collect(),
        })
    }

    /// Get direct neighbor entity IDs.
    #[napi]
    pub fn neighbors(&self, entity_id: u32) -> Result<Vec<u32>> {
        let ids = h!(self.inner.neighbors(EntityId(entity_id as u64)))?;
        Ok(ids.into_iter().map(|id| id.0 as u32).collect())
    }

    /// Timeline of all facts involving an entity, sorted by valid_at.
    #[napi]
    pub fn timeline(&self, entity_id: u32) -> Result<Vec<JsFact>> {
        let edges = h!(self.inner.timeline(EntityId(entity_id as u64)))?;
        Ok(edges.into_iter().map(edge_to_js).collect())
    }

    /// All facts valid at a given point in time (epoch milliseconds).
    #[napi]
    pub fn facts_at(&self, timestamp: f64) -> Result<Vec<JsFact>> {
        let edges = h!(self.inner.facts_at(timestamp as i64))?;
        Ok(edges.into_iter().map(edge_to_js).collect())
    }

    // ── Spreading Activation ─────────────────────────────────

    /// Spread activation from source entities through the knowledge graph.
    /// Returns an array of { entityId, activation } for all reached entities.
    #[napi]
    pub fn spread_activation(
        &self,
        sources: Vec<JsSpreadingSource>,
        params: Option<JsSpreadingParams>,
    ) -> Result<Vec<JsSpreadingResult>> {
        let core_sources: Vec<(EntityId, f64)> = sources
            .into_iter()
            .map(|s| (EntityId(s.entity_id as u64), s.weight))
            .collect();
        let core_params = match params {
            Some(p) => CoreSpreadingParams {
                s_max: p.s_max.unwrap_or(1.6),
                w_total: p.w_total.unwrap_or(1.0),
                max_depth: p.max_depth.unwrap_or(3) as u8,
                cutoff: p.cutoff.unwrap_or(0.01),
            },
            None => CoreSpreadingParams::default(),
        };
        let result = h!(self.inner.spread_activation(&core_sources, &core_params))?;
        Ok(result
            .into_iter()
            .map(|(id, activation)| JsSpreadingResult {
                entity_id: id.0 as u32,
                activation,
            })
            .collect())
    }

    // ── Reconsolidation ──────────────────────────────────────

    /// Get the current reconsolidation phase for an entity.
    /// Returns "stable", "labile", "restabilizing", or "dark". Null if entity doesn't exist.
    #[napi]
    pub fn get_memory_phase(&mut self, entity_id: u32) -> Result<Option<String>> {
        let phase = self.inner.get_memory_phase(EntityId(entity_id as u64));
        Ok(phase.map(|p| match p {
            MemoryPhase::Stable => "stable".to_string(),
            MemoryPhase::Labile { .. } => "labile".to_string(),
            MemoryPhase::Restabilizing { .. } => "restabilizing".to_string(),
            MemoryPhase::Dark { .. } => "dark".to_string(),
            _ => "unknown".to_string(),
        }))
    }

    /// Get the cumulative stability multiplier for an entity.
    /// Starts at 1.0, grows with each reconsolidation cycle.
    #[napi]
    pub fn get_stability_multiplier(&mut self, entity_id: u32) -> Result<Option<f64>> {
        Ok(self
            .inner
            .get_stability_multiplier(EntityId(entity_id as u64)))
    }

    // ── FSRS Scheduling ─────────────────────────────────────

    /// Get the current retrievability for an entity (0.0 to 1.0).
    #[napi]
    pub fn get_retrievability(&self, entity_id: u32) -> Result<Option<f64>> {
        Ok(self.inner.get_retrievability(EntityId(entity_id as u64)))
    }

    /// Get the optimal next review interval in days for an entity.
    #[napi]
    pub fn get_next_review_days(&self, entity_id: u32) -> Result<Option<f64>> {
        Ok(self.inner.get_next_review_days(EntityId(entity_id as u64)))
    }

    /// Get the current FSRS stability in days for an entity.
    #[napi]
    pub fn get_fsrs_stability(&self, entity_id: u32) -> Result<Option<f64>> {
        Ok(self.inner.get_fsrs_stability(EntityId(entity_id as u64)))
    }

    // ── Dark Nodes ──────────────────────────────────────────

    /// Scan all entities and mark those below activation threshold as Dark.
    /// Returns the number of entities newly marked as Dark.
    #[napi]
    pub fn dark_node_pass(&mut self) -> Result<u32> {
        Ok(self.inner.dark_node_pass() as u32)
    }

    /// Attempt to recover a Dark entity. Returns true if recovery occurred.
    #[napi]
    pub fn attempt_recovery(&mut self, entity_id: u32) -> Result<bool> {
        Ok(self.inner.attempt_recovery(EntityId(entity_id as u64)))
    }

    /// List all entity IDs currently in Dark state.
    #[napi]
    pub fn dark_nodes(&mut self) -> Result<Vec<u32>> {
        Ok(self.inner.dark_nodes().into_iter().map(|id| id.0 as u32).collect())
    }

    /// List dark entities eligible for garbage collection.
    #[napi]
    pub fn gc_candidates(&mut self) -> Result<Vec<u32>> {
        Ok(self.inner.gc_candidates().into_iter().map(|id| id.0 as u32).collect())
    }

    // ── Episodes ───────────────────────────────────────────

    /// Record an episode. Source must be "conversation", "document", or "api".
    #[napi]
    pub fn add_episode(
        &mut self,
        source: String,
        session_id: String,
        entity_ids: Vec<u32>,
        fact_ids: Vec<u32>,
    ) -> Result<u32> {
        let ep_source = match source.as_str() {
            "conversation" => EpisodeSource::Conversation,
            "document" => EpisodeSource::Document,
            "api" => EpisodeSource::Api,
            _ => {
                return Err(Error::from_reason(format!(
                    "unknown episode source: '{}' (expected 'conversation', 'document', or 'api')",
                    source
                )))
            }
        };
        let eids: Vec<EntityId> = entity_ids.into_iter().map(|id| EntityId(id as u64)).collect();
        let fids: Vec<EdgeId> = fact_ids.into_iter().map(|id| EdgeId(id as u64)).collect();
        let id = h!(self.inner.add_episode(ep_source, &session_id, &eids, &fids))?;
        Ok(id as u32)
    }

    // ── Persistence ────────────────────────────────────────

    /// Flush all data to the backing file (crash-safe via tmp+rename).
    #[napi]
    pub fn flush(&self) -> Result<()> {
        h!(self.inner.flush())
    }

    /// Copy the current state to a snapshot file.
    #[napi]
    pub fn snapshot(&self, dest: String) -> Result<()> {
        h!(self.inner.snapshot(&dest))
    }

    // ── Stats ──────────────────────────────────────────────

    /// Get summary statistics about the knowledge graph.
    #[napi]
    pub fn stats(&self) -> Result<JsStats> {
        let s = h!(self.inner.stats())?;
        Ok(JsStats {
            entities: s.entities as u32,
            edges: s.edges as u32,
            episodes: s.episodes as u32,
        })
    }
}

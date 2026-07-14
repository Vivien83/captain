#![deny(clippy::all)]

use std::collections::HashMap;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

// Rename the crate import to avoid collision with the #[pymodule] name.
use hora_graph_core as hora;

use hora::{
    Edge, EdgeId, EntityId, EntityUpdate, EpisodeSource, FactUpdate, HoraConfig,
    HoraCore as CoreHoraCore, MemoryPhase, Properties, PropertyValue,
    SpreadingParams as CoreSpreadingParams, TraverseOpts,
};

// ── Error conversion ─────────────────────────────────────────

fn to_py(e: hora::HoraError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

// ── Property conversion ──────────────────────────────────────

fn props_to_py(py: Python<'_>, props: &Properties) -> PyObject {
    let dict = PyDict::new_bound(py);
    for (k, v) in props {
        match v {
            PropertyValue::String(s) => dict.set_item(k, s).unwrap(),
            PropertyValue::Int(i) => dict.set_item(k, i).unwrap(),
            PropertyValue::Float(f) => dict.set_item(k, f).unwrap(),
            PropertyValue::Bool(b) => dict.set_item(k, b).unwrap(),
            _ => dict.set_item(k, format!("{:?}", v)).unwrap(),
        };
    }
    dict.into()
}

fn py_to_props(dict: Option<HashMap<String, PyObject>>, py: Python<'_>) -> Option<Properties> {
    dict.map(|m| {
        m.into_iter()
            .map(|(k, v)| {
                let pv = if let Ok(b) = v.extract::<bool>(py) {
                    PropertyValue::Bool(b)
                } else if let Ok(i) = v.extract::<i64>(py) {
                    PropertyValue::Int(i)
                } else if let Ok(f) = v.extract::<f64>(py) {
                    PropertyValue::Float(f)
                } else if let Ok(s) = v.extract::<String>(py) {
                    PropertyValue::String(s)
                } else {
                    PropertyValue::String(format!("{}", v))
                };
                (k, pv)
            })
            .collect()
    })
}

fn edge_to_dict(py: Python<'_>, e: &Edge) -> PyObject {
    let dict = PyDict::new_bound(py);
    dict.set_item("id", e.id.0).unwrap();
    dict.set_item("source", e.source.0).unwrap();
    dict.set_item("target", e.target.0).unwrap();
    dict.set_item("relation_type", &e.relation_type).unwrap();
    dict.set_item("description", &e.description).unwrap();
    dict.set_item("confidence", e.confidence).unwrap();
    dict.set_item("valid_at", e.valid_at).unwrap();
    dict.set_item("invalid_at", e.invalid_at).unwrap();
    dict.set_item("created_at", e.created_at).unwrap();
    dict.into()
}

// ── HoraCore class ───────────────────────────────────────────

#[pyclass]
struct HoraCore {
    inner: CoreHoraCore,
}

#[pymethods]
impl HoraCore {
    /// Create a new in-memory HoraCore instance.
    #[staticmethod]
    #[pyo3(signature = (embedding_dims=0))]
    fn new_memory(embedding_dims: u16) -> PyResult<Self> {
        let config = HoraConfig { embedding_dims, ..Default::default() };
        let inner = CoreHoraCore::new(config).map_err(to_py)?;
        Ok(Self { inner })
    }

    /// Open a file-backed HoraCore instance.
    #[staticmethod]
    #[pyo3(signature = (path, embedding_dims=0))]
    fn open(path: &str, embedding_dims: u16) -> PyResult<Self> {
        let config = HoraConfig { embedding_dims, ..Default::default() };
        let inner = CoreHoraCore::open(path, config).map_err(to_py)?;
        Ok(Self { inner })
    }

    // ── CRUD Entities ────────────────────────────────────────

    /// Add a new entity. Returns its ID.
    #[pyo3(signature = (entity_type, name, properties=None, embedding=None))]
    fn add_entity(
        &mut self,
        py: Python<'_>,
        entity_type: &str,
        name: &str,
        properties: Option<HashMap<String, PyObject>>,
        embedding: Option<Vec<f32>>,
    ) -> PyResult<u64> {
        let props = py_to_props(properties, py);
        let id = self
            .inner
            .add_entity(entity_type, name, props, embedding.as_deref())
            .map_err(to_py)?;
        Ok(id.0)
    }

    /// Get an entity by ID. Returns None if not found.
    fn get_entity(&mut self, py: Python<'_>, id: u64) -> PyResult<Option<PyObject>> {
        let entity = self.inner.get_entity(EntityId(id)).map_err(to_py)?;
        Ok(entity.map(|e| {
            let dict = PyDict::new_bound(py);
            dict.set_item("id", e.id.0).unwrap();
            dict.set_item("entity_type", &e.entity_type).unwrap();
            dict.set_item("name", &e.name).unwrap();
            dict.set_item("properties", props_to_py(py, &e.properties)).unwrap();
            dict.set_item("embedding", &e.embedding).unwrap();
            dict.set_item("created_at", e.created_at).unwrap();
            dict.into()
        }))
    }

    /// Update an entity. Only provided fields are changed.
    #[pyo3(signature = (id, name=None, entity_type=None, properties=None, embedding=None))]
    fn update_entity(
        &mut self,
        py: Python<'_>,
        id: u64,
        name: Option<String>,
        entity_type: Option<String>,
        properties: Option<HashMap<String, PyObject>>,
        embedding: Option<Vec<f32>>,
    ) -> PyResult<()> {
        let update = EntityUpdate {
            name,
            entity_type,
            properties: py_to_props(properties, py),
            embedding,
        };
        self.inner.update_entity(EntityId(id), update).map_err(to_py)
    }

    /// Delete an entity and its connected edges.
    fn delete_entity(&mut self, id: u64) -> PyResult<()> {
        self.inner.delete_entity(EntityId(id)).map_err(to_py)
    }

    // ── CRUD Facts ───────────────────────────────────────────

    /// Add a directed edge (fact) between two entities. Returns its ID.
    #[pyo3(signature = (source, target, relation, description="", confidence=None))]
    fn add_fact(
        &mut self,
        source: u64,
        target: u64,
        relation: &str,
        description: &str,
        confidence: Option<f32>,
    ) -> PyResult<u64> {
        let id = self
            .inner
            .add_fact(EntityId(source), EntityId(target), relation, description, confidence)
            .map_err(to_py)?;
        Ok(id.0)
    }

    /// Get a fact by ID. Returns None if not found.
    fn get_fact(&self, py: Python<'_>, id: u64) -> PyResult<Option<PyObject>> {
        let edge = self.inner.get_fact(EdgeId(id)).map_err(to_py)?;
        Ok(edge.map(|e| edge_to_dict(py, &e)))
    }

    /// Update a fact. Only provided fields are changed.
    #[pyo3(signature = (id, confidence=None, description=None))]
    fn update_fact(
        &mut self,
        id: u64,
        confidence: Option<f32>,
        description: Option<String>,
    ) -> PyResult<()> {
        let update = FactUpdate { confidence, description };
        self.inner.update_fact(EdgeId(id), update).map_err(to_py)
    }

    /// Mark a fact as invalid (bi-temporal soft-delete).
    fn invalidate_fact(&mut self, id: u64) -> PyResult<()> {
        self.inner.invalidate_fact(EdgeId(id)).map_err(to_py)
    }

    /// Delete a fact permanently.
    fn delete_fact(&mut self, id: u64) -> PyResult<()> {
        self.inner.delete_fact(EdgeId(id)).map_err(to_py)
    }

    /// Get all facts involving an entity.
    fn get_entity_facts(&self, py: Python<'_>, entity_id: u64) -> PyResult<Vec<PyObject>> {
        let edges = self.inner.get_entity_facts(EntityId(entity_id)).map_err(to_py)?;
        Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
    }

    // ── Search ───────────────────────────────────────────────

    /// Hybrid search (text + vector). Returns list of {entity_id, score}.
    #[pyo3(signature = (query=None, embedding=None, top_k=10))]
    fn search(
        &mut self,
        py: Python<'_>,
        query: Option<&str>,
        embedding: Option<Vec<f32>>,
        top_k: usize,
    ) -> PyResult<Vec<PyObject>> {
        let opts = hora::SearchOpts { top_k, ..Default::default() };
        let hits = self.inner.search(query, embedding.as_deref(), opts).map_err(to_py)?;
        Ok(hits
            .iter()
            .map(|h| {
                let dict = PyDict::new_bound(py);
                dict.set_item("entity_id", h.entity_id.0).unwrap();
                dict.set_item("score", h.score).unwrap();
                dict.into()
            })
            .collect())
    }

    // ── Traversal ────────────────────────────────────────────

    /// BFS traversal from a start entity. Returns {entity_ids, edge_ids}.
    #[pyo3(signature = (start_id, depth=3))]
    fn traverse(&self, py: Python<'_>, start_id: u64, depth: u32) -> PyResult<PyObject> {
        let opts = TraverseOpts { depth };
        let result = self.inner.traverse(EntityId(start_id), opts).map_err(to_py)?;
        let dict = PyDict::new_bound(py);
        let eids: Vec<u64> = result.entity_ids.into_iter().map(|id| id.0).collect();
        let fids: Vec<u64> = result.edge_ids.into_iter().map(|id| id.0).collect();
        dict.set_item("entity_ids", eids).unwrap();
        dict.set_item("edge_ids", fids).unwrap();
        Ok(dict.into())
    }

    /// Get direct neighbor entity IDs.
    fn neighbors(&self, entity_id: u64) -> PyResult<Vec<u64>> {
        let ids = self.inner.neighbors(EntityId(entity_id)).map_err(to_py)?;
        Ok(ids.into_iter().map(|id| id.0).collect())
    }

    /// Timeline of all facts involving an entity, sorted by valid_at.
    fn timeline(&self, py: Python<'_>, entity_id: u64) -> PyResult<Vec<PyObject>> {
        let edges = self.inner.timeline(EntityId(entity_id)).map_err(to_py)?;
        Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
    }

    /// All facts valid at a given timestamp (epoch milliseconds).
    fn facts_at(&self, py: Python<'_>, timestamp: i64) -> PyResult<Vec<PyObject>> {
        let edges = self.inner.facts_at(timestamp).map_err(to_py)?;
        Ok(edges.iter().map(|e| edge_to_dict(py, e)).collect())
    }

    // ── Spreading Activation ─────────────────────────────────

    /// Spread activation from source entities.
    /// sources: list of (entity_id, weight) tuples.
    #[pyo3(signature = (sources, s_max=1.6, w_total=1.0, max_depth=3, cutoff=0.01))]
    fn spread_activation(
        &self,
        py: Python<'_>,
        sources: Vec<(u64, f64)>,
        s_max: f64,
        w_total: f64,
        max_depth: u8,
        cutoff: f64,
    ) -> PyResult<Vec<PyObject>> {
        let core_sources: Vec<(EntityId, f64)> =
            sources.into_iter().map(|(id, w)| (EntityId(id), w)).collect();
        let params = CoreSpreadingParams { s_max, w_total, max_depth, cutoff };
        let result = self.inner.spread_activation(&core_sources, &params).map_err(to_py)?;
        Ok(result
            .into_iter()
            .map(|(id, activation)| {
                let dict = PyDict::new_bound(py);
                dict.set_item("entity_id", id.0).unwrap();
                dict.set_item("activation", activation).unwrap();
                dict.into_py(py)
            })
            .collect())
    }

    // ── Memory ───────────────────────────────────────────────

    /// Get the reconsolidation phase for an entity.
    fn get_memory_phase(&mut self, entity_id: u64) -> PyResult<Option<String>> {
        let phase = self.inner.get_memory_phase(EntityId(entity_id));
        Ok(phase.map(|p| match p {
            MemoryPhase::Stable => "stable".to_string(),
            MemoryPhase::Labile { .. } => "labile".to_string(),
            MemoryPhase::Restabilizing { .. } => "restabilizing".to_string(),
            MemoryPhase::Dark { .. } => "dark".to_string(),
            _ => "unknown".to_string(),
        }))
    }

    fn get_retrievability(&self, entity_id: u64) -> PyResult<Option<f64>> {
        Ok(self.inner.get_retrievability(EntityId(entity_id)))
    }

    fn get_next_review_days(&self, entity_id: u64) -> PyResult<Option<f64>> {
        Ok(self.inner.get_next_review_days(EntityId(entity_id)))
    }

    fn dark_node_pass(&mut self) -> PyResult<u32> {
        Ok(self.inner.dark_node_pass() as u32)
    }

    fn dark_nodes(&mut self) -> PyResult<Vec<u64>> {
        Ok(self.inner.dark_nodes().into_iter().map(|id| id.0).collect())
    }

    // ── Episodes ─────────────────────────────────────────────

    /// Record an episode. source: "conversation", "document", or "api".
    fn add_episode(
        &mut self,
        source: &str,
        session_id: &str,
        entity_ids: Vec<u64>,
        fact_ids: Vec<u64>,
    ) -> PyResult<u64> {
        let ep_source = match source {
            "conversation" => EpisodeSource::Conversation,
            "document" => EpisodeSource::Document,
            "api" => EpisodeSource::Api,
            _ => return Err(PyValueError::new_err(format!("unknown source: '{}'", source))),
        };
        let eids: Vec<EntityId> = entity_ids.into_iter().map(EntityId).collect();
        let fids: Vec<EdgeId> = fact_ids.into_iter().map(EdgeId).collect();
        let id = self.inner.add_episode(ep_source, session_id, &eids, &fids).map_err(to_py)?;
        Ok(id)
    }

    // ── Persistence ──────────────────────────────────────────

    /// Flush all data to the backing file.
    fn flush(&self) -> PyResult<()> {
        self.inner.flush().map_err(to_py)
    }

    /// Copy current state to a snapshot file.
    fn snapshot(&self, dest: &str) -> PyResult<()> {
        self.inner.snapshot(dest).map_err(to_py)
    }

    // ── Stats ────────────────────────────────────────────────

    /// Get summary statistics.
    fn stats(&self, py: Python<'_>) -> PyResult<PyObject> {
        let s = self.inner.stats().map_err(to_py)?;
        let dict = PyDict::new_bound(py);
        dict.set_item("entities", s.entities).unwrap();
        dict.set_item("edges", s.edges).unwrap();
        dict.set_item("episodes", s.episodes).unwrap();
        Ok(dict.into())
    }
}

// ── Module ───────────────────────────────────────────────────

#[pymodule]
#[pyo3(name = "hora_graph_core")]
fn hora_python(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<HoraCore>()?;
    Ok(())
}

//! WASM binding for hora-graph-core.
//!
//! Memory backend only (no filesystem in WASM). Properties and search
//! results are passed as JS objects via serde-wasm-bindgen.

use wasm_bindgen::prelude::*;

use hora_graph_core::{
    EdgeId, EntityId, EntityUpdate, EpisodeSource, FactUpdate, HoraConfig,
    HoraCore as CoreHoraCore, Properties, PropertyValue, SearchOpts, TraverseOpts,
};

// ── Error conversion ─────────────────────────────────────────

fn to_js(e: hora_graph_core::HoraError) -> JsValue {
    JsValue::from_str(&e.to_string())
}

// ── Property conversion ──────────────────────────────────────

fn js_to_props(val: JsValue) -> Option<Properties> {
    if val.is_undefined() || val.is_null() {
        return None;
    }
    let obj: &js_sys::Object = val.dyn_ref()?;
    let keys = js_sys::Object::keys(obj);
    let mut props = Properties::new();
    for i in 0..keys.length() {
        let key = keys.get(i);
        let k = key.as_string()?;
        let v = js_sys::Reflect::get(obj, &key).ok()?;
        let pv = if let Some(b) = v.as_bool() {
            PropertyValue::Bool(b)
        } else if let Some(f) = v.as_f64() {
            // Check if it's an integer (no fractional part and within i64 range)
            if f.fract() == 0.0 && f >= i64::MIN as f64 && f <= i64::MAX as f64 {
                PropertyValue::Int(f as i64)
            } else {
                PropertyValue::Float(f)
            }
        } else if let Some(s) = v.as_string() {
            PropertyValue::String(s)
        } else {
            PropertyValue::String(format!("{:?}", v))
        };
        props.insert(k, pv);
    }
    Some(props)
}

fn props_to_js(props: &Properties) -> JsValue {
    let obj = js_sys::Object::new();
    for (k, v) in props.iter() {
        let jv: JsValue = match v {
            PropertyValue::String(s) => s.into(),
            PropertyValue::Int(i) => (*i as f64).into(),
            PropertyValue::Float(f) => (*f).into(),
            PropertyValue::Bool(b) => (*b).into(),
            _ => format!("{:?}", v).into(),
        };
        let _ = js_sys::Reflect::set(&obj, &k.into(), &jv);
    }
    obj.into()
}

// ── HoraCore class ───────────────────────────────────────────

#[wasm_bindgen]
pub struct HoraCore {
    inner: CoreHoraCore,
}

#[wasm_bindgen]
impl HoraCore {
    /// Create a new in-memory HoraCore instance.
    #[wasm_bindgen(js_name = "newMemory")]
    pub fn new_memory(embedding_dims: Option<u16>) -> Result<HoraCore, JsValue> {
        let config = HoraConfig {
            embedding_dims: embedding_dims.unwrap_or(0),
            ..Default::default()
        };
        let inner = CoreHoraCore::new(config).map_err(to_js)?;
        Ok(Self { inner })
    }

    // ── CRUD Entities ────────────────────────────────────────

    /// Add a new entity. Returns its ID.
    #[wasm_bindgen(js_name = "addEntity")]
    pub fn add_entity(
        &mut self,
        entity_type: &str,
        name: &str,
        properties: JsValue,
        embedding: Option<Vec<f32>>,
    ) -> Result<u32, JsValue> {
        let props = js_to_props(properties);
        let id = self
            .inner
            .add_entity(entity_type, name, props, embedding.as_deref())
            .map_err(to_js)?;
        Ok(id.0 as u32)
    }

    /// Get an entity by ID. Returns null if not found.
    #[wasm_bindgen(js_name = "getEntity")]
    pub fn get_entity(&mut self, id: u32) -> Result<JsValue, JsValue> {
        let entity = self.inner.get_entity(EntityId(id as u64)).map_err(to_js)?;
        match entity {
            None => Ok(JsValue::NULL),
            Some(e) => {
                let obj = js_sys::Object::new();
                js_sys::Reflect::set(&obj, &"id".into(), &(e.id.0 as u32).into())?;
                js_sys::Reflect::set(&obj, &"entityType".into(), &e.entity_type.into())?;
                js_sys::Reflect::set(&obj, &"name".into(), &e.name.into())?;
                js_sys::Reflect::set(&obj, &"properties".into(), &props_to_js(&e.properties))?;
                js_sys::Reflect::set(&obj, &"createdAt".into(), &(e.created_at as f64).into())?;

                if let Some(emb) = &e.embedding {
                    let arr = js_sys::Float32Array::new_with_length(emb.len() as u32);
                    arr.copy_from(emb);
                    js_sys::Reflect::set(&obj, &"embedding".into(), &arr.into())?;
                } else {
                    js_sys::Reflect::set(&obj, &"embedding".into(), &JsValue::NULL)?;
                }

                Ok(obj.into())
            }
        }
    }

    /// Update an entity.
    #[wasm_bindgen(js_name = "updateEntity")]
    pub fn update_entity(
        &mut self,
        id: u32,
        name: Option<String>,
        entity_type: Option<String>,
        properties: JsValue,
    ) -> Result<(), JsValue> {
        let update = EntityUpdate {
            name,
            entity_type,
            properties: js_to_props(properties),
            embedding: None,
        };
        self.inner.update_entity(EntityId(id as u64), update).map_err(to_js)
    }

    /// Delete an entity.
    #[wasm_bindgen(js_name = "deleteEntity")]
    pub fn delete_entity(&mut self, id: u32) -> Result<(), JsValue> {
        self.inner.delete_entity(EntityId(id as u64)).map_err(to_js)
    }

    // ── CRUD Facts ───────────────────────────────────────────

    /// Add a fact. Returns its ID.
    #[wasm_bindgen(js_name = "addFact")]
    pub fn add_fact(
        &mut self,
        source: u32,
        target: u32,
        relation: &str,
        description: &str,
        confidence: Option<f32>,
    ) -> Result<u32, JsValue> {
        let id = self
            .inner
            .add_fact(
                EntityId(source as u64),
                EntityId(target as u64),
                relation,
                description,
                confidence,
            )
            .map_err(to_js)?;
        Ok(id.0 as u32)
    }

    /// Get a fact by ID.
    #[wasm_bindgen(js_name = "getFact")]
    pub fn get_fact(&self, id: u32) -> Result<JsValue, JsValue> {
        let edge = self.inner.get_fact(EdgeId(id as u64)).map_err(to_js)?;
        match edge {
            None => Ok(JsValue::NULL),
            Some(e) => {
                let obj = js_sys::Object::new();
                js_sys::Reflect::set(&obj, &"id".into(), &(e.id.0 as u32).into())?;
                js_sys::Reflect::set(&obj, &"source".into(), &(e.source.0 as u32).into())?;
                js_sys::Reflect::set(&obj, &"target".into(), &(e.target.0 as u32).into())?;
                js_sys::Reflect::set(&obj, &"relationType".into(), &e.relation_type.into())?;
                js_sys::Reflect::set(&obj, &"description".into(), &e.description.into())?;
                js_sys::Reflect::set(&obj, &"confidence".into(), &e.confidence.into())?;
                js_sys::Reflect::set(&obj, &"validAt".into(), &(e.valid_at as f64).into())?;
                js_sys::Reflect::set(&obj, &"invalidAt".into(), &(e.invalid_at as f64).into())?;
                js_sys::Reflect::set(&obj, &"createdAt".into(), &(e.created_at as f64).into())?;
                Ok(obj.into())
            }
        }
    }

    /// Update a fact.
    #[wasm_bindgen(js_name = "updateFact")]
    pub fn update_fact(
        &mut self,
        id: u32,
        confidence: Option<f32>,
        description: Option<String>,
    ) -> Result<(), JsValue> {
        let update = FactUpdate { confidence, description };
        self.inner.update_fact(EdgeId(id as u64), update).map_err(to_js)
    }

    /// Invalidate a fact (bi-temporal soft-delete).
    #[wasm_bindgen(js_name = "invalidateFact")]
    pub fn invalidate_fact(&mut self, id: u32) -> Result<(), JsValue> {
        self.inner.invalidate_fact(EdgeId(id as u64)).map_err(to_js)
    }

    /// Delete a fact permanently.
    #[wasm_bindgen(js_name = "deleteFact")]
    pub fn delete_fact(&mut self, id: u32) -> Result<(), JsValue> {
        self.inner.delete_fact(EdgeId(id as u64)).map_err(to_js)
    }

    /// Get all facts involving an entity.
    #[wasm_bindgen(js_name = "getEntityFacts")]
    pub fn get_entity_facts(&self, entity_id: u32) -> Result<JsValue, JsValue> {
        let edges = self
            .inner
            .get_entity_facts(EntityId(entity_id as u64))
            .map_err(to_js)?;

        let arr = js_sys::Array::new();
        for e in &edges {
            let obj = js_sys::Object::new();
            js_sys::Reflect::set(&obj, &"id".into(), &(e.id.0 as u32).into())?;
            js_sys::Reflect::set(&obj, &"source".into(), &(e.source.0 as u32).into())?;
            js_sys::Reflect::set(&obj, &"target".into(), &(e.target.0 as u32).into())?;
            js_sys::Reflect::set(&obj, &"relationType".into(), &e.relation_type.clone().into())?;
            arr.push(&obj);
        }
        Ok(arr.into())
    }

    // ── Search ───────────────────────────────────────────────

    /// Hybrid search. Returns array of {entityId, score}.
    #[wasm_bindgen]
    pub fn search(
        &mut self,
        query: Option<String>,
        embedding: Option<Vec<f32>>,
        top_k: Option<usize>,
    ) -> Result<JsValue, JsValue> {
        let opts = SearchOpts {
            top_k: top_k.unwrap_or(10),
            ..Default::default()
        };
        let hits = self
            .inner
            .search(query.as_deref(), embedding.as_deref(), opts)
            .map_err(to_js)?;

        let arr = js_sys::Array::new();
        for h in &hits {
            let obj = js_sys::Object::new();
            js_sys::Reflect::set(&obj, &"entityId".into(), &(h.entity_id.0 as u32).into())?;
            js_sys::Reflect::set(&obj, &"score".into(), &h.score.into())?;
            arr.push(&obj);
        }
        Ok(arr.into())
    }

    // ── Traversal ────────────────────────────────────────────

    /// BFS traversal. Returns {entityIds, edgeIds}.
    #[wasm_bindgen]
    pub fn traverse(
        &self,
        start_id: u32,
        depth: Option<u32>,
    ) -> Result<JsValue, JsValue> {
        let opts = TraverseOpts { depth: depth.unwrap_or(3) };
        let result = self
            .inner
            .traverse(EntityId(start_id as u64), opts)
            .map_err(to_js)?;

        let obj = js_sys::Object::new();
        let eids: Vec<u32> = result.entity_ids.into_iter().map(|id| id.0 as u32).collect();
        let fids: Vec<u32> = result.edge_ids.into_iter().map(|id| id.0 as u32).collect();
        js_sys::Reflect::set(
            &obj,
            &"entityIds".into(),
            &serde_wasm_bindgen::to_value(&eids).unwrap(),
        )?;
        js_sys::Reflect::set(
            &obj,
            &"edgeIds".into(),
            &serde_wasm_bindgen::to_value(&fids).unwrap(),
        )?;
        Ok(obj.into())
    }

    /// Get neighbor entity IDs.
    #[wasm_bindgen]
    pub fn neighbors(&self, entity_id: u32) -> Result<Vec<u32>, JsValue> {
        let ids = self
            .inner
            .neighbors(EntityId(entity_id as u64))
            .map_err(to_js)?;
        Ok(ids.into_iter().map(|id| id.0 as u32).collect())
    }

    // ── Episodes ─────────────────────────────────────────────

    /// Record an episode.
    #[wasm_bindgen(js_name = "addEpisode")]
    pub fn add_episode(
        &mut self,
        source: &str,
        session_id: &str,
        entity_ids: Vec<u32>,
        fact_ids: Vec<u32>,
    ) -> Result<u32, JsValue> {
        let ep_source = match source {
            "conversation" => EpisodeSource::Conversation,
            "document" => EpisodeSource::Document,
            "api" => EpisodeSource::Api,
            _ => return Err(JsValue::from_str(&format!("unknown source: '{}'", source))),
        };
        let eids: Vec<EntityId> = entity_ids.into_iter().map(|id| EntityId(id as u64)).collect();
        let fids: Vec<EdgeId> = fact_ids.into_iter().map(|id| EdgeId(id as u64)).collect();
        let id = self
            .inner
            .add_episode(ep_source, session_id, &eids, &fids)
            .map_err(to_js)?;
        Ok(id as u32)
    }

    // ── Stats ────────────────────────────────────────────────

    /// Get summary statistics.
    #[wasm_bindgen]
    pub fn stats(&self) -> Result<JsValue, JsValue> {
        let s = self.inner.stats().map_err(to_js)?;
        let obj = js_sys::Object::new();
        js_sys::Reflect::set(&obj, &"entities".into(), &(s.entities as u32).into())?;
        js_sys::Reflect::set(&obj, &"edges".into(), &(s.edges as u32).into())?;
        js_sys::Reflect::set(&obj, &"episodes".into(), &(s.episodes as u32).into())?;
        Ok(obj.into())
    }
}

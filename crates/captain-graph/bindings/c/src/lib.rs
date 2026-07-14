#![allow(clippy::missing_safety_doc)]
//! C FFI binding for hora-graph-core.
//!
//! Opaque pointer pattern: the caller receives `*mut HoraCore` and passes it
//! to every function. Errors are retrievable via `hora_last_error()`.
//! The caller must free all returned strings/arrays with `hora_free_*` functions.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::slice;

use hora_graph_core::{
    EdgeId, EntityId, EntityUpdate, EpisodeSource, FactUpdate, HoraConfig,
    HoraCore, Properties, PropertyValue, SearchOpts, TraverseOpts,
};

// ── Thread-local error ──────────────────────────────────────

thread_local! {
    static LAST_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

fn set_error(msg: &str) {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = CString::new(msg).ok();
    });
}

fn clear_error() {
    LAST_ERROR.with(|e| {
        *e.borrow_mut() = None;
    });
}

fn from_hora_err(e: hora_graph_core::HoraError) -> i32 {
    set_error(&e.to_string());
    -1
}

// ── Helpers ─────────────────────────────────────────────────

unsafe fn cstr_to_str<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(s) }.to_str().ok()
}

fn string_to_c(s: String) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => ptr::null_mut(),
    }
}

fn props_from_json(json: *const c_char) -> Option<Properties> {
    let s = unsafe { cstr_to_str(json)? };
    let map: std::collections::HashMap<String, serde_json::Value> =
        serde_json::from_str(s).ok()?;
    let mut props = Properties::new();
    for (k, v) in map {
        let pv = match v {
            serde_json::Value::Bool(b) => PropertyValue::Bool(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    PropertyValue::Int(i)
                } else {
                    PropertyValue::Float(n.as_f64().unwrap_or(0.0))
                }
            }
            serde_json::Value::String(s) => PropertyValue::String(s),
            _ => PropertyValue::String(v.to_string()),
        };
        props.insert(k, pv);
    }
    Some(props)
}

fn props_to_json(props: &Properties) -> String {
    let map: std::collections::HashMap<&str, serde_json::Value> = props
        .iter()
        .map(|(k, v)| {
            let jv = match v {
                PropertyValue::String(s) => serde_json::Value::String(s.clone()),
                PropertyValue::Int(i) => serde_json::json!(*i),
                PropertyValue::Float(f) => serde_json::json!(*f),
                PropertyValue::Bool(b) => serde_json::Value::Bool(*b),
                _ => serde_json::Value::String(format!("{:?}", v)),
            };
            (k.as_str(), jv)
        })
        .collect();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

// ── Result types ────────────────────────────────────────────

/// An entity returned from the FFI layer.
#[repr(C)]
pub struct HoraEntity {
    pub id: u64,
    pub entity_type: *mut c_char,
    pub name: *mut c_char,
    pub properties_json: *mut c_char,
    pub embedding: *mut f32,
    pub embedding_len: u32,
    pub created_at: i64,
}

/// A fact (edge) returned from the FFI layer.
#[repr(C)]
pub struct HoraFact {
    pub id: u64,
    pub source: u64,
    pub target: u64,
    pub relation_type: *mut c_char,
    pub description: *mut c_char,
    pub confidence: f32,
    pub valid_at: i64,
    pub invalid_at: i64,
    pub created_at: i64,
}

/// A search hit returned from the FFI layer.
#[repr(C)]
pub struct HoraSearchHit {
    pub entity_id: u64,
    pub score: f32,
}

/// Traversal result returned from the FFI layer.
#[repr(C)]
pub struct HoraTraverseResult {
    pub entity_ids: *mut u64,
    pub entity_count: u32,
    pub edge_ids: *mut u64,
    pub edge_count: u32,
}

/// Storage statistics.
#[repr(C)]
pub struct HoraStats {
    pub entities: u64,
    pub edges: u64,
    pub episodes: u64,
}

// ── Error ───────────────────────────────────────────────────

/// Get the last error message. Returns NULL if no error.
/// The returned pointer is valid until the next FFI call on the same thread.
/// Do NOT free this pointer.
#[no_mangle]
pub extern "C" fn hora_last_error() -> *const c_char {
    LAST_ERROR.with(|e| {
        match e.borrow().as_ref() {
            Some(cs) => cs.as_ptr(),
            None => ptr::null(),
        }
    })
}

// ── Lifecycle ───────────────────────────────────────────────

/// Create a new in-memory HoraCore instance.
/// Returns NULL on failure (check `hora_last_error()`).
#[no_mangle]
pub extern "C" fn hora_new(embedding_dims: u16) -> *mut HoraCore {
    clear_error();
    let config = HoraConfig { embedding_dims, ..Default::default() };
    match HoraCore::new(config) {
        Ok(core) => Box::into_raw(Box::new(core)),
        Err(e) => {
            set_error(&e.to_string());
            ptr::null_mut()
        }
    }
}

/// Open a file-backed HoraCore instance.
/// Returns NULL on failure (check `hora_last_error()`).
#[no_mangle]
pub unsafe extern "C" fn hora_open(path: *const c_char, embedding_dims: u16) -> *mut HoraCore {
    clear_error();
    let path_str = match unsafe { cstr_to_str(path) } {
        Some(s) => s,
        None => {
            set_error("path is null or invalid UTF-8");
            return ptr::null_mut();
        }
    };
    let config = HoraConfig { embedding_dims, ..Default::default() };
    match HoraCore::open(path_str, config) {
        Ok(core) => Box::into_raw(Box::new(core)),
        Err(e) => {
            set_error(&e.to_string());
            ptr::null_mut()
        }
    }
}

/// Free a HoraCore instance.
#[no_mangle]
pub unsafe extern "C" fn hora_free(core: *mut HoraCore) {
    if !core.is_null() {
        drop(unsafe { Box::from_raw(core) });
    }
}

// ── Persistence ─────────────────────────────────────────────

/// Flush all data to the backing file. Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_flush(core: *mut HoraCore) -> i32 {
    clear_error();
    let core = unsafe { &*core };
    match core.flush() {
        Ok(()) => 0,
        Err(e) => from_hora_err(e),
    }
}

/// Copy current state to a snapshot file. Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_snapshot(core: *mut HoraCore, dest: *const c_char) -> i32 {
    clear_error();
    let core = unsafe { &*core };
    let dest_str = match unsafe { cstr_to_str(dest) } {
        Some(s) => s,
        None => {
            set_error("dest is null or invalid UTF-8");
            return -1;
        }
    };
    match core.snapshot(dest_str) {
        Ok(()) => 0,
        Err(e) => from_hora_err(e),
    }
}

// ── CRUD Entities ───────────────────────────────────────────

/// Add a new entity. Returns entity ID on success, 0 on error.
/// `properties_json` is a JSON string (or NULL for no properties).
/// `embedding` is a pointer to f32 array (or NULL), with `embedding_len` elements.
#[no_mangle]
pub unsafe extern "C" fn hora_add_entity(
    core: *mut HoraCore,
    entity_type: *const c_char,
    name: *const c_char,
    properties_json: *const c_char,
    embedding: *const f32,
    embedding_len: u32,
) -> u64 {
    clear_error();
    let core = unsafe { &mut *core };
    let etype = match unsafe { cstr_to_str(entity_type) } {
        Some(s) => s,
        None => { set_error("entity_type is null or invalid UTF-8"); return 0; }
    };
    let n = match unsafe { cstr_to_str(name) } {
        Some(s) => s,
        None => { set_error("name is null or invalid UTF-8"); return 0; }
    };
    let props = if properties_json.is_null() { None } else { props_from_json(properties_json) };
    let emb = if embedding.is_null() || embedding_len == 0 {
        None
    } else {
        Some(unsafe { slice::from_raw_parts(embedding, embedding_len as usize) })
    };
    match core.add_entity(etype, n, props, emb) {
        Ok(id) => id.0,
        Err(e) => { from_hora_err(e); 0 }
    }
}

/// Get an entity by ID. Returns a heap-allocated HoraEntity, or NULL if not found.
/// The caller must free with `hora_free_entity()`.
#[no_mangle]
pub unsafe extern "C" fn hora_get_entity(core: *mut HoraCore, id: u64) -> *mut HoraEntity {
    clear_error();
    let core = unsafe { &mut *core };
    match core.get_entity(EntityId(id)) {
        Ok(Some(e)) => {
            let (emb_ptr, emb_len) = match &e.embedding {
                Some(v) => {
                    let mut boxed = v.clone().into_boxed_slice();
                    let ptr = boxed.as_mut_ptr();
                    let len = boxed.len() as u32;
                    std::mem::forget(boxed);
                    (ptr, len)
                }
                None => (ptr::null_mut(), 0),
            };
            let out = HoraEntity {
                id: e.id.0,
                entity_type: string_to_c(e.entity_type),
                name: string_to_c(e.name),
                properties_json: string_to_c(props_to_json(&e.properties)),
                embedding: emb_ptr,
                embedding_len: emb_len,
                created_at: e.created_at,
            };
            Box::into_raw(Box::new(out))
        }
        Ok(None) => ptr::null_mut(),
        Err(e) => { from_hora_err(e); ptr::null_mut() }
    }
}

/// Free a HoraEntity returned by `hora_get_entity`.
#[no_mangle]
pub unsafe extern "C" fn hora_free_entity(entity: *mut HoraEntity) {
    if entity.is_null() { return; }
    let e = unsafe { Box::from_raw(entity) };
    if !e.entity_type.is_null() { drop(unsafe { CString::from_raw(e.entity_type) }); }
    if !e.name.is_null() { drop(unsafe { CString::from_raw(e.name) }); }
    if !e.properties_json.is_null() { drop(unsafe { CString::from_raw(e.properties_json) }); }
    if !e.embedding.is_null() && e.embedding_len > 0 {
        drop(unsafe { Vec::from_raw_parts(e.embedding, e.embedding_len as usize, e.embedding_len as usize) });
    }
}

/// Update an entity. Pass NULL for fields you don't want to change.
/// Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_update_entity(
    core: *mut HoraCore,
    id: u64,
    name: *const c_char,
    entity_type: *const c_char,
    properties_json: *const c_char,
) -> i32 {
    clear_error();
    let core = unsafe { &mut *core };
    let update = EntityUpdate {
        name: unsafe { cstr_to_str(name) }.map(String::from),
        entity_type: unsafe { cstr_to_str(entity_type) }.map(String::from),
        properties: if properties_json.is_null() { None } else { props_from_json(properties_json) },
        embedding: None,
    };
    match core.update_entity(EntityId(id), update) {
        Ok(()) => 0,
        Err(e) => from_hora_err(e),
    }
}

/// Delete an entity. Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_delete_entity(core: *mut HoraCore, id: u64) -> i32 {
    clear_error();
    let core = unsafe { &mut *core };
    match core.delete_entity(EntityId(id)) {
        Ok(()) => 0,
        Err(e) => from_hora_err(e),
    }
}

// ── CRUD Facts ──────────────────────────────────────────────

/// Add a fact. Returns fact ID on success, 0 on error.
/// Pass `confidence < 0` to use the default.
#[no_mangle]
pub unsafe extern "C" fn hora_add_fact(
    core: *mut HoraCore,
    source: u64,
    target: u64,
    relation: *const c_char,
    description: *const c_char,
    confidence: f32,
) -> u64 {
    clear_error();
    let core = unsafe { &mut *core };
    let rel = match unsafe { cstr_to_str(relation) } {
        Some(s) => s,
        None => { set_error("relation is null or invalid UTF-8"); return 0; }
    };
    let desc = unsafe { cstr_to_str(description) }.unwrap_or("");
    let conf = if confidence < 0.0 { None } else { Some(confidence) };
    match core.add_fact(EntityId(source), EntityId(target), rel, desc, conf) {
        Ok(id) => id.0,
        Err(e) => { from_hora_err(e); 0 }
    }
}

/// Get a fact by ID. Returns a heap-allocated HoraFact, or NULL if not found.
/// The caller must free with `hora_free_fact()`.
#[no_mangle]
pub unsafe extern "C" fn hora_get_fact(core: *mut HoraCore, id: u64) -> *mut HoraFact {
    clear_error();
    let core = unsafe { &*core };
    match core.get_fact(EdgeId(id)) {
        Ok(Some(e)) => {
            let out = HoraFact {
                id: e.id.0,
                source: e.source.0,
                target: e.target.0,
                relation_type: string_to_c(e.relation_type),
                description: string_to_c(e.description),
                confidence: e.confidence,
                valid_at: e.valid_at,
                invalid_at: e.invalid_at,
                created_at: e.created_at,
            };
            Box::into_raw(Box::new(out))
        }
        Ok(None) => ptr::null_mut(),
        Err(e) => { from_hora_err(e); ptr::null_mut() }
    }
}

/// Free a HoraFact returned by `hora_get_fact`.
#[no_mangle]
pub unsafe extern "C" fn hora_free_fact(fact: *mut HoraFact) {
    if fact.is_null() { return; }
    let f = unsafe { Box::from_raw(fact) };
    if !f.relation_type.is_null() { drop(unsafe { CString::from_raw(f.relation_type) }); }
    if !f.description.is_null() { drop(unsafe { CString::from_raw(f.description) }); }
}

/// Update a fact. Pass `confidence < 0` to keep unchanged.
/// Pass NULL description to keep unchanged. Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_update_fact(
    core: *mut HoraCore,
    id: u64,
    confidence: f32,
    description: *const c_char,
) -> i32 {
    clear_error();
    let core = unsafe { &mut *core };
    let update = FactUpdate {
        confidence: if confidence < 0.0 { None } else { Some(confidence) },
        description: unsafe { cstr_to_str(description) }.map(String::from),
    };
    match core.update_fact(EdgeId(id), update) {
        Ok(()) => 0,
        Err(e) => from_hora_err(e),
    }
}

/// Invalidate a fact (bi-temporal soft-delete). Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_invalidate_fact(core: *mut HoraCore, id: u64) -> i32 {
    clear_error();
    let core = unsafe { &mut *core };
    match core.invalidate_fact(EdgeId(id)) {
        Ok(()) => 0,
        Err(e) => from_hora_err(e),
    }
}

/// Delete a fact permanently. Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_delete_fact(core: *mut HoraCore, id: u64) -> i32 {
    clear_error();
    let core = unsafe { &mut *core };
    match core.delete_fact(EdgeId(id)) {
        Ok(()) => 0,
        Err(e) => from_hora_err(e),
    }
}

/// Get all facts involving an entity.
/// Returns an array of HoraFact pointers. `out_count` receives the count.
/// The caller must free each fact with `hora_free_fact()` and the array with `hora_free_fact_array()`.
#[no_mangle]
pub unsafe extern "C" fn hora_get_entity_facts(
    core: *mut HoraCore,
    entity_id: u64,
    out_count: *mut u32,
) -> *mut *mut HoraFact {
    clear_error();
    let core = unsafe { &*core };
    match core.get_entity_facts(EntityId(entity_id)) {
        Ok(edges) => {
            let mut ptrs: Vec<*mut HoraFact> = edges.iter().map(|e| {
                let out = HoraFact {
                    id: e.id.0,
                    source: e.source.0,
                    target: e.target.0,
                    relation_type: string_to_c(e.relation_type.clone()),
                    description: string_to_c(e.description.clone()),
                    confidence: e.confidence,
                    valid_at: e.valid_at,
                    invalid_at: e.invalid_at,
                    created_at: e.created_at,
                };
                Box::into_raw(Box::new(out))
            }).collect();
            if !out_count.is_null() {
                unsafe { *out_count = ptrs.len() as u32 };
            }
            let ptr = ptrs.as_mut_ptr();
            std::mem::forget(ptrs);
            ptr
        }
        Err(e) => {
            from_hora_err(e);
            if !out_count.is_null() { unsafe { *out_count = 0 }; }
            ptr::null_mut()
        }
    }
}

/// Free a fact array returned by `hora_get_entity_facts`.
#[no_mangle]
pub unsafe extern "C" fn hora_free_fact_array(facts: *mut *mut HoraFact, count: u32) {
    if facts.is_null() { return; }
    let arr = unsafe { Vec::from_raw_parts(facts, count as usize, count as usize) };
    for ptr in arr {
        unsafe { hora_free_fact(ptr) };
    }
}

// ── Search ──────────────────────────────────────────────────

/// Hybrid search. Returns array of HoraSearchHit. `out_count` receives the count.
/// Pass NULL for `query` or `embedding` to skip that leg.
/// The caller must free with `hora_free_search_hits()`.
#[no_mangle]
pub unsafe extern "C" fn hora_search(
    core: *mut HoraCore,
    query: *const c_char,
    embedding: *const f32,
    embedding_len: u32,
    top_k: u32,
    out_count: *mut u32,
) -> *mut HoraSearchHit {
    clear_error();
    let core = unsafe { &mut *core };
    let q = unsafe { cstr_to_str(query) };
    let emb = if embedding.is_null() || embedding_len == 0 {
        None
    } else {
        Some(unsafe { slice::from_raw_parts(embedding, embedding_len as usize) })
    };
    let opts = SearchOpts { top_k: top_k as usize, ..Default::default() };
    match core.search(q, emb, opts) {
        Ok(hits) => {
            let mut result: Vec<HoraSearchHit> = hits.iter().map(|h| HoraSearchHit {
                entity_id: h.entity_id.0,
                score: h.score,
            }).collect();
            if !out_count.is_null() {
                unsafe { *out_count = result.len() as u32 };
            }
            let ptr = result.as_mut_ptr();
            std::mem::forget(result);
            ptr
        }
        Err(e) => {
            from_hora_err(e);
            if !out_count.is_null() { unsafe { *out_count = 0 }; }
            ptr::null_mut()
        }
    }
}

/// Free search hits returned by `hora_search`.
#[no_mangle]
pub unsafe extern "C" fn hora_free_search_hits(hits: *mut HoraSearchHit, count: u32) {
    if hits.is_null() { return; }
    drop(unsafe { Vec::from_raw_parts(hits, count as usize, count as usize) });
}

// ── Traversal ───────────────────────────────────────────────

/// BFS traversal. Returns a heap-allocated HoraTraverseResult, or NULL on error.
/// The caller must free with `hora_free_traverse_result()`.
#[no_mangle]
pub unsafe extern "C" fn hora_traverse(
    core: *mut HoraCore,
    start_id: u64,
    depth: u32,
) -> *mut HoraTraverseResult {
    clear_error();
    let core = unsafe { &*core };
    let opts = TraverseOpts { depth };
    match core.traverse(EntityId(start_id), opts) {
        Ok(result) => {
            let mut eids: Vec<u64> = result.entity_ids.into_iter().map(|id| id.0).collect();
            let mut fids: Vec<u64> = result.edge_ids.into_iter().map(|id| id.0).collect();
            let out = HoraTraverseResult {
                entity_count: eids.len() as u32,
                edge_count: fids.len() as u32,
                entity_ids: eids.as_mut_ptr(),
                edge_ids: fids.as_mut_ptr(),
            };
            std::mem::forget(eids);
            std::mem::forget(fids);
            Box::into_raw(Box::new(out))
        }
        Err(e) => { from_hora_err(e); ptr::null_mut() }
    }
}

/// Free a HoraTraverseResult returned by `hora_traverse`.
#[no_mangle]
pub unsafe extern "C" fn hora_free_traverse_result(result: *mut HoraTraverseResult) {
    if result.is_null() { return; }
    let r = unsafe { Box::from_raw(result) };
    if !r.entity_ids.is_null() && r.entity_count > 0 {
        drop(unsafe { Vec::from_raw_parts(r.entity_ids, r.entity_count as usize, r.entity_count as usize) });
    }
    if !r.edge_ids.is_null() && r.edge_count > 0 {
        drop(unsafe { Vec::from_raw_parts(r.edge_ids, r.edge_count as usize, r.edge_count as usize) });
    }
}

/// Get neighbor entity IDs. Returns array of u64. `out_count` receives the count.
/// The caller must free with `hora_free_ids()`.
#[no_mangle]
pub unsafe extern "C" fn hora_neighbors(
    core: *mut HoraCore,
    entity_id: u64,
    out_count: *mut u32,
) -> *mut u64 {
    clear_error();
    let core = unsafe { &*core };
    match core.neighbors(EntityId(entity_id)) {
        Ok(ids) => {
            let mut result: Vec<u64> = ids.into_iter().map(|id| id.0).collect();
            if !out_count.is_null() {
                unsafe { *out_count = result.len() as u32 };
            }
            let ptr = result.as_mut_ptr();
            std::mem::forget(result);
            ptr
        }
        Err(e) => {
            from_hora_err(e);
            if !out_count.is_null() { unsafe { *out_count = 0 }; }
            ptr::null_mut()
        }
    }
}

/// Free an array of u64 IDs returned by `hora_neighbors`.
#[no_mangle]
pub unsafe extern "C" fn hora_free_ids(ids: *mut u64, count: u32) {
    if ids.is_null() { return; }
    drop(unsafe { Vec::from_raw_parts(ids, count as usize, count as usize) });
}

// ── Episodes ────────────────────────────────────────────────

/// Record an episode. `source` is "conversation", "document", or "api".
/// Returns episode ID on success, 0 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_add_episode(
    core: *mut HoraCore,
    source: *const c_char,
    session_id: *const c_char,
    entity_ids: *const u64,
    entity_count: u32,
    fact_ids: *const u64,
    fact_count: u32,
) -> u64 {
    clear_error();
    let core = unsafe { &mut *core };
    let src_str = match unsafe { cstr_to_str(source) } {
        Some(s) => s,
        None => { set_error("source is null or invalid UTF-8"); return 0; }
    };
    let sess = match unsafe { cstr_to_str(session_id) } {
        Some(s) => s,
        None => { set_error("session_id is null or invalid UTF-8"); return 0; }
    };
    let ep_source = match src_str {
        "conversation" => EpisodeSource::Conversation,
        "document" => EpisodeSource::Document,
        "api" => EpisodeSource::Api,
        _ => { set_error(&format!("unknown source: '{}'", src_str)); return 0; }
    };
    let eids: Vec<EntityId> = if entity_ids.is_null() || entity_count == 0 {
        vec![]
    } else {
        unsafe { slice::from_raw_parts(entity_ids, entity_count as usize) }
            .iter().map(|&id| EntityId(id)).collect()
    };
    let fids: Vec<EdgeId> = if fact_ids.is_null() || fact_count == 0 {
        vec![]
    } else {
        unsafe { slice::from_raw_parts(fact_ids, fact_count as usize) }
            .iter().map(|&id| EdgeId(id)).collect()
    };
    match core.add_episode(ep_source, sess, &eids, &fids) {
        Ok(id) => id,
        Err(e) => { from_hora_err(e); 0 }
    }
}

// ── Stats ───────────────────────────────────────────────────

/// Get summary statistics. Returns 0 on success, -1 on error.
#[no_mangle]
pub unsafe extern "C" fn hora_stats(core: *mut HoraCore, out: *mut HoraStats) -> i32 {
    clear_error();
    let core = unsafe { &*core };
    match core.stats() {
        Ok(s) => {
            if !out.is_null() {
                unsafe {
                    (*out).entities = s.entities;
                    (*out).edges = s.edges;
                    (*out).episodes = s.episodes;
                }
            }
            0
        }
        Err(e) => from_hora_err(e),
    }
}

// ── Free strings ────────────────────────────────────────────

/// Free a C string returned by any hora function.
#[no_mangle]
pub unsafe extern "C" fn hora_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_lifecycle() {
        let core = hora_new(0);
        assert!(!core.is_null());
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_add_and_get_entity() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let name = CString::new("Alice").unwrap();

        let id = unsafe {
            hora_add_entity(core, etype.as_ptr(), name.as_ptr(), ptr::null(), ptr::null(), 0)
        };
        assert!(id > 0);

        let entity = unsafe { hora_get_entity(core, id) };
        assert!(!entity.is_null());

        let e = unsafe { &*entity };
        assert_eq!(e.id, id);
        let got_name = unsafe { CStr::from_ptr(e.name) }.to_str().unwrap();
        assert_eq!(got_name, "Alice");

        unsafe { hora_free_entity(entity) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_entity_not_found() {
        let core = hora_new(0);
        let entity = unsafe { hora_get_entity(core, 9999) };
        assert!(entity.is_null());
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_entity_with_properties() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let name = CString::new("Bob").unwrap();
        let props = CString::new(r#"{"age":42,"active":true,"city":"Paris"}"#).unwrap();

        let id = unsafe {
            hora_add_entity(core, etype.as_ptr(), name.as_ptr(), props.as_ptr(), ptr::null(), 0)
        };
        assert!(id > 0);

        let entity = unsafe { hora_get_entity(core, id) };
        let e = unsafe { &*entity };
        let json_str = unsafe { CStr::from_ptr(e.properties_json) }.to_str().unwrap();
        let parsed: serde_json::Value = serde_json::from_str(json_str).unwrap();
        assert_eq!(parsed["age"], 42);
        assert_eq!(parsed["active"], true);
        assert_eq!(parsed["city"], "Paris");

        unsafe { hora_free_entity(entity) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_entity_with_embedding() {
        let core = hora_new(3);
        let etype = CString::new("concept").unwrap();
        let name = CString::new("North").unwrap();
        let emb = [1.0f32, 0.0, 0.0];

        let id = unsafe {
            hora_add_entity(core, etype.as_ptr(), name.as_ptr(), ptr::null(), emb.as_ptr(), 3)
        };

        let entity = unsafe { hora_get_entity(core, id) };
        let e = unsafe { &*entity };
        assert_eq!(e.embedding_len, 3);
        let slice = unsafe { slice::from_raw_parts(e.embedding, 3) };
        assert_eq!(slice[0], 1.0);

        unsafe { hora_free_entity(entity) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_update_entity() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let name = CString::new("Alice").unwrap();
        let id = unsafe {
            hora_add_entity(core, etype.as_ptr(), name.as_ptr(), ptr::null(), ptr::null(), 0)
        };

        let new_name = CString::new("Bob").unwrap();
        let rc = unsafe { hora_update_entity(core, id, new_name.as_ptr(), ptr::null(), ptr::null()) };
        assert_eq!(rc, 0);

        let entity = unsafe { hora_get_entity(core, id) };
        let got_name = unsafe { CStr::from_ptr((*entity).name) }.to_str().unwrap();
        assert_eq!(got_name, "Bob");

        unsafe { hora_free_entity(entity) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_delete_entity() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let name = CString::new("Alice").unwrap();
        let id = unsafe {
            hora_add_entity(core, etype.as_ptr(), name.as_ptr(), ptr::null(), ptr::null(), 0)
        };
        let rc = unsafe { hora_delete_entity(core, id) };
        assert_eq!(rc, 0);

        let entity = unsafe { hora_get_entity(core, id) };
        assert!(entity.is_null());

        unsafe { hora_free(core) };
    }

    #[test]
    fn test_add_and_get_fact() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("Alice").unwrap();
        let n2 = CString::new("Bob").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        let b = unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };

        let rel = CString::new("knows").unwrap();
        let desc = CString::new("met at conf").unwrap();
        let fid = unsafe { hora_add_fact(core, a, b, rel.as_ptr(), desc.as_ptr(), 0.9) };
        assert!(fid > 0);

        let fact = unsafe { hora_get_fact(core, fid) };
        assert!(!fact.is_null());
        let f = unsafe { &*fact };
        assert_eq!(f.source, a);
        assert_eq!(f.target, b);
        let got_rel = unsafe { CStr::from_ptr(f.relation_type) }.to_str().unwrap();
        assert_eq!(got_rel, "knows");
        assert!((f.confidence - 0.9).abs() < 0.01);

        unsafe { hora_free_fact(fact) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_update_fact() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("A").unwrap();
        let n2 = CString::new("B").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        let b = unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };
        let rel = CString::new("knows").unwrap();
        let desc = CString::new("initial").unwrap();
        let fid = unsafe { hora_add_fact(core, a, b, rel.as_ptr(), desc.as_ptr(), -1.0) };

        let new_desc = CString::new("updated").unwrap();
        let rc = unsafe { hora_update_fact(core, fid, 0.5, new_desc.as_ptr()) };
        assert_eq!(rc, 0);

        let fact = unsafe { hora_get_fact(core, fid) };
        let f = unsafe { &*fact };
        let got_desc = unsafe { CStr::from_ptr(f.description) }.to_str().unwrap();
        assert_eq!(got_desc, "updated");
        assert!((f.confidence - 0.5).abs() < 0.01);

        unsafe { hora_free_fact(fact) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_invalidate_fact() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("A").unwrap();
        let n2 = CString::new("B").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        let b = unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };
        let rel = CString::new("knows").unwrap();
        let fid = unsafe { hora_add_fact(core, a, b, rel.as_ptr(), ptr::null(), -1.0) };

        let rc = unsafe { hora_invalidate_fact(core, fid) };
        assert_eq!(rc, 0);

        let fact = unsafe { hora_get_fact(core, fid) };
        let f = unsafe { &*fact };
        assert!(f.invalid_at > 0);

        unsafe { hora_free_fact(fact) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_delete_fact() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("A").unwrap();
        let n2 = CString::new("B").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        let b = unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };
        let rel = CString::new("knows").unwrap();
        let fid = unsafe { hora_add_fact(core, a, b, rel.as_ptr(), ptr::null(), -1.0) };

        let rc = unsafe { hora_delete_fact(core, fid) };
        assert_eq!(rc, 0);

        let fact = unsafe { hora_get_fact(core, fid) };
        assert!(fact.is_null());

        unsafe { hora_free(core) };
    }

    #[test]
    fn test_get_entity_facts() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("A").unwrap();
        let n2 = CString::new("B").unwrap();
        let n3 = CString::new("C").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        let b = unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };
        let c = unsafe { hora_add_entity(core, etype.as_ptr(), n3.as_ptr(), ptr::null(), ptr::null(), 0) };
        let rel = CString::new("knows").unwrap();
        unsafe { hora_add_fact(core, a, b, rel.as_ptr(), ptr::null(), -1.0) };
        unsafe { hora_add_fact(core, a, c, rel.as_ptr(), ptr::null(), -1.0) };

        let mut count: u32 = 0;
        let facts = unsafe { hora_get_entity_facts(core, a, &mut count) };
        assert_eq!(count, 2);

        unsafe { hora_free_fact_array(facts, count) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_text_search() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("Alice Wonderland").unwrap();
        let n2 = CString::new("Bob Builder").unwrap();
        unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };

        let query = CString::new("Alice").unwrap();
        let mut count: u32 = 0;
        let hits = unsafe { hora_search(core, query.as_ptr(), ptr::null(), 0, 5, &mut count) };
        assert!(count >= 1);

        let slice = unsafe { slice::from_raw_parts(hits, count as usize) };
        assert!(slice[0].score > 0.0);

        unsafe { hora_free_search_hits(hits, count) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_vector_search() {
        let core = hora_new(3);
        let etype = CString::new("concept").unwrap();
        let n1 = CString::new("North").unwrap();
        let n2 = CString::new("South").unwrap();
        let e1 = [1.0f32, 0.0, 0.0];
        let e2 = [-1.0f32, 0.0, 0.0];
        unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), e1.as_ptr(), 3) };
        unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), e2.as_ptr(), 3) };

        let q_emb = [1.0f32, 0.0, 0.0];
        let mut count: u32 = 0;
        let hits = unsafe { hora_search(core, ptr::null(), q_emb.as_ptr(), 3, 2, &mut count) };
        assert!(count >= 1);

        // First result should be "North"
        let slice = unsafe { slice::from_raw_parts(hits, count as usize) };
        let entity = unsafe { hora_get_entity(core, slice[0].entity_id) };
        let got_name = unsafe { CStr::from_ptr((*entity).name) }.to_str().unwrap();
        assert_eq!(got_name, "North");

        unsafe { hora_free_entity(entity) };
        unsafe { hora_free_search_hits(hits, count) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_traverse() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("A").unwrap();
        let n2 = CString::new("B").unwrap();
        let n3 = CString::new("C").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        let b = unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };
        let c = unsafe { hora_add_entity(core, etype.as_ptr(), n3.as_ptr(), ptr::null(), ptr::null(), 0) };
        let rel = CString::new("knows").unwrap();
        unsafe { hora_add_fact(core, a, b, rel.as_ptr(), ptr::null(), -1.0) };
        unsafe { hora_add_fact(core, b, c, rel.as_ptr(), ptr::null(), -1.0) };

        let result = unsafe { hora_traverse(core, a, 2) };
        assert!(!result.is_null());
        let r = unsafe { &*result };
        assert!(r.entity_count >= 2);

        unsafe { hora_free_traverse_result(result) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_neighbors() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("A").unwrap();
        let n2 = CString::new("B").unwrap();
        let n3 = CString::new("C").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        let b = unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };
        let c = unsafe { hora_add_entity(core, etype.as_ptr(), n3.as_ptr(), ptr::null(), ptr::null(), 0) };
        let rel = CString::new("knows").unwrap();
        unsafe { hora_add_fact(core, a, b, rel.as_ptr(), ptr::null(), -1.0) };
        unsafe { hora_add_fact(core, a, c, rel.as_ptr(), ptr::null(), -1.0) };

        let mut count: u32 = 0;
        let ids = unsafe { hora_neighbors(core, a, &mut count) };
        assert_eq!(count, 2);

        unsafe { hora_free_ids(ids, count) };
        unsafe { hora_free(core) };
    }

    #[test]
    fn test_episode() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("A").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };

        let src = CString::new("conversation").unwrap();
        let sess = CString::new("s1").unwrap();
        let eids = [a];
        let ep_id = unsafe {
            hora_add_episode(core, src.as_ptr(), sess.as_ptr(), eids.as_ptr(), 1, ptr::null(), 0)
        };
        assert!(ep_id > 0);

        unsafe { hora_free(core) };
    }

    #[test]
    fn test_episode_bad_source() {
        let core = hora_new(0);
        let src = CString::new("invalid").unwrap();
        let sess = CString::new("s1").unwrap();
        let ep_id = unsafe {
            hora_add_episode(core, src.as_ptr(), sess.as_ptr(), ptr::null(), 0, ptr::null(), 0)
        };
        assert_eq!(ep_id, 0);
        let err = hora_last_error();
        assert!(!err.is_null());

        unsafe { hora_free(core) };
    }

    #[test]
    fn test_stats() {
        let core = hora_new(0);
        let etype = CString::new("person").unwrap();
        let n1 = CString::new("A").unwrap();
        let n2 = CString::new("B").unwrap();
        let a = unsafe { hora_add_entity(core, etype.as_ptr(), n1.as_ptr(), ptr::null(), ptr::null(), 0) };
        let b = unsafe { hora_add_entity(core, etype.as_ptr(), n2.as_ptr(), ptr::null(), ptr::null(), 0) };
        let rel = CString::new("knows").unwrap();
        unsafe { hora_add_fact(core, a, b, rel.as_ptr(), ptr::null(), -1.0) };

        let mut stats = HoraStats { entities: 0, edges: 0, episodes: 0 };
        let rc = unsafe { hora_stats(core, &mut stats) };
        assert_eq!(rc, 0);
        assert_eq!(stats.entities, 2);
        assert_eq!(stats.edges, 1);
        assert_eq!(stats.episodes, 0);

        unsafe { hora_free(core) };
    }

    #[test]
    fn test_last_error_null_initially() {
        clear_error();
        let err = hora_last_error();
        assert!(err.is_null());
    }
}

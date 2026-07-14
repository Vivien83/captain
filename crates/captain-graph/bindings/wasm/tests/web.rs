//! wasm-bindgen tests for hora-graph-wasm.

use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

use hora_graph_wasm::HoraCore;

// ── Constructor ─────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_new_memory() {
    let _core = HoraCore::new_memory(None).unwrap();
}

#[wasm_bindgen_test]
fn test_new_memory_with_dims() {
    let _core = HoraCore::new_memory(Some(384)).unwrap();
}

// ── Entity CRUD ─────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_add_and_get_entity() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let id = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    assert!(id > 0);

    let entity = core.get_entity(id).unwrap();
    assert!(!entity.is_null());

    let name = js_sys::Reflect::get(&entity, &"name".into()).unwrap();
    assert_eq!(name.as_string().unwrap(), "Alice");

    let etype = js_sys::Reflect::get(&entity, &"entityType".into()).unwrap();
    assert_eq!(etype.as_string().unwrap(), "person");
}

#[wasm_bindgen_test]
fn test_get_entity_not_found() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let entity = core.get_entity(9999).unwrap();
    assert!(entity.is_null());
}

#[wasm_bindgen_test]
fn test_update_entity() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let id = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();

    core.update_entity(id, Some("Bob".into()), None, JsValue::NULL).unwrap();

    let entity = core.get_entity(id).unwrap();
    let name = js_sys::Reflect::get(&entity, &"name".into()).unwrap();
    assert_eq!(name.as_string().unwrap(), "Bob");
}

#[wasm_bindgen_test]
fn test_delete_entity() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let id = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    core.delete_entity(id).unwrap();

    let entity = core.get_entity(id).unwrap();
    assert!(entity.is_null());
}

// ── Properties ──────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_entity_with_properties() {
    let mut core = HoraCore::new_memory(None).unwrap();

    let props = js_sys::Object::new();
    js_sys::Reflect::set(&props, &"age".into(), &42.into()).unwrap();
    js_sys::Reflect::set(&props, &"active".into(), &true.into()).unwrap();
    js_sys::Reflect::set(&props, &"city".into(), &"Paris".into()).unwrap();

    let id = core.add_entity("person", "Alice", props.into(), None).unwrap();
    let entity = core.get_entity(id).unwrap();
    let p = js_sys::Reflect::get(&entity, &"properties".into()).unwrap();

    let age = js_sys::Reflect::get(&p, &"age".into()).unwrap();
    assert_eq!(age.as_f64().unwrap(), 42.0);

    let active = js_sys::Reflect::get(&p, &"active".into()).unwrap();
    assert_eq!(active.as_bool().unwrap(), true);

    let city = js_sys::Reflect::get(&p, &"city".into()).unwrap();
    assert_eq!(city.as_string().unwrap(), "Paris");
}

// ── Embeddings ──────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_entity_with_embedding() {
    let mut core = HoraCore::new_memory(Some(3)).unwrap();
    let emb = vec![1.0f32, 0.0, 0.0];
    let id = core.add_entity("concept", "North", JsValue::NULL, Some(emb)).unwrap();

    let entity = core.get_entity(id).unwrap();
    let embedding = js_sys::Reflect::get(&entity, &"embedding".into()).unwrap();
    assert!(!embedding.is_null());

    let arr = js_sys::Float32Array::from(embedding);
    assert_eq!(arr.length(), 3);
    assert_eq!(arr.get_index(0), 1.0);
}

// ── Fact CRUD ───────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_add_and_get_fact() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();

    let fid = core.add_fact(a, b, "knows", "met at conference", Some(0.9)).unwrap();
    assert!(fid > 0);

    let fact = core.get_fact(fid).unwrap();
    assert!(!fact.is_null());

    let rel = js_sys::Reflect::get(&fact, &"relationType".into()).unwrap();
    assert_eq!(rel.as_string().unwrap(), "knows");

    let conf = js_sys::Reflect::get(&fact, &"confidence".into()).unwrap();
    let c = conf.as_f64().unwrap();
    assert!((c - 0.9).abs() < 0.01);
}

#[wasm_bindgen_test]
fn test_update_fact() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();
    let fid = core.add_fact(a, b, "knows", "initial", None).unwrap();

    core.update_fact(fid, Some(0.5), Some("updated desc".into())).unwrap();

    let fact = core.get_fact(fid).unwrap();
    let desc = js_sys::Reflect::get(&fact, &"description".into()).unwrap();
    assert_eq!(desc.as_string().unwrap(), "updated desc");
}

#[wasm_bindgen_test]
fn test_invalidate_fact() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();
    let fid = core.add_fact(a, b, "knows", "", None).unwrap();

    core.invalidate_fact(fid).unwrap();

    let fact = core.get_fact(fid).unwrap();
    let invalid_at = js_sys::Reflect::get(&fact, &"invalidAt".into()).unwrap();
    assert!(invalid_at.as_f64().unwrap() > 0.0);
}

#[wasm_bindgen_test]
fn test_delete_fact() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();
    let fid = core.add_fact(a, b, "knows", "", None).unwrap();

    core.delete_fact(fid).unwrap();

    let fact = core.get_fact(fid).unwrap();
    assert!(fact.is_null());
}

#[wasm_bindgen_test]
fn test_get_entity_facts() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();
    let c = core.add_entity("person", "Carol", JsValue::NULL, None).unwrap();
    core.add_fact(a, b, "knows", "", None).unwrap();
    core.add_fact(a, c, "likes", "", None).unwrap();

    let facts = core.get_entity_facts(a).unwrap();
    let arr = js_sys::Array::from(&facts);
    assert_eq!(arr.length(), 2);
}

// ── Search ──────────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_text_search() {
    let mut core = HoraCore::new_memory(None).unwrap();
    core.add_entity("person", "Alice Wonderland", JsValue::NULL, None).unwrap();
    core.add_entity("person", "Bob Builder", JsValue::NULL, None).unwrap();

    let results = core.search(Some("Alice".into()), None, Some(5)).unwrap();
    let arr = js_sys::Array::from(&results);
    assert!(arr.length() >= 1);

    let first = arr.get(0);
    let score = js_sys::Reflect::get(&first, &"score".into()).unwrap();
    assert!(score.as_f64().unwrap() > 0.0);
}

#[wasm_bindgen_test]
fn test_vector_search() {
    let mut core = HoraCore::new_memory(Some(3)).unwrap();
    core.add_entity("concept", "North", JsValue::NULL, Some(vec![1.0, 0.0, 0.0])).unwrap();
    core.add_entity("concept", "South", JsValue::NULL, Some(vec![-1.0, 0.0, 0.0])).unwrap();
    core.add_entity("concept", "NorthEast", JsValue::NULL, Some(vec![0.7, 0.7, 0.0])).unwrap();

    let results = core.search(None, Some(vec![1.0, 0.0, 0.0]), Some(3)).unwrap();
    let arr = js_sys::Array::from(&results);
    assert!(arr.length() >= 1);

    // First result should be "North" (most similar)
    let first = arr.get(0);
    let eid = js_sys::Reflect::get(&first, &"entityId".into()).unwrap();
    let entity = core.get_entity(eid.as_f64().unwrap() as u32).unwrap();
    let name = js_sys::Reflect::get(&entity, &"name".into()).unwrap();
    assert_eq!(name.as_string().unwrap(), "North");
}

// ── Traversal ───────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_traverse() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();
    let c = core.add_entity("person", "Carol", JsValue::NULL, None).unwrap();
    core.add_fact(a, b, "knows", "", None).unwrap();
    core.add_fact(b, c, "knows", "", None).unwrap();

    let result = core.traverse(a, Some(2)).unwrap();
    let entity_ids = js_sys::Reflect::get(&result, &"entityIds".into()).unwrap();
    let arr: Vec<u32> = serde_wasm_bindgen::from_value(entity_ids).unwrap();
    assert!(arr.len() >= 2); // Should reach at least a and b
}

#[wasm_bindgen_test]
fn test_neighbors() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();
    let c = core.add_entity("person", "Carol", JsValue::NULL, None).unwrap();
    core.add_fact(a, b, "knows", "", None).unwrap();
    core.add_fact(a, c, "likes", "", None).unwrap();

    let ids = core.neighbors(a).unwrap();
    assert_eq!(ids.len(), 2);
}

// ── Episodes ────────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_add_episode() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();
    let fid = core.add_fact(a, b, "knows", "", None).unwrap();

    let entity_ids = js_sys::Uint32Array::new_with_length(2);
    entity_ids.set_index(0, a);
    entity_ids.set_index(1, b);
    let fact_ids = js_sys::Uint32Array::new_with_length(1);
    fact_ids.set_index(0, fid);

    let ep_id = core.add_episode("conversation", "sess-1", entity_ids.to_vec(), fact_ids.to_vec()).unwrap();
    assert!(ep_id > 0);
}

#[wasm_bindgen_test]
fn test_episode_sources() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("thing", "X", JsValue::NULL, None).unwrap();

    for src in &["conversation", "document", "api"] {
        let eids = vec![a];
        let fids = vec![];
        core.add_episode(src, "s1", eids, fids).unwrap();
    }

    let stats = core.stats().unwrap();
    let episodes = js_sys::Reflect::get(&stats, &"episodes".into()).unwrap();
    assert_eq!(episodes.as_f64().unwrap(), 3.0);
}

#[wasm_bindgen_test]
fn test_episode_bad_source() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let result = core.add_episode("invalid", "s1", vec![], vec![]);
    assert!(result.is_err());
}

// ── Stats ───────────────────────────────────────────────────

#[wasm_bindgen_test]
fn test_stats_empty() {
    let core = HoraCore::new_memory(None).unwrap();
    let stats = core.stats().unwrap();
    let entities = js_sys::Reflect::get(&stats, &"entities".into()).unwrap();
    assert_eq!(entities.as_f64().unwrap(), 0.0);
}

#[wasm_bindgen_test]
fn test_stats_after_operations() {
    let mut core = HoraCore::new_memory(None).unwrap();
    let a = core.add_entity("person", "Alice", JsValue::NULL, None).unwrap();
    let b = core.add_entity("person", "Bob", JsValue::NULL, None).unwrap();
    core.add_fact(a, b, "knows", "", None).unwrap();

    let stats = core.stats().unwrap();
    let entities = js_sys::Reflect::get(&stats, &"entities".into()).unwrap();
    assert_eq!(entities.as_f64().unwrap(), 2.0);
    let edges = js_sys::Reflect::get(&stats, &"edges".into()).unwrap();
    assert_eq!(edges.as_f64().unwrap(), 1.0);
}

//! Backend conformance tests.
//!
//! A single generic test suite exercised on every StorageOps backend to
//! guarantee identical behaviour regardless of the underlying storage.

use hora_graph_core::storage::memory::MemoryStorage;
use hora_graph_core::storage::traits::StorageOps;
use hora_graph_core::{
    Edge, EdgeId, Entity, EntityId, Episode, EpisodeSource, PropertyValue, StorageStats,
};
use std::collections::HashMap;

// ── Factories ────────────────────────────────────────────────

fn entity(id: u64, name: &str, etype: &str) -> Entity {
    Entity {
        id: EntityId(id),
        entity_type: etype.to_string(),
        name: name.to_string(),
        properties: HashMap::new(),
        embedding: None,
        created_at: 1000,
    }
}

fn entity_with_props(id: u64) -> Entity {
    let mut props = HashMap::new();
    props.insert("lang".to_string(), PropertyValue::String("Rust".into()));
    props.insert("stars".to_string(), PropertyValue::Int(42));
    props.insert("score".to_string(), PropertyValue::Float(9.5));
    props.insert("active".to_string(), PropertyValue::Bool(true));
    Entity {
        id: EntityId(id),
        entity_type: "project".to_string(),
        name: "hora".to_string(),
        properties: props,
        embedding: None,
        created_at: 1000,
    }
}

fn entity_with_embedding(id: u64) -> Entity {
    Entity {
        id: EntityId(id),
        entity_type: "vec".to_string(),
        name: "embedded".to_string(),
        properties: HashMap::new(),
        embedding: Some(vec![1.0, -0.5, 0.0, std::f32::consts::PI]),
        created_at: 1000,
    }
}

fn edge(id: u64, source: u64, target: u64) -> Edge {
    Edge {
        id: EdgeId(id),
        source: EntityId(source),
        target: EntityId(target),
        relation_type: "related_to".to_string(),
        description: "test".to_string(),
        confidence: 1.0,
        valid_at: 1000,
        invalid_at: 0,
        created_at: 1000,
    }
}

fn episode(id: u64) -> Episode {
    Episode {
        id,
        source: EpisodeSource::Conversation,
        session_id: "sess-1".to_string(),
        entity_ids: vec![EntityId(1), EntityId(2)],
        fact_ids: vec![EdgeId(10), EdgeId(20)],
        created_at: 2000,
        consolidation_count: 0,
    }
}

// ── Generic conformance suite ────────────────────────────────

fn test_backend_conformance(s: &mut dyn StorageOps) {
    // --- Empty state ---
    assert_eq!(
        s.stats(),
        StorageStats {
            entities: 0,
            edges: 0,
            episodes: 0
        }
    );
    assert!(s.get_entity(EntityId(1)).unwrap().is_none());
    assert!(s.get_edge(EdgeId(1)).unwrap().is_none());
    assert!(s.get_episode(1).unwrap().is_none());
    assert!(s.scan_all_entities().unwrap().is_empty());
    assert!(s.scan_all_edges().unwrap().is_empty());
    assert!(s.scan_all_episodes().unwrap().is_empty());

    // --- Entity CRUD ---
    s.put_entity(entity(1, "rust", "language")).unwrap();
    let got = s.get_entity(EntityId(1)).unwrap().unwrap();
    assert_eq!(got.name, "rust");
    assert_eq!(got.entity_type, "language");
    assert_eq!(got.created_at, 1000);

    // Upsert
    s.put_entity(entity(1, "rust-lang", "programming")).unwrap();
    let got = s.get_entity(EntityId(1)).unwrap().unwrap();
    assert_eq!(got.name, "rust-lang");
    assert_eq!(got.entity_type, "programming");

    // Delete
    assert!(s.delete_entity(EntityId(1)).unwrap());
    assert!(s.get_entity(EntityId(1)).unwrap().is_none());
    assert!(!s.delete_entity(EntityId(1)).unwrap()); // idempotent

    // --- Properties round-trip ---
    let ep = entity_with_props(10);
    s.put_entity(ep).unwrap();
    let got = s.get_entity(EntityId(10)).unwrap().unwrap();
    assert_eq!(
        got.properties.get("lang"),
        Some(&PropertyValue::String("Rust".into()))
    );
    assert_eq!(got.properties.get("stars"), Some(&PropertyValue::Int(42)));
    assert_eq!(
        got.properties.get("score"),
        Some(&PropertyValue::Float(9.5))
    );
    assert_eq!(
        got.properties.get("active"),
        Some(&PropertyValue::Bool(true))
    );
    assert_eq!(got.properties.len(), 4);

    // --- Embedding round-trip ---
    let ev = entity_with_embedding(20);
    s.put_entity(ev).unwrap();
    let got = s.get_entity(EntityId(20)).unwrap().unwrap();
    assert_eq!(
        got.embedding.unwrap(),
        vec![1.0, -0.5, 0.0, std::f32::consts::PI]
    );

    // No embedding
    s.put_entity(entity(21, "no-emb", "test")).unwrap();
    assert!(s
        .get_entity(EntityId(21))
        .unwrap()
        .unwrap()
        .embedding
        .is_none());

    // --- Edge CRUD ---
    s.put_entity(entity(100, "a", "node")).unwrap();
    s.put_entity(entity(101, "b", "node")).unwrap();
    s.put_entity(entity(102, "c", "node")).unwrap();

    s.put_edge(edge(200, 100, 101)).unwrap();
    let got = s.get_edge(EdgeId(200)).unwrap().unwrap();
    assert_eq!(got.source, EntityId(100));
    assert_eq!(got.target, EntityId(101));
    assert_eq!(got.relation_type, "related_to");
    assert_eq!(got.confidence, 1.0);

    assert!(s.get_edge(EdgeId(999)).unwrap().is_none());

    assert!(s.delete_edge(EdgeId(200)).unwrap());
    assert!(s.get_edge(EdgeId(200)).unwrap().is_none());
    assert!(!s.delete_edge(EdgeId(200)).unwrap());

    // --- Entity edges (both directions) ---
    s.put_edge(edge(300, 100, 101)).unwrap(); // 100 → 101
    s.put_edge(edge(301, 102, 100)).unwrap(); // 102 → 100

    let edges = s.get_entity_edges(EntityId(100)).unwrap();
    assert_eq!(edges.len(), 2);
    let ids = s.get_entity_edge_ids(EntityId(100)).unwrap();
    assert_eq!(ids.len(), 2);

    // Node with no edges
    s.put_entity(entity(103, "lonely", "node")).unwrap();
    assert!(s.get_entity_edges(EntityId(103)).unwrap().is_empty());
    assert!(s.get_entity_edge_ids(EntityId(103)).unwrap().is_empty());

    // Self-referencing edge
    s.put_edge(edge(302, 102, 102)).unwrap();
    let edges = s.get_entity_edges(EntityId(102)).unwrap();
    // Should find edge 301 (102→100) and edge 302 (102→102)
    assert_eq!(edges.len(), 2);

    // --- Bi-temporal edge ---
    let mut temporal = edge(400, 100, 101);
    temporal.valid_at = 5000;
    temporal.invalid_at = 9000;
    temporal.confidence = 0.75;
    s.put_edge(temporal).unwrap();
    let got = s.get_edge(EdgeId(400)).unwrap().unwrap();
    assert_eq!(got.valid_at, 5000);
    assert_eq!(got.invalid_at, 9000);
    assert!((got.confidence - 0.75).abs() < f32::EPSILON);

    // --- Episode CRUD ---
    s.put_episode(episode(500)).unwrap();
    let got = s.get_episode(500).unwrap().unwrap();
    assert_eq!(got.id, 500);
    assert_eq!(got.source, EpisodeSource::Conversation);
    assert_eq!(got.session_id, "sess-1");
    assert_eq!(got.entity_ids, vec![EntityId(1), EntityId(2)]);
    assert_eq!(got.fact_ids, vec![EdgeId(10), EdgeId(20)]);
    assert_eq!(got.consolidation_count, 0);

    assert!(s.get_episode(999).unwrap().is_none());

    // Consolidation update
    assert!(s.update_episode_consolidation(500, 7).unwrap());
    assert_eq!(s.get_episode(500).unwrap().unwrap().consolidation_count, 7);
    assert!(!s.update_episode_consolidation(999, 1).unwrap());

    // All three source variants
    for (id, src) in [
        (501, EpisodeSource::Conversation),
        (502, EpisodeSource::Document),
        (503, EpisodeSource::Api),
    ] {
        let mut ep = episode(id);
        ep.source = src.clone();
        s.put_episode(ep).unwrap();
        assert_eq!(s.get_episode(id).unwrap().unwrap().source, src);
    }

    // --- Scan ---
    let all_entities = s.scan_all_entities().unwrap();
    assert!(all_entities.len() >= 6); // 10, 20, 21, 100, 101, 102, 103
    let all_edges = s.scan_all_edges().unwrap();
    assert!(all_edges.len() >= 4); // 300, 301, 302, 400
    let all_episodes = s.scan_all_episodes().unwrap();
    assert!(all_episodes.len() >= 4); // 500, 501, 502, 503

    // --- Stats consistency ---
    let stats = s.stats();
    assert_eq!(stats.entities, all_entities.len() as u64);
    assert_eq!(stats.edges, all_edges.len() as u64);
    assert_eq!(stats.episodes, all_episodes.len() as u64);
}

// ── Memory backend ───────────────────────────────────────────

#[test]
fn conformance_memory() {
    let mut storage = MemoryStorage::new();
    test_backend_conformance(&mut storage);
}

// ── SQLite backend ───────────────────────────────────────────

#[cfg(feature = "sqlite")]
#[test]
fn conformance_sqlite() {
    use hora_graph_core::storage::sqlite::SqliteStorage;
    let mut storage = SqliteStorage::open_in_memory().unwrap();
    test_backend_conformance(&mut storage);
}

// ── PostgreSQL backend ───────────────────────────────────────

#[cfg(feature = "postgres")]
#[test]
fn conformance_postgres() {
    use hora_graph_core::storage::pg::PostgresStorage;
    let url = match std::env::var("TEST_POSTGRES_URL") {
        Ok(u) => u,
        Err(_) => return, // skip if no PG available
    };
    let mut storage = PostgresStorage::connect(&url).unwrap();
    // Clean slate
    storage
        .execute_batch("DELETE FROM edges; DELETE FROM episodes; DELETE FROM entities;")
        .unwrap();
    test_backend_conformance(&mut storage);
}

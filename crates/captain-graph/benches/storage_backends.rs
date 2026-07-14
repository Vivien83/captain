use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use hora_graph_core::storage::memory::MemoryStorage;
use hora_graph_core::storage::traits::StorageOps;
use hora_graph_core::{Edge, EdgeId, Entity, EntityId, Episode, EpisodeSource};
use std::collections::HashMap;

fn entity(id: u64) -> Entity {
    Entity {
        id: EntityId(id),
        entity_type: "node".to_string(),
        name: format!("entity_{}", id),
        properties: HashMap::new(),
        embedding: Some(vec![0.1; 384]),
        created_at: 1000,
    }
}

fn edge(id: u64, source: u64, target: u64) -> Edge {
    Edge {
        id: EdgeId(id),
        source: EntityId(source),
        target: EntityId(target),
        relation_type: "related".to_string(),
        description: String::new(),
        confidence: 1.0,
        valid_at: 1000,
        invalid_at: 0,
        created_at: 1000,
    }
}

#[allow(dead_code)]
fn episode(id: u64) -> Episode {
    Episode {
        id,
        source: EpisodeSource::Conversation,
        session_id: "bench".to_string(),
        entity_ids: vec![EntityId(1), EntityId(2), EntityId(3)],
        fact_ids: vec![EdgeId(1), EdgeId(2)],
        created_at: 2000,
        consolidation_count: 0,
    }
}

// ── Benchmark functions ──────────────────────────────────────

fn bench_put_entity(c: &mut Criterion, name: &str, storage: &mut dyn StorageOps) {
    let mut id = 1u64;
    c.bench_with_input(BenchmarkId::new("put_entity", name), &(), |b, _| {
        b.iter(|| {
            storage.put_entity(entity(id)).unwrap();
            id += 1;
        });
    });
}

fn bench_get_entity(c: &mut Criterion, name: &str, storage: &mut dyn StorageOps) {
    // Seed
    for i in 1..=1000 {
        storage.put_entity(entity(i)).unwrap();
    }
    c.bench_with_input(BenchmarkId::new("get_entity", name), &(), |b, _| {
        let mut i = 1u64;
        b.iter(|| {
            storage.get_entity(EntityId(i)).unwrap();
            i = (i % 1000) + 1;
        });
    });
}

fn bench_put_edge(c: &mut Criterion, name: &str, storage: &mut dyn StorageOps) {
    for i in 1..=100 {
        storage.put_entity(entity(i)).unwrap();
    }
    let mut id = 1u64;
    c.bench_with_input(BenchmarkId::new("put_edge", name), &(), |b, _| {
        b.iter(|| {
            storage
                .put_edge(edge(id, (id % 100) + 1, ((id + 1) % 100) + 1))
                .unwrap();
            id += 1;
        });
    });
}

fn bench_scan_entities(c: &mut Criterion, name: &str, storage: &mut dyn StorageOps) {
    for i in 1..=1000 {
        storage.put_entity(entity(i)).unwrap();
    }
    c.bench_with_input(BenchmarkId::new("scan_1k_entities", name), &(), |b, _| {
        b.iter(|| {
            storage.scan_all_entities().unwrap();
        });
    });
}

fn bench_batch_insert(c: &mut Criterion, name: &str, storage: &mut dyn StorageOps) {
    c.bench_with_input(BenchmarkId::new("batch_1k_entities", name), &(), |b, _| {
        let mut base = 100_000u64;
        b.iter(|| {
            for i in 0..1000 {
                storage.put_entity(entity(base + i)).unwrap();
            }
            base += 1000;
        });
    });
}

// ── Runner ───────────────────────────────────────────────────

fn bench_memory(c: &mut Criterion) {
    let name = "memory";
    {
        let mut s = MemoryStorage::new();
        bench_put_entity(c, name, &mut s);
    }
    {
        let mut s = MemoryStorage::new();
        bench_get_entity(c, name, &mut s);
    }
    {
        let mut s = MemoryStorage::new();
        bench_put_edge(c, name, &mut s);
    }
    {
        let mut s = MemoryStorage::new();
        bench_scan_entities(c, name, &mut s);
    }
    {
        let mut s = MemoryStorage::new();
        bench_batch_insert(c, name, &mut s);
    }
}

#[cfg(feature = "sqlite")]
fn bench_sqlite(c: &mut Criterion) {
    use hora_graph_core::storage::sqlite::SqliteStorage;
    let name = "sqlite";
    {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        bench_put_entity(c, name, &mut s);
    }
    {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        bench_get_entity(c, name, &mut s);
    }
    {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        bench_put_edge(c, name, &mut s);
    }
    {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        bench_scan_entities(c, name, &mut s);
    }
    {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        bench_batch_insert(c, name, &mut s);
    }
}

#[cfg(feature = "sqlite")]
criterion_group!(sqlite_benches, bench_sqlite);

criterion_group!(memory_benches, bench_memory);

#[cfg(feature = "sqlite")]
criterion_main!(memory_benches, sqlite_benches);

#[cfg(not(feature = "sqlite"))]
criterion_main!(memory_benches);

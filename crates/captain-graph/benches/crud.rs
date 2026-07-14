use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use hora_graph_core::{DedupConfig, EntityId, HoraConfig, HoraCore};

fn bench_config() -> HoraConfig {
    HoraConfig {
        dedup: DedupConfig::disabled(),
        ..Default::default()
    }
}

// ── Zero-dep LCG RNG (reproducible, seed=42) ─────────────────────
struct SimpleRng(u64);

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0
    }
    fn next_usize(&mut self) -> usize {
        self.next_u64() as usize
    }
}

// ── Benchmarks ────────────────────────────────────────────────────

fn bench_add_entity(c: &mut Criterion) {
    c.bench_function("add_entity", |b| {
        let mut hora = HoraCore::new(bench_config()).unwrap();
        b.iter(|| {
            hora.add_entity("test", "entity", None, None).unwrap();
        });
    });
}

fn bench_get_entity(c: &mut Criterion) {
    let mut hora = HoraCore::new(bench_config()).unwrap();
    let ids: Vec<EntityId> = (0..10_000)
        .map(|i| {
            hora.add_entity("test", &format!("e{}", i), None, None)
                .unwrap()
        })
        .collect();

    c.bench_function("get_entity", |b| {
        let mut idx = 0usize;
        b.iter(|| {
            let id = ids[idx % ids.len()];
            let _ = hora.get_entity(id).unwrap();
            idx = idx.wrapping_add(1);
        });
    });
}

fn bench_add_fact(c: &mut Criterion) {
    c.bench_function("add_fact", |b| {
        let mut hora = HoraCore::new(bench_config()).unwrap();
        let mut rng = SimpleRng::new(42);

        // Pre-create entities so add_fact doesn't fail on missing entities
        let ids: Vec<EntityId> = (0..1_000)
            .map(|i| {
                hora.add_entity("node", &format!("n{}", i), None, None)
                    .unwrap()
            })
            .collect();

        b.iter(|| {
            let src = ids[rng.next_usize() % ids.len()];
            let tgt = ids[rng.next_usize() % ids.len()];
            let _ = hora.add_fact(src, tgt, "related", "", None);
        });
    });
}

fn bench_batch_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("batch_insert");
    // Reduce sample size for large batches
    group.sample_size(10);

    for &(n_entities, n_edges) in &[(1_000, 5_000), (10_000, 50_000), (100_000, 500_000)] {
        group.bench_with_input(
            BenchmarkId::new("entities+edges", format!("{}+{}", n_entities, n_edges)),
            &(n_entities, n_edges),
            |b, &(n_ent, n_edg)| {
                b.iter(|| {
                    let mut hora = HoraCore::new(bench_config()).unwrap();
                    let mut rng = SimpleRng::new(42);

                    let ids: Vec<EntityId> = (0..n_ent)
                        .map(|i| {
                            hora.add_entity("node", &format!("n{}", i), None, None)
                                .unwrap()
                        })
                        .collect();

                    for _ in 0..n_edg {
                        let src = ids[rng.next_usize() % ids.len()];
                        let tgt = ids[rng.next_usize() % ids.len()];
                        if src != tgt {
                            let _ = hora.add_fact(src, tgt, "related", "", None);
                        }
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_persistence_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("persistence");
    group.sample_size(10);

    // Build a graph with 10K entities + 50K edges, then bench flush+open
    let mut hora = HoraCore::new(bench_config()).unwrap();
    let mut rng = SimpleRng::new(42);

    let ids: Vec<EntityId> = (0..10_000)
        .map(|i| {
            hora.add_entity("node", &format!("n{}", i), None, None)
                .unwrap()
        })
        .collect();

    for _ in 0..50_000 {
        let src = ids[rng.next_usize() % ids.len()];
        let tgt = ids[rng.next_usize() % ids.len()];
        if src != tgt {
            let _ = hora.add_fact(src, tgt, "related", "", None);
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.hora");

    // Bench: flush (serialize to disk)
    let file_hora = HoraCore::open(&path, bench_config()).unwrap();
    // We need a file-backed instance for flush, so snapshot first
    hora.snapshot(&path).unwrap();

    group.bench_function("flush_10k", |b| {
        let flush_path = dir.path().join("flush_bench.hora");
        hora.snapshot(&flush_path).unwrap();
        let flush_hora = HoraCore::open(&flush_path, bench_config()).unwrap();
        b.iter(|| {
            flush_hora.flush().unwrap();
        });
    });

    // Bench: open (deserialize from disk)
    group.bench_function("open_10k", |b| {
        b.iter(|| {
            let _ = HoraCore::open(&path, bench_config()).unwrap();
        });
    });

    drop(file_hora);
    group.finish();
}

criterion_group!(
    benches,
    bench_add_entity,
    bench_get_entity,
    bench_add_fact,
    bench_batch_insert,
    bench_persistence_roundtrip,
);
criterion_main!(benches);

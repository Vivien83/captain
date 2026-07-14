use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use hora_graph_core::{DedupConfig, EntityId, HoraConfig, HoraCore, TraverseOpts};

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

/// Build a random graph for benchmark use.
fn setup_random_graph(n_entities: usize, n_edges: usize) -> (HoraCore, Vec<EntityId>) {
    let mut hora = HoraCore::new(bench_config()).unwrap();
    let mut rng = SimpleRng::new(42);

    let ids: Vec<EntityId> = (0..n_entities)
        .map(|i| {
            hora.add_entity("node", &format!("n{}", i), None, None)
                .unwrap()
        })
        .collect();

    for _ in 0..n_edges {
        let src = ids[rng.next_usize() % ids.len()];
        let tgt = ids[rng.next_usize() % ids.len()];
        if src != tgt {
            let _ = hora.add_fact(src, tgt, "related", "", None);
        }
    }

    (hora, ids)
}

// ── BFS ───────────────────────────────────────────────────────────

fn bench_bfs(c: &mut Criterion) {
    let mut group = c.benchmark_group("bfs");
    group.sample_size(20);

    let (hora, _ids) = setup_random_graph(100_000, 500_000);
    let start = EntityId(1);

    for depth in [1, 2, 3] {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.iter(|| {
                let _ = hora.traverse(start, TraverseOpts { depth }).unwrap();
            });
        });
    }

    group.finish();
}

// ── Timeline ──────────────────────────────────────────────────────

fn bench_timeline(c: &mut Criterion) {
    // Single entity connected to 100 facts
    let mut hora = HoraCore::new(bench_config()).unwrap();
    let hub = hora.add_entity("hub", "central", None, None).unwrap();

    for i in 0..100 {
        let target = hora
            .add_entity("node", &format!("t{}", i), None, None)
            .unwrap();
        hora.add_fact(hub, target, "link", "", None).unwrap();
    }

    c.bench_function("timeline_100_facts", |b| {
        b.iter(|| {
            hora.timeline(hub).unwrap();
        });
    });
}

// ── facts_at ──────────────────────────────────────────────────────

fn bench_facts_at(c: &mut Criterion) {
    let mut group = c.benchmark_group("facts_at");
    group.sample_size(20);

    let (hora, _ids) = setup_random_graph(10_000, 100_000);

    // Query at "now" — should match all edges (none invalidated)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    group.bench_function("100k_edges", |b| {
        b.iter(|| {
            hora.facts_at(now).unwrap();
        });
    });

    group.finish();
}

criterion_group!(benches, bench_bfs, bench_timeline, bench_facts_at,);
criterion_main!(benches);

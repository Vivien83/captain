use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use hora_graph_core::{DedupConfig, HoraConfig, HoraCore};

fn bench_config() -> HoraConfig {
    HoraConfig {
        dedup: DedupConfig::disabled(),
        ..Default::default()
    }
}

// ── Zero-dep LCG RNG ─────────────────────────────────────────────
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

const WORDS: &[&str] = &[
    "authentication",
    "database",
    "engine",
    "graph",
    "knowledge",
    "memory",
    "network",
    "query",
    "rust",
    "search",
    "server",
    "storage",
    "system",
    "temporal",
    "vector",
    "index",
    "cache",
    "node",
    "edge",
    "entity",
    "embedding",
    "traversal",
    "algorithm",
    "benchmark",
    "performance",
    "optimization",
    "concurrent",
    "distributed",
    "parallel",
    "streaming",
];

fn random_text(rng: &mut SimpleRng, word_count: usize) -> String {
    (0..word_count)
        .map(|_| WORDS[rng.next_usize() % WORDS.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

fn build_graph(n: usize) -> HoraCore {
    let mut hora = HoraCore::new(bench_config()).unwrap();
    let mut rng = SimpleRng::new(42);

    for i in 0..n {
        let name = format!("entity_{}", i);
        let desc = random_text(&mut rng, 8);
        hora.add_entity(
            "node",
            &name,
            Some(hora_graph_core::props! { "description" => desc.as_str() }),
            None,
        )
        .unwrap();
    }
    hora
}

fn bench_bm25_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("bm25_search");
    group.sample_size(20);

    for n in [1_000, 10_000, 100_000] {
        let mut hora = build_graph(n);

        group.bench_with_input(BenchmarkId::new("top10", n), &n, |bench, _| {
            bench.iter(|| {
                hora.text_search("authentication graph engine", 10).unwrap();
            });
        });
    }

    group.finish();
}

fn bench_bm25_index(c: &mut Criterion) {
    let mut group = c.benchmark_group("bm25_index");

    // Measure indexing speed: add_entity with text
    group.bench_function("add_entity_with_text", |b| {
        let mut hora = HoraCore::new(bench_config()).unwrap();
        let mut rng = SimpleRng::new(42);
        let mut i = 0u64;

        b.iter(|| {
            let desc = random_text(&mut rng, 8);
            hora.add_entity(
                "node",
                &format!("e{}", i),
                Some(hora_graph_core::props! { "description" => desc.as_str() }),
                None,
            )
            .unwrap();
            i += 1;
        });
    });

    group.finish();
}

criterion_group!(benches, bench_bm25_search, bench_bm25_index);
criterion_main!(benches);

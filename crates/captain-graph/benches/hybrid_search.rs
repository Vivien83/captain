use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use hora_graph_core::{DedupConfig, HoraConfig, HoraCore, SearchOpts};

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
    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
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

fn random_embedding(rng: &mut SimpleRng, dims: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dims).map(|_| rng.next_f32() - 0.5).collect();
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v
}

const DIMS: usize = 128;

fn build_hybrid_graph(n: usize) -> HoraCore {
    let config = HoraConfig {
        embedding_dims: DIMS as u16,
        dedup: DedupConfig::disabled(),
    };
    let mut hora = HoraCore::new(config).unwrap();
    let mut rng = SimpleRng::new(42);

    for i in 0..n {
        let name = format!("entity_{}", i);
        let desc = random_text(&mut rng, 8);
        let emb = random_embedding(&mut rng, DIMS);
        hora.add_entity(
            "node",
            &name,
            Some(hora_graph_core::props! { "description" => desc.as_str() }),
            Some(&emb),
        )
        .unwrap();
    }
    hora
}

fn bench_hybrid_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("hybrid_search");
    group.sample_size(20);

    let mut rng = SimpleRng::new(99);
    let query_emb = random_embedding(&mut rng, DIMS);
    let query_text = "authentication graph engine";

    for n in [1_000, 10_000] {
        let mut hora = build_hybrid_graph(n);

        // Both legs
        group.bench_with_input(BenchmarkId::new("both_legs_top10", n), &n, |bench, _| {
            bench.iter(|| {
                hora.search(
                    Some(query_text),
                    Some(&query_emb),
                    SearchOpts {
                        top_k: 10,
                        ..Default::default()
                    },
                )
                .unwrap();
            });
        });

        // Text only
        group.bench_with_input(BenchmarkId::new("text_only_top10", n), &n, |bench, _| {
            bench.iter(|| {
                hora.search(
                    Some(query_text),
                    None,
                    SearchOpts {
                        top_k: 10,
                        ..Default::default()
                    },
                )
                .unwrap();
            });
        });

        // Vector only
        group.bench_with_input(BenchmarkId::new("vector_only_top10", n), &n, |bench, _| {
            bench.iter(|| {
                hora.search(
                    None,
                    Some(&query_emb),
                    SearchOpts {
                        top_k: 10,
                        ..Default::default()
                    },
                )
                .unwrap();
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_hybrid_search);
criterion_main!(benches);

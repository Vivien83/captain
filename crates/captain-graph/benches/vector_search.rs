use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use hora_graph_core::search::vector::{cosine_scalar, cosine_similarity};
use hora_graph_core::{DedupConfig, HoraConfig, HoraCore};

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
    fn next_f32(&mut self) -> f32 {
        // Generate a float in [-1.0, 1.0]
        (self.next_u64() as i64 as f64 / i64::MAX as f64) as f32
    }
}

fn random_vec(rng: &mut SimpleRng, dim: usize) -> Vec<f32> {
    (0..dim).map(|_| rng.next_f32()).collect()
}

// ── Cosine similarity benchmarks ──────────────────────────────────

fn bench_cosine_similarity(c: &mut Criterion) {
    let mut group = c.benchmark_group("cosine_similarity");
    let mut rng = SimpleRng::new(42);

    for dim in [128, 384, 768, 1536] {
        let a = random_vec(&mut rng, dim);
        let b = random_vec(&mut rng, dim);

        group.bench_with_input(BenchmarkId::new("dispatch", dim), &dim, |bench, _| {
            bench.iter(|| cosine_similarity(&a, &b).unwrap());
        });

        group.bench_with_input(BenchmarkId::new("scalar", dim), &dim, |bench, _| {
            bench.iter(|| cosine_scalar(&a, &b));
        });
    }

    group.finish();
}

// ── Brute-force vector_search benchmarks ──────────────────────────

fn bench_vector_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("vector_search");
    group.sample_size(10);

    let dim = 384;
    let mut rng = SimpleRng::new(42);

    for n in [1_000, 10_000, 100_000] {
        // Build graph with n entities, each with a 384-dim embedding
        let config = HoraConfig {
            embedding_dims: dim,
            dedup: DedupConfig::disabled(),
        };
        let mut hora = HoraCore::new(config).unwrap();

        for i in 0..n {
            let emb = random_vec(&mut rng, dim as usize);
            hora.add_entity("node", &format!("n{}", i), None, Some(&emb))
                .unwrap();
        }

        let query = random_vec(&mut rng, dim as usize);

        group.bench_with_input(BenchmarkId::new("top100", n), &n, |bench, _| {
            bench.iter(|| {
                hora.vector_search(&query, 100).unwrap();
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_cosine_similarity, bench_vector_search);
criterion_main!(benches);

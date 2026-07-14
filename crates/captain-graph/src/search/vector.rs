//! SIMD-accelerated vector similarity search.
//!
//! Dispatch: NEON (AArch64) → AVX2+FMA (x86_64) → scalar fallback.
//! Selected once at first call via `OnceLock`.

use std::sync::OnceLock;

use crate::core::types::EntityId;
use crate::search::SearchHit;

// ── Dispatch ──────────────────────────────────────────────────────

type CosineFn = fn(&[f32], &[f32]) -> f32;
static COSINE_FN: OnceLock<CosineFn> = OnceLock::new();

/// Cosine similarity between two vectors of equal length.
///
/// Automatically dispatches to the fastest available implementation
/// (NEON on AArch64, AVX2+FMA on x86_64, scalar fallback otherwise).
///
/// Returns `Err` if the vectors have mismatched lengths instead of panicking,
/// so a single malformed/stale embedding cannot crash a whole search.
///
/// Returns `Ok(0.0)` if either vector has zero norm.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Result<f32, String> {
    if a.len() != b.len() {
        return Err(format!(
            "vectors must have equal length (got {} and {})",
            a.len(),
            b.len()
        ));
    }
    let f = COSINE_FN.get_or_init(select_impl);
    Ok(f(a, b))
}

fn select_impl() -> CosineFn {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma") {
            // SAFETY: AVX2+FMA detected above; cosine_similarity asserts equal lengths.
            return |a, b| unsafe { cosine_avx2(a, b) };
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON is always available on AArch64; cosine_similarity asserts equal lengths.
        return |a, b| unsafe { cosine_neon(a, b) };
    }

    #[allow(unreachable_code)]
    cosine_scalar
}

// ── Scalar fallback ───────────────────────────────────────────────

/// Pure scalar cosine similarity. Always correct, used as reference.
pub fn cosine_scalar(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;

    for i in 0..a.len() {
        let ai = a[i] as f64;
        let bi = b[i] as f64;
        dot += ai * bi;
        norm_a += ai * ai;
        norm_b += bi * bi;
    }

    let denom = (norm_a * norm_b).sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    (dot / denom) as f32
}

// ── NEON (AArch64) ────────────────────────────────────────────────

/// SAFETY: Caller must ensure `a.len() == b.len()` and that NEON is available.
/// `cosine_similarity` enforces both via `assert_eq!` + AArch64 target gate.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn cosine_neon(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::aarch64::*;

    let n = a.len();
    let chunks = n / 16; // Process 16 floats per iteration (4 accumulators × 4 lanes)
    let remainder = n % 16;

    // 4 accumulators to mask FMA latency
    let mut dot0 = vdupq_n_f32(0.0);
    let mut dot1 = vdupq_n_f32(0.0);
    let mut dot2 = vdupq_n_f32(0.0);
    let mut dot3 = vdupq_n_f32(0.0);

    let mut na0 = vdupq_n_f32(0.0);
    let mut na1 = vdupq_n_f32(0.0);
    let mut na2 = vdupq_n_f32(0.0);
    let mut na3 = vdupq_n_f32(0.0);

    let mut nb0 = vdupq_n_f32(0.0);
    let mut nb1 = vdupq_n_f32(0.0);
    let mut nb2 = vdupq_n_f32(0.0);
    let mut nb3 = vdupq_n_f32(0.0);

    let pa = a.as_ptr();
    let pb = b.as_ptr();

    for i in 0..chunks {
        let base = i * 16;
        let a0 = vld1q_f32(pa.add(base));
        let a1 = vld1q_f32(pa.add(base + 4));
        let a2 = vld1q_f32(pa.add(base + 8));
        let a3 = vld1q_f32(pa.add(base + 12));

        let b0 = vld1q_f32(pb.add(base));
        let b1 = vld1q_f32(pb.add(base + 4));
        let b2 = vld1q_f32(pb.add(base + 8));
        let b3 = vld1q_f32(pb.add(base + 12));

        dot0 = vfmaq_f32(dot0, a0, b0);
        dot1 = vfmaq_f32(dot1, a1, b1);
        dot2 = vfmaq_f32(dot2, a2, b2);
        dot3 = vfmaq_f32(dot3, a3, b3);

        na0 = vfmaq_f32(na0, a0, a0);
        na1 = vfmaq_f32(na1, a1, a1);
        na2 = vfmaq_f32(na2, a2, a2);
        na3 = vfmaq_f32(na3, a3, a3);

        nb0 = vfmaq_f32(nb0, b0, b0);
        nb1 = vfmaq_f32(nb1, b1, b1);
        nb2 = vfmaq_f32(nb2, b2, b2);
        nb3 = vfmaq_f32(nb3, b3, b3);
    }

    // Reduce 4 accumulators → 1
    let dot_sum = vaddq_f32(vaddq_f32(dot0, dot1), vaddq_f32(dot2, dot3));
    let na_sum = vaddq_f32(vaddq_f32(na0, na1), vaddq_f32(na2, na3));
    let nb_sum = vaddq_f32(vaddq_f32(nb0, nb1), vaddq_f32(nb2, nb3));

    // Horizontal sum
    let mut dot_total = vaddvq_f32(dot_sum);
    let mut na_total = vaddvq_f32(na_sum);
    let mut nb_total = vaddvq_f32(nb_sum);

    // Handle remainder elements
    let rem_start = chunks * 16;
    for i in 0..remainder {
        let ai = *pa.add(rem_start + i);
        let bi = *pb.add(rem_start + i);
        dot_total += ai * bi;
        na_total += ai * ai;
        nb_total += bi * bi;
    }

    let denom = (na_total * nb_total).sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    dot_total / denom
}

// ── AVX2 + FMA (x86_64) ──────────────────────────────────────────

/// SAFETY: Caller must ensure `a.len() == b.len()` and that AVX2+FMA are available.
/// `cosine_similarity` enforces both via `assert_eq!` + `is_x86_feature_detected!`.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
unsafe fn cosine_avx2(a: &[f32], b: &[f32]) -> f32 {
    use std::arch::x86_64::*;

    let n = a.len();
    let chunks = n / 32; // 4 accumulators × 8 lanes = 32 floats
    let remainder = n % 32;

    let mut dot0 = _mm256_setzero_ps();
    let mut dot1 = _mm256_setzero_ps();
    let mut dot2 = _mm256_setzero_ps();
    let mut dot3 = _mm256_setzero_ps();

    let mut na0 = _mm256_setzero_ps();
    let mut na1 = _mm256_setzero_ps();
    let mut na2 = _mm256_setzero_ps();
    let mut na3 = _mm256_setzero_ps();

    let mut nb0 = _mm256_setzero_ps();
    let mut nb1 = _mm256_setzero_ps();
    let mut nb2 = _mm256_setzero_ps();
    let mut nb3 = _mm256_setzero_ps();

    let pa = a.as_ptr();
    let pb = b.as_ptr();

    for i in 0..chunks {
        let base = i * 32;
        let a0 = _mm256_loadu_ps(pa.add(base));
        let a1 = _mm256_loadu_ps(pa.add(base + 8));
        let a2 = _mm256_loadu_ps(pa.add(base + 16));
        let a3 = _mm256_loadu_ps(pa.add(base + 24));

        let b0 = _mm256_loadu_ps(pb.add(base));
        let b1 = _mm256_loadu_ps(pb.add(base + 8));
        let b2 = _mm256_loadu_ps(pb.add(base + 16));
        let b3 = _mm256_loadu_ps(pb.add(base + 24));

        dot0 = _mm256_fmadd_ps(a0, b0, dot0);
        dot1 = _mm256_fmadd_ps(a1, b1, dot1);
        dot2 = _mm256_fmadd_ps(a2, b2, dot2);
        dot3 = _mm256_fmadd_ps(a3, b3, dot3);

        na0 = _mm256_fmadd_ps(a0, a0, na0);
        na1 = _mm256_fmadd_ps(a1, a1, na1);
        na2 = _mm256_fmadd_ps(a2, a2, na2);
        na3 = _mm256_fmadd_ps(a3, a3, na3);

        nb0 = _mm256_fmadd_ps(b0, b0, nb0);
        nb1 = _mm256_fmadd_ps(b1, b1, nb1);
        nb2 = _mm256_fmadd_ps(b2, b2, nb2);
        nb3 = _mm256_fmadd_ps(b3, b3, nb3);
    }

    // Reduce 4 → 1
    let dot_sum = _mm256_add_ps(_mm256_add_ps(dot0, dot1), _mm256_add_ps(dot2, dot3));
    let na_sum = _mm256_add_ps(_mm256_add_ps(na0, na1), _mm256_add_ps(na2, na3));
    let nb_sum = _mm256_add_ps(_mm256_add_ps(nb0, nb1), _mm256_add_ps(nb2, nb3));

    // Horizontal sum: 256 → 128 → scalar
    let dot_hi = _mm256_extractf128_ps(dot_sum, 1);
    let dot_lo = _mm256_castps256_ps128(dot_sum);
    let dot_128 = _mm_add_ps(dot_hi, dot_lo);
    let dot_64 = _mm_add_ps(dot_128, _mm_movehl_ps(dot_128, dot_128));
    let dot_32 = _mm_add_ss(dot_64, _mm_shuffle_ps(dot_64, dot_64, 1));
    let mut dot_total = _mm_cvtss_f32(dot_32);

    let na_hi = _mm256_extractf128_ps(na_sum, 1);
    let na_lo = _mm256_castps256_ps128(na_sum);
    let na_128 = _mm_add_ps(na_hi, na_lo);
    let na_64 = _mm_add_ps(na_128, _mm_movehl_ps(na_128, na_128));
    let na_32 = _mm_add_ss(na_64, _mm_shuffle_ps(na_64, na_64, 1));
    let mut na_total = _mm_cvtss_f32(na_32);

    let nb_hi = _mm256_extractf128_ps(nb_sum, 1);
    let nb_lo = _mm256_castps256_ps128(nb_sum);
    let nb_128 = _mm_add_ps(nb_hi, nb_lo);
    let nb_64 = _mm_add_ps(nb_128, _mm_movehl_ps(nb_128, nb_128));
    let nb_32 = _mm_add_ss(nb_64, _mm_shuffle_ps(nb_64, nb_64, 1));
    let mut nb_total = _mm_cvtss_f32(nb_32);

    // Handle remainder
    let rem_start = chunks * 32;
    for i in 0..remainder {
        let ai = *pa.add(rem_start + i);
        let bi = *pb.add(rem_start + i);
        dot_total += ai * bi;
        na_total += ai * ai;
        nb_total += bi * bi;
    }

    let denom = (na_total * nb_total).sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    dot_total / denom
}

// ── Top-K brute-force scan ────────────────────────────────────────

/// Brute-force vector search: scan all entities, return top-k by cosine similarity.
///
/// Entities without embeddings are silently skipped.
/// The query vector must match `embedding_dims` in length.
pub fn top_k_brute_force(
    query: &[f32],
    entities: &[(EntityId, &[f32])],
    k: usize,
) -> Vec<SearchHit> {
    if k == 0 || entities.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(EntityId, f32)> = Vec::with_capacity(entities.len());
    for (id, emb) in entities.iter() {
        match cosine_similarity(query, emb) {
            Ok(score) => scored.push((*id, score)),
            Err(reason) => {
                // A single entity with a mismatched embedding dimension (e.g. after a
                // migration or an external backend write) must not abort the whole search.
                eprintln!(
                    "vector search: skipping entity {:?} with incompatible embedding: {reason}",
                    id
                );
            }
        }
    }

    // Partial sort: we only need top-k
    if k < scored.len() {
        scored.select_nth_unstable_by(k, |a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
    }

    // Sort the top-k by descending score
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    scored
        .into_iter()
        .map(|(entity_id, score)| SearchHit { entity_id, score })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_vec(vals: &[f32]) -> Vec<f32> {
        vals.to_vec()
    }

    #[test]
    fn test_cosine_identical_vectors() {
        let a = make_vec(&[1.0, 2.0, 3.0, 4.0]);
        let sim = cosine_similarity(&a, &a).unwrap();
        assert!(
            (sim - 1.0).abs() < 1e-5,
            "identical vectors should have cosine ~1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_orthogonal() {
        let a = make_vec(&[1.0, 0.0, 0.0, 0.0]);
        let b = make_vec(&[0.0, 1.0, 0.0, 0.0]);
        let sim = cosine_similarity(&a, &b).unwrap();
        assert!(
            sim.abs() < 1e-5,
            "orthogonal vectors should have cosine ~0.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_opposite() {
        let a = make_vec(&[1.0, 2.0, 3.0]);
        let b = make_vec(&[-1.0, -2.0, -3.0]);
        let sim = cosine_similarity(&a, &b).unwrap();
        assert!(
            (sim + 1.0).abs() < 1e-5,
            "opposite vectors should have cosine ~-1.0, got {}",
            sim
        );
    }

    #[test]
    fn test_cosine_zero_vector() {
        let a = make_vec(&[1.0, 2.0, 3.0]);
        let b = make_vec(&[0.0, 0.0, 0.0]);
        let sim = cosine_similarity(&a, &b).unwrap();
        assert_eq!(sim, 0.0, "zero vector should give cosine 0.0");
    }

    #[test]
    fn test_cosine_scalar_vs_dispatch() {
        // 384-dim vectors (typical embedding size)
        let a: Vec<f32> = (0..384).map(|i| (i as f32 * 0.01).sin()).collect();
        let b: Vec<f32> = (0..384).map(|i| (i as f32 * 0.02).cos()).collect();

        let scalar = cosine_scalar(&a, &b);
        let dispatch = cosine_similarity(&a, &b).unwrap();

        assert!(
            (scalar - dispatch).abs() < 1e-5,
            "SIMD result ({}) should match scalar ({}) within 1e-5",
            dispatch,
            scalar,
        );
    }

    #[test]
    fn test_cosine_non_aligned_length() {
        // Test with length that doesn't divide evenly into SIMD lanes
        let a: Vec<f32> = (0..97).map(|i| i as f32).collect();
        let b: Vec<f32> = (0..97).map(|i| (i * 2) as f32).collect();

        let scalar = cosine_scalar(&a, &b);
        let dispatch = cosine_similarity(&a, &b).unwrap();

        assert!(
            (scalar - dispatch).abs() < 1e-5,
            "non-aligned: SIMD ({}) vs scalar ({}) differ by more than 1e-5",
            dispatch,
            scalar,
        );
    }

    #[test]
    fn test_cosine_similarity_mismatched_length_returns_err() {
        let a = make_vec(&[1.0, 2.0, 3.0]);
        let b = make_vec(&[1.0, 2.0]);
        let result = cosine_similarity(&a, &b);
        assert!(
            result.is_err(),
            "mismatched-length vectors must return an Err, not panic"
        );
    }

    #[test]
    fn test_top_k_brute_force_skips_mismatched_dimension_entity() {
        // Entity 2 has a different embedding dimension (e.g. stale from a migration
        // or written by an external backend). It must be skipped, not crash the search.
        let v1 = vec![1.0, 0.0, 0.0];
        let v2_wrong_dim = vec![0.5, 0.5]; // mismatched dimension
        let v3 = vec![0.9, 0.1, 0.0];
        let query = vec![1.0, 0.0, 0.0];

        let entities: Vec<(EntityId, &[f32])> = vec![
            (EntityId(1), &v1),
            (EntityId(2), &v2_wrong_dim),
            (EntityId(3), &v3),
        ];

        let results = top_k_brute_force(&query, &entities, 10);

        // Only the two well-formed entities should be scored and returned.
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|hit| hit.entity_id != EntityId(2)));
        assert_eq!(results[0].entity_id, EntityId(1));
    }

    #[test]
    fn test_top_k_basic() {
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![0.9, 0.1, 0.0];
        let v3 = vec![0.0, 1.0, 0.0];
        let query = vec![1.0, 0.0, 0.0];

        let entities: Vec<(EntityId, &[f32])> =
            vec![(EntityId(1), &v1), (EntityId(2), &v2), (EntityId(3), &v3)];

        let results = top_k_brute_force(&query, &entities, 2);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entity_id, EntityId(1)); // exact match
        assert_eq!(results[1].entity_id, EntityId(2)); // close match
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_top_k_larger_than_corpus() {
        let v1 = vec![1.0, 0.0];
        let entities: Vec<(EntityId, &[f32])> = vec![(EntityId(1), &v1)];
        let query = vec![1.0, 0.0];

        let results = top_k_brute_force(&query, &entities, 100);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_top_k_empty() {
        let query = vec![1.0, 0.0];
        let results = top_k_brute_force(&query, &[], 10);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_top_k_zero_k() {
        let v1 = vec![1.0, 0.0];
        let entities: Vec<(EntityId, &[f32])> = vec![(EntityId(1), &v1)];
        let query = vec![1.0, 0.0];

        let results = top_k_brute_force(&query, &entities, 0);
        assert_eq!(results.len(), 0);
    }
}

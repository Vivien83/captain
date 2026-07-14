//! Reciprocal Rank Fusion (RRF) for hybrid search.
//!
//! Combines vector and BM25 search results using rank-based fusion,
//! which is insensitive to score scales.

use std::collections::HashMap;

use crate::core::types::EntityId;
use crate::search::SearchHit;

const K_RRF: f64 = 60.0;
const VECTOR_WEIGHT: f64 = 0.7;
const BM25_WEIGHT: f64 = 0.3;

/// Fuse vector search and BM25 search results using Reciprocal Rank Fusion.
///
/// Each result list should be pre-sorted by descending score (as returned by
/// `vector_search` and `text_search`). The rank is derived from position.
///
/// Documents found by both legs will naturally rank higher than those found
/// by only one leg, because they accumulate RRF score from both.
pub fn rrf_fuse(
    vector_results: Option<&[SearchHit]>,
    bm25_results: Option<&[SearchHit]>,
    top_k: usize,
) -> Vec<SearchHit> {
    if top_k == 0 {
        return Vec::new();
    }

    let mut scores: HashMap<EntityId, f64> = HashMap::new();

    if let Some(vr) = vector_results {
        for (rank, hit) in vr.iter().enumerate() {
            *scores.entry(hit.entity_id).or_default() +=
                VECTOR_WEIGHT / (K_RRF + rank as f64 + 1.0);
        }
    }

    if let Some(br) = bm25_results {
        for (rank, hit) in br.iter().enumerate() {
            *scores.entry(hit.entity_id).or_default() += BM25_WEIGHT / (K_RRF + rank as f64 + 1.0);
        }
    }

    // Top-k extraction
    let mut ranked: Vec<(EntityId, f64)> = scores.into_iter().collect();

    if top_k < ranked.len() {
        ranked.select_nth_unstable_by(top_k, |a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        ranked.truncate(top_k);
    }

    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    ranked
        .into_iter()
        .map(|(entity_id, score)| SearchHit {
            entity_id,
            score: score as f32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(id: u64, score: f32) -> SearchHit {
        SearchHit {
            entity_id: EntityId(id),
            score,
        }
    }

    #[test]
    fn test_rrf_both_legs_ranks_higher() {
        // Entity 1 appears in both legs, entity 2 only in vector, entity 3 only in BM25
        let vec_results = vec![hit(1, 0.95), hit(2, 0.80)];
        let bm25_results = vec![hit(1, 5.0), hit(3, 3.0)];

        let results = rrf_fuse(Some(&vec_results), Some(&bm25_results), 10);

        // Entity 1 should be first (found by both legs)
        assert_eq!(results[0].entity_id, EntityId(1));
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_rrf_vector_only() {
        let vec_results = vec![hit(1, 0.95), hit(2, 0.80)];

        let results = rrf_fuse(Some(&vec_results), None, 10);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entity_id, EntityId(1));
    }

    #[test]
    fn test_rrf_bm25_only() {
        let bm25_results = vec![hit(3, 5.0), hit(4, 3.0)];

        let results = rrf_fuse(None, Some(&bm25_results), 10);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entity_id, EntityId(3));
    }

    #[test]
    fn test_rrf_no_legs() {
        let results = rrf_fuse(None, None, 10);
        assert!(results.is_empty());
    }

    #[test]
    fn test_rrf_top_k_limit() {
        let vec_results: Vec<SearchHit> = (1..=20).map(|i| hit(i, 1.0 - i as f32 * 0.01)).collect();

        let results = rrf_fuse(Some(&vec_results), None, 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_rrf_scores_descending() {
        let vec_results = vec![hit(1, 0.9), hit(2, 0.8), hit(3, 0.7)];
        let bm25_results = vec![hit(4, 5.0), hit(1, 4.0), hit(5, 3.0)];

        let results = rrf_fuse(Some(&vec_results), Some(&bm25_results), 10);

        for w in results.windows(2) {
            assert!(w[0].score >= w[1].score);
        }
    }
}

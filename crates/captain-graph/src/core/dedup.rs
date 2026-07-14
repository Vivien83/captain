//! Entity deduplication — name exact, Jaccard tokens, cosine embedding.
//!
//! Three detectors, checked in order: normalized name exact match,
//! cosine embedding similarity, Jaccard token overlap.

use std::collections::HashSet;

use crate::core::entity::Entity;
use crate::core::types::{DedupConfig, EntityId};
use crate::search::bm25::tokenize;
use crate::search::vector::cosine_similarity;

/// Normalize a name for exact-match comparison.
///
/// Lowercases, trims, replaces hyphens/underscores with spaces,
/// and collapses consecutive whitespace into single space.
pub fn normalize_name(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    // Collapse whitespace
    let mut result = String::with_capacity(s.len());
    let mut prev_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    result
}

/// Jaccard similarity between two token sets: |A ∩ B| / |A ∪ B|.
///
/// Returns 0.0 if both sets are empty.
pub fn jaccard_similarity(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    intersection as f32 / union as f32
}

/// Check if a new entity is a duplicate of any existing entity of the same type.
///
/// Returns `Some(EntityId)` of the first detected duplicate, or `None` if unique.
/// Detection priority: name exact → cosine embedding → Jaccard tokens.
pub fn find_duplicate(
    name: &str,
    embedding: Option<&[f32]>,
    entity_type: &str,
    candidates: &[Entity],
    config: &DedupConfig,
) -> Option<EntityId> {
    if !config.enabled {
        return None;
    }

    let norm_name = normalize_name(name);
    let new_tokens = tokenize(name);

    for candidate in candidates {
        if candidate.entity_type != entity_type {
            continue;
        }

        // 1. Normalized name exact match
        if config.name_exact && normalize_name(&candidate.name) == norm_name {
            return Some(candidate.id);
        }

        // 2. Cosine embedding similarity
        if config.cosine_threshold > 0.0 {
            if let (Some(new_emb), Some(ref cand_emb)) = (embedding, &candidate.embedding) {
                if new_emb.len() == cand_emb.len() {
                    if let Ok(sim) = cosine_similarity(new_emb, cand_emb) {
                        if sim >= config.cosine_threshold {
                            return Some(candidate.id);
                        }
                    }
                }
            }
        }

        // 3. Jaccard token overlap
        if config.jaccard_threshold > 0.0 {
            let cand_tokens = tokenize(&candidate.name);
            let sim = jaccard_similarity(&new_tokens, &cand_tokens);
            if sim >= config.jaccard_threshold {
                return Some(candidate.id);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_name_basic() {
        assert_eq!(normalize_name("Hora Engine"), "hora engine");
        assert_eq!(normalize_name("hora-engine"), "hora engine");
        assert_eq!(normalize_name("hora_engine"), "hora engine");
        assert_eq!(normalize_name("  HORA   Engine  "), "hora engine");
    }

    #[test]
    fn test_normalize_name_preserves_content() {
        assert_eq!(normalize_name("rust"), "rust");
        assert_eq!(
            normalize_name("Rust Programming Language"),
            "rust programming language"
        );
    }

    #[test]
    fn test_jaccard_identical() {
        let a = vec!["rust".into(), "engine".into()];
        let b = vec!["rust".into(), "engine".into()];
        assert!((jaccard_similarity(&a, &b) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_jaccard_disjoint() {
        let a = vec!["rust".into(), "engine".into()];
        let b = vec!["python".into(), "framework".into()];
        assert!(jaccard_similarity(&a, &b).abs() < f32::EPSILON);
    }

    #[test]
    fn test_jaccard_partial_overlap() {
        let a = vec!["rust".into(), "engine".into(), "graph".into()];
        let b = vec!["rust".into(), "engine".into(), "database".into()];
        // intersection=2, union=4 → 0.5
        let sim = jaccard_similarity(&a, &b);
        assert!((sim - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_jaccard_empty() {
        let empty: Vec<String> = vec![];
        assert!(jaccard_similarity(&empty, &empty).abs() < f32::EPSILON);
    }

    #[test]
    fn test_find_duplicate_name_exact() {
        let entities = vec![Entity {
            id: EntityId(1),
            entity_type: "project".into(),
            name: "Hora Engine".into(),
            properties: Default::default(),
            embedding: None,
            created_at: 0,
        }];
        let config = DedupConfig::default();

        // "hora-engine" normalizes to "hora engine" == "hora engine"
        let result = find_duplicate("hora-engine", None, "project", &entities, &config);
        assert_eq!(result, Some(EntityId(1)));
    }

    #[test]
    fn test_find_duplicate_cosine_embedding() {
        let emb_a = vec![1.0, 0.0, 0.0];
        let emb_b = vec![0.99, 0.1, 0.0]; // very similar

        let entities = vec![Entity {
            id: EntityId(1),
            entity_type: "concept".into(),
            name: "alpha".into(),
            properties: Default::default(),
            embedding: Some(emb_a),
            created_at: 0,
        }];
        let config = DedupConfig {
            cosine_threshold: 0.92,
            name_exact: false, // disable name check to test cosine only
            jaccard_threshold: 0.0,
            ..Default::default()
        };

        let result = find_duplicate("beta", Some(&emb_b), "concept", &entities, &config);
        assert_eq!(result, Some(EntityId(1)));
    }

    #[test]
    fn test_find_duplicate_cosine_below_threshold() {
        let emb_a = vec![1.0, 0.0, 0.0];
        let emb_b = vec![0.0, 1.0, 0.0]; // orthogonal

        let entities = vec![Entity {
            id: EntityId(1),
            entity_type: "concept".into(),
            name: "alpha".into(),
            properties: Default::default(),
            embedding: Some(emb_a),
            created_at: 0,
        }];
        let config = DedupConfig {
            cosine_threshold: 0.92,
            name_exact: false,
            jaccard_threshold: 0.0,
            ..Default::default()
        };

        let result = find_duplicate("beta", Some(&emb_b), "concept", &entities, &config);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_duplicate_jaccard() {
        let entities = vec![Entity {
            id: EntityId(1),
            entity_type: "project".into(),
            name: "rust graph engine".into(),
            properties: Default::default(),
            embedding: None,
            created_at: 0,
        }];
        let config = DedupConfig {
            name_exact: false,
            jaccard_threshold: 0.6,
            cosine_threshold: 0.0,
            ..Default::default()
        };

        // "rust graph database" → tokens: [rust, graph, database]
        // intersection with [rust, graph, engine] = 2, union = 4 → 0.5 < 0.6
        let result = find_duplicate("rust graph database", None, "project", &entities, &config);
        assert_eq!(result, None);

        // "rust graph engine fast" → tokens: [rust, graph, engine, fast]
        // intersection = 3, union = 4 → 0.75 >= 0.6
        let result = find_duplicate(
            "rust graph engine fast",
            None,
            "project",
            &entities,
            &config,
        );
        assert_eq!(result, Some(EntityId(1)));
    }

    #[test]
    fn test_find_duplicate_different_type_ignored() {
        let entities = vec![Entity {
            id: EntityId(1),
            entity_type: "project".into(),
            name: "hora engine".into(),
            properties: Default::default(),
            embedding: None,
            created_at: 0,
        }];
        let config = DedupConfig::default();

        // Same name but different type → not a duplicate
        let result = find_duplicate("hora engine", None, "person", &entities, &config);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_duplicate_disabled() {
        let entities = vec![Entity {
            id: EntityId(1),
            entity_type: "project".into(),
            name: "hora engine".into(),
            properties: Default::default(),
            embedding: None,
            created_at: 0,
        }];
        let config = DedupConfig {
            enabled: false,
            ..Default::default()
        };

        // Exact same name, same type, but dedup disabled
        let result = find_duplicate("hora engine", None, "project", &entities, &config);
        assert_eq!(result, None);
    }
}

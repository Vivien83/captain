//! Spreading activation with ACT-R fan effect.
//!
//! Propagates activation through the knowledge graph via BFS.
//! Fan effect: `S_ji = S_max - ln(fan)` — inhibition when fan > e^S_max ≈ 5.
//!
//! Reference: Anderson & Lebiere 1998, "The Atomic Components of Thought"

use std::collections::{HashMap, HashSet, VecDeque};

use crate::core::types::EntityId;

/// Parameters for spreading activation.
#[derive(Debug, Clone)]
pub struct SpreadingParams {
    /// Maximum associative strength (default 1.6, standard ACT-R).
    pub s_max: f64,
    /// Total activation weight distributed across sources (default 1.0).
    pub w_total: f64,
    /// Maximum propagation depth (default 3).
    pub max_depth: u8,
    /// Minimum absolute activation to continue propagating (default 0.01).
    pub cutoff: f64,
}

impl Default for SpreadingParams {
    fn default() -> Self {
        Self {
            s_max: 1.6,
            w_total: 1.0,
            max_depth: 3,
            cutoff: 0.01,
        }
    }
}

/// Spread activation from source nodes through a graph.
///
/// `sources`: (entity_id, weight) pairs — the activation origins.
/// `get_neighbors`: closure returning the neighbor IDs for a given entity.
///   The closure takes an `EntityId` and returns a `Vec<EntityId>`.
/// `params`: spreading parameters.
///
/// Returns accumulated activation per entity. Values can be negative (inhibition
/// from high-fan nodes).
pub fn spread_activation<F>(
    sources: &[(EntityId, f64)],
    get_neighbors: F,
    params: &SpreadingParams,
) -> HashMap<EntityId, f64>
where
    F: Fn(EntityId) -> Vec<EntityId>,
{
    let mut activations: HashMap<EntityId, f64> = HashMap::new();
    // Track visited (node, depth) to allow revisit at different depths
    // but prevent infinite loops at the same depth
    let mut visited: HashSet<EntityId> = HashSet::new();
    let mut queue: VecDeque<(EntityId, f64, u8)> = VecDeque::new();

    let n_sources = sources.len().max(1) as f64;

    // Initialize: push sources into the frontier
    for &(id, weight) in sources {
        *activations.entry(id).or_default() += weight;
        if visited.insert(id) {
            queue.push_back((id, weight, 0));
        }
    }

    while let Some((node, incoming, depth)) = queue.pop_front() {
        if depth >= params.max_depth {
            continue;
        }

        let neighbors = get_neighbors(node);
        let fan = neighbors.len();
        if fan == 0 {
            continue;
        }

        // Fan effect: S_ji = s_max - ln(fan)
        // Negative when fan > e^s_max (≈4.95 for s_max=1.6)
        let s_ji = params.s_max - (fan as f64).ln();
        let w_j = params.w_total / n_sources;
        let outgoing = w_j * s_ji * incoming.signum();

        // Cutoff: stop if outgoing activation is too small
        if outgoing.abs() < params.cutoff {
            continue;
        }

        for neighbor in neighbors {
            *activations.entry(neighbor).or_default() += outgoing;

            if visited.insert(neighbor) {
                queue.push_back((neighbor, outgoing, depth + 1));
            }
        }
    }

    activations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph(edges: &[(u64, u64)]) -> impl Fn(EntityId) -> Vec<EntityId> + '_ {
        move |id: EntityId| {
            edges
                .iter()
                .filter_map(|&(a, b)| {
                    if a == id.0 {
                        Some(EntityId(b))
                    } else if b == id.0 {
                        Some(EntityId(a))
                    } else {
                        None
                    }
                })
                .collect()
        }
    }

    #[test]
    fn test_simple_spread_a_to_b() {
        // A → B
        let graph = make_graph(&[(1, 2)]);
        let sources = vec![(EntityId(1), 1.0)];
        let params = SpreadingParams::default();

        let result = spread_activation(&sources, graph, &params);

        // B should receive activation
        assert!(result.contains_key(&EntityId(2)));
        let b_act = result[&EntityId(2)];
        assert!(
            b_act > 0.0,
            "B should have positive activation, got {b_act}"
        );
    }

    #[test]
    fn test_fan_effect_positive() {
        // A connected to B, C, D (fan=3, s_ji = 1.6 - ln(3) ≈ 0.50 > 0)
        let graph = make_graph(&[(1, 2), (1, 3), (1, 4)]);
        let sources = vec![(EntityId(1), 1.0)];
        let params = SpreadingParams::default();

        let result = spread_activation(&sources, graph, &params);

        // Fan=3 → positive spreading
        assert!(result[&EntityId(2)] > 0.0);
        assert!(result[&EntityId(3)] > 0.0);
        assert!(result[&EntityId(4)] > 0.0);
    }

    #[test]
    fn test_fan_effect_inhibition() {
        // A connected to B1..B10 (fan=10, s_ji = 1.6 - ln(10) ≈ -0.70 < 0)
        let edges: Vec<(u64, u64)> = (2..=11).map(|i| (1, i)).collect();
        let graph = make_graph(&edges);
        let sources = vec![(EntityId(1), 1.0)];
        let params = SpreadingParams::default();

        let result = spread_activation(&sources, graph, &params);

        // Fan=10 → negative spreading (inhibition)
        for i in 2..=11 {
            let act = result[&EntityId(i)];
            assert!(
                act < 0.0,
                "Entity {i} should have negative activation (inhibition), got {act}"
            );
        }
    }

    #[test]
    fn test_cycle_no_infinite_loop() {
        // A → B → A (cycle)
        let graph = make_graph(&[(1, 2)]);
        let sources = vec![(EntityId(1), 1.0)];
        let params = SpreadingParams::default();

        // Should terminate without hanging
        let result = spread_activation(&sources, graph, &params);
        assert!(result.contains_key(&EntityId(1)));
        assert!(result.contains_key(&EntityId(2)));
    }

    #[test]
    fn test_depth_limit() {
        // A → B → C → D, depth=2
        let graph = make_graph(&[(1, 2), (2, 3), (3, 4)]);
        let sources = vec![(EntityId(1), 1.0)];
        let params = SpreadingParams {
            max_depth: 2,
            ..Default::default()
        };

        let result = spread_activation(&sources, graph, &params);

        // A, B, C should have activation; D should not (beyond depth 2)
        assert!(result.contains_key(&EntityId(1)));
        assert!(result.contains_key(&EntityId(2)));
        assert!(result.contains_key(&EntityId(3)));
        // D might have 0 activation or not be in the map
        let d_act = result.get(&EntityId(4)).copied().unwrap_or(0.0);
        assert!(
            d_act.abs() < f64::EPSILON,
            "D should have no activation at depth 2, got {d_act}"
        );
    }

    #[test]
    fn test_no_neighbors_stops() {
        // Isolated node
        let graph = make_graph(&[]);
        let sources = vec![(EntityId(1), 1.0)];
        let params = SpreadingParams::default();

        let result = spread_activation(&sources, graph, &params);
        assert_eq!(result.len(), 1);
        assert!((result[&EntityId(1)] - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_multiple_sources() {
        // A → C, B → C
        let graph = make_graph(&[(1, 3), (2, 3)]);
        let sources = vec![(EntityId(1), 1.0), (EntityId(2), 1.0)];
        let params = SpreadingParams::default();

        let result = spread_activation(&sources, graph, &params);

        // C receives activation from both A and B
        let c_act = result[&EntityId(3)];
        assert!(
            c_act > 0.0,
            "C should have positive activation from 2 sources"
        );
    }

    #[test]
    fn test_cutoff_stops_weak_signals() {
        // A → B → C → D, with high cutoff
        let graph = make_graph(&[(1, 2), (2, 3), (3, 4)]);
        let sources = vec![(EntityId(1), 1.0)];
        let params = SpreadingParams {
            cutoff: 10.0, // very high → nothing propagates
            ..Default::default()
        };

        let result = spread_activation(&sources, graph, &params);
        // Only source should be present
        assert_eq!(result.len(), 1);
    }
}

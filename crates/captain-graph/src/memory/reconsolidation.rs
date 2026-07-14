//! Reconsolidation window — memory destabilization on strong reactivation.
//!
//! State machine: Stable → Labile (on strong access) → Restabilizing (after 5h)
//! → Stable (after 6h, with stability boost).
//!
//! Weak reactivation does NOT destabilize — only BLL record_access occurs.
//!
//! Reference: Nader et al. 2000, PMC2948875

/// Phase of the reconsolidation state machine.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MemoryPhase {
    /// Memory is consolidated and stable.
    Stable,
    /// Memory has been destabilized by strong reactivation.
    /// Plastic for `labile_window_secs` (default 5h).
    Labile {
        /// When the destabilization occurred (epoch seconds).
        destabilized_at: f64,
    },
    /// Labile window expired; memory is being reconsolidated.
    /// Lasts `restabilization_secs` (default 6h), then returns to Stable with boost.
    Restabilizing {
        /// When restabilization began (epoch seconds).
        started_at: f64,
    },
    /// Memory silenced — below activation threshold for too long.
    /// Invisible to search by default, recoverable via strong external cue.
    Dark {
        /// When the entity was silenced (epoch seconds).
        silenced_at: f64,
    },
}

/// Configurable parameters for reconsolidation.
#[derive(Debug, Clone)]
pub struct ReconsolidationParams {
    /// Duration of the labile (plastic) window in seconds (default 18000 = 5h).
    pub labile_window_secs: f64,
    /// Duration of the restabilization phase in seconds (default 21600 = 6h).
    pub restabilization_secs: f64,
    /// Minimum activation strength to trigger destabilization (default 0.5).
    pub destabilization_threshold: f64,
    /// Stability multiplier applied after each successful restabilization (default 1.2).
    pub restabilization_boost: f64,
}

impl Default for ReconsolidationParams {
    fn default() -> Self {
        Self {
            labile_window_secs: 18_000.0,   // 5 hours
            restabilization_secs: 21_600.0, // 6 hours
            destabilization_threshold: 0.5,
            restabilization_boost: 1.2,
        }
    }
}

/// Per-entity reconsolidation state.
#[derive(Debug, Clone)]
pub struct ReconsolidationState {
    phase: MemoryPhase,
    /// Cumulative stability multiplier (starts at 1.0, *= boost each restabilization).
    /// Will be consumed by FSRS in v0.3e.
    stability_multiplier: f64,
}

impl ReconsolidationState {
    /// Create a new reconsolidation state (starts Stable).
    pub fn new() -> Self {
        Self {
            phase: MemoryPhase::Stable,
            stability_multiplier: 1.0,
        }
    }

    /// Current phase of the reconsolidation state machine.
    pub fn phase(&self) -> &MemoryPhase {
        &self.phase
    }

    /// Cumulative stability multiplier (1.0 initially, grows with each restabilization).
    pub fn stability_multiplier(&self) -> f64 {
        self.stability_multiplier
    }

    /// Resolve pending time-based transitions at the given time.
    ///
    /// Call this before reading `phase()` to ensure the state is current.
    /// Returns `true` if the phase changed.
    pub fn tick(&mut self, now: f64, params: &ReconsolidationParams) -> bool {
        let mut changed = false;
        loop {
            match self.phase {
                MemoryPhase::Stable => break,
                MemoryPhase::Labile { destabilized_at } => {
                    if now - destabilized_at >= params.labile_window_secs {
                        // Restabilization starts when the labile window expires
                        let restab_start = destabilized_at + params.labile_window_secs;
                        self.phase = MemoryPhase::Restabilizing {
                            started_at: restab_start,
                        };
                        changed = true;
                        // Continue: check if restabilization also completed
                    } else {
                        break;
                    }
                }
                MemoryPhase::Restabilizing { started_at } => {
                    if now - started_at >= params.restabilization_secs {
                        self.phase = MemoryPhase::Stable;
                        self.stability_multiplier *= params.restabilization_boost;
                        changed = true;
                        break; // Stable is terminal
                    } else {
                        break;
                    }
                }
                MemoryPhase::Dark { .. } => break, // Dark is terminal until explicit recovery
            }
        }
        changed
    }

    /// Transition to Dark state (called by dark_node_pass).
    pub fn mark_dark(&mut self, now: f64) {
        self.phase = MemoryPhase::Dark { silenced_at: now };
    }

    /// Recover from Dark state → Labile (re-encoding via strong external cue).
    /// Returns `true` if recovery occurred.
    pub fn recover(&mut self, now: f64) -> bool {
        if matches!(self.phase, MemoryPhase::Dark { .. }) {
            self.phase = MemoryPhase::Labile {
                destabilized_at: now,
            };
            true
        } else {
            false
        }
    }

    /// Whether this entity is in Dark state.
    pub fn is_dark(&self) -> bool {
        matches!(self.phase, MemoryPhase::Dark { .. })
    }

    /// Process a reactivation event.
    ///
    /// If the entity is Stable and activation exceeds the threshold,
    /// it transitions to Labile. Otherwise, no state change occurs
    /// (weak reactivation = just BLL record_access, handled by caller).
    ///
    /// Time-based transitions are resolved first via `tick()`.
    pub fn on_reactivation(
        &mut self,
        activation: f64,
        now: f64,
        params: &ReconsolidationParams,
    ) -> &MemoryPhase {
        // Resolve any pending time-based transitions first
        self.tick(now, params);

        if self.phase == MemoryPhase::Stable && activation >= params.destabilization_threshold {
            self.phase = MemoryPhase::Labile {
                destabilized_at: now,
            };
        }

        &self.phase
    }
}

impl Default for ReconsolidationState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> ReconsolidationParams {
        ReconsolidationParams::default()
    }

    #[test]
    fn test_initial_state_is_stable() {
        let state = ReconsolidationState::new();
        assert_eq!(*state.phase(), MemoryPhase::Stable);
        assert!((state.stability_multiplier() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_strong_reactivation_destabilizes() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Activation 0.8 > threshold 0.5 → Labile
        let phase = state.on_reactivation(0.8, 1000.0, &params);
        match phase {
            MemoryPhase::Labile { destabilized_at } => {
                assert!((destabilized_at - 1000.0).abs() < f64::EPSILON);
            }
            other => panic!("Expected Labile, got {other:?}"),
        }
    }

    #[test]
    fn test_weak_reactivation_stays_stable() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Activation 0.3 < threshold 0.5 → stays Stable
        let phase = state.on_reactivation(0.3, 1000.0, &params);
        assert_eq!(*phase, MemoryPhase::Stable);
    }

    #[test]
    fn test_labile_to_restabilizing_after_window() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Destabilize at t=0
        state.on_reactivation(1.0, 0.0, &params);
        assert!(matches!(state.phase(), MemoryPhase::Labile { .. }));

        // Before window (4h = 14400s) → still Labile
        let changed = state.tick(14_400.0, &params);
        assert!(!changed);
        assert!(matches!(state.phase(), MemoryPhase::Labile { .. }));

        // After window (5h = 18000s) → Restabilizing
        let changed = state.tick(18_000.0, &params);
        assert!(changed);
        assert!(matches!(state.phase(), MemoryPhase::Restabilizing { .. }));
    }

    #[test]
    fn test_restabilizing_to_stable_with_boost() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Destabilize at t=0, advance past labile window
        state.on_reactivation(1.0, 0.0, &params);
        state.tick(18_000.0, &params); // → Restabilizing at t=18000

        assert!((state.stability_multiplier() - 1.0).abs() < f64::EPSILON);

        // Before restabilization complete (t=18000 + 20000 = 38000, need 18000+21600=39600)
        let changed = state.tick(38_000.0, &params);
        assert!(!changed);
        assert!(matches!(state.phase(), MemoryPhase::Restabilizing { .. }));

        // After restabilization (t=39600) → Stable with boost
        let changed = state.tick(39_600.0, &params);
        assert!(changed);
        assert_eq!(*state.phase(), MemoryPhase::Stable);
        assert!((state.stability_multiplier() - 1.2).abs() < f64::EPSILON);
    }

    #[test]
    fn test_full_cycle_stable_labile_restabilizing_stable() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Cycle 1
        state.on_reactivation(1.0, 0.0, &params);
        assert!(matches!(state.phase(), MemoryPhase::Labile { .. }));

        state.tick(18_000.0, &params);
        assert!(matches!(state.phase(), MemoryPhase::Restabilizing { .. }));

        state.tick(39_600.0, &params);
        assert_eq!(*state.phase(), MemoryPhase::Stable);
        assert!((state.stability_multiplier() - 1.2).abs() < f64::EPSILON);

        // Cycle 2: destabilize again
        state.on_reactivation(0.6, 40_000.0, &params);
        assert!(matches!(state.phase(), MemoryPhase::Labile { .. }));

        state.tick(58_000.0, &params); // 40000 + 18000
        assert!(matches!(state.phase(), MemoryPhase::Restabilizing { .. }));

        state.tick(79_600.0, &params); // 58000 + 21600
        assert_eq!(*state.phase(), MemoryPhase::Stable);
        // 1.2 * 1.2 = 1.44
        assert!((state.stability_multiplier() - 1.44).abs() < 0.001);
    }

    #[test]
    fn test_reactivation_during_labile_no_restart() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Destabilize at t=0
        state.on_reactivation(1.0, 0.0, &params);
        let orig_at = match state.phase() {
            MemoryPhase::Labile { destabilized_at } => *destabilized_at,
            _ => panic!("Expected Labile"),
        };

        // Another strong reactivation at t=1000 during Labile → stays Labile, doesn't restart
        state.on_reactivation(1.0, 1000.0, &params);
        match state.phase() {
            MemoryPhase::Labile { destabilized_at } => {
                assert!((destabilized_at - orig_at).abs() < f64::EPSILON);
            }
            other => panic!("Expected Labile with original timestamp, got {other:?}"),
        }
    }

    #[test]
    fn test_tick_on_stable_is_noop() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        let changed = state.tick(100_000.0, &params);
        assert!(!changed);
        assert_eq!(*state.phase(), MemoryPhase::Stable);
    }

    #[test]
    fn test_threshold_exact_boundary() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Activation exactly at threshold → destabilizes (>=)
        let phase = state.on_reactivation(0.5, 1000.0, &params);
        assert!(matches!(phase, MemoryPhase::Labile { .. }));
    }

    #[test]
    fn test_threshold_just_below() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Activation just below threshold → stays Stable
        let phase = state.on_reactivation(0.4999, 1000.0, &params);
        assert_eq!(*phase, MemoryPhase::Stable);
    }

    #[test]
    fn test_on_reactivation_resolves_pending_transitions() {
        let mut state = ReconsolidationState::new();
        let params = default_params();

        // Destabilize at t=0
        state.on_reactivation(1.0, 0.0, &params);

        // At t=40000 (past labile + restabilization), a new strong reactivation
        // should first tick through Labile→Restabilizing→Stable, then destabilize again
        let phase = state.on_reactivation(1.0, 40_000.0, &params);
        // Should be Labile again (new destabilization after completing full cycle)
        assert!(matches!(phase, MemoryPhase::Labile { .. }));
        assert!((state.stability_multiplier() - 1.2).abs() < f64::EPSILON);
    }

    #[test]
    fn test_custom_params() {
        let mut state = ReconsolidationState::new();
        let params = ReconsolidationParams {
            labile_window_secs: 100.0,
            restabilization_secs: 200.0,
            destabilization_threshold: 0.1,
            restabilization_boost: 2.0,
        };

        state.on_reactivation(0.15, 0.0, &params);
        assert!(matches!(state.phase(), MemoryPhase::Labile { .. }));

        state.tick(100.0, &params);
        assert!(matches!(state.phase(), MemoryPhase::Restabilizing { .. }));

        state.tick(300.0, &params);
        assert_eq!(*state.phase(), MemoryPhase::Stable);
        assert!((state.stability_multiplier() - 2.0).abs() < f64::EPSILON);
    }
}

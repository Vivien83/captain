//! FSRS — Free Spaced Repetition Scheduler.
//!
//! Complementary to ACT-R: ACT-R answers "who to retrieve now?" (real-time),
//! FSRS answers "when to schedule the next review?" (planning).
//!
//! Retrievability: `R(t, S) = (1 + F·t/S)^(-w20)`
//! where `F = 0.9^(-1/w20) - 1` and w20 ≈ 0.2.
//!
//! Reference: Expertium 2024, "The FSRS Algorithm"

/// Parameters for FSRS scheduling.
#[derive(Debug, Clone)]
pub struct FsrsParams {
    /// Power-law exponent (default 0.2).
    pub w20: f64,
    /// Desired retention probability for scheduling (default 0.9).
    pub desired_retention: f64,
    /// Initial stability in days for a new memory (default 1.0).
    pub initial_stability_days: f64,
    /// Minimum stability in days (floor, default 0.1).
    pub min_stability_days: f64,
}

impl Default for FsrsParams {
    fn default() -> Self {
        Self {
            w20: 0.2,
            desired_retention: 0.9,
            initial_stability_days: 1.0,
            min_stability_days: 0.1,
        }
    }
}

/// Per-entity FSRS state.
#[derive(Debug, Clone)]
pub struct FsrsState {
    /// Current stability in days (how long before retrievability drops to ~0.9).
    stability_days: f64,
    /// Epoch seconds of the last review (access that counts as review).
    last_review_at: f64,
}

const SECS_PER_DAY: f64 = 86_400.0;

impl FsrsState {
    /// Create a new FSRS state at the given creation time.
    pub fn new(created_at: f64, initial_stability_days: f64) -> Self {
        Self {
            stability_days: initial_stability_days,
            last_review_at: created_at,
        }
    }

    /// Current stability in days.
    pub fn stability_days(&self) -> f64 {
        self.stability_days
    }

    /// Epoch seconds of the last review.
    pub fn last_review_at(&self) -> f64 {
        self.last_review_at
    }

    /// Compute the factor F from w20: `F = 0.9^(-1/w20) - 1`.
    fn compute_f(w20: f64) -> f64 {
        0.9_f64.powf(-1.0 / w20) - 1.0
    }

    /// Current retrievability at time `now` (epoch seconds).
    ///
    /// Returns a value in [0, 1]:
    /// - R(t=0) = 1.0 (just after review)
    /// - R(t→∞) → 0.0 (completely forgotten)
    pub fn current_retrievability(&self, now: f64, params: &FsrsParams) -> f64 {
        let t_days = ((now - self.last_review_at) / SECS_PER_DAY).max(0.0);
        let f = Self::compute_f(params.w20);
        (1.0 + f * t_days / self.stability_days).powf(-params.w20)
    }

    /// Compute the optimal review interval in days for a desired retention.
    ///
    /// `interval = S / F · (r^(-1/w20) - 1)`
    pub fn next_review_interval_days(&self, desired_retention: f64, params: &FsrsParams) -> f64 {
        let f = Self::compute_f(params.w20);
        if f.abs() < f64::EPSILON {
            return self.stability_days;
        }
        self.stability_days / f * (desired_retention.powf(-1.0 / params.w20) - 1.0)
    }

    /// Record a successful review, updating stability.
    ///
    /// `boost` is the reconsolidation stability_multiplier (1.0 if no boost).
    /// After review: `S = max(S * boost, min_stability_days)`.
    pub fn record_review(&mut self, now: f64, boost: f64, params: &FsrsParams) {
        self.stability_days = (self.stability_days * boost).max(params.min_stability_days);
        self.last_review_at = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> FsrsParams {
        FsrsParams::default()
    }

    #[test]
    fn test_retrievability_at_t0_is_1() {
        let state = FsrsState::new(1000.0, 1.0);
        let params = default_params();
        let r = state.current_retrievability(1000.0, &params);
        assert!((r - 1.0).abs() < 1e-10, "R(t=0) should be 1.0, got {r}");
    }

    #[test]
    fn test_retrievability_decays_over_time() {
        let state = FsrsState::new(0.0, 1.0);
        let params = default_params();

        let r_1d = state.current_retrievability(SECS_PER_DAY, &params);
        let r_7d = state.current_retrievability(7.0 * SECS_PER_DAY, &params);
        let r_30d = state.current_retrievability(30.0 * SECS_PER_DAY, &params);

        assert!(r_1d < 1.0, "R after 1 day should be < 1.0, got {r_1d}");
        assert!(
            r_7d < r_1d,
            "R after 7d={r_7d} should be < R after 1d={r_1d}"
        );
        assert!(
            r_30d < r_7d,
            "R after 30d={r_30d} should be < R after 7d={r_7d}"
        );
        assert!(r_30d > 0.0, "R should still be > 0");
    }

    #[test]
    fn test_retrievability_approaches_zero() {
        let state = FsrsState::new(0.0, 1.0);
        let params = default_params();

        // After a very long time (10 years), R decays but slowly with w20=0.2
        let r = state.current_retrievability(365.0 * 10.0 * SECS_PER_DAY, &params);
        assert!(r < 0.3, "R after 10 years should be low, got {r}");
        assert!(r > 0.0, "R should still be > 0");
    }

    #[test]
    fn test_stability_increases_with_review_boost() {
        let mut state = FsrsState::new(0.0, 1.0);
        let params = default_params();

        assert!((state.stability_days() - 1.0).abs() < f64::EPSILON);

        // Review with boost 1.2
        state.record_review(SECS_PER_DAY, 1.2, &params);
        assert!(
            (state.stability_days() - 1.2).abs() < f64::EPSILON,
            "S should be 1.2 after boost, got {}",
            state.stability_days()
        );

        // Review with boost 1.5
        state.record_review(2.0 * SECS_PER_DAY, 1.5, &params);
        assert!(
            (state.stability_days() - 1.8).abs() < 0.001,
            "S should be 1.2 * 1.5 = 1.8, got {}",
            state.stability_days()
        );
    }

    #[test]
    fn test_next_review_interval_increases_with_stability() {
        let params = default_params();

        let state_low = FsrsState::new(0.0, 1.0);
        let state_high = FsrsState::new(0.0, 10.0);

        let interval_low = state_low.next_review_interval_days(0.9, &params);
        let interval_high = state_high.next_review_interval_days(0.9, &params);

        assert!(
            interval_high > interval_low,
            "Higher stability should give longer interval: low={interval_low}, high={interval_high}"
        );
    }

    #[test]
    fn test_next_review_interval_for_default_retention() {
        let state = FsrsState::new(0.0, 1.0);
        let params = default_params();

        // With desired_retention = 0.9, the interval should equal S
        // because F is calibrated so that R = 0.9 when t = S
        let interval = state.next_review_interval_days(0.9, &params);
        assert!(
            (interval - 1.0).abs() < 0.01,
            "With r=0.9, interval should ≈ S=1.0 day, got {interval}"
        );
    }

    #[test]
    fn test_review_respects_min_stability() {
        let mut state = FsrsState::new(0.0, 0.05);
        let params = FsrsParams {
            min_stability_days: 0.1,
            ..Default::default()
        };

        // Even with boost < 1, stability should not go below min
        state.record_review(SECS_PER_DAY, 0.5, &params);
        assert!(
            (state.stability_days() - 0.1).abs() < f64::EPSILON,
            "Stability should be clamped to min 0.1, got {}",
            state.stability_days()
        );
    }

    #[test]
    fn test_review_updates_last_review_at() {
        let mut state = FsrsState::new(0.0, 1.0);
        let params = default_params();

        state.record_review(5000.0, 1.0, &params);
        assert!((state.last_review_at() - 5000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_retrievability_resets_after_review() {
        let mut state = FsrsState::new(0.0, 1.0);
        let params = default_params();

        // After 100 days with S=1, R ≈ 0.43 (well below 1.0)
        let r_before = state.current_retrievability(100.0 * SECS_PER_DAY, &params);
        assert!(
            r_before < 0.5,
            "R at 100 days should be < 0.5, got {r_before}"
        );

        // Review at day 100
        state.record_review(100.0 * SECS_PER_DAY, 1.0, &params);

        // Immediately after review, R is back to 1.0
        let r_after = state.current_retrievability(100.0 * SECS_PER_DAY, &params);
        assert!(
            (r_after - 1.0).abs() < 1e-10,
            "R should be 1.0 right after review, got {r_after}"
        );
    }

    #[test]
    fn test_f_constant_value() {
        // F = 0.9^(-1/0.2) - 1 = 0.9^(-5) - 1
        let f = FsrsState::compute_f(0.2);
        let expected = 0.9_f64.powf(-5.0) - 1.0;
        assert!(
            (f - expected).abs() < 1e-10,
            "F should be {expected}, got {f}"
        );
    }
}

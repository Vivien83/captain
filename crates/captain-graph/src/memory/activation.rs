//! ACT-R Base-Level Learning (BLL) — activation decay with Petrov 2006 approximation.
//!
//! Each entity has an `ActivationState` tracking recent accesses (exact) and
//! historical accesses (aggregated). Activation decays as a power law:
//! `B(i) = ln(Σ tⱼ^(-d))` where d=0.5.
//!
//! Reference: Petrov 2006, "Computationally Efficient Approximation of the
//! Base-Level Learning Equation in ACT-R"

/// Default decay exponent (power law of forgetting, Anderson 1991).
const DEFAULT_DECAY: f64 = 0.5;

/// Number of recent access timestamps kept exact (Petrov window).
const RECENT_WINDOW: usize = 10;

/// Minimum time delta to avoid division by zero (seconds).
const MIN_DELTA: f64 = 0.001;

/// Per-entity activation state using Petrov hybrid approximation.
///
/// Keeps the last 10 accesses exact for the recency bump, and aggregates
/// older accesses into a running sum for O(1) memory.
#[derive(Debug, Clone)]
pub struct ActivationState {
    /// Epoch seconds when this entity was created.
    pub created_at: f64,
    /// Number of accesses evacuated to the aggregate.
    pub historical_count: u32,
    /// Running sum of `delta^(-d)` for historical accesses.
    pub historical_sum: f64,
    /// Timestamps (epoch seconds) of the most recent accesses.
    recent_accesses: [f64; RECENT_WINDOW],
    /// How many slots in `recent_accesses` are used (0..=10).
    recent_count: u8,
    /// Decay exponent (default 0.5).
    decay_d: f64,
    /// Cached activation score (valid only if `!dirty`).
    cached_activation: f64,
    /// Whether the cache needs recomputation.
    dirty: bool,
    /// Cumulative SHY downscaling factor (default 1.0, multiplicative on BLL score).
    shy_factor: f64,
}

impl ActivationState {
    /// Create a new activation state at the given creation time.
    pub fn new(created_at: f64) -> Self {
        Self {
            created_at,
            historical_count: 0,
            historical_sum: 0.0,
            recent_accesses: [0.0; RECENT_WINDOW],
            recent_count: 0,
            decay_d: DEFAULT_DECAY,
            cached_activation: f64::NEG_INFINITY,
            dirty: true,
            shy_factor: 1.0,
        }
    }

    /// Compute BLL activation: `B(i) = ln(Σ tⱼ^(-d))`.
    ///
    /// Uses log-sum-exp for numerical stability on very old memories.
    pub fn compute_activation(&mut self, now: f64) -> f64 {
        if !self.dirty {
            return self.cached_activation;
        }

        let d = self.decay_d;

        // Collect log-terms for log-sum-exp stability
        // Each term: t^(-d) = exp(-d * ln(t))
        // So log-term = -d * ln(delta)
        let mut log_terms: Vec<f64> = Vec::with_capacity(RECENT_WINDOW + 1);

        // Recent accesses: exact computation
        for i in 0..self.recent_count as usize {
            let delta = (now - self.recent_accesses[i]).max(MIN_DELTA);
            log_terms.push(-d * delta.ln());
        }

        // Historical aggregate: if we have historical accesses, add them as a single term.
        // historical_sum = Σ delta_old^(-d), so ln(historical_sum) is the log-term.
        if self.historical_count > 0 && self.historical_sum > 0.0 {
            log_terms.push(self.historical_sum.ln());
        }

        let result = if log_terms.is_empty() {
            f64::NEG_INFINITY
        } else {
            log_sum_exp(&log_terms)
        };

        let scaled = result * self.shy_factor;
        self.cached_activation = scaled;
        self.dirty = false;
        scaled
    }

    /// Record an access at the given timestamp (epoch seconds).
    ///
    /// If the recent window is full, the oldest access is evacuated to the
    /// historical aggregate before inserting the new one.
    pub fn record_access(&mut self, now: f64) {
        let d = self.decay_d;

        if self.recent_count as usize >= RECENT_WINDOW {
            // Evacuate the oldest to aggregate
            let oldest = self.recent_accesses[0];
            let delta = (now - oldest).max(MIN_DELTA);
            self.historical_sum += delta.powf(-d);
            self.historical_count += 1;
            // Shift left
            self.recent_accesses.copy_within(1..RECENT_WINDOW, 0);
            self.recent_accesses[RECENT_WINDOW - 1] = now;
        } else {
            self.recent_accesses[self.recent_count as usize] = now;
            self.recent_count += 1;
        }
        self.dirty = true;
    }

    /// Total number of recorded accesses (recent + historical).
    pub fn total_accesses(&self) -> u32 {
        self.historical_count + self.recent_count as u32
    }

    /// Apply SHY homeostatic downscaling: multiply the activation score by `factor`.
    ///
    /// This is cumulative: two calls with 0.78 result in `shy_factor = 0.78 * 0.78 = 0.6084`.
    /// Affects both positive and negative activations (amplitude reduction).
    pub fn apply_shy_downscaling(&mut self, factor: f64) {
        self.shy_factor *= factor;
        self.dirty = true;
    }

    /// Current cumulative SHY downscaling factor (1.0 = no downscaling applied).
    pub fn shy_factor(&self) -> f64 {
        self.shy_factor
    }

    /// Timestamp of the most recent access (epoch seconds), or `None` if never accessed.
    pub fn last_access_time(&self) -> Option<f64> {
        if self.recent_count > 0 {
            Some(self.recent_accesses[self.recent_count as usize - 1])
        } else {
            None
        }
    }
}

/// Numerically stable log-sum-exp: `ln(Σ exp(xᵢ))`.
///
/// Avoids overflow/underflow by factoring out the maximum.
fn log_sum_exp(log_terms: &[f64]) -> f64 {
    if log_terms.is_empty() {
        return f64::NEG_INFINITY;
    }
    let max_log = log_terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if max_log.is_infinite() {
        return f64::NEG_INFINITY;
    }
    let sum_exp: f64 = log_terms.iter().map(|&l| (l - max_log).exp()).sum();
    max_log + sum_exp.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activation_after_one_access() {
        let mut state = ActivationState::new(0.0);
        state.record_access(1.0);
        let act = state.compute_activation(2.0);
        // delta=1.0, term = 1.0^(-0.5) = 1.0, B = ln(1.0) = 0.0
        assert!(act.is_finite());
        assert!((act - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_activation_decay_over_time() {
        let mut state = ActivationState::new(0.0);
        state.record_access(1.0);

        let act_soon = state.compute_activation(2.0);
        state.dirty = true; // force recompute
        let act_later = state.compute_activation(1000.0);

        // Activation should decrease over time (power law decay)
        assert!(
            act_soon > act_later,
            "act_soon={act_soon} should be > act_later={act_later}"
        );
    }

    #[test]
    fn test_multiple_accesses_higher_activation() {
        let mut state1 = ActivationState::new(0.0);
        state1.record_access(1.0);
        let act1 = state1.compute_activation(100.0);

        let mut state2 = ActivationState::new(0.0);
        state2.record_access(1.0);
        state2.record_access(50.0);
        state2.record_access(90.0);
        let act2 = state2.compute_activation(100.0);

        // More accesses → higher activation
        assert!(act2 > act1, "act2={act2} should be > act1={act1}");
    }

    #[test]
    fn test_record_access_overflow_evacuates() {
        let mut state = ActivationState::new(0.0);

        // Fill the 10-slot window
        for i in 1..=10 {
            state.record_access(i as f64);
        }
        assert_eq!(state.recent_count, 10);
        assert_eq!(state.historical_count, 0);

        // 11th access → evacuates oldest to historical
        state.record_access(11.0);
        assert_eq!(state.recent_count, 10);
        assert_eq!(state.historical_count, 1);
        assert!(state.historical_sum > 0.0);

        // Activation should still be computable
        let act = state.compute_activation(12.0);
        assert!(act.is_finite());
    }

    #[test]
    fn test_petrov_approximation_accuracy() {
        // Compare hybrid (Petrov) vs "exact" (all recent) for moderate access count.
        // With <=10 accesses, Petrov is exact (all in recent window).
        let mut state = ActivationState::new(0.0);
        let now = 1000.0;

        for i in 1..=10 {
            state.record_access(i as f64 * 10.0);
        }

        let petrov_act = state.compute_activation(now);

        // Compute exact manually
        let d = 0.5;
        let mut exact_sum = 0.0_f64;
        for i in 1..=10 {
            let delta = (now - i as f64 * 10.0).max(MIN_DELTA);
            exact_sum += delta.powf(-d);
        }
        let exact_act = exact_sum.ln();

        assert!(
            (petrov_act - exact_act).abs() < 0.01,
            "petrov={petrov_act}, exact={exact_act}"
        );
    }

    #[test]
    fn test_petrov_with_historical_aggregate() {
        // Fill beyond 10 accesses, then compare activation is still reasonable.
        let mut state = ActivationState::new(0.0);
        let now = 10000.0;

        // 20 accesses spread over time
        for i in 1..=20 {
            state.record_access(i as f64 * 100.0);
        }

        assert_eq!(state.historical_count, 10);
        assert_eq!(state.recent_count, 10);

        let act = state.compute_activation(now);
        assert!(act.is_finite());
        // 20 accesses should give higher activation than 10
        let mut state10 = ActivationState::new(0.0);
        for i in 11..=20 {
            state10.record_access(i as f64 * 100.0);
        }
        let act10 = state10.compute_activation(now);
        assert!(
            act > act10,
            "20 accesses ({act}) should > 10 accesses ({act10})"
        );
    }

    #[test]
    fn test_numerical_stability_very_old() {
        // Very old access (10^8 seconds ago) → should not produce NaN/Inf
        let mut state = ActivationState::new(0.0);
        state.record_access(1.0);
        let act = state.compute_activation(1e8 + 1.0);
        assert!(act.is_finite(), "activation should be finite, got {act}");
        // Should be very negative (nearly forgotten)
        assert!(act < 0.0);
    }

    #[test]
    fn test_numerical_stability_very_recent() {
        // Access at nearly the same time → very high activation
        let mut state = ActivationState::new(0.0);
        state.record_access(100.0);
        let act = state.compute_activation(100.001);
        assert!(act.is_finite(), "activation should be finite, got {act}");
        assert!(act > 0.0);
    }

    #[test]
    fn test_no_accesses_returns_neg_infinity() {
        let mut state = ActivationState::new(0.0);
        let act = state.compute_activation(100.0);
        assert!(act.is_infinite() && act < 0.0);
    }

    #[test]
    fn test_cache_returns_same_value() {
        let mut state = ActivationState::new(0.0);
        state.record_access(1.0);
        let act1 = state.compute_activation(100.0);
        let act2 = state.compute_activation(100.0);
        assert!((act1 - act2).abs() < f64::EPSILON);
    }

    #[test]
    fn test_total_accesses() {
        let mut state = ActivationState::new(0.0);
        for i in 1..=15 {
            state.record_access(i as f64);
        }
        assert_eq!(state.total_accesses(), 15);
    }
}

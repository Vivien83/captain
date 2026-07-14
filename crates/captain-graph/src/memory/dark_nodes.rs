//! Dark nodes — active forgetting inspired by Rac1 pathway.
//!
//! Entities below the activation threshold for too long are silenced (Dark).
//! Dark nodes are invisible to search by default but recoverable via
//! strong external reactivation.
//!
//! Reference: Davis & Bhatt 2015 (Rac1-dependent forgetting)

/// Parameters for dark node detection and recovery.
#[derive(Debug, Clone)]
pub struct DarkNodeParams {
    /// Activation below which an entity becomes a dark node candidate (default -2.0).
    pub silencing_threshold: f64,
    /// Minimum time since last access before silencing (seconds, default 604800 = 7 days).
    pub silencing_delay_secs: f64,
    /// Minimum activation for recovery from Dark state (default 1.5).
    pub recovery_threshold: f64,
    /// Time in Dark state before eligible for GC (seconds, default 7776000 = 90 days).
    pub gc_eligible_after_secs: f64,
}

impl Default for DarkNodeParams {
    fn default() -> Self {
        Self {
            silencing_threshold: -2.0,
            silencing_delay_secs: 604_800.0, // 7 days
            recovery_threshold: 1.5,
            gc_eligible_after_secs: 7_776_000.0, // 90 days
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_params() {
        let p = DarkNodeParams::default();
        assert!((p.silencing_threshold - (-2.0)).abs() < f64::EPSILON);
        assert!((p.silencing_delay_secs - 604_800.0).abs() < f64::EPSILON);
        assert!((p.recovery_threshold - 1.5).abs() < f64::EPSILON);
        assert!((p.gc_eligible_after_secs - 7_776_000.0).abs() < f64::EPSILON);
    }
}

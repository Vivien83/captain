use serde::{Deserialize, Serialize};

/// Maximum accepted value for one configured cost window.
///
/// This is deliberately far above practical subscription or API spending while
/// still rejecting accidental exponent-sized values at configuration boundaries.
pub const MAX_BUDGET_LIMIT_USD: f64 = 1_000_000_000.0;

/// Global spending budget configuration.
///
/// Set limits to 0.0 for unlimited. All limits apply across all agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetConfig {
    /// Maximum total cost in USD per hour (0.0 = unlimited).
    pub max_hourly_usd: f64,
    /// Maximum total cost in USD per day (0.0 = unlimited).
    pub max_daily_usd: f64,
    /// Maximum total cost in USD per month (0.0 = unlimited).
    pub max_monthly_usd: f64,
    /// Alert threshold as a fraction (0.0 - 1.0). Trigger warnings at this % of any limit.
    pub alert_threshold: f64,
    /// Default per-agent hourly token limit override. When set (> 0), all agents
    /// will be overridden to this value. Set to 0 to keep each agent's own limit.
    /// Use this to globally raise or lower the token budget for all agents.
    pub default_max_llm_tokens_per_hour: u64,
}

impl BudgetConfig {
    /// Validate values before publishing them into a live runtime.
    pub fn validate(&self) -> Result<(), String> {
        validate_cost_limit("max_hourly_usd", self.max_hourly_usd)?;
        validate_cost_limit("max_daily_usd", self.max_daily_usd)?;
        validate_cost_limit("max_monthly_usd", self.max_monthly_usd)?;

        if !self.alert_threshold.is_finite() {
            return Err("alert_threshold must be finite".to_string());
        }
        if !(0.0..=1.0).contains(&self.alert_threshold) {
            return Err("alert_threshold must be between 0 and 1".to_string());
        }
        if self.default_max_llm_tokens_per_hour > i64::MAX as u64 {
            return Err("default_max_llm_tokens_per_hour exceeds TOML integer range".to_string());
        }
        Ok(())
    }
}

fn validate_cost_limit(name: &str, value: f64) -> Result<(), String> {
    if !value.is_finite() {
        return Err(format!("{name} must be finite"));
    }
    if value < 0.0 {
        return Err(format!("{name} must be non-negative"));
    }
    if value > MAX_BUDGET_LIMIT_USD {
        return Err(format!(
            "{name} exceeds the maximum supported value of {MAX_BUDGET_LIMIT_USD}"
        ));
    }
    Ok(())
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            max_hourly_usd: 0.0,
            max_daily_usd: 0.0,
            max_monthly_usd: 0.0,
            alert_threshold: 0.8,
            default_max_llm_tokens_per_hour: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_defaults_keep_runtime_unlimited() {
        let budget = BudgetConfig::default();

        assert_eq!(budget.max_hourly_usd, 0.0);
        assert_eq!(budget.max_daily_usd, 0.0);
        assert_eq!(budget.max_monthly_usd, 0.0);
        assert_eq!(budget.alert_threshold, 0.8);
        assert_eq!(budget.default_max_llm_tokens_per_hour, 0);
    }

    #[test]
    fn budget_deserializes_partial_toml_with_defaults() {
        let budget: BudgetConfig = toml::from_str(
            r#"
            max_daily_usd = 25.5
            default_max_llm_tokens_per_hour = 100000
            "#,
        )
        .unwrap();

        assert_eq!(budget.max_hourly_usd, 0.0);
        assert_eq!(budget.max_daily_usd, 25.5);
        assert_eq!(budget.max_monthly_usd, 0.0);
        assert_eq!(budget.alert_threshold, 0.8);
        assert_eq!(budget.default_max_llm_tokens_per_hour, 100_000);
    }

    #[test]
    fn budget_roundtrips_all_limits() {
        let budget = BudgetConfig {
            max_hourly_usd: 1.25,
            max_daily_usd: 10.0,
            max_monthly_usd: 200.0,
            alert_threshold: 0.65,
            default_max_llm_tokens_per_hour: 42_000,
        };

        let encoded = toml::to_string(&budget).unwrap();
        let decoded: BudgetConfig = toml::from_str(&encoded).unwrap();

        assert_eq!(decoded.max_hourly_usd, 1.25);
        assert_eq!(decoded.max_daily_usd, 10.0);
        assert_eq!(decoded.max_monthly_usd, 200.0);
        assert_eq!(decoded.alert_threshold, 0.65);
        assert_eq!(decoded.default_max_llm_tokens_per_hour, 42_000);
    }

    #[test]
    fn budget_validation_rejects_unsafe_numeric_values() {
        for value in [-1.0, f64::NAN, f64::INFINITY, 1.0e308] {
            let budget = BudgetConfig {
                max_hourly_usd: value,
                ..BudgetConfig::default()
            };
            assert!(budget.validate().is_err(), "{value:?} must be rejected");
        }

        let invalid_alert = BudgetConfig {
            alert_threshold: 1.01,
            ..BudgetConfig::default()
        };
        assert!(invalid_alert.validate().is_err());

        let invalid_tokens = BudgetConfig {
            default_max_llm_tokens_per_hour: i64::MAX as u64 + 1,
            ..BudgetConfig::default()
        };
        assert!(invalid_tokens.validate().is_err());
    }
}

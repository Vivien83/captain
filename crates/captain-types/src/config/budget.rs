use serde::{Deserialize, Serialize};

/// Global spending budget configuration.
///
/// Set limits to 0.0 for unlimited. All limits apply across all agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

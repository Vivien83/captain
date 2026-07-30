use super::CaptainKernel;
use crate::error::{KernelError, KernelResult};
use captain_types::agent::AgentId;
use captain_types::config::BudgetConfig;
use std::fmt;

/// Failure while validating or durably publishing a live budget snapshot.
#[derive(Debug)]
pub enum BudgetConfigUpdateError {
    Validation(String),
    Persistence(String),
}

impl fmt::Display for BudgetConfigUpdateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Validation(message) => write!(formatter, "{message}"),
            Self::Persistence(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for BudgetConfigUpdateError {}

impl CaptainKernel {
    /// Return one coherent live budget snapshot.
    pub fn budget_config(&self) -> BudgetConfig {
        self.live_budget_config
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Serialize a partial update, persist the exact candidate, then publish it.
    ///
    /// Readers see either the complete previous snapshot or the complete new
    /// snapshot. A persistence failure leaves the live state unchanged.
    pub fn update_budget_config<F>(
        &self,
        update: F,
    ) -> Result<BudgetConfig, BudgetConfigUpdateError>
    where
        F: FnOnce(&mut BudgetConfig),
    {
        self.update_budget_config_with_persistence(update, persist_budget_snapshot)
    }

    fn update_budget_config_with_persistence<F, P>(
        &self,
        update: F,
        persist: P,
    ) -> Result<BudgetConfig, BudgetConfigUpdateError>
    where
        F: FnOnce(&mut BudgetConfig),
        P: FnOnce(&std::path::Path, &BudgetConfig) -> Result<(), String>,
    {
        let _update_guard = self
            .budget_config_update_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut candidate = self.budget_config();
        update(&mut candidate);
        candidate
            .validate()
            .map_err(BudgetConfigUpdateError::Validation)?;
        persist(&self.config.home_dir.join("config.toml"), &candidate)
            .map_err(BudgetConfigUpdateError::Persistence)?;
        self.publish_live_budget(candidate.clone());
        Ok(candidate)
    }

    pub(super) fn publish_reloaded_budget_config(
        &self,
        candidate: BudgetConfig,
    ) -> Result<(), String> {
        candidate.validate()?;
        self.publish_live_budget(candidate);
        Ok(())
    }

    fn publish_live_budget(&self, candidate: BudgetConfig) {
        let mut live = self
            .live_budget_config
            .write()
            .unwrap_or_else(|error| error.into_inner());
        *live = candidate;
    }

    pub(super) fn check_turn_budget(&self, agent_id: AgentId) -> KernelResult<()> {
        self.metering
            .check_global_budget(&self.budget_config())
            .map_err(KernelError::Captain)?;
        self.scheduler
            .check_quota(agent_id)
            .map_err(KernelError::Captain)
    }
}

fn persist_budget_snapshot(path: &std::path::Path, budget: &BudgetConfig) -> Result<(), String> {
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let mut document = content
        .parse::<toml::Value>()
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    let root = document
        .as_table_mut()
        .ok_or_else(|| format!("{} must contain a TOML table", path.display()))?;
    let budget_table = root
        .entry("budget")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| "[budget] must be a TOML table".to_string())?;

    update_persisted_budget_table(budget_table, budget);
    let serialized = toml::to_string_pretty(&document)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    captain_types::durable_fs::atomic_write(path, serialized.as_bytes())
        .map_err(|error| format!("persist {}: {error}", path.display()))
}

fn update_persisted_budget_table(
    budget_table: &mut toml::map::Map<String, toml::Value>,
    budget: &BudgetConfig,
) {
    budget_table.insert(
        "max_hourly_usd".into(),
        toml::Value::Float(budget.max_hourly_usd),
    );
    budget_table.insert(
        "max_daily_usd".into(),
        toml::Value::Float(budget.max_daily_usd),
    );
    budget_table.insert(
        "max_monthly_usd".into(),
        toml::Value::Float(budget.max_monthly_usd),
    );
    budget_table.insert(
        "alert_threshold".into(),
        toml::Value::Float(budget.alert_threshold),
    );
    budget_table.insert(
        "default_max_llm_tokens_per_hour".into(),
        toml::Value::Integer(budget.default_max_llm_tokens_per_hour as i64),
    );
}

#[cfg(test)]
mod tests {
    use super::{update_persisted_budget_table, BudgetConfigUpdateError};
    use crate::error::KernelError;
    use crate::kernel::CaptainKernel;
    use captain_memory::usage::UsageRecord;
    use captain_types::config::{BudgetConfig, KernelConfig};
    use captain_types::error::CaptainError;

    #[test]
    fn persisted_budget_keeps_default_hourly_token_guard() {
        let budget = BudgetConfig {
            max_hourly_usd: 1.0,
            max_daily_usd: 5.0,
            max_monthly_usd: 25.0,
            alert_threshold: 0.75,
            default_max_llm_tokens_per_hour: 345_678,
        };
        let mut table = toml::map::Map::new();

        update_persisted_budget_table(&mut table, &budget);

        assert_eq!(
            table["default_max_llm_tokens_per_hour"].as_integer(),
            Some(345_678)
        );
        assert_eq!(table["max_hourly_usd"].as_float(), Some(1.0));
        assert_eq!(table["alert_threshold"].as_float(), Some(0.75));
    }

    #[test]
    fn turn_budget_enforces_the_live_global_snapshot() {
        let temporary = tempfile::tempdir().unwrap();
        let home_dir = temporary.path().join("home");
        let kernel = CaptainKernel::boot_with_config(KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            budget: BudgetConfig {
                max_hourly_usd: 1.0,
                ..BudgetConfig::default()
            },
            ..KernelConfig::default()
        })
        .unwrap();
        let agent_id = kernel.registry.list()[0].id;
        kernel
            .metering
            .record(&UsageRecord {
                agent_id,
                model: "test".to_string(),
                input_tokens: 1,
                output_tokens: 1,
                cached_input_tokens: 0,
                cache_creation_tokens: 0,
                cost_usd: 1.0,
                tool_calls: 0,
            })
            .unwrap();

        assert!(matches!(
            kernel.check_turn_budget(agent_id),
            Err(KernelError::Captain(CaptainError::QuotaExceeded(_)))
        ));
        kernel.shutdown();
    }

    #[test]
    fn persistence_failure_keeps_the_previous_live_snapshot() {
        let temporary = tempfile::tempdir().unwrap();
        let home_dir = temporary.path().join("home");
        let kernel = CaptainKernel::boot_with_config(KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        })
        .unwrap();
        let previous = kernel.budget_config();

        let result = kernel.update_budget_config_with_persistence(
            |budget| {
                budget.max_hourly_usd = 42.0;
            },
            |path, candidate| {
                assert_eq!(path, home_dir.join("config.toml"));
                assert_eq!(candidate.max_hourly_usd, 42.0);
                assert_eq!(
                    kernel.budget_config(),
                    previous,
                    "the candidate must remain unpublished until persistence succeeds"
                );
                Err("injected durable-write failure".to_string())
            },
        );

        assert!(matches!(
            result,
            Err(BudgetConfigUpdateError::Persistence(_))
        ));
        assert_eq!(kernel.budget_config(), previous);
        kernel.shutdown();
    }
}

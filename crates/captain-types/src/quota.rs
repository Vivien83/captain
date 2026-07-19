//! Structured quota contracts shared by runtime, API, and user surfaces.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable machine-readable quota scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaScope {
    /// Captain's per-agent rolling token guard.
    AgentHourlyTokens,
    /// Per-agent cost guard over one hour.
    AgentHourlyCost,
    /// Per-agent cost guard over one day.
    AgentDailyCost,
    /// Per-agent cost guard over one calendar month.
    AgentMonthlyCost,
    /// Global cost guard over one hour.
    GlobalHourlyCost,
    /// Global cost guard over one day.
    GlobalDailyCost,
    /// Global cost guard over one calendar month.
    GlobalMonthlyCost,
    /// A provider-managed subscription or account allowance.
    ProviderSubscription,
}

/// Unit used by a quota amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaUnit {
    /// LLM tokens.
    Tokens,
    /// United States dollars.
    Usd,
    /// Percentage of an allowance.
    Percent,
}

/// Where a provider quota observation came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderQuotaSource {
    /// The provider's account usage endpoint.
    AccountStatus,
    /// HTTP response headers returned with a model call.
    ResponseHeaders,
    /// A provider-specific server-sent event.
    StreamEvent,
    /// Headers or payload returned with a rejected request.
    ErrorResponse,
}

/// Operational severity derived from provider-reported consumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaAlertLevel {
    /// Below 70 percent.
    Normal,
    /// At least 70 percent.
    Warning,
    /// At least 90 percent.
    Critical,
    /// At least 100 percent or explicitly reported as reached.
    Exhausted,
}

impl fmt::Display for QuotaAlertLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Normal => "normal",
            Self::Warning => "warning",
            Self::Critical => "critical",
            Self::Exhausted => "exhausted",
        })
    }
}

/// One provider-managed rolling quota window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderQuotaWindow {
    /// Percentage consumed, as reported by the provider.
    pub used_percent: f64,
    /// Window duration reported by the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
    /// Provider-computed delay until reset, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_after_seconds: Option<u64>,
    /// Absolute provider reset timestamp, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Utc>>,
}

/// Optional credits returned by the subscription provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCreditsSnapshot {
    /// Whether the account currently has credits.
    pub has_credits: bool,
    /// Whether the credit balance is unlimited.
    pub unlimited: bool,
    /// Provider-formatted balance; kept as text to avoid rounding money.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
}

/// Complete observation for one provider-managed limit family.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderQuotaSnapshot {
    /// Stable Captain provider id, for example `codex`.
    pub provider: String,
    /// Server-provided metered limit id, normalized for storage.
    pub limit_id: String,
    /// Human-readable server-provided limit name, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_name: Option<String>,
    /// Shorter provider window, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary: Option<ProviderQuotaWindow>,
    /// Longer provider window, often weekly, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<ProviderQuotaWindow>,
    /// Account credits, when the provider exposes them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<ProviderCreditsSnapshot>,
    /// Provider subscription plan label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    /// Provider reason identifying which allowance was reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_reached_type: Option<String>,
    /// Observation source.
    pub source: ProviderQuotaSource,
    /// Time Captain observed this provider response.
    pub observed_at: DateTime<Utc>,
}

impl ProviderQuotaSnapshot {
    /// Highest operational severity across this limit's windows.
    pub fn alert_level(&self) -> QuotaAlertLevel {
        if self.rate_limit_reached_type.is_some() {
            return QuotaAlertLevel::Exhausted;
        }
        let used = self
            .primary
            .iter()
            .chain(self.secondary.iter())
            .map(|window| window.used_percent)
            .fold(0.0_f64, f64::max);
        if used >= 100.0 {
            QuotaAlertLevel::Exhausted
        } else if used >= 90.0 {
            QuotaAlertLevel::Critical
        } else if used >= 70.0 {
            QuotaAlertLevel::Warning
        } else {
            QuotaAlertLevel::Normal
        }
    }

    /// Window currently responsible for an exhausted status.
    pub fn blocking_window(&self) -> Option<&ProviderQuotaWindow> {
        self.primary
            .iter()
            .chain(self.secondary.iter())
            .filter(|window| window.used_percent >= 100.0)
            .min_by_key(|window| window.resets_at)
            .or_else(|| self.primary.as_ref().or(self.secondary.as_ref()))
    }
}

/// Actionable quota failure carried unchanged across process boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuotaExceededInfo {
    /// Stable error code for clients.
    pub code: String,
    /// Which guard rejected the operation.
    pub scope: QuotaScope,
    /// Provider owning the allowance, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Agent affected by the guard, when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Observed usage.
    pub used: f64,
    /// Configured or provider-reported limit.
    pub limit: f64,
    /// Unit shared by `used` and `limit`.
    pub unit: QuotaUnit,
    /// Window duration when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
    /// Earliest known instant at which the operation may be retried.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Utc>>,
    /// Retry delay computed when the error was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u64>,
    /// Human-readable, actionable explanation.
    pub message: String,
}

impl QuotaExceededInfo {
    /// Build Captain's durable rolling hourly token guard failure.
    pub fn agent_hourly_tokens(
        agent_id: impl Into<String>,
        used: u64,
        limit: u64,
        resets_at: Option<DateTime<Utc>>,
    ) -> Self {
        let agent_id = agent_id.into();
        let retry_after_seconds = resets_at.map(seconds_until);
        let reset_hint = resets_at
            .map(|timestamp| format!(" Reprise estimée à {}.", timestamp.to_rfc3339()))
            .unwrap_or_default();
        Self {
            code: "captain_agent_hourly_token_quota".to_string(),
            scope: QuotaScope::AgentHourlyTokens,
            provider: None,
            agent_id: Some(agent_id.clone()),
            used: used as f64,
            limit: limit as f64,
            unit: QuotaUnit::Tokens,
            window_seconds: Some(3_600),
            resets_at,
            retry_after_seconds,
            message: format!(
                "Quota horaire Captain atteint pour l'agent {agent_id}: {used} / {limit} tokens.{reset_hint} Ce plafond interne est distinct du quota d'abonnement du provider."
            ),
        }
    }

    /// Build one of Captain's cost guard failures.
    pub fn cost(scope: QuotaScope, agent_id: Option<String>, used: f64, limit: f64) -> Self {
        let (code, label, window_seconds) = match scope {
            QuotaScope::AgentHourlyCost => (
                "captain_agent_hourly_cost_quota",
                "quota horaire de coût de l'agent",
                Some(3_600),
            ),
            QuotaScope::AgentDailyCost => (
                "captain_agent_daily_cost_quota",
                "quota journalier de coût de l'agent",
                Some(86_400),
            ),
            QuotaScope::AgentMonthlyCost => (
                "captain_agent_monthly_cost_quota",
                "quota mensuel de coût de l'agent",
                None,
            ),
            QuotaScope::GlobalHourlyCost => (
                "captain_global_hourly_cost_quota",
                "budget horaire global",
                Some(3_600),
            ),
            QuotaScope::GlobalDailyCost => (
                "captain_global_daily_cost_quota",
                "budget journalier global",
                Some(86_400),
            ),
            QuotaScope::GlobalMonthlyCost => (
                "captain_global_monthly_cost_quota",
                "budget mensuel global",
                None,
            ),
            _ => ("captain_cost_quota", "quota de coût Captain", None),
        };
        Self {
            code: code.to_string(),
            scope,
            provider: None,
            agent_id,
            used,
            limit,
            unit: QuotaUnit::Usd,
            window_seconds,
            resets_at: None,
            retry_after_seconds: None,
            message: format!("{label} atteint: ${used:.4} / ${limit:.4}."),
        }
    }

    /// Build a provider subscription failure from provider-reported data.
    pub fn provider_subscription(snapshot: &ProviderQuotaSnapshot) -> Self {
        let window = snapshot.blocking_window();
        let used = window.map(|value| value.used_percent).unwrap_or(100.0);
        let resets_at = window.and_then(|value| value.resets_at);
        let retry_after_seconds = resets_at
            .map(seconds_until)
            .or_else(|| window.and_then(|value| value.reset_after_seconds));
        let window_hint = window
            .and_then(|value| value.window_seconds)
            .map(human_window)
            .map(|duration| format!(" sur la fenêtre {duration}"))
            .unwrap_or_default();
        let reset_hint = resets_at
            .map(|timestamp| {
                format!(
                    " Reprise annoncée par le provider à {}.",
                    timestamp.to_rfc3339()
                )
            })
            .unwrap_or_default();
        let name = snapshot
            .limit_name
            .as_deref()
            .unwrap_or(snapshot.limit_id.as_str());
        Self {
            code: "provider_subscription_quota".to_string(),
            scope: QuotaScope::ProviderSubscription,
            provider: Some(snapshot.provider.clone()),
            agent_id: None,
            used,
            limit: 100.0,
            unit: QuotaUnit::Percent,
            window_seconds: window.and_then(|value| value.window_seconds),
            resets_at,
            retry_after_seconds,
            message: format!(
                "Quota d'abonnement {} atteint pour {name}{window_hint}: {used:.1} %.{reset_hint} Captain rapporte la limite fournie par le provider et ne la réinitialise pas.",
                snapshot.provider
            ),
        }
    }
}

impl fmt::Display for QuotaExceededInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn seconds_until(timestamp: DateTime<Utc>) -> u64 {
    (timestamp - Utc::now()).num_seconds().max(0) as u64
}

fn human_window(seconds: u64) -> String {
    if seconds % 604_800 == 0 {
        format!("{} sem.", seconds / 604_800)
    } else if seconds % 86_400 == 0 {
        format!("{} j", seconds / 86_400)
    } else if seconds % 3_600 == 0 {
        format!("{} h", seconds / 3_600)
    } else if seconds % 60 == 0 {
        format!("{} min", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn hourly_token_error_is_machine_readable_and_actionable() {
        let reset = Utc::now() + Duration::minutes(20);
        let info = QuotaExceededInfo::agent_hourly_tokens("agent-1", 228_733, 200_000, Some(reset));

        assert_eq!(info.code, "captain_agent_hourly_token_quota");
        assert_eq!(info.scope, QuotaScope::AgentHourlyTokens);
        assert_eq!(info.window_seconds, Some(3_600));
        assert!(info
            .retry_after_seconds
            .is_some_and(|seconds| seconds <= 1_200));
        assert!(info.message.contains("distinct du quota d'abonnement"));
    }

    #[test]
    fn provider_error_uses_real_window_duration_and_reset() {
        let reset = Utc::now() + Duration::hours(5);
        let snapshot = ProviderQuotaSnapshot {
            provider: "codex".to_string(),
            limit_id: "codex".to_string(),
            limit_name: Some("Codex".to_string()),
            primary: Some(ProviderQuotaWindow {
                used_percent: 100.0,
                window_seconds: Some(18_000),
                reset_after_seconds: None,
                resets_at: Some(reset),
            }),
            secondary: None,
            credits: None,
            plan_type: Some("plus".to_string()),
            rate_limit_reached_type: Some("primary".to_string()),
            source: ProviderQuotaSource::ResponseHeaders,
            observed_at: Utc::now(),
        };

        let info = QuotaExceededInfo::provider_subscription(&snapshot);
        assert_eq!(info.scope, QuotaScope::ProviderSubscription);
        assert!(info.message.contains("fenêtre 5 h"));
        assert!(info.message.contains(&reset.to_rfc3339()));
    }
}

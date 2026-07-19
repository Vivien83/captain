//! Shared TUI model for provider-reported subscription quotas.

use chrono::{DateTime, Utc};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProviderQuotaWindow {
    pub used_percent: f64,
    pub window_seconds: Option<u64>,
    pub reset_after_seconds: Option<u64>,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderCredits {
    pub has_credits: bool,
    pub unlimited: bool,
    pub balance: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProviderQuota {
    pub provider: String,
    pub limit_id: String,
    pub limit_name: String,
    pub plan_type: Option<String>,
    pub alert_level: String,
    pub stale: bool,
    pub primary: Option<ProviderQuotaWindow>,
    pub secondary: Option<ProviderQuotaWindow>,
    pub credits: Option<ProviderCredits>,
    pub rate_limit_reached_type: Option<String>,
    pub observed_at: Option<DateTime<Utc>>,
}

impl ProviderQuota {
    pub(crate) fn from_json(value: &serde_json::Value) -> Self {
        let limit_id = value["limit_id"].as_str().unwrap_or("quota").to_string();
        Self {
            provider: value["provider"].as_str().unwrap_or("provider").to_string(),
            limit_name: value["limit_name"]
                .as_str()
                .unwrap_or(&limit_id)
                .to_string(),
            limit_id,
            plan_type: value["plan_type"].as_str().map(str::to_string),
            alert_level: value["alert_level"]
                .as_str()
                .unwrap_or("normal")
                .to_string(),
            stale: value["stale"].as_bool().unwrap_or(false),
            primary: provider_window_from_json(&value["primary"]),
            secondary: provider_window_from_json(&value["secondary"]),
            credits: provider_credits_from_json(&value["credits"]),
            rate_limit_reached_type: value["rate_limit_reached_type"]
                .as_str()
                .map(str::to_string),
            observed_at: parse_timestamp(&value["observed_at"]),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderQuotaStatus {
    pub state: String,
    pub reported_by_provider: bool,
    pub quotas: Vec<ProviderQuota>,
}

impl Default for ProviderQuotaStatus {
    fn default() -> Self {
        Self {
            state: "unavailable".to_string(),
            reported_by_provider: false,
            quotas: Vec::new(),
        }
    }
}

impl ProviderQuotaStatus {
    pub(crate) fn from_budget_payload(value: &serde_json::Value) -> Self {
        Self::from_provider_payload(&value["provider_subscriptions"])
    }

    pub(crate) fn from_provider_payload(value: &serde_json::Value) -> Self {
        let quotas = value["items"]
            .as_array()
            .map(|items| items.iter().map(ProviderQuota::from_json).collect())
            .unwrap_or_default();
        Self {
            state: value["state"].as_str().unwrap_or("unavailable").to_string(),
            reported_by_provider: value["reported_by_provider"].as_bool().unwrap_or(false),
            quotas,
        }
    }

    pub(crate) fn has_observation(&self) -> bool {
        self.reported_by_provider && !self.quotas.is_empty()
    }
}

fn provider_window_from_json(value: &serde_json::Value) -> Option<ProviderQuotaWindow> {
    Some(ProviderQuotaWindow {
        used_percent: value["used_percent"].as_f64()?,
        window_seconds: value["window_seconds"].as_u64(),
        reset_after_seconds: value["reset_after_seconds"].as_u64(),
        resets_at: parse_timestamp(&value["resets_at"]),
    })
}

fn provider_credits_from_json(value: &serde_json::Value) -> Option<ProviderCredits> {
    value.as_object()?;
    Some(ProviderCredits {
        has_credits: value["has_credits"].as_bool().unwrap_or(false),
        unlimited: value["unlimited"].as_bool().unwrap_or(false),
        balance: value["balance"].as_str().map(str::to_string),
    })
}

fn parse_timestamp(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_provider_reported_quota_field() {
        let status = ProviderQuotaStatus::from_provider_payload(&serde_json::json!({
            "state": "warning",
            "reported_by_provider": true,
            "items": [{
                "provider": "codex",
                "limit_id": "codex_bengalfox",
                "limit_name": "GPT-5.3-Codex-Spark",
                "plan_type": "pro",
                "alert_level": "warning",
                "stale": false,
                "primary": {
                    "used_percent": 72.5,
                    "window_seconds": 604800,
                    "reset_after_seconds": 300,
                    "resets_at": "2026-07-19T18:00:00Z"
                },
                "credits": {
                    "has_credits": true,
                    "unlimited": false,
                    "balance": "17.50"
                },
                "rate_limit_reached_type": null,
                "observed_at": "2026-07-18T18:00:00Z"
            }]
        }));

        assert!(status.has_observation());
        assert_eq!(status.state, "warning");
        assert_eq!(status.quotas.len(), 1);
        let quota = &status.quotas[0];
        assert_eq!(quota.limit_id, "codex_bengalfox");
        assert_eq!(quota.plan_type.as_deref(), Some("pro"));
        assert_eq!(
            quota.primary.as_ref().unwrap().window_seconds,
            Some(604_800)
        );
        assert_eq!(
            quota.primary.as_ref().unwrap().reset_after_seconds,
            Some(300)
        );
        assert_eq!(
            quota.credits.as_ref().unwrap().balance.as_deref(),
            Some("17.50")
        );
        assert!(quota.observed_at.is_some());
    }

    #[test]
    fn missing_official_observation_stays_unavailable() {
        let status = ProviderQuotaStatus::from_provider_payload(&serde_json::json!({
            "state": "unavailable",
            "reported_by_provider": false,
            "items": []
        }));

        assert!(!status.has_observation());
        assert_eq!(status, ProviderQuotaStatus::default());
    }
}

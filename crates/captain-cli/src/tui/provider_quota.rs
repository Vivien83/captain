//! Shared TUI model for provider-reported subscription quotas.

use chrono::{DateTime, Utc};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProviderQuotaWindow {
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub remaining_source: Option<String>,
    pub window_seconds: Option<u64>,
    pub reset_after_seconds: Option<u64>,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderSpendControlLimit {
    pub source: Option<String>,
    pub limit: String,
    pub used: String,
    pub remaining: String,
    pub used_percent: i32,
    pub remaining_percent: i32,
    pub reset_after_seconds: u64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderSpendControl {
    pub reached: bool,
    pub individual_limit: Option<ProviderSpendControlLimit>,
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
    pub spend_control: Option<ProviderSpendControl>,
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
            spend_control: provider_spend_control_from_json(&value["spend_control"]),
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
    let used_percent = value["used_percent"].as_f64()?;
    Some(ProviderQuotaWindow {
        used_percent,
        remaining_percent: value["remaining_percent"]
            .as_f64()
            .unwrap_or_else(|| (100.0 - used_percent).clamp(0.0, 100.0)),
        remaining_source: value["remaining_source"].as_str().map(str::to_string),
        window_seconds: value["window_seconds"].as_u64(),
        reset_after_seconds: value["reset_after_seconds"].as_u64(),
        resets_at: parse_timestamp(&value["resets_at"]),
    })
}

fn provider_spend_control_from_json(value: &serde_json::Value) -> Option<ProviderSpendControl> {
    let object = value.as_object()?;
    let individual = object
        .get("individual_limit")
        .and_then(serde_json::Value::as_object)
        .map(|limit| ProviderSpendControlLimit {
            source: limit
                .get("source")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            limit: limit
                .get("limit")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            used: limit
                .get("used")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            remaining: limit
                .get("remaining")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            used_percent: limit
                .get("used_percent")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default() as i32,
            remaining_percent: limit
                .get("remaining_percent")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or_default() as i32,
            reset_after_seconds: limit
                .get("reset_after_seconds")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            resets_at: limit.get("resets_at").and_then(parse_timestamp),
        });
    Some(ProviderSpendControl {
        reached: object
            .get("reached")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        individual_limit: individual,
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
                    "remaining_percent": 27.5,
                    "remaining_source": "derived_from_provider_used_percent",
                    "window_seconds": 604800,
                    "reset_after_seconds": 300,
                    "resets_at": "2026-07-19T18:00:00Z"
                },
                "credits": {
                    "has_credits": true,
                    "unlimited": false,
                    "balance": "17.50"
                },
                "spend_control": {
                    "reached": false,
                    "individual_limit": {
                        "source": "monthly",
                        "limit": "200.00",
                        "used": "56.00",
                        "remaining": "144.00",
                        "used_percent": 28,
                        "remaining_percent": 72,
                        "reset_after_seconds": 86400,
                        "resets_at": "2026-08-01T00:00:00Z"
                    }
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
        assert_eq!(quota.primary.as_ref().unwrap().remaining_percent, 27.5);
        assert_eq!(
            quota.credits.as_ref().unwrap().balance.as_deref(),
            Some("17.50")
        );
        let spend = quota
            .spend_control
            .as_ref()
            .and_then(|control| control.individual_limit.as_ref())
            .unwrap();
        assert_eq!(spend.remaining, "144.00");
        assert_eq!(spend.remaining_percent, 72);
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

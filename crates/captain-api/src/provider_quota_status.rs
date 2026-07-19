//! Public-safe presentation of provider-reported subscription quotas.

use captain_memory::provider_quota::ProviderQuotaStore;
use captain_types::quota::{ProviderQuotaSnapshot, QuotaAlertLevel};
use chrono::{DateTime, Utc};

const STALE_AFTER_SECONDS: i64 = 15 * 60;

/// Build a stable status object without ever inferring provider allowances.
pub fn build_provider_subscription_status(store: &ProviderQuotaStore) -> serde_json::Value {
    match store.list_current() {
        Ok(snapshots) => provider_subscription_status_from_snapshots(&snapshots, Utc::now()),
        Err(error) => {
            tracing::warn!(error = %error, "Provider subscription quota status unavailable");
            unavailable_status("storage_unavailable")
        }
    }
}

fn provider_subscription_status_from_snapshots(
    snapshots: &[ProviderQuotaSnapshot],
    now: DateTime<Utc>,
) -> serde_json::Value {
    if snapshots.is_empty() {
        return unavailable_status("not_observed");
    }

    let mut items = Vec::with_capacity(snapshots.len());
    let mut highest_alert = QuotaAlertLevel::Normal;
    let mut any_stale = false;
    let mut newest_observation: Option<DateTime<Utc>> = None;
    for snapshot in snapshots {
        let age_seconds = now
            .signed_duration_since(snapshot.observed_at)
            .num_seconds()
            .max(0);
        let stale = age_seconds > STALE_AFTER_SECONDS;
        let alert = snapshot.alert_level();
        highest_alert = highest_alert.max(alert);
        any_stale |= stale;
        newest_observation = Some(
            newest_observation
                .map(|current| current.max(snapshot.observed_at))
                .unwrap_or(snapshot.observed_at),
        );

        let mut item = serde_json::to_value(snapshot).unwrap_or_default();
        if let Some(object) = item.as_object_mut() {
            object.insert("alert_level".to_string(), serde_json::json!(alert));
            object.insert("age_seconds".to_string(), serde_json::json!(age_seconds));
            object.insert("stale".to_string(), serde_json::json!(stale));
        }
        items.push(item);
    }

    let state = match highest_alert {
        QuotaAlertLevel::Exhausted => "exhausted",
        QuotaAlertLevel::Critical => "critical",
        QuotaAlertLevel::Warning => "warning",
        QuotaAlertLevel::Normal if any_stale => "stale",
        QuotaAlertLevel::Normal => "ok",
    };
    serde_json::json!({
        "state": state,
        "reported_by_provider": true,
        "contract": "official_provider_signals",
        "observed_count": items.len(),
        "newest_observed_at": newest_observation,
        "stale_after_seconds": STALE_AFTER_SECONDS,
        "items": items,
    })
}

fn unavailable_status(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "state": "unavailable",
        "reason": reason,
        "reported_by_provider": false,
        "contract": "official_provider_signals",
        "observed_count": 0,
        "newest_observed_at": null,
        "stale_after_seconds": STALE_AFTER_SECONDS,
        "items": [],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::quota::{ProviderQuotaSource, ProviderQuotaWindow};
    use chrono::Duration;

    fn snapshot(used_percent: f64, observed_at: DateTime<Utc>) -> ProviderQuotaSnapshot {
        ProviderQuotaSnapshot {
            provider: "codex".to_string(),
            limit_id: "codex".to_string(),
            limit_name: Some("Codex".to_string()),
            primary: Some(ProviderQuotaWindow {
                used_percent,
                window_seconds: Some(18_000),
                reset_after_seconds: Some(300),
                resets_at: Some(observed_at + Duration::minutes(5)),
            }),
            secondary: None,
            credits: None,
            plan_type: Some("plus".to_string()),
            rate_limit_reached_type: None,
            source: ProviderQuotaSource::AccountStatus,
            observed_at,
        }
    }

    #[test]
    fn empty_status_is_unknown_not_unlimited() {
        let status = provider_subscription_status_from_snapshots(&[], Utc::now());
        assert_eq!(status["state"], "unavailable");
        assert_eq!(status["reason"], "not_observed");
        assert_eq!(status["reported_by_provider"], false);
    }

    #[test]
    fn status_marks_exhausted_and_stale_provider_observations() {
        let now = Utc::now();
        let status = provider_subscription_status_from_snapshots(
            &[
                snapshot(100.0, now),
                snapshot(50.0, now - Duration::minutes(20)),
            ],
            now,
        );
        assert_eq!(status["state"], "exhausted");
        assert_eq!(status["items"][0]["alert_level"], "exhausted");
        assert_eq!(status["items"][1]["stale"], true);
    }
}

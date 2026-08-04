//! Public-safe presentation of provider-reported subscription quotas.

use captain_memory::provider_quota::ProviderQuotaStore;
use captain_memory::provider_quota_reset::ProviderQuotaResetQueueStatus;
use captain_types::quota::{ProviderQuotaSnapshot, QuotaAlertLevel};
use chrono::{DateTime, Utc};

const STALE_AFTER_SECONDS: i64 = 15 * 60;

/// Build a stable status object without ever inferring provider allowances.
pub fn build_provider_subscription_status(store: &ProviderQuotaStore) -> serde_json::Value {
    let mut status = match store.list_current() {
        Ok(snapshots) => provider_subscription_status_from_snapshots(&snapshots, Utc::now()),
        Err(error) => {
            tracing::warn!(error = %error, "Provider subscription quota status unavailable");
            unavailable_status("storage_unavailable")
        }
    };
    let notification_status = match store.reset_notification_queue_status() {
        Ok(queue) => reset_notification_status(&queue),
        Err(error) => {
            tracing::warn!(error = %error, "Provider quota reset notification status unavailable");
            serde_json::json!({
                "state": "unavailable",
                "reason": "storage_unavailable",
            })
        }
    };
    if let Some(object) = status.as_object_mut() {
        object.insert("reset_notifications".to_string(), notification_status);
    }
    status
}

fn reset_notification_status(queue: &ProviderQuotaResetQueueStatus) -> serde_json::Value {
    let state = if queue.requires_attention() {
        "attention"
    } else if queue.pending + queue.delivering + queue.retry_wait > 0 {
        "active"
    } else {
        "ok"
    };
    let mut value = serde_json::to_value(queue).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        object.insert("state".to_string(), serde_json::json!(state));
    }
    value
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
            add_remaining_projection(object, "primary");
            add_remaining_projection(object, "secondary");
            if let Some(limit) = object
                .get_mut("spend_control")
                .and_then(serde_json::Value::as_object_mut)
                .and_then(|control| control.get_mut("individual_limit"))
                .and_then(serde_json::Value::as_object_mut)
            {
                limit.insert(
                    "remaining_source".to_string(),
                    serde_json::json!("provider_reported"),
                );
            }
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

fn add_remaining_projection(
    object: &mut serde_json::Map<String, serde_json::Value>,
    window_name: &str,
) {
    let Some(window) = object
        .get_mut(window_name)
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let Some(used) = window
        .get("used_percent")
        .and_then(serde_json::Value::as_f64)
    else {
        return;
    };
    window.insert(
        "remaining_percent".to_string(),
        serde_json::json!((100.0 - used).clamp(0.0, 100.0)),
    );
    window.insert(
        "remaining_source".to_string(),
        serde_json::json!("derived_from_provider_used_percent"),
    );
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
            spend_control: None,
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

    #[test]
    fn status_exposes_remaining_capacity_without_relabeling_provider_data() {
        let now = Utc::now();
        let status = provider_subscription_status_from_snapshots(&[snapshot(19.0, now)], now);

        assert_eq!(status["items"][0]["primary"]["used_percent"], 19.0);
        assert_eq!(status["items"][0]["primary"]["remaining_percent"], 81.0);
        assert_eq!(
            status["items"][0]["primary"]["remaining_source"],
            "derived_from_provider_used_percent"
        );
    }

    #[test]
    fn reset_notification_status_surfaces_uncertain_delivery() {
        let queue = ProviderQuotaResetQueueStatus {
            pending: 1,
            uncertain: 1,
            ..ProviderQuotaResetQueueStatus::default()
        };

        let status = reset_notification_status(&queue);
        assert_eq!(status["state"], "attention");
        assert_eq!(status["pending"], 1);
        assert_eq!(status["uncertain"], 1);
    }
}

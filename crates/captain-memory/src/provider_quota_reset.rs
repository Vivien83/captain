//! Confirmed provider-quota resets and their crash-safe notification outbox.

use crate::provider_quota::ProviderQuotaStore;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::quota::{ProviderQuotaSnapshot, ProviderQuotaSource, ProviderQuotaWindow};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

const RESET_CLOCK_TOLERANCE: Duration = Duration::minutes(10);
const MIN_USAGE_DROP_PERCENT: f64 = 0.5;
const STRONG_USAGE_DROP_PERCENT: f64 = 5.0;
const DEFAULT_MAX_ATTEMPTS: u32 = 24;
const RETRY_BASE_MS: i64 = 15_000;
const RETRY_MAX_MS: i64 = 60 * 60 * 1_000;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderQuotaResetWindowKind {
    Primary,
    Secondary,
    SpendControl,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderQuotaResetWindow {
    pub kind: ProviderQuotaResetWindowKind,
    pub previous_used_percent: f64,
    pub current_used_percent: f64,
    pub previous_resets_at: DateTime<Utc>,
    pub current_resets_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderQuotaResetNotification {
    pub id: String,
    pub provider: String,
    pub limit_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
    pub source: ProviderQuotaSource,
    pub observed_at: DateTime<Utc>,
    pub windows: Vec<ProviderQuotaResetWindow>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaimedProviderQuotaResetNotification {
    pub notification: ProviderQuotaResetNotification,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub lease_owner: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProviderQuotaResetQueueStatus {
    pub pending: usize,
    pub delivering: usize,
    pub retry_wait: usize,
    pub delivered: usize,
    pub suppressed: usize,
    pub dead: usize,
    pub uncertain: usize,
}

impl ProviderQuotaResetQueueStatus {
    pub fn requires_attention(&self) -> bool {
        self.dead > 0 || self.uncertain > 0
    }
}

pub(crate) fn confirmed_reset_notification(
    previous: Option<&ProviderQuotaSnapshot>,
    current: &ProviderQuotaSnapshot,
) -> Option<ProviderQuotaResetNotification> {
    let previous = previous?;
    let mut windows = Vec::new();
    if let Some(reset) = confirmed_window_reset(
        ProviderQuotaResetWindowKind::Primary,
        previous.primary.as_ref(),
        current.primary.as_ref(),
        current.observed_at,
    ) {
        windows.push(reset);
    }
    if let Some(reset) = confirmed_window_reset(
        ProviderQuotaResetWindowKind::Secondary,
        previous.secondary.as_ref(),
        current.secondary.as_ref(),
        current.observed_at,
    ) {
        windows.push(reset);
    }
    if let Some(reset) = confirmed_spend_control_reset(previous, current) {
        windows.push(reset);
    }
    if windows.is_empty() {
        return None;
    }

    Some(ProviderQuotaResetNotification {
        id: uuid::Uuid::new_v4().to_string(),
        provider: current.provider.clone(),
        limit_id: current.limit_id.clone(),
        limit_name: current
            .limit_name
            .clone()
            .or_else(|| previous.limit_name.clone()),
        plan_type: current
            .plan_type
            .clone()
            .or_else(|| previous.plan_type.clone()),
        source: current.source,
        observed_at: current.observed_at,
        windows,
    })
}

fn confirmed_window_reset(
    kind: ProviderQuotaResetWindowKind,
    previous: Option<&ProviderQuotaWindow>,
    current: Option<&ProviderQuotaWindow>,
    observed_at: DateTime<Utc>,
) -> Option<ProviderQuotaResetWindow> {
    let previous = previous?;
    let current = current?;
    let previous_resets_at = previous.resets_at?;
    let current_resets_at = current.resets_at?;
    if !reset_identity_advanced(
        previous_resets_at,
        current_resets_at,
        current.window_seconds.or(previous.window_seconds),
    ) {
        return None;
    }
    let usage_drop = previous.used_percent - current.used_percent;
    let old_window_due = previous_resets_at <= observed_at + RESET_CLOCK_TOLERANCE;
    if usage_drop < MIN_USAGE_DROP_PERCENT
        || (!old_window_due && usage_drop < STRONG_USAGE_DROP_PERCENT)
    {
        return None;
    }
    Some(ProviderQuotaResetWindow {
        kind,
        previous_used_percent: previous.used_percent,
        current_used_percent: current.used_percent,
        previous_resets_at,
        current_resets_at,
        window_seconds: current.window_seconds.or(previous.window_seconds),
    })
}

fn confirmed_spend_control_reset(
    previous_snapshot: &ProviderQuotaSnapshot,
    current_snapshot: &ProviderQuotaSnapshot,
) -> Option<ProviderQuotaResetWindow> {
    let previous = previous_snapshot
        .spend_control
        .as_ref()?
        .individual_limit
        .as_ref()?;
    let current = current_snapshot
        .spend_control
        .as_ref()?
        .individual_limit
        .as_ref()?;
    let previous_resets_at = previous.resets_at?;
    let current_resets_at = current.resets_at?;
    if !reset_identity_advanced(previous_resets_at, current_resets_at, None) {
        return None;
    }
    let previous_used_percent = f64::from(previous.used_percent);
    let current_used_percent = f64::from(current.used_percent);
    let usage_drop = previous_used_percent - current_used_percent;
    let old_window_due = previous_resets_at <= current_snapshot.observed_at + RESET_CLOCK_TOLERANCE;
    if usage_drop < MIN_USAGE_DROP_PERCENT
        || (!old_window_due && usage_drop < STRONG_USAGE_DROP_PERCENT)
    {
        return None;
    }
    Some(ProviderQuotaResetWindow {
        kind: ProviderQuotaResetWindowKind::SpendControl,
        previous_used_percent,
        current_used_percent,
        previous_resets_at,
        current_resets_at,
        window_seconds: None,
    })
}

fn reset_identity_advanced(
    previous: DateTime<Utc>,
    current: DateTime<Utc>,
    window_seconds: Option<u64>,
) -> bool {
    let advance = current.signed_duration_since(previous).num_seconds();
    let minimum_advance = window_seconds
        .map(|seconds| (seconds / 2).max(60))
        .unwrap_or(60);
    advance >= i64::try_from(minimum_advance).unwrap_or(i64::MAX)
}

pub(crate) fn enqueue_reset_notification(
    transaction: &Transaction<'_>,
    notification: &ProviderQuotaResetNotification,
) -> CaptainResult<()> {
    let payload = serde_json::to_string(notification)
        .map_err(|error| CaptainError::Serialization(error.to_string()))?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(CaptainError::Serialization(
            "provider quota reset notification exceeds the durable payload limit".to_string(),
        ));
    }
    let now = notification.observed_at.timestamp_millis().max(0);
    transaction
        .execute(
            "INSERT INTO provider_quota_reset_outbox
                (id, provider, limit_id, payload_json, status, attempt_count,
                 max_attempts, run_after, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5, ?6, ?6, ?6)",
            rusqlite::params![
                notification.id,
                notification.provider,
                notification.limit_id,
                payload,
                DEFAULT_MAX_ATTEMPTS,
                now,
            ],
        )
        .map_err(|error| CaptainError::Memory(error.to_string()))?;
    Ok(())
}

impl ProviderQuotaStore {
    pub fn reset_notification_queue_status(&self) -> CaptainResult<ProviderQuotaResetQueueStatus> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        conn.query_row(
            "SELECT
                 COALESCE(SUM(status = 'pending'), 0),
                 COALESCE(SUM(status = 'delivering'), 0),
                 COALESCE(SUM(status = 'retry_wait'), 0),
                 COALESCE(SUM(status = 'delivered'), 0),
                 COALESCE(SUM(status = 'suppressed'), 0),
                 COALESCE(SUM(status = 'dead'), 0),
                 COALESCE(SUM(status = 'uncertain'), 0)
             FROM provider_quota_reset_outbox",
            [],
            |row| {
                Ok(ProviderQuotaResetQueueStatus {
                    pending: row.get::<_, i64>(0)?.max(0) as usize,
                    delivering: row.get::<_, i64>(1)?.max(0) as usize,
                    retry_wait: row.get::<_, i64>(2)?.max(0) as usize,
                    delivered: row.get::<_, i64>(3)?.max(0) as usize,
                    suppressed: row.get::<_, i64>(4)?.max(0) as usize,
                    dead: row.get::<_, i64>(5)?.max(0) as usize,
                    uncertain: row.get::<_, i64>(6)?.max(0) as usize,
                })
            },
        )
        .map_err(|error| CaptainError::Memory(error.to_string()))
    }

    pub fn claim_reset_notification(
        &self,
        lease_owner: &str,
        now_unix_ms: i64,
        lease_ms: i64,
    ) -> CaptainResult<Option<ClaimedProviderQuotaResetNotification>> {
        if lease_owner.is_empty() || lease_owner.len() > 96 || now_unix_ms < 0 || lease_ms <= 0 {
            return Err(CaptainError::InvalidInput(
                "invalid provider quota reset notification lease".to_string(),
            ));
        }
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let transaction = conn
            .transaction()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        transaction
            .execute(
                "UPDATE provider_quota_reset_outbox
                 SET status = 'uncertain', lease_owner = NULL, lease_expires_at = NULL,
                     last_error = 'delivery outcome unknown after worker interruption',
                     updated_at = ?1
                 WHERE status = 'delivering' AND lease_expires_at <= ?1",
                [now_unix_ms],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;

        let row: Option<(String, String, i64, i64)> = transaction
            .query_row(
                "SELECT id, payload_json, attempt_count, max_attempts
                 FROM provider_quota_reset_outbox
                 WHERE status IN ('pending', 'retry_wait') AND run_after <= ?1
                 ORDER BY run_after ASC, created_at ASC, id ASC
                 LIMIT 1",
                [now_unix_ms],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let Some((id, payload, attempt_count, max_attempts)) = row else {
            transaction
                .commit()
                .map_err(|error| CaptainError::Memory(error.to_string()))?;
            return Ok(None);
        };

        let notification = serde_json::from_str::<ProviderQuotaResetNotification>(&payload).ok();
        if notification
            .as_ref()
            .is_none_or(|notification| notification.id != id || notification.windows.is_empty())
        {
            transaction
                .execute(
                    "UPDATE provider_quota_reset_outbox
                     SET status = 'dead', last_error = 'invalid durable notification payload',
                         updated_at = ?2
                     WHERE id = ?1",
                    rusqlite::params![id, now_unix_ms],
                )
                .map_err(|error| CaptainError::Memory(error.to_string()))?;
            transaction
                .commit()
                .map_err(|error| CaptainError::Memory(error.to_string()))?;
            return Err(CaptainError::Serialization(
                "invalid durable provider quota reset notification".to_string(),
            ));
        }
        let notification = notification.expect("notification validated above");
        let next_attempt = attempt_count.saturating_add(1);
        let lease_expires_at = now_unix_ms.saturating_add(lease_ms);
        let changed = transaction
            .execute(
                "UPDATE provider_quota_reset_outbox
                 SET status = 'delivering', attempt_count = ?2, lease_owner = ?3,
                     lease_expires_at = ?4, updated_at = ?5
                 WHERE id = ?1 AND status IN ('pending', 'retry_wait')",
                rusqlite::params![id, next_attempt, lease_owner, lease_expires_at, now_unix_ms,],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        if changed != 1 {
            return Err(CaptainError::Memory(
                "provider quota reset notification claim lost".to_string(),
            ));
        }
        transaction
            .commit()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        Ok(Some(ClaimedProviderQuotaResetNotification {
            notification,
            attempt_count: u32::try_from(next_attempt).unwrap_or(u32::MAX),
            max_attempts: u32::try_from(max_attempts).unwrap_or(u32::MAX),
            lease_owner: lease_owner.to_string(),
        }))
    }

    pub fn complete_reset_notification(
        &self,
        claimed: &ClaimedProviderQuotaResetNotification,
        external_message_id: Option<&str>,
        now_unix_ms: i64,
    ) -> CaptainResult<()> {
        let external_message_id = external_message_id.map(|value| bounded(value, 256));
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let changed = conn
            .execute(
                "UPDATE provider_quota_reset_outbox
                 SET status = 'delivered', lease_owner = NULL, lease_expires_at = NULL,
                     external_message_id = ?3, delivered_at = ?4, last_error = NULL,
                     updated_at = ?4
                 WHERE id = ?1 AND status = 'delivering' AND lease_owner = ?2",
                rusqlite::params![
                    claimed.notification.id,
                    claimed.lease_owner,
                    external_message_id,
                    now_unix_ms,
                ],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        require_one_delivery_change(changed)
    }

    pub fn retry_reset_notification(
        &self,
        claimed: &ClaimedProviderQuotaResetNotification,
        error: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<()> {
        let exhausted = claimed.attempt_count >= claimed.max_attempts;
        let status = if exhausted { "dead" } else { "retry_wait" };
        let run_after = if exhausted {
            now_unix_ms
        } else {
            now_unix_ms.saturating_add(retry_delay_ms(claimed.attempt_count))
        };
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let changed = conn
            .execute(
                "UPDATE provider_quota_reset_outbox
                 SET status = ?3, run_after = ?4, lease_owner = NULL,
                     lease_expires_at = NULL, last_error = ?5, updated_at = ?6
                 WHERE id = ?1 AND status = 'delivering' AND lease_owner = ?2",
                rusqlite::params![
                    claimed.notification.id,
                    claimed.lease_owner,
                    status,
                    run_after,
                    bounded(error, 2_048),
                    now_unix_ms,
                ],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        require_one_delivery_change(changed)
    }

    pub fn suppress_pending_reset_notifications(
        &self,
        reason: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<usize> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let transaction = conn
            .transaction()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        transaction
            .execute(
                "UPDATE provider_quota_reset_outbox
                 SET status = 'uncertain', lease_owner = NULL, lease_expires_at = NULL,
                     last_error = 'delivery outcome unknown after worker interruption',
                     updated_at = MAX(?1, created_at)
                 WHERE status = 'delivering' AND lease_expires_at <= ?1",
                [now_unix_ms],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let suppressed = transaction
            .execute(
                "UPDATE provider_quota_reset_outbox
                 SET status = 'suppressed', last_error = ?1,
                     updated_at = MAX(?2, created_at)
                 WHERE status IN ('pending', 'retry_wait')",
                rusqlite::params![bounded(reason, 2_048), now_unix_ms],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        transaction
            .commit()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        Ok(suppressed)
    }
}

fn retry_delay_ms(attempt_count: u32) -> i64 {
    let shift = attempt_count.saturating_sub(1).min(20);
    RETRY_BASE_MS
        .saturating_mul(1_i64.checked_shl(shift).unwrap_or(i64::MAX))
        .min(RETRY_MAX_MS)
}

fn require_one_delivery_change(changed: usize) -> CaptainResult<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(CaptainError::Memory(
            "provider quota reset notification lease no longer matches".to_string(),
        ))
    }
}

fn bounded(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;
    use captain_types::quota::{ProviderQuotaSource, ProviderQuotaWindow};
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};

    fn store() -> ProviderQuotaStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        ProviderQuotaStore::new(Arc::new(Mutex::new(conn)))
    }

    fn snapshot(
        used: f64,
        reset: DateTime<Utc>,
        observed_at: DateTime<Utc>,
    ) -> ProviderQuotaSnapshot {
        ProviderQuotaSnapshot {
            provider: "codex".to_string(),
            limit_id: "codex".to_string(),
            limit_name: Some("Codex".to_string()),
            primary: Some(ProviderQuotaWindow {
                used_percent: used,
                window_seconds: Some(18_000),
                reset_after_seconds: None,
                resets_at: Some(reset),
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
    fn reset_requires_advanced_provider_identity_and_replenished_capacity() {
        let observed = Utc::now();
        let old_reset = observed - Duration::minutes(1);
        let next_reset = old_reset + Duration::hours(5);
        let previous = snapshot(96.0, old_reset, observed - Duration::minutes(5));

        assert!(confirmed_reset_notification(
            Some(&previous),
            &snapshot(97.0, next_reset, observed)
        )
        .is_none());
        assert!(
            confirmed_reset_notification(Some(&previous), &snapshot(2.0, old_reset, observed))
                .is_none()
        );

        let reset =
            confirmed_reset_notification(Some(&previous), &snapshot(2.0, next_reset, observed))
                .expect("provider reset should be confirmed");
        assert_eq!(reset.windows.len(), 1);
        assert_eq!(reset.windows[0].kind, ProviderQuotaResetWindowKind::Primary);
        assert_eq!(reset.windows[0].previous_used_percent, 96.0);
        assert_eq!(reset.windows[0].current_used_percent, 2.0);
    }

    #[test]
    fn small_reset_after_drift_does_not_create_a_reset() {
        let observed = Utc::now();
        let previous = snapshot(42.0, observed + Duration::hours(4), observed);
        let current = snapshot(
            41.0,
            observed + Duration::hours(4) + Duration::seconds(5),
            observed + Duration::seconds(5),
        );

        assert!(confirmed_reset_notification(Some(&previous), &current).is_none());
    }

    #[test]
    fn record_claim_complete_is_atomic_and_duplicate_safe() {
        let store = store();
        let observed = Utc::now();
        let old_reset = observed - Duration::minutes(1);
        let current = snapshot(3.0, old_reset + Duration::hours(5), observed);
        store
            .record(&snapshot(95.0, old_reset, observed - Duration::minutes(5)))
            .unwrap();
        let change = store.record(&current).unwrap();
        assert_eq!(change.confirmed_resets.len(), 1);
        store.record(&current).unwrap();
        assert_eq!(store.reset_notification_queue_status().unwrap().pending, 1);

        let now = observed.timestamp_millis();
        let claimed = store
            .claim_reset_notification("test-worker", now, 120_000)
            .unwrap()
            .expect("notification should be claimable");
        assert_eq!(claimed.attempt_count, 1);
        assert_eq!(
            store.reset_notification_queue_status().unwrap().delivering,
            1
        );
        store
            .complete_reset_notification(&claimed, Some("telegram-42"), now + 1_000)
            .unwrap();
        assert_eq!(
            store.reset_notification_queue_status().unwrap().delivered,
            1
        );
    }

    #[test]
    fn expired_delivery_becomes_uncertain_and_is_never_replayed() {
        let store = store();
        let observed = Utc::now();
        let old_reset = observed - Duration::minutes(1);
        store
            .record(&snapshot(90.0, old_reset, observed - Duration::minutes(5)))
            .unwrap();
        store
            .record(&snapshot(1.0, old_reset + Duration::hours(5), observed))
            .unwrap();
        store
            .claim_reset_notification("worker-before-crash", observed.timestamp_millis(), 100)
            .unwrap()
            .unwrap();

        assert!(store
            .claim_reset_notification("worker-after-crash", observed.timestamp_millis() + 101, 100,)
            .unwrap()
            .is_none());
        let status = store.reset_notification_queue_status().unwrap();
        assert_eq!(status.uncertain, 1);
        assert!(status.requires_attention());
    }

    #[test]
    fn failed_delivery_retries_only_after_the_durable_backoff() {
        let store = store();
        let observed = Utc::now();
        let old_reset = observed - Duration::minutes(1);
        store
            .record(&snapshot(90.0, old_reset, observed - Duration::minutes(5)))
            .unwrap();
        store
            .record(&snapshot(1.0, old_reset + Duration::hours(5), observed))
            .unwrap();

        let now = observed.timestamp_millis();
        let claimed = store
            .claim_reset_notification("first-worker", now, 120_000)
            .unwrap()
            .unwrap();
        store
            .retry_reset_notification(&claimed, "temporary Telegram outage", now + 1)
            .unwrap();
        let status = store.reset_notification_queue_status().unwrap();
        assert_eq!(status.delivering, 0);
        assert_eq!(status.retry_wait, 1);

        assert!(store
            .claim_reset_notification("early-worker", now + RETRY_BASE_MS, 120_000)
            .unwrap()
            .is_none());
        let retried = store
            .claim_reset_notification("retry-worker", now + RETRY_BASE_MS + 1, 120_000)
            .unwrap()
            .expect("notification should become claimable after the backoff");
        assert_eq!(retried.attempt_count, 2);
    }

    #[test]
    fn retry_budget_exhaustion_moves_the_delivery_to_dead() {
        let store = store();
        let observed = Utc::now();
        let old_reset = observed - Duration::minutes(1);
        store
            .record(&snapshot(90.0, old_reset, observed - Duration::minutes(5)))
            .unwrap();
        store
            .record(&snapshot(1.0, old_reset + Duration::hours(5), observed))
            .unwrap();
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE provider_quota_reset_outbox SET max_attempts = 1",
                [],
            )
            .unwrap();

        let now = observed.timestamp_millis();
        let claimed = store
            .claim_reset_notification("final-worker", now, 120_000)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.max_attempts, 1);
        store
            .retry_reset_notification(&claimed, "permanent delivery failure", now + 1)
            .unwrap();

        let status = store.reset_notification_queue_status().unwrap();
        assert_eq!(status.retry_wait, 0);
        assert_eq!(status.dead, 1);
        assert!(status.requires_attention());
    }

    #[test]
    fn silent_mode_suppresses_backlog_without_deleting_the_audit_row() {
        let store = store();
        let observed = Utc::now();
        let old_reset = observed - Duration::minutes(1);
        store
            .record(&snapshot(80.0, old_reset, observed - Duration::minutes(5)))
            .unwrap();
        store
            .record(&snapshot(0.0, old_reset + Duration::hours(5), observed))
            .unwrap();

        assert_eq!(
            store
                .suppress_pending_reset_notifications("silent_mode", observed.timestamp_millis(),)
                .unwrap(),
            1
        );
        let status = store.reset_notification_queue_status().unwrap();
        assert_eq!(status.pending, 0);
        assert_eq!(status.suppressed, 1);
    }

    #[test]
    fn silent_mode_reconciles_an_expired_in_flight_delivery() {
        let store = store();
        let observed = Utc::now();
        let old_reset = observed - Duration::minutes(1);
        store
            .record(&snapshot(80.0, old_reset, observed - Duration::minutes(5)))
            .unwrap();
        store
            .record(&snapshot(0.0, old_reset + Duration::hours(5), observed))
            .unwrap();
        store
            .claim_reset_notification("worker-before-crash", observed.timestamp_millis(), 100)
            .unwrap()
            .unwrap();

        assert_eq!(
            store
                .suppress_pending_reset_notifications(
                    "silent_mode",
                    observed.timestamp_millis() + 101,
                )
                .unwrap(),
            0
        );
        let status = store.reset_notification_queue_status().unwrap();
        assert_eq!(status.delivering, 0);
        assert_eq!(status.uncertain, 1);
        assert!(status.requires_attention());
    }
}

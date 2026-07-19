//! Durable provider subscription quota snapshots and transition history.

use captain_types::error::{CaptainError, CaptainResult};
use captain_types::quota::{ProviderQuotaSnapshot, QuotaAlertLevel};
use rusqlite::{Connection, OptionalExtension};
use std::sync::{Arc, Mutex};

/// Why a provider quota observation is worth announcing and journaling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderQuotaChange {
    pub first_seen: bool,
    pub alert_changed: bool,
    pub reset_changed: bool,
    pub metadata_changed: bool,
    pub previous_alert: Option<QuotaAlertLevel>,
    pub current_alert: QuotaAlertLevel,
}

impl ProviderQuotaChange {
    /// True when this observation should produce a structured log/event.
    pub fn should_announce(&self) -> bool {
        self.first_seen || self.alert_changed || self.reset_changed || self.metadata_changed
    }

    fn kind(&self) -> &'static str {
        if self.first_seen {
            "first_seen"
        } else if self.alert_changed {
            "alert_changed"
        } else if self.reset_changed {
            "reset_changed"
        } else {
            "metadata_changed"
        }
    }
}

/// SQLite-backed provider quota state.
#[derive(Clone)]
pub struct ProviderQuotaStore {
    conn: Arc<Mutex<Connection>>,
}

impl ProviderQuotaStore {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Persist the latest snapshot and journal meaningful transitions only.
    pub fn record(&self, snapshot: &ProviderQuotaSnapshot) -> CaptainResult<ProviderQuotaChange> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let previous_json: Option<String> = conn
            .query_row(
                "SELECT snapshot_json FROM provider_quota_snapshots
                 WHERE provider = ?1 AND limit_id = ?2",
                rusqlite::params![snapshot.provider, snapshot.limit_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let previous = previous_json
            .as_deref()
            .and_then(|json| serde_json::from_str::<ProviderQuotaSnapshot>(json).ok());
        let effective_snapshot = merge_static_metadata(snapshot, previous.as_ref());
        let current_alert = effective_snapshot.alert_level();
        let previous_alert = previous.as_ref().map(ProviderQuotaSnapshot::alert_level);
        let change = ProviderQuotaChange {
            first_seen: previous.is_none(),
            alert_changed: previous_alert.is_some_and(|level| level != current_alert),
            reset_changed: previous.as_ref().is_some_and(|value| {
                reset_fingerprint(value) != reset_fingerprint(&effective_snapshot)
            }),
            metadata_changed: previous.as_ref().is_some_and(|value| {
                value.limit_name != effective_snapshot.limit_name
                    || value.plan_type != effective_snapshot.plan_type
                    || value.credits != effective_snapshot.credits
                    || value.rate_limit_reached_type != effective_snapshot.rate_limit_reached_type
            }),
            previous_alert,
            current_alert,
        };
        let snapshot_json = serde_json::to_string(&effective_snapshot)
            .map_err(|error| CaptainError::Serialization(error.to_string()))?;
        let transaction = conn
            .transaction()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        transaction
            .execute(
                "INSERT INTO provider_quota_snapshots
                    (provider, limit_id, snapshot_json, alert_level, observed_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
                 ON CONFLICT(provider, limit_id) DO UPDATE SET
                    snapshot_json = excluded.snapshot_json,
                    alert_level = excluded.alert_level,
                    observed_at = excluded.observed_at,
                    updated_at = datetime('now')",
                rusqlite::params![
                    effective_snapshot.provider,
                    effective_snapshot.limit_id,
                    snapshot_json,
                    current_alert.to_string(),
                    effective_snapshot.observed_at.to_rfc3339(),
                ],
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        if change.should_announce() {
            transaction
                .execute(
                    "INSERT INTO provider_quota_events
                        (id, provider, limit_id, change_kind, alert_level, snapshot_json, observed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        uuid::Uuid::new_v4().to_string(),
                        effective_snapshot.provider,
                        effective_snapshot.limit_id,
                        change.kind(),
                        current_alert.to_string(),
                        snapshot_json,
                        effective_snapshot.observed_at.to_rfc3339(),
                    ],
                )
                .map_err(|error| CaptainError::Memory(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        Ok(change)
    }

    /// Return the latest provider observations, newest first.
    pub fn list_current(&self) -> CaptainResult<Vec<ProviderQuotaSnapshot>> {
        let conn = self
            .conn
            .lock()
            .map_err(|error| CaptainError::Internal(error.to_string()))?;
        let mut stmt = conn
            .prepare(
                "SELECT snapshot_json FROM provider_quota_snapshots
                 ORDER BY observed_at DESC, provider ASC, limit_id ASC",
            )
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| CaptainError::Memory(error.to_string()))?;
        let mut snapshots = Vec::new();
        for row in rows {
            let json = row.map_err(|error| CaptainError::Memory(error.to_string()))?;
            snapshots.push(
                serde_json::from_str(&json)
                    .map_err(|error| CaptainError::Serialization(error.to_string()))?,
            );
        }
        Ok(snapshots)
    }

    #[cfg(test)]
    fn event_count(&self) -> usize {
        self.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM provider_quota_events", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap() as usize
    }
}

fn merge_static_metadata(
    snapshot: &ProviderQuotaSnapshot,
    previous: Option<&ProviderQuotaSnapshot>,
) -> ProviderQuotaSnapshot {
    let mut effective = snapshot.clone();
    if snapshot.source == captain_types::quota::ProviderQuotaSource::AccountStatus {
        return effective;
    }
    let Some(previous) = previous else {
        return effective;
    };
    effective.limit_name = effective.limit_name.or_else(|| previous.limit_name.clone());
    effective.plan_type = effective.plan_type.or_else(|| previous.plan_type.clone());
    effective.credits = effective.credits.or_else(|| previous.credits.clone());
    effective
}

fn reset_fingerprint(snapshot: &ProviderQuotaSnapshot) -> (Option<i64>, Option<i64>) {
    (
        snapshot
            .primary
            .as_ref()
            .and_then(|window| window.resets_at)
            .map(|value| value.timestamp()),
        snapshot
            .secondary
            .as_ref()
            .and_then(|window| window.resets_at)
            .map(|value| value.timestamp()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;
    use captain_types::quota::{ProviderCreditsSnapshot, ProviderQuotaSource, ProviderQuotaWindow};
    use chrono::{Duration, Utc};

    fn store() -> ProviderQuotaStore {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        ProviderQuotaStore::new(Arc::new(Mutex::new(conn)))
    }

    fn snapshot(used_percent: f64, reset: chrono::DateTime<Utc>) -> ProviderQuotaSnapshot {
        ProviderQuotaSnapshot {
            provider: "codex".to_string(),
            limit_id: "codex".to_string(),
            limit_name: Some("Codex".to_string()),
            primary: Some(ProviderQuotaWindow {
                used_percent,
                window_seconds: Some(18_000),
                reset_after_seconds: None,
                resets_at: Some(reset),
            }),
            secondary: None,
            credits: None,
            plan_type: Some("plus".to_string()),
            rate_limit_reached_type: None,
            source: ProviderQuotaSource::AccountStatus,
            observed_at: Utc::now(),
        }
    }

    #[test]
    fn journals_transitions_without_spamming_usage_updates() {
        let store = store();
        let reset = Utc::now() + Duration::hours(5);
        assert!(store.record(&snapshot(10.0, reset)).unwrap().first_seen);
        assert_eq!(store.event_count(), 1);

        assert!(!store
            .record(&snapshot(20.0, reset))
            .unwrap()
            .should_announce());
        assert_eq!(store.event_count(), 1);

        let warning = store.record(&snapshot(75.0, reset)).unwrap();
        assert!(warning.alert_changed);
        assert_eq!(warning.current_alert, QuotaAlertLevel::Warning);
        assert_eq!(store.event_count(), 2);

        assert!(
            store
                .record(&snapshot(76.0, reset + Duration::hours(1)))
                .unwrap()
                .reset_changed
        );
        assert_eq!(store.event_count(), 3);
        assert_eq!(store.list_current().unwrap().len(), 1);
    }

    #[test]
    fn partial_live_observation_preserves_account_metadata() {
        let store = store();
        let reset = Utc::now() + Duration::hours(5);
        let mut account = snapshot(10.0, reset);
        account.credits = Some(ProviderCreditsSnapshot {
            has_credits: true,
            unlimited: false,
            balance: Some("4.2".to_string()),
        });
        store.record(&account).unwrap();

        let mut headers = snapshot(20.0, reset);
        headers.source = ProviderQuotaSource::ResponseHeaders;
        headers.limit_name = None;
        headers.plan_type = None;
        headers.credits = None;
        store.record(&headers).unwrap();

        let current = store.list_current().unwrap().pop().unwrap();
        assert_eq!(current.limit_name.as_deref(), Some("Codex"));
        assert_eq!(current.plan_type.as_deref(), Some("plus"));
        assert_eq!(current.credits.unwrap().balance.as_deref(), Some("4.2"));
        assert_eq!(current.source, ProviderQuotaSource::ResponseHeaders);
    }
}

//! Durable retry queue for per-agent API callbacks.

use crate::{
    agent_api_audit,
    agent_api_egress::{deliver_agent_api_callback, AgentApiCallbackDelivery},
};
use captain_runtime::audit::AuditLog;
use captain_types::agent::AgentId;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
    time::Duration,
};
use uuid::Uuid;

const QUEUE_FILE_NAME: &str = "agent_api_egress_queue.json";
const MAX_QUEUE_ENTRIES: usize = 50;
const MAX_DRAIN_BATCH: usize = 10;
const MAX_RETRY_ROUNDS: u8 = 3;
const LEASE_SECS: i64 = 300;

static QUEUE_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgentApiQueuedCallback {
    pub id: String,
    pub agent_id: AgentId,
    pub event: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub next_attempt_at: DateTime<Utc>,
    pub attempts: u8,
    pub max_attempts: u8,
    pub last_error: Option<String>,
    pub dead_letter: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentApiEgressQueueSummary {
    pub agent_id: String,
    pub pending: usize,
    pub due: usize,
    pub dead_letters: usize,
    pub items: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AgentApiEgressRetryResult {
    pub id: String,
    pub agent_id: String,
    pub event: String,
    pub status: &'static str,
    pub attempts: u8,
    pub delivered: bool,
    pub dead_letter: bool,
    pub outcome: String,
}

#[derive(Debug)]
pub(crate) enum AgentApiEgressRetryError {
    NotFound,
    Store(String),
}

pub(crate) async fn enqueue_agent_api_callback(
    home_dir: &Path,
    agent_id: &AgentId,
    payload: &serde_json::Value,
    last_error: Option<&str>,
) -> Result<String, String> {
    let _guard = QUEUE_LOCK.lock().await;
    let mut queue = load_queue(home_dir)?;
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    queue.push(AgentApiQueuedCallback {
        id: id.clone(),
        agent_id: *agent_id,
        event: callback_event(payload),
        payload: payload.clone(),
        created_at: now,
        next_attempt_at: now + retry_delay(0),
        attempts: 0,
        max_attempts: MAX_RETRY_ROUNDS,
        last_error: last_error.map(ToOwned::to_owned),
        dead_letter: false,
    });
    trim_queue(&mut queue);
    save_queue(home_dir, &queue)?;
    Ok(id)
}

pub(crate) async fn agent_api_egress_queue_summary(
    home_dir: &Path,
    agent_id: &AgentId,
) -> Result<AgentApiEgressQueueSummary, String> {
    let _guard = QUEUE_LOCK.lock().await;
    let queue = load_queue(home_dir)?;
    Ok(queue_summary(agent_id, &queue))
}

pub(crate) async fn agent_api_egress_queue_entries(
    home_dir: &Path,
) -> Result<Vec<AgentApiQueuedCallback>, String> {
    let _guard = QUEUE_LOCK.lock().await;
    load_queue(home_dir)
}

pub(crate) async fn retry_agent_api_callback_now(
    home_dir: &Path,
    audit_log: &AuditLog,
    agent_id: &AgentId,
    id: &str,
) -> Result<AgentApiEgressRetryResult, AgentApiEgressRetryError> {
    let entry = reserve_manual_retry(home_dir, agent_id, id).await?;
    let delivery = deliver_agent_api_callback(&entry.agent_id, &entry.payload).await;
    let delivered = delivery.delivered();
    let outcome = delivery.audit_outcome();
    let dead_letter = update_after_attempt(home_dir, &entry.id, delivery).await;
    let attempts = queued_attempts(home_dir, &entry.id).await.unwrap_or(0);
    let status = if delivered {
        "delivered"
    } else if dead_letter {
        "dead_letter"
    } else {
        "queued"
    };
    record_drain_audit(audit_log, &entry, &format!("manual_retry {outcome}"));
    Ok(AgentApiEgressRetryResult {
        id: entry.id,
        agent_id: entry.agent_id.to_string(),
        event: entry.event,
        status,
        attempts,
        delivered,
        dead_letter,
        outcome,
    })
}

pub(crate) fn spawn_agent_api_egress_queue_drain(home_dir: PathBuf, audit_log: Arc<AuditLog>) {
    tokio::spawn(async move {
        loop {
            let summary = drain_due_agent_api_callbacks(&home_dir, audit_log.as_ref()).await;
            if summary.failed > 0 {
                tracing::warn!(
                    drained = summary.drained,
                    delivered = summary.delivered,
                    failed = summary.failed,
                    dead_lettered = summary.dead_lettered,
                    "agent API callback queue drain completed with failures"
                );
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

#[derive(Debug, Default)]
struct DrainSummary {
    drained: usize,
    delivered: usize,
    failed: usize,
    dead_lettered: usize,
}

async fn drain_due_agent_api_callbacks(home_dir: &Path, audit_log: &AuditLog) -> DrainSummary {
    let due = match reserve_due_callbacks(home_dir).await {
        Ok(due) => due,
        Err(err) => {
            tracing::warn!(error = %err, "agent API callback queue reserve failed");
            return DrainSummary::default();
        }
    };

    let mut summary = DrainSummary {
        drained: due.len(),
        ..DrainSummary::default()
    };
    for entry in due {
        let delivery = deliver_agent_api_callback(&entry.agent_id, &entry.payload).await;
        let outcome = delivery.audit_outcome();
        if delivery.delivered() {
            summary.delivered += 1;
        } else {
            summary.failed += 1;
        }
        if update_after_attempt(home_dir, &entry.id, delivery).await {
            summary.dead_lettered += 1;
            record_drain_audit(audit_log, &entry, &format!("dead_letter {outcome}"));
        } else {
            record_drain_audit(audit_log, &entry, &format!("retry {outcome}"));
        }
    }
    summary
}

async fn reserve_due_callbacks(home_dir: &Path) -> Result<Vec<AgentApiQueuedCallback>, String> {
    let _guard = QUEUE_LOCK.lock().await;
    let mut queue = load_queue(home_dir)?;
    let now = Utc::now();
    let mut due = Vec::new();
    for entry in queue
        .iter_mut()
        .filter(|entry| !entry.dead_letter && entry.next_attempt_at <= now)
        .take(MAX_DRAIN_BATCH)
    {
        due.push(entry.clone());
        entry.next_attempt_at = now + ChronoDuration::seconds(LEASE_SECS);
    }
    if !due.is_empty() {
        save_queue(home_dir, &queue)?;
    }
    Ok(due)
}

async fn reserve_manual_retry(
    home_dir: &Path,
    agent_id: &AgentId,
    id: &str,
) -> Result<AgentApiQueuedCallback, AgentApiEgressRetryError> {
    let _guard = QUEUE_LOCK.lock().await;
    let mut queue = load_queue(home_dir).map_err(AgentApiEgressRetryError::Store)?;
    let now = Utc::now();
    let Some(entry) = queue
        .iter_mut()
        .find(|entry| entry.agent_id == *agent_id && entry.id == id)
    else {
        return Err(AgentApiEgressRetryError::NotFound);
    };
    entry.attempts = 0;
    entry.max_attempts = MAX_RETRY_ROUNDS;
    entry.dead_letter = false;
    entry.last_error = None;
    entry.next_attempt_at = now + ChronoDuration::seconds(LEASE_SECS);
    let reserved = entry.clone();
    save_queue(home_dir, &queue).map_err(AgentApiEgressRetryError::Store)?;
    Ok(reserved)
}

async fn update_after_attempt(
    home_dir: &Path,
    id: &str,
    delivery: AgentApiCallbackDelivery,
) -> bool {
    let _guard = QUEUE_LOCK.lock().await;
    let mut queue = match load_queue(home_dir) {
        Ok(queue) => queue,
        Err(err) => {
            tracing::warn!(error = %err, "agent API callback queue update failed");
            return false;
        }
    };
    let Some(idx) = queue.iter().position(|entry| entry.id == id) else {
        return false;
    };
    if delivery.delivered() {
        queue.remove(idx);
        let _ = save_queue(home_dir, &queue);
        return false;
    }

    let now = Utc::now();
    let entry = &mut queue[idx];
    entry.attempts = entry.attempts.saturating_add(1);
    entry.last_error = delivery
        .error_message()
        .map(ToOwned::to_owned)
        .or_else(|| Some("callback not delivered".to_string()));
    if entry.attempts >= entry.max_attempts || !delivery.should_queue() {
        entry.dead_letter = true;
        entry.next_attempt_at = now;
        let _ = save_queue(home_dir, &queue);
        return true;
    }
    entry.next_attempt_at = now + retry_delay(entry.attempts);
    let _ = save_queue(home_dir, &queue);
    false
}

async fn queued_attempts(home_dir: &Path, id: &str) -> Option<u8> {
    let _guard = QUEUE_LOCK.lock().await;
    load_queue(home_dir)
        .ok()
        .and_then(|queue| queue.into_iter().find(|entry| entry.id == id))
        .map(|entry| entry.attempts)
}

fn queue_summary(
    agent_id: &AgentId,
    queue: &[AgentApiQueuedCallback],
) -> AgentApiEgressQueueSummary {
    let now = Utc::now();
    let matching = queue
        .iter()
        .filter(|entry| entry.agent_id == *agent_id)
        .collect::<Vec<_>>();
    let pending = matching.iter().filter(|entry| !entry.dead_letter).count();
    let due = matching
        .iter()
        .filter(|entry| !entry.dead_letter && entry.next_attempt_at <= now)
        .count();
    let dead_letters = matching.iter().filter(|entry| entry.dead_letter).count();
    let items = matching
        .into_iter()
        .rev()
        .take(20)
        .map(|entry| {
            serde_json::json!({
                "id": entry.id,
                "event": entry.event,
                "created_at": entry.created_at,
                "next_attempt_at": entry.next_attempt_at,
                "attempts": entry.attempts,
                "max_attempts": entry.max_attempts,
                "last_error": entry.last_error,
                "dead_letter": entry.dead_letter,
            })
        })
        .collect();
    AgentApiEgressQueueSummary {
        agent_id: agent_id.to_string(),
        pending,
        due,
        dead_letters,
        items,
    }
}

fn load_queue(home_dir: &Path) -> Result<Vec<AgentApiQueuedCallback>, String> {
    let path = queue_path(home_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|err| format!("failed to read agent API egress queue: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("invalid agent API egress queue: {err}"))
}

fn save_queue(home_dir: &Path, queue: &[AgentApiQueuedCallback]) -> Result<(), String> {
    let path = queue_path(home_dir);
    if let Some(parent) = path.parent() {
        captain_types::durable_fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create agent API queue dir: {err}"))?;
        secure_dir(parent);
    }
    let raw = serde_json::to_vec_pretty(queue)
        .map_err(|err| format!("failed to encode agent API egress queue: {err}"))?;
    captain_types::durable_fs::atomic_write(&path, &raw)
        .map_err(|err| format!("failed to persist agent API egress queue: {err}"))?;
    secure_file(&path);
    Ok(())
}

fn trim_queue(queue: &mut Vec<AgentApiQueuedCallback>) {
    while queue.len() > MAX_QUEUE_ENTRIES {
        let idx = queue
            .iter()
            .position(|entry| entry.dead_letter)
            .unwrap_or(0);
        queue.remove(idx);
    }
}

fn queue_path(home_dir: &Path) -> PathBuf {
    home_dir.join(QUEUE_FILE_NAME)
}

fn callback_event(payload: &serde_json::Value) -> String {
    payload
        .get("event")
        .and_then(|value| value.as_str())
        .unwrap_or("agent_api.callback")
        .to_string()
}

fn record_drain_audit(audit_log: &AuditLog, entry: &AgentApiQueuedCallback, outcome: &str) {
    let request_id = agent_api_audit::request_id_from_payload(&entry.payload);
    agent_api_audit::record_egress_callback(
        audit_log,
        &entry.agent_id,
        request_id.as_deref(),
        &entry.event,
        outcome,
    );
}

fn retry_delay(attempts: u8) -> ChronoDuration {
    let secs = 60_i64.saturating_mul(2_i64.saturating_pow(attempts.min(4) as u32));
    ChronoDuration::seconds(secs)
}

fn secure_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn secure_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_id() -> AgentId {
        "01234567-89ab-cdef-0123-456789abcdef".parse().unwrap()
    }

    #[tokio::test]
    async fn enqueue_is_bounded_and_summarized() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = sample_agent_id();
        for idx in 0..(MAX_QUEUE_ENTRIES + 2) {
            let payload = serde_json::json!({"event": "agent_api.completed", "idx": idx});
            enqueue_agent_api_callback(tmp.path(), &agent_id, &payload, Some("HTTP 503"))
                .await
                .unwrap();
        }
        let queue = load_queue(tmp.path()).unwrap();
        assert_eq!(queue.len(), MAX_QUEUE_ENTRIES);
        let summary = agent_api_egress_queue_summary(tmp.path(), &agent_id)
            .await
            .unwrap();
        assert_eq!(summary.pending, MAX_QUEUE_ENTRIES);
        assert_eq!(summary.dead_letters, 0);
        assert_eq!(summary.items.len(), 20);
    }

    #[test]
    fn retry_delay_grows() {
        assert!(retry_delay(2) > retry_delay(1));
    }

    #[tokio::test]
    async fn manual_retry_missing_entry_reports_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AuditLog::new();
        let err = retry_agent_api_callback_now(tmp.path(), &log, &sample_agent_id(), "missing")
            .await
            .unwrap_err();
        assert!(matches!(err, AgentApiEgressRetryError::NotFound));
    }

    #[tokio::test]
    async fn manual_retry_resets_dead_letter_before_attempt() {
        let tmp = tempfile::tempdir().unwrap();
        let log = AuditLog::new();
        let agent_id = sample_agent_id();
        let payload = serde_json::json!({"event": "agent_api.completed", "request_id": "r1"});
        let id = enqueue_agent_api_callback(tmp.path(), &agent_id, &payload, Some("HTTP 503"))
            .await
            .unwrap();
        {
            let mut queue = load_queue(tmp.path()).unwrap();
            queue[0].attempts = queue[0].max_attempts;
            queue[0].dead_letter = true;
            save_queue(tmp.path(), &queue).unwrap();
        }

        let result = retry_agent_api_callback_now(tmp.path(), &log, &agent_id, &id)
            .await
            .unwrap();
        assert_eq!(result.status, "dead_letter");
        assert_eq!(result.attempts, 1);
        let queue = load_queue(tmp.path()).unwrap();
        assert_eq!(queue[0].attempts, 1);
        assert!(queue[0].dead_letter);
    }
}

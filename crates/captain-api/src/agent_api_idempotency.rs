//! Durable idempotency guard for per-agent API ingress retries.

use captain_types::agent::AgentId;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::LazyLock,
};

const STORE_FILE_NAME: &str = "agent_api_idempotency.json";
const MAX_IDEMPOTENCY_ENTRIES: usize = 500;
const MAX_REQUEST_ID_DISPLAY: usize = 180;
pub(crate) const AGENT_API_IDEMPOTENCY_TTL_SECS: i64 = 24 * 60 * 60;

static IDEMPOTENCY_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AgentApiIdempotencyEntry {
    pub agent_id: AgentId,
    pub request_id: String,
    request_key: String,
    fingerprint: String,
    pub first_seen_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub status: AgentApiIdempotencyStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentApiIdempotencyStatus {
    InProgress,
    Completed,
    Failed,
}

impl AgentApiIdempotencyStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug)]
pub(crate) enum AgentApiIdempotencyDecision {
    Fresh,
    Duplicate(AgentApiIdempotencyEntry),
    Conflict(AgentApiIdempotencyEntry),
}

pub(crate) fn request_fingerprint(body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body);
    hex::encode(hasher.finalize())
}

pub(crate) async fn reserve_agent_api_request(
    home_dir: &Path,
    agent_id: &AgentId,
    request_id: Option<&str>,
    fingerprint: &str,
) -> Result<AgentApiIdempotencyDecision, String> {
    let Some(request_id) = normalize_request_id(request_id) else {
        return Ok(AgentApiIdempotencyDecision::Fresh);
    };

    let _guard = IDEMPOTENCY_LOCK.lock().await;
    let mut entries = load_entries(home_dir)?;
    prune_expired(&mut entries);

    let request_key = request_key(&request_id);
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry.agent_id == *agent_id && entry.request_key == request_key)
        .cloned()
    {
        if entry.fingerprint == fingerprint {
            save_entries(home_dir, &entries)?;
            return Ok(AgentApiIdempotencyDecision::Duplicate(entry));
        }
        save_entries(home_dir, &entries)?;
        return Ok(AgentApiIdempotencyDecision::Conflict(entry));
    }

    let now = Utc::now();
    entries.push(AgentApiIdempotencyEntry {
        agent_id: *agent_id,
        request_id: display_request_id(&request_id),
        request_key,
        fingerprint: fingerprint.to_string(),
        first_seen_at: now,
        updated_at: now,
        status: AgentApiIdempotencyStatus::InProgress,
    });
    trim_entries(&mut entries);
    save_entries(home_dir, &entries)?;
    Ok(AgentApiIdempotencyDecision::Fresh)
}

pub(crate) async fn mark_agent_api_request_status(
    home_dir: &Path,
    agent_id: &AgentId,
    request_id: Option<&str>,
    status: AgentApiIdempotencyStatus,
) -> Result<(), String> {
    let Some(request_id) = normalize_request_id(request_id) else {
        return Ok(());
    };

    let _guard = IDEMPOTENCY_LOCK.lock().await;
    let mut entries = load_entries(home_dir)?;
    prune_expired(&mut entries);
    let request_key = request_key(&request_id);
    if let Some(entry) = entries
        .iter_mut()
        .find(|entry| entry.agent_id == *agent_id && entry.request_key == request_key)
    {
        entry.status = status;
        entry.updated_at = Utc::now();
    }
    save_entries(home_dir, &entries)
}

fn load_entries(home_dir: &Path) -> Result<Vec<AgentApiIdempotencyEntry>, String> {
    let path = store_path(home_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|err| format!("failed to read agent API idempotency store: {err}"))?;
    serde_json::from_str(&raw).map_err(|err| format!("invalid agent API idempotency store: {err}"))
}

fn save_entries(home_dir: &Path, entries: &[AgentApiIdempotencyEntry]) -> Result<(), String> {
    let path = store_path(home_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create agent API idempotency dir: {err}"))?;
        secure_dir(parent);
    }
    let raw = serde_json::to_vec_pretty(entries)
        .map_err(|err| format!("failed to encode agent API idempotency store: {err}"))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, raw)
        .map_err(|err| format!("failed to write agent API idempotency store: {err}"))?;
    std::fs::rename(&tmp, &path)
        .map_err(|err| format!("failed to commit agent API idempotency store: {err}"))?;
    secure_file(&path);
    Ok(())
}

fn prune_expired(entries: &mut Vec<AgentApiIdempotencyEntry>) {
    let cutoff = Utc::now() - ChronoDuration::seconds(AGENT_API_IDEMPOTENCY_TTL_SECS);
    entries.retain(|entry| entry.updated_at >= cutoff);
}

fn trim_entries(entries: &mut Vec<AgentApiIdempotencyEntry>) {
    entries.sort_by_key(|entry| entry.updated_at);
    while entries.len() > MAX_IDEMPOTENCY_ENTRIES {
        entries.remove(0);
    }
}

fn normalize_request_id(request_id: Option<&str>) -> Option<String> {
    request_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn request_key(request_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request_id.as_bytes());
    hex::encode(hasher.finalize())
}

fn display_request_id(request_id: &str) -> String {
    if request_id.len() <= MAX_REQUEST_ID_DISPLAY {
        return request_id.to_string();
    }
    let mut boundary = MAX_REQUEST_ID_DISPLAY;
    while !request_id.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...", &request_id[..boundary])
}

fn store_path(home_dir: &Path) -> PathBuf {
    home_dir.join(STORE_FILE_NAME)
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
    async fn repeated_request_id_is_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = sample_agent_id();
        let fingerprint = request_fingerprint(br#"{"request_id":"one","message":"hi"}"#);

        let first =
            reserve_agent_api_request(tmp.path(), &agent_id, Some("one"), &fingerprint).await;
        assert!(matches!(first.unwrap(), AgentApiIdempotencyDecision::Fresh));

        let second =
            reserve_agent_api_request(tmp.path(), &agent_id, Some("one"), &fingerprint).await;
        assert!(matches!(
            second.unwrap(),
            AgentApiIdempotencyDecision::Duplicate(_)
        ));
    }

    #[tokio::test]
    async fn reused_request_id_with_different_body_conflicts() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = sample_agent_id();
        let first = request_fingerprint(br#"{"request_id":"one","message":"hi"}"#);
        let second = request_fingerprint(br#"{"request_id":"one","message":"bye"}"#);

        reserve_agent_api_request(tmp.path(), &agent_id, Some("one"), &first)
            .await
            .unwrap();
        let decision = reserve_agent_api_request(tmp.path(), &agent_id, Some("one"), &second)
            .await
            .unwrap();
        assert!(matches!(decision, AgentApiIdempotencyDecision::Conflict(_)));
    }

    #[tokio::test]
    async fn completion_updates_duplicate_status() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_id = sample_agent_id();
        let fingerprint = request_fingerprint(br#"{"request_id":"one","message":"hi"}"#);

        reserve_agent_api_request(tmp.path(), &agent_id, Some("one"), &fingerprint)
            .await
            .unwrap();
        mark_agent_api_request_status(
            tmp.path(),
            &agent_id,
            Some("one"),
            AgentApiIdempotencyStatus::Completed,
        )
        .await
        .unwrap();

        let decision = reserve_agent_api_request(tmp.path(), &agent_id, Some("one"), &fingerprint)
            .await
            .unwrap();
        let AgentApiIdempotencyDecision::Duplicate(entry) = decision else {
            panic!("expected duplicate");
        };
        assert_eq!(entry.status, AgentApiIdempotencyStatus::Completed);
    }
}

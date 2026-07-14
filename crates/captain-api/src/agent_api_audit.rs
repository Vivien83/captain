//! Operational audit helpers for the per-agent API surface.

use captain_runtime::audit::{AuditAction, AuditEntry, AuditLog};
use captain_types::agent::AgentId;
use serde::Serialize;

const MAX_AUDIT_TEXT: usize = 180;
const MAX_AUDIT_SCAN: usize = 1000;

#[derive(Debug, Serialize)]
pub(crate) struct AgentApiAuditSummary {
    agent_id: String,
    returned: usize,
    items: Vec<serde_json::Value>,
}

pub(crate) fn record_ingress_denied(audit_log: &AuditLog, agent_id: &AgentId, reason: &str) {
    record(
        audit_log,
        agent_id,
        AuditAction::AuthAttempt,
        "ingress",
        "denied",
        None,
        &[],
        clean_outcome(reason),
    );
}

pub(crate) fn record_ingress_rejected(
    audit_log: &AuditLog,
    agent_id: &AgentId,
    request_id: Option<&str>,
    reason: &str,
) {
    record(
        audit_log,
        agent_id,
        AuditAction::AgentMessage,
        "ingress",
        "rejected",
        request_id,
        &[],
        clean_outcome(reason),
    );
}

pub(crate) fn record_ingress_accepted(
    audit_log: &AuditLog,
    agent_id: &AgentId,
    request_id: Option<&str>,
    message_bytes: usize,
    metadata_present: bool,
) {
    record(
        audit_log,
        agent_id,
        AuditAction::AgentMessage,
        "ingress",
        "accepted",
        request_id,
        &[
            ("message_bytes", serde_json::json!(message_bytes)),
            ("metadata_present", serde_json::json!(metadata_present)),
        ],
        "accepted".to_string(),
    );
}

pub(crate) fn record_ingress_duplicate(
    audit_log: &AuditLog,
    agent_id: &AgentId,
    request_id: Option<&str>,
    original_status: &str,
) {
    record(
        audit_log,
        agent_id,
        AuditAction::AgentMessage,
        "ingress",
        "duplicate",
        request_id,
        &[(
            "original_status",
            serde_json::json!(clip_text(original_status, MAX_AUDIT_TEXT)),
        )],
        "duplicate".to_string(),
    );
}

pub(crate) fn record_ingress_completed(
    audit_log: &AuditLog,
    agent_id: &AgentId,
    request_id: Option<&str>,
    iterations: u32,
) {
    record(
        audit_log,
        agent_id,
        AuditAction::AgentMessage,
        "ingress",
        "completed",
        request_id,
        &[("iterations", serde_json::json!(iterations))],
        "completed".to_string(),
    );
}

pub(crate) fn record_ingress_failed(
    audit_log: &AuditLog,
    agent_id: &AgentId,
    request_id: Option<&str>,
    error: &str,
) {
    record(
        audit_log,
        agent_id,
        AuditAction::AgentMessage,
        "ingress",
        "failed",
        request_id,
        &[],
        clean_outcome(error),
    );
}

pub(crate) fn record_egress_callback(
    audit_log: &AuditLog,
    agent_id: &AgentId,
    request_id: Option<&str>,
    event: &str,
    outcome: &str,
) {
    record(
        audit_log,
        agent_id,
        AuditAction::NetworkAccess,
        "egress",
        "callback",
        request_id,
        &[("event", serde_json::json!(clip_text(event, MAX_AUDIT_TEXT)))],
        clean_outcome(outcome),
    );
}

pub(crate) fn recent_agent_api_events(
    audit_log: &AuditLog,
    agent_id: &AgentId,
    limit: usize,
) -> AgentApiAuditSummary {
    let limit = limit.clamp(1, 100);
    let scan = limit.saturating_mul(20).clamp(limit, MAX_AUDIT_SCAN);
    let agent_id_str = agent_id.to_string();
    let items = audit_log
        .recent(scan)
        .into_iter()
        .rev()
        .filter(|entry| entry.agent_id == agent_id_str && is_agent_api_entry(entry))
        .take(limit)
        .map(entry_json)
        .collect::<Vec<_>>();

    AgentApiAuditSummary {
        agent_id: agent_id_str,
        returned: items.len(),
        items,
    }
}

pub(crate) fn request_id_from_payload(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("request_id")
        .and_then(|value| value.as_str())
        .map(|value| clip_text(value, MAX_AUDIT_TEXT))
}

#[allow(clippy::too_many_arguments)]
fn record(
    audit_log: &AuditLog,
    agent_id: &AgentId,
    action: AuditAction,
    direction: &str,
    phase: &str,
    request_id: Option<&str>,
    fields: &[(&str, serde_json::Value)],
    outcome: String,
) {
    audit_log.record(
        agent_id.to_string(),
        action,
        detail_json(direction, phase, request_id, fields),
        outcome,
    );
}

fn detail_json(
    direction: &str,
    phase: &str,
    request_id: Option<&str>,
    fields: &[(&str, serde_json::Value)],
) -> String {
    let mut map = serde_json::Map::new();
    map.insert("scope".to_string(), serde_json::json!("agent_api"));
    map.insert("direction".to_string(), serde_json::json!(direction));
    map.insert("phase".to_string(), serde_json::json!(phase));
    if let Some(request_id) = request_id.and_then(clean_optional_text) {
        map.insert("request_id".to_string(), serde_json::json!(request_id));
    }
    for (key, value) in fields {
        map.insert((*key).to_string(), value.clone());
    }
    serde_json::Value::Object(map).to_string()
}

fn is_agent_api_entry(entry: &AuditEntry) -> bool {
    serde_json::from_str::<serde_json::Value>(&entry.detail)
        .ok()
        .and_then(|detail| {
            detail
                .get("scope")
                .and_then(|scope| scope.as_str())
                .map(str::to_owned)
        })
        .is_some_and(|scope| scope == "agent_api")
}

fn entry_json(entry: AuditEntry) -> serde_json::Value {
    let detail = serde_json::from_str::<serde_json::Value>(&entry.detail)
        .unwrap_or_else(|_| serde_json::json!({ "raw": entry.detail }));
    serde_json::json!({
        "seq": entry.seq,
        "timestamp": entry.timestamp,
        "agent_id": entry.agent_id,
        "action": format!("{:?}", entry.action),
        "detail": detail,
        "outcome": entry.outcome,
        "hash": entry.hash,
    })
}

fn clean_optional_text(value: &str) -> Option<String> {
    let value = clip_text(value.trim(), MAX_AUDIT_TEXT);
    (!value.is_empty()).then_some(value)
}

fn clean_outcome(value: &str) -> String {
    clip_text(value.trim(), MAX_AUDIT_TEXT)
}

fn clip_text(value: &str, max: usize) -> String {
    let cleaned = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    if cleaned.len() <= max {
        return cleaned;
    }
    let mut boundary = max;
    while !cleaned.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}...", &cleaned[..boundary])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_agent_id() -> AgentId {
        "01234567-89ab-cdef-0123-456789abcdef".parse().unwrap()
    }

    #[test]
    fn audit_detail_is_structured_and_bounded() {
        let log = AuditLog::new();
        let request_id = "request\n".repeat(80);
        record_ingress_accepted(&log, &sample_agent_id(), Some(&request_id), 42, true);

        let entry = log.recent(1).pop().unwrap();
        let detail = serde_json::from_str::<serde_json::Value>(&entry.detail).unwrap();
        assert_eq!(detail["scope"], "agent_api");
        assert_eq!(detail["direction"], "ingress");
        assert_eq!(detail["message_bytes"], 42);
        assert!(detail["request_id"].as_str().unwrap().len() <= MAX_AUDIT_TEXT + 3);
    }

    #[test]
    fn recent_agent_api_events_filters_by_agent_and_scope() {
        let log = AuditLog::new();
        let agent_id = sample_agent_id();
        let other_id: AgentId = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".parse().unwrap();

        log.record(
            agent_id.to_string(),
            AuditAction::AgentMessage,
            "plain agent message",
            "ok",
        );
        record_ingress_completed(&log, &other_id, Some("other"), 1);
        record_ingress_completed(&log, &agent_id, Some("kept"), 2);

        let summary = recent_agent_api_events(&log, &agent_id, 10);
        assert_eq!(summary.returned, 1);
        assert_eq!(summary.items[0]["detail"]["request_id"], "kept");
    }
}

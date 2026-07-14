//! Agent API status helpers.

use crate::agent_api_egress_queue::AgentApiQueuedCallback;
use chrono::{DateTime, Utc};
use std::collections::BTreeSet;

pub(crate) fn build_agent_api_status(
    entries: &[AgentApiQueuedCallback],
    now: DateTime<Utc>,
) -> serde_json::Value {
    let pending = entries.iter().filter(|entry| !entry.dead_letter).count();
    let due = entries
        .iter()
        .filter(|entry| !entry.dead_letter && entry.next_attempt_at <= now)
        .count();
    let dead_letters = entries.iter().filter(|entry| entry.dead_letter).count();
    let agents_with_queue = entries
        .iter()
        .map(|entry| entry.agent_id.to_string())
        .collect::<BTreeSet<_>>()
        .len();

    let mut last_errors = entries
        .iter()
        .filter_map(|entry| {
            let error = entry.last_error.as_ref()?;
            Some(serde_json::json!({
                "id": entry.id,
                "agent_id": entry.agent_id.to_string(),
                "event": entry.event,
                "created_at": entry.created_at.to_rfc3339(),
                "next_attempt_at": entry.next_attempt_at.to_rfc3339(),
                "attempts": entry.attempts,
                "max_attempts": entry.max_attempts,
                "dead_letter": entry.dead_letter,
                "error_kind": agent_api_error_kind(error),
                "error_preview": safe_agent_api_error_preview(error),
            }))
        })
        .collect::<Vec<_>>();
    last_errors.sort_by(|a, b| {
        b["created_at"]
            .as_str()
            .unwrap_or("")
            .cmp(a["created_at"].as_str().unwrap_or(""))
    });
    last_errors.truncate(5);

    let state = if dead_letters > 0 {
        "dead_letter"
    } else if due > 0 {
        "attention"
    } else if pending > 0 {
        "retrying"
    } else {
        "ok"
    };

    serde_json::json!({
        "egress_queue": {
            "state": state,
            "readable": true,
            "pending": pending,
            "due": due,
            "dead_letters": dead_letters,
            "agents_with_queue": agents_with_queue,
            "last_errors": last_errors,
        }
    })
}

pub(crate) fn unavailable_agent_api_status(issue: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "egress_queue": {
            "state": "unavailable",
            "readable": false,
            "pending": 0,
            "due": 0,
            "dead_letters": 0,
            "agents_with_queue": 0,
            "last_errors": [],
            "issue": issue.into(),
        }
    })
}

fn agent_api_error_kind(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("429") || lower.contains("rate limit") {
        "rate_limit"
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else if lower.contains("http 5") {
        "transient_http"
    } else if lower.contains("http 4")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
    {
        "target_or_auth"
    } else if lower.contains("missing") || lower.contains("too short") {
        "configuration"
    } else {
        "callback_failed"
    }
}

fn safe_agent_api_error_preview(error: &str) -> String {
    let trimmed = error.trim();
    if agent_api_error_contains_sensitive_fragment(trimmed) {
        return format!(
            "{}; inspect per-agent API egress status for callback details",
            agent_api_error_kind(trimmed)
        );
    }
    truncate_chars(trimmed, 180)
}

fn agent_api_error_contains_sensitive_fragment(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("authorization")
        || lower.contains("bearer ")
        || lower.contains("token")
        || lower.contains("secret")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("password")
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::AgentId;
    use chrono::Duration;

    fn entry(now: DateTime<Utc>, dead_letter: bool, error: &str) -> AgentApiQueuedCallback {
        AgentApiQueuedCallback {
            id: "queue-1".to_string(),
            agent_id: "01234567-89ab-cdef-0123-456789abcdef"
                .parse::<AgentId>()
                .unwrap(),
            event: "agent_api.completed".to_string(),
            payload: serde_json::json!({"hidden": "payload"}),
            created_at: now,
            next_attempt_at: now - Duration::seconds(1),
            attempts: 2,
            max_attempts: 3,
            last_error: Some(error.to_string()),
            dead_letter,
        }
    }

    #[test]
    fn agent_api_status_counts_due_and_dead_letters() {
        let now = Utc::now();
        let status = build_agent_api_status(
            &[
                entry(now, false, "callback returned HTTP 503"),
                entry(now, true, "HTTP 400"),
            ],
            now,
        );
        let queue = &status["egress_queue"];

        assert_eq!(queue["state"], "dead_letter");
        assert_eq!(queue["pending"], 1);
        assert_eq!(queue["due"], 1);
        assert_eq!(queue["dead_letters"], 1);
        assert_eq!(queue["agents_with_queue"], 1);
    }

    #[test]
    fn agent_api_status_redacts_urls_from_error_preview() {
        let now = Utc::now();
        let status = build_agent_api_status(
            &[entry(
                now,
                false,
                "callback request failed: error sending request for url (https://example.com/hook?token=secret)",
            )],
            now,
        );
        let preview = status["egress_queue"]["last_errors"][0]["error_preview"]
            .as_str()
            .unwrap();

        assert!(preview.contains("inspect per-agent API egress status"));
        assert!(!preview.contains("token=secret"));
    }

    #[test]
    fn agent_api_status_redacts_secret_words_from_error_preview() {
        let now = Utc::now();
        let status = build_agent_api_status(
            &[entry(
                now,
                false,
                "callback failed with Authorization: Bearer captain_token_123",
            )],
            now,
        );
        let preview = status["egress_queue"]["last_errors"][0]["error_preview"]
            .as_str()
            .unwrap();

        assert!(preview.contains("inspect per-agent API egress status"));
        assert!(!preview.contains("captain_token_123"));
    }
}

use serde_json::Value;

#[derive(Clone, Default)]
pub struct StatusIssue {
    pub kind: String,
    pub severity: String,
    pub summary: String,
    pub action: String,
}

#[derive(Clone, Default)]
pub struct StatusSnapshot {
    pub status: String,
    pub runtime_health_state: String,
    pub runtime_health_issue_count: u64,
    pub runtime_health_issues: Vec<StatusIssue>,
    pub agent_count: u64,
    pub active_run_count: u64,
    pub process_count: u64,
    pub uptime_secs: u64,
    pub version: String,
    pub provider: String,
    pub model: String,
    pub channel_ready_count: u64,
    pub channel_total: u64,
    pub tool_runs_running: u64,
    pub tool_runs_completed: u64,
    pub tool_runs_failed: u64,
    pub agent_api_state: String,
    pub agent_api_pending: u64,
    pub agent_api_due: u64,
    pub agent_api_dead_letters: u64,
    pub consciousness_state: String,
    pub consciousness_signals: Vec<String>,
    pub consciousness_actions: Vec<String>,
    pub streaming_active: u64,
    pub streaming_completed: u64,
    pub streaming_last_first_signal_ms: Option<u64>,
    pub streaming_last_first_token_ms: Option<u64>,
    pub streaming_last_total_ms: Option<u64>,
    pub shutdown_status: String,
    pub disk_available_gib: Option<f64>,
    pub disk_cleanup_recommended: bool,
    pub native_embeddings_ready: Option<bool>,
    pub native_voice_tts_ready: Option<bool>,
    pub native_voice_stt_ready: Option<bool>,
    pub budget_total_tokens_used: u64,
    pub project_attention_count: u64,
    pub goals_active: u64,
    pub goals_escalated: u64,
    pub cron_due: u64,
    pub cron_enabled: u64,
}

impl StatusSnapshot {
    pub fn from_json(body: &Value) -> Self {
        Self {
            status: string_at(body, "/status", "unknown"),
            runtime_health_state: string_at(body, "/runtime_health/state", "unknown"),
            runtime_health_issue_count: u64_at(body, "/runtime_health/issue_count"),
            runtime_health_issues: status_issues(body.pointer("/runtime_health/issues")),
            agent_count: u64_at(body, "/agent_count"),
            active_run_count: u64_at(body, "/active_run_count"),
            process_count: u64_at(body, "/process_count"),
            uptime_secs: u64_any(body, &["/uptime_seconds", "/uptime_secs"]),
            version: string_at(body, "/version", "?"),
            provider: string_any(body, &["/default_provider", "/provider"], ""),
            model: string_any(body, &["/default_model", "/model"], ""),
            channel_ready_count: u64_at(body, "/channels/ready_count"),
            channel_total: u64_at(body, "/channels/total"),
            tool_runs_running: u64_at(body, "/tool_runs/running"),
            tool_runs_completed: u64_at(body, "/tool_runs/completed"),
            tool_runs_failed: u64_at(body, "/tool_runs/failed"),
            agent_api_state: string_at(body, "/agent_api/egress_queue/state", "unknown"),
            agent_api_pending: u64_at(body, "/agent_api/egress_queue/pending"),
            agent_api_due: u64_at(body, "/agent_api/egress_queue/due"),
            agent_api_dead_letters: u64_at(body, "/agent_api/egress_queue/dead_letters"),
            consciousness_state: string_at(body, "/consciousness/state", "unknown"),
            consciousness_signals: string_vec_at(body, "/consciousness/signals"),
            consciousness_actions: string_vec_at(body, "/consciousness/operator_actions"),
            streaming_active: u64_at(body, "/streaming/active"),
            streaming_completed: u64_at(body, "/streaming/completed"),
            streaming_last_first_signal_ms: optional_u64_at(
                body,
                "/streaming/last/first_signal_ms",
            ),
            streaming_last_first_token_ms: optional_u64_at(body, "/streaming/last/first_token_ms"),
            streaming_last_total_ms: optional_u64_at(body, "/streaming/last/total_ms"),
            shutdown_status: string_at(body, "/shutdown/status", "unknown"),
            disk_available_gib: optional_f64_at(body, "/disk/available_gib"),
            disk_cleanup_recommended: bool_at(body, "/disk/cleanup_recommended"),
            native_embeddings_ready: optional_bool_at(body, "/native_embeddings/ready"),
            native_voice_tts_ready: optional_bool_at(body, "/native_voice/tts_ready"),
            native_voice_stt_ready: optional_bool_at(body, "/native_voice/stt_ready"),
            budget_total_tokens_used: u64_at(body, "/budget/total_tokens_used"),
            project_attention_count: u64_at(body, "/workload/projects/attention_count"),
            goals_active: u64_at(body, "/workload/goals/active"),
            goals_escalated: u64_at(body, "/workload/goals/escalated"),
            cron_due: u64_at(body, "/workload/automation/cron_due"),
            cron_enabled: u64_at(body, "/workload/automation/cron_enabled"),
        }
    }

    pub fn in_process(agent_count: u64, version: String) -> Self {
        Self {
            status: "in-process".to_string(),
            runtime_health_state: "local".to_string(),
            agent_count,
            version,
            ..Self::default()
        }
    }
}

fn status_issues(value: Option<&Value>) -> Vec<StatusIssue> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| StatusIssue {
                    kind: string_at(item, "/kind", "issue"),
                    severity: string_at(item, "/severity", "watch"),
                    summary: string_at(item, "/summary", ""),
                    action: string_at(item, "/action", ""),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn string_at(value: &Value, pointer: &str, default: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_string()
}

fn string_any(value: &Value, pointers: &[&str], default: &str) -> String {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
        .unwrap_or(default)
        .to_string()
}

fn string_vec_at(value: &Value, pointer: &str) -> Vec<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn u64_at(value: &Value, pointer: &str) -> u64 {
    optional_u64_at(value, pointer).unwrap_or(0)
}

fn u64_any(value: &Value, pointers: &[&str]) -> u64 {
    pointers
        .iter()
        .find_map(|pointer| optional_u64_at(value, pointer))
        .unwrap_or(0)
}

fn optional_u64_at(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(Value::as_u64)
}

fn optional_f64_at(value: &Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer).and_then(Value::as_f64)
}

fn bool_at(value: &Value, pointer: &str) -> bool {
    value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn optional_bool_at(value: &Value, pointer: &str) -> Option<bool> {
    value.pointer(pointer).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn status_snapshot_maps_live_status_contract() {
        let snapshot = StatusSnapshot::from_json(&json!({
            "status": "running",
            "version": "0.1.0-dev",
            "agent_count": 5,
            "active_run_count": 1,
            "process_count": 2,
            "uptime_seconds": 3661,
            "default_provider": "codex",
            "default_model": "gpt-5.5",
            "channels": {"ready_count": 2, "total": 4},
            "runtime_health": {
                "state": "warn",
                "issue_count": 1,
                "issues": [{
                    "kind": "consciousness",
                    "severity": "warn",
                    "summary": "Operational awareness is warn.",
                    "action": "Inspect recent logs."
                }]
            },
            "tool_runs": {"running": 3, "completed": 42, "failed": 1},
            "agent_api": {
                "egress_queue": {
                    "state": "attention",
                    "pending": 2,
                    "due": 1,
                    "dead_letters": 0
                }
            },
            "consciousness": {
                "state": "warn",
                "signals": ["goals_escalated:1"],
                "operator_actions": ["Review escalated goals."]
            },
            "streaming": {
                "active": 0,
                "completed": 8,
                "last": {
                    "first_signal_ms": 1000,
                    "first_token_ms": 3200,
                    "total_ms": 5500
                }
            },
            "shutdown": {"status": "idle"},
            "disk": {"available_gib": 42.9, "cleanup_recommended": false},
            "native_embeddings": {"ready": false},
            "native_voice": {"tts_ready": true, "stt_ready": true},
            "budget": {"total_tokens_used": 12345},
            "workload": {
                "projects": {"attention_count": 4},
                "goals": {"active": 1, "escalated": 1},
                "automation": {"cron_due": 0, "cron_enabled": 5}
            }
        }));

        assert_eq!(snapshot.runtime_health_state, "warn");
        assert_eq!(snapshot.runtime_health_issues[0].kind, "consciousness");
        assert_eq!(snapshot.agent_count, 5);
        assert_eq!(snapshot.tool_runs_running, 3);
        assert_eq!(snapshot.agent_api_state, "attention");
        assert_eq!(snapshot.consciousness_signals, vec!["goals_escalated:1"]);
        assert_eq!(snapshot.streaming_last_first_token_ms, Some(3200));
        assert_eq!(snapshot.native_embeddings_ready, Some(false));
        assert_eq!(snapshot.goals_escalated, 1);
    }

    #[test]
    fn status_snapshot_handles_legacy_minimal_status() {
        let snapshot = StatusSnapshot::from_json(&json!({
            "agent_count": 2,
            "uptime_secs": 10,
            "provider": "codex",
            "model": "gpt-5"
        }));

        assert_eq!(snapshot.status, "unknown");
        assert_eq!(snapshot.agent_count, 2);
        assert_eq!(snapshot.uptime_secs, 10);
        assert_eq!(snapshot.provider, "codex");
        assert_eq!(snapshot.model, "gpt-5");
        assert!(snapshot.runtime_health_issues.is_empty());
    }
}

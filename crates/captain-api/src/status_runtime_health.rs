//! Runtime health rollup for operator status.

use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Severity {
    Ok,
    Watch,
    Warn,
    Critical,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Watch => "watch",
            Self::Warn => "warn",
            Self::Critical => "critical",
        }
    }
}

pub fn build_runtime_health_status(
    llm_driver_ready: bool,
    channels: &Value,
    workload: &Value,
    agent_api: &Value,
    consciousness: &Value,
    disk: &Value,
    shutdown: &Value,
    budget: &Value,
) -> Value {
    let mut issues = Vec::new();
    let mut max_severity = Severity::Ok;

    if !llm_driver_ready {
        push_issue(
            &mut issues,
            &mut max_severity,
            Severity::Critical,
            "llm_driver_unavailable",
            "Default LLM driver is not ready.",
            "Fix provider credentials/configuration before starting long work.",
        );
    }

    let locked_channels = channels["locked"].as_array().map(Vec::len).unwrap_or(0);
    if locked_channels > 0 {
        push_issue(
            &mut issues,
            &mut max_severity,
            Severity::Warn,
            "channel_readiness",
            &format!("{locked_channels} configured channel(s) are locked."),
            "Run `captain channel list` and complete missing fields or allowlists.",
        );
    }

    let project_attention = workload["projects"]["attention_count"]
        .as_u64()
        .unwrap_or(0);
    if project_attention > 0 {
        push_issue(
            &mut issues,
            &mut max_severity,
            Severity::Warn,
            "project_attention",
            &format!("{project_attention} project(s) need operator attention."),
            "Review `Project Attention` and resume, answer, or replan blocked work.",
        );
    }

    add_automation_delivery_issue(&mut issues, &mut max_severity, workload);
    add_agent_api_issue(&mut issues, &mut max_severity, agent_api);
    add_consciousness_issue(&mut issues, &mut max_severity, consciousness);
    add_disk_issue(&mut issues, &mut max_severity, disk);
    add_shutdown_issue(&mut issues, &mut max_severity, shutdown);
    add_provider_quota_issue(&mut issues, &mut max_severity, budget);

    let actions = unique_actions(&issues);
    serde_json::json!({
        "state": max_severity.as_str(),
        "issue_count": issues.len(),
        "issues": issues,
        "operator_actions": actions,
    })
}

fn add_provider_quota_issue(issues: &mut Vec<Value>, max_severity: &mut Severity, budget: &Value) {
    let provider = &budget["provider_subscriptions"];
    let state = provider["state"].as_str().unwrap_or("unavailable");
    let (severity, summary, fallback_action) = match state {
        "exhausted" => (
            Severity::Critical,
            "The provider subscription quota is exhausted.",
            "Wait for the provider-reported reset or change the configured model deliberately.",
        ),
        "critical" => (
            Severity::Warn,
            "The provider subscription quota is near exhaustion.",
            "Inspect provider quota windows before starting long work.",
        ),
        "warning" => (
            Severity::Watch,
            "The provider subscription quota crossed the warning threshold.",
            "Inspect provider quota windows before starting long work.",
        ),
        "stale" => (
            Severity::Watch,
            "The last provider subscription quota observation is stale.",
            "Verify the Codex session and network before relying on the displayed allowance.",
        ),
        _ => return,
    };
    let action = budget["operator_actions"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .find(|item| item.contains("Provider subscription"))
        })
        .unwrap_or(fallback_action);
    push_issue(
        issues,
        max_severity,
        severity,
        "provider_subscription_quota",
        summary,
        action,
    );
}

fn add_shutdown_issue(issues: &mut Vec<Value>, max_severity: &mut Severity, shutdown: &Value) {
    if shutdown["status"].as_str().unwrap_or("idle") != "draining" {
        return;
    }
    let active = shutdown["active_work_count"]
        .as_u64()
        .unwrap_or_else(|| shutdown["active_run_count"].as_u64().unwrap_or(0));
    let trigger = shutdown["trigger"].as_str().unwrap_or("control");
    let action = shutdown["operator_actions"]
        .as_array()
        .and_then(|items| items.iter().find_map(|item| item.as_str()))
        .unwrap_or("Inspect active work before retrying shutdown.");
    push_issue(
        issues,
        max_severity,
        Severity::Watch,
        "shutdown_draining",
        &format!("Shutdown via {trigger} is waiting on {active} active work item(s)."),
        action,
    );
}

fn add_disk_issue(issues: &mut Vec<Value>, max_severity: &mut Severity, disk: &Value) {
    if !disk["cleanup_recommended"].as_bool().unwrap_or(false) {
        return;
    }
    let available_gib = disk["available_gib"].as_f64().unwrap_or(0.0);
    let threshold_gib = disk["cleanup_threshold_gib"].as_f64().unwrap_or(15.0);
    push_issue(
        issues,
        max_severity,
        Severity::Warn,
        "disk_space",
        &format!("Disk has {available_gib:.1} GiB free at or below {threshold_gib:.1} GiB cleanup threshold."),
        "Clean build/debug artifacts before starting long compile or install work.",
    );
}

fn add_automation_delivery_issue(
    issues: &mut Vec<Value>,
    max_severity: &mut Severity,
    workload: &Value,
) {
    let delivery = &workload["automation"]["delivery"];
    let failed = delivery["failed_jobs"].as_u64().unwrap_or(0);
    let queued = delivery["redelivery_queued"].as_u64().unwrap_or(0);
    let due = delivery["redelivery_due"].as_u64().unwrap_or(0);
    let dead = delivery["dead_letters"].as_u64().unwrap_or(0);
    if failed == 0 && queued == 0 && due == 0 && dead == 0 {
        return;
    }

    let severity = if dead > 0 || failed > 0 || due > 0 {
        Severity::Warn
    } else {
        Severity::Watch
    };
    push_issue(
        issues,
        max_severity,
        severity,
        "automation_delivery",
        &format!("{failed} failed cron delivery, {due} due retry, {dead} dead letter(s)."),
        "Inspect automation delivery details before assuming scheduled work is healthy.",
    );
}

fn add_agent_api_issue(issues: &mut Vec<Value>, max_severity: &mut Severity, agent_api: &Value) {
    let queue = &agent_api["egress_queue"];
    if queue.is_null() {
        return;
    }
    if queue["readable"].as_bool() == Some(false) {
        push_issue(
            issues,
            max_severity,
            Severity::Warn,
            "agent_api_egress_unavailable",
            "Agent API callback queue cannot be read.",
            "Inspect the agent API egress queue file before retrying callbacks.",
        );
        return;
    }

    let pending = queue["pending"].as_u64().unwrap_or(0);
    let due = queue["due"].as_u64().unwrap_or(0);
    let dead = queue["dead_letters"].as_u64().unwrap_or(0);
    if pending == 0 && due == 0 && dead == 0 {
        return;
    }
    let severity = if dead > 0 || due > 0 {
        Severity::Warn
    } else {
        Severity::Watch
    };
    push_issue(
        issues,
        max_severity,
        severity,
        "agent_api_egress",
        &format!("{pending} pending callback(s), {due} due, {dead} dead letter(s)."),
        "Inspect `/api/agents/{id}/api/egress` before retrying callbacks.",
    );
}

fn add_consciousness_issue(
    issues: &mut Vec<Value>,
    max_severity: &mut Severity,
    consciousness: &Value,
) {
    let state = consciousness["state"].as_str().unwrap_or("steady");
    let severity = match state {
        "critical" => Severity::Critical,
        "warn" => Severity::Warn,
        _ => return,
    };
    let signal_count = consciousness["signals"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    let action = consciousness["operator_actions"]
        .as_array()
        .and_then(|items| items.iter().find_map(|item| item.as_str()))
        .unwrap_or("Review consciousness signals before starting new long work.");
    push_issue(
        issues,
        max_severity,
        severity,
        "consciousness",
        &format!("Operational awareness is {state} with {signal_count} signal(s)."),
        action,
    );
}

fn push_issue(
    issues: &mut Vec<Value>,
    max_severity: &mut Severity,
    severity: Severity,
    kind: &str,
    summary: &str,
    action: &str,
) {
    *max_severity = (*max_severity).max(severity);
    issues.push(serde_json::json!({
        "severity": severity.as_str(),
        "kind": kind,
        "summary": summary,
        "action": action,
    }));
}

fn unique_actions(issues: &[Value]) -> Vec<String> {
    let mut actions = Vec::new();
    for issue in issues {
        let Some(action) = issue["action"].as_str() else {
            continue;
        };
        if !actions.iter().any(|existing| existing == action) {
            actions.push(action.to_string());
        }
    }
    actions.truncate(5);
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean_inputs() -> (Value, Value, Value, Value, Value, Value, Value) {
        (
            serde_json::json!({"locked": []}),
            serde_json::json!({
                "projects": {"attention_count": 0},
                "automation": {
                    "delivery": {
                        "failed_jobs": 0,
                        "redelivery_queued": 0,
                        "redelivery_due": 0,
                        "dead_letters": 0
                    }
                }
            }),
            serde_json::json!({
                "egress_queue": {
                    "readable": true,
                    "pending": 0,
                    "due": 0,
                    "dead_letters": 0
                }
            }),
            serde_json::json!({"state": "steady", "signals": [], "operator_actions": []}),
            serde_json::json!({
                "state": "ok",
                "available_gib": 42.0,
                "cleanup_threshold_gib": 15.0,
                "cleanup_recommended": false
            }),
            serde_json::json!({"status": "idle", "active_run_count": 0}),
            serde_json::json!({
                "provider_subscriptions": {"state": "ok"},
                "operator_actions": []
            }),
        )
    }

    #[test]
    fn runtime_health_is_ok_when_core_signals_are_clean() {
        let (channels, workload, agent_api, consciousness, disk, shutdown, budget) = clean_inputs();
        let status = build_runtime_health_status(
            true,
            &channels,
            &workload,
            &agent_api,
            &consciousness,
            &disk,
            &shutdown,
            &budget,
        );

        assert_eq!(status["state"], "ok");
        assert_eq!(status["issue_count"], 0);
    }

    #[test]
    fn runtime_health_promotes_llm_failure_to_critical() {
        let (channels, workload, agent_api, consciousness, disk, shutdown, budget) = clean_inputs();
        let status = build_runtime_health_status(
            false,
            &channels,
            &workload,
            &agent_api,
            &consciousness,
            &disk,
            &shutdown,
            &budget,
        );

        assert_eq!(status["state"], "critical");
        assert_eq!(status["issues"][0]["kind"], "llm_driver_unavailable");
    }

    #[test]
    fn runtime_health_rolls_up_delivery_and_agent_api_issues() {
        let (channels, mut workload, mut agent_api, consciousness, disk, shutdown, budget) =
            clean_inputs();
        workload["automation"]["delivery"]["redelivery_due"] = serde_json::json!(1);
        agent_api["egress_queue"]["pending"] = serde_json::json!(2);
        agent_api["egress_queue"]["dead_letters"] = serde_json::json!(1);
        let status = build_runtime_health_status(
            true,
            &channels,
            &workload,
            &agent_api,
            &consciousness,
            &disk,
            &shutdown,
            &budget,
        );

        assert_eq!(status["state"], "warn");
        assert_eq!(status["issue_count"], 2);
    }

    #[test]
    fn runtime_health_warns_when_disk_cleanup_is_recommended() {
        let (channels, workload, agent_api, consciousness, mut disk, shutdown, budget) =
            clean_inputs();
        disk["available_gib"] = serde_json::json!(14.9);
        disk["cleanup_recommended"] = serde_json::json!(true);
        let status = build_runtime_health_status(
            true,
            &channels,
            &workload,
            &agent_api,
            &consciousness,
            &disk,
            &shutdown,
            &budget,
        );

        assert_eq!(status["state"], "warn");
        assert_eq!(status["issues"][0]["kind"], "disk_space");
    }

    #[test]
    fn runtime_health_watches_shutdown_drain() {
        let (channels, workload, agent_api, consciousness, disk, _, budget) = clean_inputs();
        let shutdown = serde_json::json!({
            "status": "draining",
            "trigger": "SIGTERM",
            "active_work_count": 2,
            "active_run_count": 1,
            "active_process_count": 1,
            "operator_actions": ["Run captain status to inspect active work."]
        });
        let status = build_runtime_health_status(
            true,
            &channels,
            &workload,
            &agent_api,
            &consciousness,
            &disk,
            &shutdown,
            &budget,
        );

        assert_eq!(status["state"], "watch");
        assert_eq!(status["issues"][0]["kind"], "shutdown_draining");
    }

    #[test]
    fn runtime_health_promotes_exhausted_provider_quota_to_critical() {
        let (channels, workload, agent_api, consciousness, disk, shutdown, mut budget) =
            clean_inputs();
        budget["provider_subscriptions"]["state"] = serde_json::json!("exhausted");
        budget["operator_actions"] = serde_json::json!([
            "Provider subscription quota Codex is exhausted. Retry after 2026-07-18T18:00:00Z."
        ]);

        let status = build_runtime_health_status(
            true,
            &channels,
            &workload,
            &agent_api,
            &consciousness,
            &disk,
            &shutdown,
            &budget,
        );

        assert_eq!(status["state"], "critical");
        assert_eq!(status["issues"][0]["kind"], "provider_subscription_quota");
        assert!(status["issues"][0]["action"]
            .as_str()
            .unwrap()
            .contains("2026-07-18T18:00:00Z"));
    }
}

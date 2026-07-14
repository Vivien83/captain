use crate::project_lifecycle::runtime_progress_for_phase;
use crate::project_runtime_asks::close_runtime_project_asks_for_run;
use crate::project_runtime_checkpoints::PROJECT_RUNTIME_PROTOCOL;
use crate::project_runtime_events::append_runtime_event;
use chrono::Utc;

pub(crate) const PROJECT_RUNTIME_GENERATION: u64 = 2;

pub(crate) struct RuntimeResumeEventMetadata {
    pub(crate) trigger: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) title: &'static str,
    pub(crate) detail: &'static str,
}

pub(crate) fn runtime_run_id(runtime: &serde_json::Value) -> Option<String> {
    runtime
        .pointer("/orchestrator/run_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
}

pub(crate) fn runtime_orchestrator_allows_continue(runtime: &serde_json::Value) -> bool {
    let status = runtime
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("ready");
    let paused = runtime
        .pointer("/control/paused")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let takeover = runtime
        .pointer("/control/takeover")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    status == "running" && !paused && !takeover
}

pub(crate) fn activate_runtime_orchestrator(
    runtime: &mut serde_json::Value,
    trigger: &str,
) -> String {
    let now = Utc::now().to_rfc3339();
    let run_id = runtime_run_id(runtime).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let started_at = runtime
        .pointer("/orchestrator/started_at")
        .and_then(|v| v.as_str())
        .unwrap_or(&now)
        .to_string();
    runtime["protocol"] = serde_json::json!(PROJECT_RUNTIME_PROTOCOL);
    runtime["version"] = serde_json::json!(PROJECT_RUNTIME_GENERATION);
    runtime["orchestrator"] = serde_json::json!({
        "generation": PROJECT_RUNTIME_GENERATION,
        "run_id": run_id,
        "active": true,
        "trigger": trigger,
        "strategy": "multi_agent_parallel_gated",
        "manager": "captain",
        "started_at": started_at,
        "updated_at": now,
    });
    run_id
}

pub(crate) fn deactivate_runtime_orchestrator(runtime: &mut serde_json::Value, reason: &str) {
    let now = Utc::now().to_rfc3339();
    let mut orchestrator = runtime
        .get("orchestrator")
        .cloned()
        .filter(|value| value.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(obj) = orchestrator.as_object_mut() {
        obj.insert("active".to_string(), serde_json::json!(false));
        obj.insert("stopped_reason".to_string(), serde_json::json!(reason));
        obj.insert("updated_at".to_string(), serde_json::json!(now));
        obj.entry("generation".to_string())
            .or_insert_with(|| serde_json::json!(PROJECT_RUNTIME_GENERATION));
    }
    runtime["orchestrator"] = orchestrator;
}

pub(crate) fn runtime_resume_event_metadata(reason: Option<&str>) -> RuntimeResumeEventMetadata {
    match reason {
        Some("tool_request_approved") => RuntimeResumeEventMetadata {
            trigger: "resume_after_tool_request",
            kind: "orchestrator.resume_after_tool_request",
            title: "Run resumed after tool approval",
            detail: "Captain found an approved project tool request and is resuming without resetting completed worker phases.",
        },
        Some("project_ask_answered") => RuntimeResumeEventMetadata {
            trigger: "resume_after_user_answer",
            kind: "orchestrator.resume_after_user_answer",
            title: "Run resumed after user answer",
            detail: "Captain found a persisted project answer and is resuming without resetting completed worker phases.",
        },
        _ => RuntimeResumeEventMetadata {
            trigger: "resume_pending",
            kind: "orchestrator.resume_pending",
            title: "Run resumed from pending state",
            detail: "Captain found a persisted resume marker and is resuming without resetting completed worker phases.",
        },
    }
}

pub(crate) fn resume_runtime_orchestrator(
    runtime: &mut serde_json::Value,
    phase: &str,
    trigger: &str,
    kind: &str,
    title: &str,
    detail: &str,
    actor: &str,
) {
    let now = Utc::now().to_rfc3339();
    let run_id = activate_runtime_orchestrator(runtime, trigger);
    runtime["status"] = serde_json::json!("running");
    runtime["current_phase"] = serde_json::json!(phase);
    runtime["progress"] = serde_json::json!(runtime_progress_for_phase(phase, "running"));
    runtime["updated_at"] = serde_json::json!(now);
    runtime["control"] = serde_json::json!({ "paused": false, "takeover": false });
    append_runtime_event(
        runtime,
        kind,
        title,
        detail,
        actor,
        phase,
        "running",
        serde_json::json!({ "run_id": run_id }),
    );
}

pub(crate) fn mark_runtime_waiting(runtime: &mut serde_json::Value, phase: &str, run_id: &str) {
    runtime["status"] = serde_json::json!("paused");
    runtime["current_phase"] = serde_json::json!(phase);
    runtime["progress"] = serde_json::json!(runtime_progress_for_phase(phase, "paused"));
    deactivate_runtime_orchestrator(runtime, "paused");
    append_runtime_event(
        runtime,
        "orchestrator.waiting",
        "Run waiting",
        "Autonomous execution stopped before launching the next worker because the project is paused or in manual takeover.",
        "captain",
        phase,
        "paused",
        serde_json::json!({ "run_id": run_id }),
    );
}

pub(crate) fn mark_runtime_dispatch_started(runtime: &mut serde_json::Value, run_id: &str) {
    runtime["status"] = serde_json::json!("running");
    runtime["current_phase"] = serde_json::json!("observe");
    runtime["progress"] = serde_json::json!(runtime_progress_for_phase("observe", "running"));
    runtime["updated_at"] = serde_json::json!(Utc::now().to_rfc3339());
    append_runtime_event(
        runtime,
        "orchestrator.dispatch",
        "Worker dispatch started",
        "Captain is dispatching OBSERVE and THINK in parallel, then gated execution phases.",
        "captain",
        "observe",
        "running",
        serde_json::json!({ "run_id": run_id }),
    );
}

pub(crate) fn mark_runtime_completed(
    runtime: &mut serde_json::Value,
    run_id: &str,
    project_id: &str,
    project_slug: &str,
) {
    runtime["status"] = serde_json::json!("done");
    runtime["current_phase"] = serde_json::json!("learn");
    runtime["progress"] = serde_json::json!(100);
    runtime["updated_at"] = serde_json::json!(Utc::now().to_rfc3339());
    close_runtime_project_asks_for_run(runtime, run_id);
    deactivate_runtime_orchestrator(runtime, "completed");
    append_runtime_event(
        runtime,
        "project.completed",
        "Autonomous run completed",
        "All project runtime phases reached their verification and learning gates.",
        "captain",
        "learn",
        "done",
        serde_json::json!({
            "run_id": run_id,
            "project_id": project_id,
            "slug": project_slug,
        }),
    );
}

#[cfg(test)]
#[path = "project_runtime_orchestrator_tests.rs"]
mod project_runtime_orchestrator_tests;

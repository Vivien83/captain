use super::*;

#[test]
fn activate_runtime_orchestrator_reuses_existing_run_id() {
    let mut runtime = serde_json::json!({
        "orchestrator": {
            "run_id": "run-1",
            "started_at": "2026-05-20T00:00:00Z"
        }
    });

    let run_id = activate_runtime_orchestrator(&mut runtime, "resume");

    assert_eq!(run_id, "run-1");
    assert_eq!(runtime["orchestrator"]["active"], true);
    assert_eq!(runtime["orchestrator"]["trigger"], "resume");
    assert_eq!(runtime["protocol"], PROJECT_RUNTIME_PROTOCOL);
}

#[test]
fn deactivate_runtime_orchestrator_preserves_existing_contract() {
    let mut runtime = serde_json::json!({
        "orchestrator": {
            "generation": PROJECT_RUNTIME_GENERATION,
            "run_id": "run-1",
            "active": true,
            "trigger": "start",
            "started_at": "2026-05-20T00:00:00Z"
        }
    });

    deactivate_runtime_orchestrator(&mut runtime, "blocked");

    assert_eq!(runtime["orchestrator"]["run_id"], "run-1");
    assert_eq!(runtime["orchestrator"]["active"], false);
    assert_eq!(runtime["orchestrator"]["stopped_reason"], "blocked");
    assert_eq!(
        runtime["orchestrator"]["generation"],
        PROJECT_RUNTIME_GENERATION
    );
    assert!(runtime["orchestrator"]["updated_at"].as_str().is_some());
}

#[test]
fn deactivate_runtime_orchestrator_initializes_missing_object() {
    let mut runtime = serde_json::json!({
        "orchestrator": "malformed"
    });

    deactivate_runtime_orchestrator(&mut runtime, "paused");

    assert_eq!(runtime["orchestrator"]["active"], false);
    assert_eq!(runtime["orchestrator"]["stopped_reason"], "paused");
    assert_eq!(
        runtime["orchestrator"]["generation"],
        PROJECT_RUNTIME_GENERATION
    );
}

#[test]
fn runtime_resume_event_metadata_maps_known_reasons() {
    let tool = runtime_resume_event_metadata(Some("tool_request_approved"));
    assert_eq!(tool.trigger, "resume_after_tool_request");
    assert_eq!(tool.kind, "orchestrator.resume_after_tool_request");
    assert!(tool.detail.contains("approved project tool request"));

    let answer = runtime_resume_event_metadata(Some("project_ask_answered"));
    assert_eq!(answer.trigger, "resume_after_user_answer");
    assert_eq!(answer.kind, "orchestrator.resume_after_user_answer");
    assert!(answer.detail.contains("persisted project answer"));

    let fallback = runtime_resume_event_metadata(None);
    assert_eq!(fallback.trigger, "resume_pending");
    assert_eq!(fallback.kind, "orchestrator.resume_pending");
}

#[test]
fn runtime_orchestrator_allows_continue_only_when_running_and_uncontrolled() {
    assert!(runtime_orchestrator_allows_continue(&serde_json::json!({
        "status": "running",
        "control": { "paused": false, "takeover": false }
    })));
    assert!(!runtime_orchestrator_allows_continue(&serde_json::json!({
        "status": "ready"
    })));
    assert!(!runtime_orchestrator_allows_continue(&serde_json::json!({
        "status": "running",
        "control": { "paused": true, "takeover": false }
    })));
    assert!(!runtime_orchestrator_allows_continue(&serde_json::json!({
        "status": "running",
        "control": { "paused": false, "takeover": true }
    })));
}

#[test]
fn resume_runtime_orchestrator_reactivates_run_and_records_event() {
    let mut runtime = serde_json::json!({
        "orchestrator": {
            "run_id": "run-1",
            "active": false,
            "started_at": "2026-05-20T00:00:00Z"
        },
        "control": { "paused": true, "takeover": true },
        "timeline": []
    });

    resume_runtime_orchestrator(
        &mut runtime,
        "verify",
        "resume_after_user_answer",
        "orchestrator.resume_after_user_answer",
        "Run resumed after user answer",
        "Captain found a persisted project answer.",
        "user",
    );

    assert_eq!(runtime["status"], "running");
    assert_eq!(runtime["current_phase"], "verify");
    assert_eq!(
        runtime["progress"],
        serde_json::json!(runtime_progress_for_phase("verify", "running"))
    );
    assert_eq!(runtime["control"]["paused"], false);
    assert_eq!(runtime["control"]["takeover"], false);
    assert_eq!(runtime["orchestrator"]["run_id"], "run-1");
    assert_eq!(runtime["orchestrator"]["active"], true);
    assert_eq!(
        runtime["orchestrator"]["trigger"],
        "resume_after_user_answer"
    );
    let event = runtime["timeline"].as_array().unwrap().last().unwrap();
    assert_eq!(event["kind"], "orchestrator.resume_after_user_answer");
    assert_eq!(event["title"], "Run resumed after user answer");
    assert_eq!(event["actor"], "user");
    assert_eq!(event["phase"], "verify");
    assert_eq!(event["status"], "running");
    assert_eq!(event["data"]["run_id"], "run-1");
}

#[test]
fn mark_runtime_waiting_pauses_run_and_records_event() {
    let mut runtime = serde_json::json!({
        "orchestrator": {
            "run_id": "run-1",
            "active": true,
            "trigger": "start"
        },
        "timeline": []
    });

    mark_runtime_waiting(&mut runtime, "build", "run-1");

    assert_eq!(runtime["status"], "paused");
    assert_eq!(runtime["current_phase"], "build");
    assert_eq!(
        runtime["progress"],
        serde_json::json!(runtime_progress_for_phase("build", "paused"))
    );
    assert_eq!(runtime["orchestrator"]["active"], false);
    assert_eq!(runtime["orchestrator"]["stopped_reason"], "paused");
    let event = runtime["timeline"].as_array().unwrap().last().unwrap();
    assert_eq!(event["kind"], "orchestrator.waiting");
    assert_eq!(event["title"], "Run waiting");
    assert_eq!(event["actor"], "captain");
    assert_eq!(event["phase"], "build");
    assert_eq!(event["status"], "paused");
    assert_eq!(event["data"]["run_id"], "run-1");
}

#[test]
fn mark_runtime_dispatch_started_sets_observe_state_and_event() {
    let mut runtime = serde_json::json!({ "timeline": [] });

    mark_runtime_dispatch_started(&mut runtime, "run-42");

    assert_eq!(runtime["status"], "running");
    assert_eq!(runtime["current_phase"], "observe");
    assert_eq!(
        runtime["progress"],
        serde_json::json!(runtime_progress_for_phase("observe", "running"))
    );
    assert!(runtime["updated_at"].as_str().is_some());
    let event = runtime["timeline"].as_array().unwrap().last().unwrap();
    assert_eq!(event["kind"], "orchestrator.dispatch");
    assert_eq!(event["title"], "Worker dispatch started");
    assert_eq!(event["actor"], "captain");
    assert_eq!(event["phase"], "observe");
    assert_eq!(event["status"], "running");
    assert_eq!(event["data"]["run_id"], "run-42");
}

#[test]
fn mark_runtime_dispatch_started_initializes_missing_timeline() {
    let mut runtime = serde_json::json!({});

    mark_runtime_dispatch_started(&mut runtime, "run-43");

    let timeline = runtime["timeline"].as_array().unwrap();
    assert_eq!(timeline.len(), 1);
    assert_eq!(timeline[0]["kind"], "orchestrator.dispatch");
    assert_eq!(timeline[0]["data"]["run_id"], "run-43");
}

#[test]
fn mark_runtime_completed_sets_done_state_event_and_closes_questions() {
    let mut runtime = serde_json::json!({
        "orchestrator": {
            "run_id": "run-1",
            "active": true,
            "started_at": "2026-05-20T00:00:00Z"
        },
        "user_questions": [
            { "run_id": "run-1", "status": "pending", "delivery": "waiting_for_user" },
            { "run_id": "run-2", "status": "pending", "delivery": "waiting_for_user" },
            { "run_id": "run-1", "status": "answered", "delivery": "web" }
        ],
        "timeline": []
    });

    mark_runtime_completed(&mut runtime, "run-1", "project-1", "slug-1");

    assert_eq!(runtime["status"], "done");
    assert_eq!(runtime["current_phase"], "learn");
    assert_eq!(runtime["progress"], 100);
    assert_eq!(runtime["orchestrator"]["active"], false);
    assert_eq!(runtime["orchestrator"]["stopped_reason"], "completed");
    assert_eq!(runtime["user_questions"][0]["status"], "closed");
    assert_eq!(runtime["user_questions"][0]["delivery"], "run_completed");
    assert_eq!(runtime["user_questions"][1]["status"], "pending");
    assert_eq!(runtime["user_questions"][2]["status"], "answered");
    let event = runtime["timeline"].as_array().unwrap().last().unwrap();
    assert_eq!(event["kind"], "project.completed");
    assert_eq!(event["title"], "Autonomous run completed");
    assert_eq!(event["actor"], "captain");
    assert_eq!(event["phase"], "learn");
    assert_eq!(event["status"], "done");
    assert_eq!(event["data"]["run_id"], "run-1");
    assert_eq!(event["data"]["project_id"], "project-1");
    assert_eq!(event["data"]["slug"], "slug-1");
}

#[test]
fn mark_runtime_completed_initializes_missing_timeline() {
    let mut runtime = serde_json::json!({});

    mark_runtime_completed(&mut runtime, "run-2", "project-2", "slug-2");

    let timeline = runtime["timeline"].as_array().unwrap();
    assert_eq!(timeline.len(), 1);
    assert_eq!(timeline[0]["kind"], "project.completed");
    assert_eq!(timeline[0]["data"]["run_id"], "run-2");
    assert_eq!(runtime["orchestrator"]["active"], false);
    assert_eq!(runtime["orchestrator"]["stopped_reason"], "completed");
}

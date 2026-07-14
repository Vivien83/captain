use super::*;
use crate::project_lifecycle::runtime_progress_for_phase;
use captain_memory::project::{Project, ProjectStatus};

fn project() -> Project {
    Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Ship safely".to_string(),
        status: ProjectStatus::Active,
        deadline: None,
        created_at: 0,
        updated_at: 0,
        metadata: serde_json::json!({}),
    }
}

#[test]
fn mark_runtime_worker_skipped_sets_phase_progress_and_event() {
    let project = project();
    let spec = &RUNTIME_WORKER_SPECS[3];
    let mut runtime = serde_json::json!({
        "status": "running",
        "current_phase": "plan",
        "timeline": []
    });

    mark_runtime_worker_skipped(&mut runtime, &project, spec, "run-1");

    assert_eq!(runtime["status"], "running");
    assert_eq!(runtime["current_phase"], "build");
    assert_eq!(
        runtime["progress"],
        serde_json::json!(runtime_progress_for_phase("build", "running"))
    );
    let event = runtime["timeline"].as_array().unwrap().last().unwrap();
    assert_eq!(event["kind"], "worker.skipped");
    assert_eq!(event["title"], "builder already completed");
    assert_eq!(event["actor"], "captain");
    assert_eq!(event["phase"], "build");
    assert_eq!(event["status"], "done");
    assert_eq!(event["data"]["run_id"], "run-1");
    assert_eq!(event["data"]["worker_id"], "demo-build");
}

#[test]
fn mark_runtime_worker_skipped_initializes_missing_timeline() {
    let project = project();
    let spec = &RUNTIME_WORKER_SPECS[5];
    let mut runtime = serde_json::json!({});

    mark_runtime_worker_skipped(&mut runtime, &project, spec, "run-2");

    let timeline = runtime["timeline"].as_array().unwrap();
    assert_eq!(timeline.len(), 1);
    assert_eq!(timeline[0]["kind"], "worker.skipped");
    assert_eq!(timeline[0]["phase"], "verify");
    assert_eq!(timeline[0]["data"]["worker_id"], "demo-verify");
}

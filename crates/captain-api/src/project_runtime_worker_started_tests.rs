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
fn mark_runtime_worker_started_sets_runtime_worker_and_event() {
    let project = project();
    let spec = &RUNTIME_WORKER_SPECS[3];
    let tools = vec!["file_read".to_string(), "shell_exec".to_string()];
    let mut runtime = serde_json::json!({ "timeline": [] });

    mark_runtime_worker_started(&mut runtime, &project, spec, "run-1", "agent-1", &tools);

    assert_eq!(runtime["status"], "running");
    assert_eq!(runtime["current_phase"], "build");
    assert_eq!(
        runtime["progress"],
        serde_json::json!(runtime_progress_for_phase("build", "running"))
    );
    let worker = runtime["workers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|worker| worker["phase"] == "build")
        .unwrap();
    assert_eq!(worker["status"], "running");
    assert_eq!(worker["agent_id"], "agent-1");
    assert_eq!(worker["run_id"], "run-1");
    assert_eq!(worker["authorized_tools"], serde_json::json!(tools));
    assert!(worker["started_at"].as_str().is_some());
    let event = runtime["timeline"].as_array().unwrap().last().unwrap();
    assert_eq!(event["kind"], "worker.started");
    assert_eq!(event["title"], "builder started");
    assert_eq!(event["actor"], "agent-1");
    assert_eq!(event["phase"], "build");
    assert_eq!(event["status"], "running");
    assert_eq!(event["data"]["worker_id"], "demo-build");
    assert_eq!(event["data"]["authorized_tools"], serde_json::json!(tools));
}

#[test]
fn mark_runtime_worker_started_clears_phase_resume_pending() {
    let project = project();
    let spec = &RUNTIME_WORKER_SPECS[3];
    let mut runtime = serde_json::json!({
        "resume_pending": { "phase": "build", "reason": "project_ask_answered" },
        "worker_results": {
            "build": { "resume_pending": true, "summary": "old" }
        },
        "workers": [
            {
                "id": "demo-build",
                "phase": "build",
                "status": "ready",
                "resume_pending": true,
                "resume_after_user_answer": true
            }
        ]
    });

    mark_runtime_worker_started(&mut runtime, &project, spec, "run-2", "agent-2", &[]);

    assert!(runtime.get("resume_pending").is_none());
    assert!(runtime["workers"][0].get("resume_pending").is_none());
    assert!(runtime["workers"][0]
        .get("resume_after_user_answer")
        .is_none());
    assert!(runtime["worker_results"]["build"]
        .get("resume_pending")
        .is_none());
    assert_eq!(runtime["worker_results"]["build"]["summary"], "old");
}

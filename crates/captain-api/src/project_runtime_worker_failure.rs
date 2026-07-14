use crate::project_lifecycle::runtime_progress_for_phase;
use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_orchestrator::deactivate_runtime_orchestrator;
use crate::project_runtime_workers::{runtime_worker_id, upsert_runtime_worker, RuntimeWorkerSpec};
use captain_memory::project;
use chrono::Utc;
use serde_json::Value;

pub(crate) fn mark_runtime_worker_failed(
    runtime: &mut Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    agent_id: &str,
    error: &str,
) {
    let phase = spec.phase;
    upsert_runtime_worker(runtime, project, spec, |worker| {
        worker.insert("status".to_string(), serde_json::json!("failed"));
        worker.insert("agent_id".to_string(), serde_json::json!(agent_id));
        worker.insert(
            "completed_at".to_string(),
            serde_json::json!(Utc::now().to_rfc3339()),
        );
        worker.insert("error".to_string(), serde_json::json!(error));
    });
    runtime["status"] = serde_json::json!("blocked");
    runtime["progress"] = serde_json::json!(runtime_progress_for_phase(phase, "paused"));
    deactivate_runtime_orchestrator(runtime, "failed");
    append_runtime_event(
        runtime,
        "worker.failed",
        &format!("{} failed", spec.role),
        error,
        agent_id,
        phase,
        "failed",
        serde_json::json!({
            "run_id": run_id,
            "worker_id": runtime_worker_id(project, phase),
            "agent_id": agent_id,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_runtime_workers::RUNTIME_WORKER_SPECS;
    use captain_memory::project::ProjectStatus;

    fn project() -> project::Project {
        project::Project {
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
    fn mark_runtime_worker_failed_sets_worker_runtime_and_event() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let mut runtime = serde_json::json!({
            "workers": [{ "id": "demo-build", "phase": "build", "status": "running" }],
            "orchestrator": { "active": true, "run_id": "run-1" },
            "timeline": []
        });

        mark_runtime_worker_failed(
            &mut runtime,
            &project,
            spec,
            "run-1",
            "agent-1",
            "missing command",
        );

        let worker = &runtime["workers"][0];
        assert_eq!(worker["status"], "failed");
        assert_eq!(worker["agent_id"], "agent-1");
        assert_eq!(worker["error"], "missing command");
        assert!(worker["completed_at"].as_str().unwrap_or("").contains('T'));
        assert_eq!(runtime["status"], "blocked");
        assert_eq!(
            runtime["progress"],
            serde_json::json!(runtime_progress_for_phase("build", "paused"))
        );
        assert_eq!(runtime["orchestrator"]["active"], false);
        assert_eq!(runtime["orchestrator"]["stopped_reason"], "failed");
        assert_eq!(runtime["timeline"][0]["kind"], "worker.failed");
        assert_eq!(runtime["timeline"][0]["status"], "failed");
        assert_eq!(runtime["timeline"][0]["data"]["worker_id"], "demo-build");
    }

    #[test]
    fn mark_runtime_worker_failed_initializes_missing_worker_store() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let mut runtime = serde_json::json!({ "timeline": [] });

        mark_runtime_worker_failed(
            &mut runtime,
            &project,
            spec,
            "run-2",
            "agent-2",
            "agent loop failed",
        );

        let worker = runtime["workers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|worker| worker["id"] == "demo-build")
            .unwrap();
        assert_eq!(worker["status"], "failed");
        assert_eq!(worker["error"], "agent loop failed");
        assert_eq!(runtime["orchestrator"]["active"], false);
        assert_eq!(runtime["timeline"][0]["data"]["run_id"], "run-2");
    }
}

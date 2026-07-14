use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_workers::{runtime_worker_id, upsert_runtime_worker, RuntimeWorkerSpec};
use captain_memory::project;
use serde_json::Value;

pub(crate) struct RuntimeWorkerCleanup {
    pub(crate) status: &'static str,
    pub(crate) detail: &'static str,
    pub(crate) error: Option<String>,
    pub(crate) stopped_at: String,
}

pub(crate) fn runtime_worker_cleanup_success(stopped_at: String) -> RuntimeWorkerCleanup {
    RuntimeWorkerCleanup {
        status: "stopped",
        detail: "Captain stopped the completed worker agent after storing its result. The runtime keeps the summary and agent id for traceability.",
        error: None,
        stopped_at,
    }
}

pub(crate) fn runtime_worker_cleanup_failure(
    stopped_at: String,
    error: String,
) -> RuntimeWorkerCleanup {
    RuntimeWorkerCleanup {
        status: "cleanup_failed",
        detail: "Captain stored the worker result, but the completed worker agent could not be stopped automatically. Review active agents if it remains listed.",
        error: Some(error),
        stopped_at,
    }
}

pub(crate) fn mark_runtime_worker_cleaned(
    runtime: &mut Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    agent_id: &str,
    cleanup: &RuntimeWorkerCleanup,
) {
    let phase = spec.phase;
    upsert_runtime_worker(runtime, project, spec, |worker| {
        worker.insert(
            "cleanup_status".to_string(),
            serde_json::json!(cleanup.status),
        );
        worker.insert(
            "stopped_at".to_string(),
            serde_json::json!(cleanup.stopped_at),
        );
        if let Some(error) = cleanup.error.clone() {
            worker.insert("cleanup_error".to_string(), serde_json::json!(error));
        } else {
            worker.remove("cleanup_error");
        }
    });
    if runtime
        .pointer(&format!("/worker_results/{phase}"))
        .map(|value| value.is_object())
        .unwrap_or(false)
    {
        runtime["worker_results"][phase]["cleanup_status"] = serde_json::json!(cleanup.status);
        runtime["worker_results"][phase]["stopped_at"] = serde_json::json!(cleanup.stopped_at);
        runtime["worker_results"][phase]["cleanup_error"] =
            serde_json::json!(cleanup.error.clone());
    }
    append_runtime_event(
        runtime,
        "worker.cleaned",
        &format!("{} {}", spec.role, cleanup.status),
        cleanup.detail,
        "captain",
        phase,
        cleanup.status,
        serde_json::json!({
            "run_id": run_id,
            "worker_id": runtime_worker_id(project, phase),
            "agent_id": agent_id,
            "stopped_at": cleanup.stopped_at,
            "cleanup_error": cleanup.error.clone(),
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
    fn mark_runtime_worker_cleaned_records_stopped_cleanup() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let cleanup = runtime_worker_cleanup_success("2026-05-25T00:00:00Z".to_string());
        let mut runtime = serde_json::json!({
            "workers": [{
                "id": "demo-build",
                "phase": "build",
                "status": "done",
                "cleanup_error": "old"
            }],
            "worker_results": { "build": { "status": "done" } },
            "timeline": []
        });

        mark_runtime_worker_cleaned(&mut runtime, &project, spec, "run-1", "agent-1", &cleanup);

        assert_eq!(runtime["workers"][0]["cleanup_status"], "stopped");
        assert_eq!(runtime["workers"][0]["stopped_at"], "2026-05-25T00:00:00Z");
        assert!(runtime["workers"][0].get("cleanup_error").is_none());
        assert_eq!(
            runtime["worker_results"]["build"]["cleanup_status"],
            "stopped"
        );
        assert_eq!(
            runtime["worker_results"]["build"]["cleanup_error"],
            serde_json::Value::Null
        );
        assert_eq!(runtime["timeline"][0]["kind"], "worker.cleaned");
        assert_eq!(runtime["timeline"][0]["status"], "stopped");
        assert_eq!(runtime["timeline"][0]["actor"], "captain");
    }

    #[test]
    fn mark_runtime_worker_cleaned_records_cleanup_failure() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let cleanup = runtime_worker_cleanup_failure(
            "2026-05-25T00:01:00Z".to_string(),
            "agent still running".to_string(),
        );
        let mut runtime = serde_json::json!({
            "workers": [{ "id": "demo-build", "phase": "build", "status": "done" }],
            "worker_results": { "build": { "status": "done" } },
            "timeline": []
        });

        mark_runtime_worker_cleaned(&mut runtime, &project, spec, "run-2", "agent-2", &cleanup);

        assert_eq!(runtime["workers"][0]["cleanup_status"], "cleanup_failed");
        assert_eq!(
            runtime["workers"][0]["cleanup_error"],
            "agent still running"
        );
        assert_eq!(
            runtime["worker_results"]["build"]["cleanup_error"],
            "agent still running"
        );
        assert_eq!(runtime["timeline"][0]["status"], "cleanup_failed");
        assert_eq!(
            runtime["timeline"][0]["data"]["cleanup_error"],
            "agent still running"
        );
    }
}

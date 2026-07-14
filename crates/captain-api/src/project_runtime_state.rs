use crate::project_lifecycle::{
    is_valid_lifecycle_phase, lifecycle_from_metadata, runtime_progress_for_phase,
};
use crate::project_runtime_asks::ensure_runtime_question_store;
use crate::project_runtime_checkpoints::PROJECT_RUNTIME_PROTOCOL;
use crate::project_runtime_defaults::{default_project_parallelism, project_session_id};
use crate::project_runtime_events::runtime_event;
use crate::project_runtime_orchestrator::PROJECT_RUNTIME_GENERATION;
use crate::project_runtime_workers::runtime_workers_for_project;
use captain_memory::project;
use chrono::Utc;

pub(crate) fn project_runtime_state_for_project(
    project: &project::Project,
    status_override: Option<&str>,
    checkpoint_runtime: Option<serde_json::Value>,
    manager_agent: serde_json::Value,
) -> serde_json::Value {
    let mut runtime = select_project_runtime(project, checkpoint_runtime, &manager_agent);
    ensure_runtime_object(project, &mut runtime, &manager_agent);
    let phase = project_runtime_phase(project, &runtime);
    let status = project_runtime_status(&runtime, status_override);
    let progress = project_runtime_progress(&runtime, &phase, &status);
    hydrate_project_runtime(
        project,
        &mut runtime,
        &phase,
        &status,
        progress,
        &manager_agent,
    );
    ensure_runtime_question_store(&mut runtime);
    runtime
}

fn select_project_runtime(
    project: &project::Project,
    checkpoint_runtime: Option<serde_json::Value>,
    manager_agent: &serde_json::Value,
) -> serde_json::Value {
    project
        .metadata
        .get("runtime")
        .cloned()
        .filter(|value| value.is_object())
        .or_else(|| checkpoint_runtime.filter(|value| value.is_object()))
        .unwrap_or_else(|| default_project_runtime(project, "ready", "observe", manager_agent))
}

fn ensure_runtime_object(
    project: &project::Project,
    runtime: &mut serde_json::Value,
    manager_agent: &serde_json::Value,
) {
    if !runtime.is_object() {
        *runtime = default_project_runtime(project, "ready", "observe", manager_agent);
    }
}

fn project_runtime_phase(project: &project::Project, runtime: &serde_json::Value) -> String {
    let metadata_phase = lifecycle_from_metadata(&project.metadata)
        .get("current_phase")
        .and_then(|value| value.as_str())
        .unwrap_or("observe")
        .to_string();
    runtime
        .get("current_phase")
        .and_then(|value| value.as_str())
        .filter(|phase| is_valid_lifecycle_phase(phase))
        .map(str::to_string)
        .unwrap_or(metadata_phase)
}

fn project_runtime_status(runtime: &serde_json::Value, status_override: Option<&str>) -> String {
    status_override
        .or_else(|| runtime.get("status").and_then(|value| value.as_str()))
        .unwrap_or("ready")
        .to_string()
}

fn project_runtime_progress(runtime: &serde_json::Value, phase: &str, status: &str) -> u64 {
    runtime
        .get("progress")
        .and_then(|value| value.as_u64())
        .unwrap_or_else(|| runtime_progress_for_phase(phase, status))
}

fn hydrate_project_runtime(
    project: &project::Project,
    runtime: &mut serde_json::Value,
    phase: &str,
    status: &str,
    progress: u64,
    manager_agent: &serde_json::Value,
) {
    let now = Utc::now().to_rfc3339();
    if let Some(obj) = runtime.as_object_mut() {
        hydrate_runtime_identity(obj, project, phase, status, progress, &now, manager_agent);
        hydrate_runtime_state_defaults(obj, project, status);
        hydrate_runtime_timeline(obj, phase, status);
    }
}

fn hydrate_runtime_identity(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    project: &project::Project,
    phase: &str,
    status: &str,
    progress: u64,
    now: &str,
    manager_agent: &serde_json::Value,
) {
    obj.insert(
        "protocol".to_string(),
        serde_json::json!(PROJECT_RUNTIME_PROTOCOL),
    );
    obj.insert(
        "version".to_string(),
        serde_json::json!(PROJECT_RUNTIME_GENERATION),
    );
    obj.insert("status".to_string(), serde_json::json!(status));
    obj.insert("current_phase".to_string(), serde_json::json!(phase));
    obj.insert("progress".to_string(), serde_json::json!(progress));
    obj.entry("created_at".to_string())
        .or_insert_with(|| serde_json::json!(now));
    obj.insert("updated_at".to_string(), serde_json::json!(now));
    obj.insert(
        "session_id".to_string(),
        serde_json::json!(project_session_id(project)),
    );
    obj.insert("manager_agent".to_string(), manager_agent.clone());
}

fn hydrate_runtime_state_defaults(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    project: &project::Project,
    status: &str,
) {
    obj.entry("parallelism".to_string()).or_insert_with(|| {
        serde_json::json!({
            "max_parallel_agents": default_project_parallelism(),
            "running": 0,
            "policy": "same_provider_model_fit",
        })
    });
    obj.entry("workers".to_string())
        .or_insert_with(|| runtime_workers_for_project(project));
    obj.entry("worker_results".to_string())
        .or_insert_with(|| serde_json::json!({}));
    obj.entry("user_questions".to_string())
        .or_insert_with(|| serde_json::json!([]));
    obj.entry("control".to_string()).or_insert_with(|| {
        serde_json::json!({
            "paused": status == "paused",
            "takeover": false,
        })
    });
    obj.entry("orchestrator".to_string()).or_insert_with(|| {
        serde_json::json!({
            "generation": PROJECT_RUNTIME_GENERATION,
            "active": false,
            "strategy": "multi_agent_parallel_gated",
            "manager": "captain",
        })
    });
}

fn hydrate_runtime_timeline(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    phase: &str,
    status: &str,
) {
    obj.entry("timeline".to_string()).or_insert_with(|| {
        serde_json::json!([runtime_event(
            "project.ready",
            "Project runtime ready",
            "Captain has enough project context to start an autonomous development run.",
            "captain",
            phase,
            status,
            serde_json::json!({})
        )])
    });
}

fn default_project_runtime(
    project: &project::Project,
    status: &str,
    phase: &str,
    manager_agent: &serde_json::Value,
) -> serde_json::Value {
    let now = Utc::now().to_rfc3339();
    serde_json::json!({
        "protocol": PROJECT_RUNTIME_PROTOCOL,
        "version": PROJECT_RUNTIME_GENERATION,
        "status": status,
        "current_phase": phase,
        "progress": runtime_progress_for_phase(phase, status),
        "created_at": now,
        "updated_at": now,
        "session_id": project_session_id(project),
        "manager_agent": manager_agent,
        "parallelism": {
            "max_parallel_agents": default_project_parallelism(),
            "running": 0,
            "policy": "same_provider_model_fit",
        },
        "workers": runtime_workers_for_project(project),
        "worker_results": {},
        "user_questions": [],
        "control": {
            "paused": status == "paused",
            "takeover": false,
        },
        "orchestrator": {
            "generation": PROJECT_RUNTIME_GENERATION,
            "active": false,
            "strategy": "multi_agent_parallel_gated",
            "manager": "captain",
        },
        "timeline": [
            runtime_event(
                "project.ready",
                "Project runtime ready",
                "Captain has enough project context to start an autonomous development run.",
                "captain",
                phase,
                status,
                serde_json::json!({})
            )
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::project::ProjectStatus;

    fn project(metadata: serde_json::Value) -> project::Project {
        project::Project {
            id: "project-1".to_string(),
            name: "Demo Project".to_string(),
            slug: "demo-project".to_string(),
            goal: "Ship safely".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata,
        }
    }

    fn manager_agent() -> serde_json::Value {
        serde_json::json!({
            "id": "captain-id",
            "name": "captain",
            "provider": "codex",
            "model": "gpt-5.5",
            "role": "project_manager"
        })
    }

    #[test]
    fn runtime_state_prefers_metadata_runtime_before_checkpoint() {
        let project = project(serde_json::json!({
            "runtime": {
                "status": "running",
                "current_phase": "build",
                "progress": 55
            }
        }));
        let checkpoint = Some(serde_json::json!({
            "status": "done",
            "current_phase": "learn",
            "progress": 100
        }));

        let runtime =
            project_runtime_state_for_project(&project, None, checkpoint, manager_agent());

        assert_eq!(runtime["status"], "running");
        assert_eq!(runtime["current_phase"], "build");
        assert_eq!(runtime["progress"], 55);
        assert_eq!(runtime["session_id"], "project-demo-project");
        assert_eq!(runtime["manager_agent"]["id"], "captain-id");
        assert!(runtime["workers"].as_array().unwrap().len() >= 7);
        assert!(runtime["user_questions"].as_array().unwrap().is_empty());
        assert_eq!(runtime["timeline"][0]["kind"], "project.ready");
    }

    #[test]
    fn runtime_state_uses_checkpoint_when_metadata_runtime_is_missing() {
        let project = project(serde_json::json!({}));
        let checkpoint = Some(serde_json::json!({
            "status": "blocked",
            "current_phase": "verify"
        }));

        let runtime =
            project_runtime_state_for_project(&project, None, checkpoint, manager_agent());

        assert_eq!(runtime["status"], "blocked");
        assert_eq!(runtime["current_phase"], "verify");
        assert_eq!(runtime["progress"], 88);
        assert_eq!(runtime["manager_agent"]["role"], "project_manager");
    }

    #[test]
    fn runtime_state_falls_back_to_default_when_runtime_sources_are_invalid() {
        let project = project(serde_json::json!({
            "runtime": "corrupt",
            "lifecycle": {"current_phase": "learn"}
        }));
        let checkpoint = Some(serde_json::json!("also-corrupt"));

        let runtime =
            project_runtime_state_for_project(&project, None, checkpoint, manager_agent());

        assert_eq!(runtime["status"], "ready");
        assert_eq!(runtime["current_phase"], "observe");
        assert_eq!(
            runtime["progress"],
            serde_json::json!(runtime_progress_for_phase("observe", "ready"))
        );
        assert_eq!(runtime["session_id"], "project-demo-project");
        assert_eq!(runtime["timeline"][0]["kind"], "project.ready");
        assert!(runtime["workers"].as_array().unwrap().len() >= 7);
    }

    #[test]
    fn runtime_state_applies_status_override_and_lifecycle_phase_fallback() {
        let project = project(serde_json::json!({
            "lifecycle": {
                "current_phase": "execute"
            },
            "runtime": {
                "status": "ready"
            }
        }));

        let runtime =
            project_runtime_state_for_project(&project, Some("paused"), None, manager_agent());

        assert_eq!(runtime["status"], "paused");
        assert_eq!(runtime["current_phase"], "execute");
        assert_eq!(runtime["progress"], 70);
        assert_eq!(runtime["control"]["paused"], true);
    }
}

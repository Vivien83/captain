use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_worker_manifest::runtime_worker_agent_name;
use crate::project_runtime_workers::{runtime_worker_id, upsert_runtime_worker, RuntimeWorkerSpec};
use crate::routes::AppState;
use captain_memory::project;
use captain_types::agent::AgentEntry;
use std::sync::Arc;

pub(crate) fn mark_runtime_worker_recovered(
    runtime: &mut serde_json::Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    cleared_agent_id: Option<&str>,
) {
    let phase = spec.phase;
    upsert_runtime_worker(runtime, project, spec, |worker| {
        worker.insert("status".to_string(), serde_json::json!("ready"));
        worker.insert("agent_id".to_string(), serde_json::Value::Null);
        worker.insert(
            "recovered_from_stale_run".to_string(),
            serde_json::json!(true),
        );
        if let Some(agent_id) = cleared_agent_id {
            worker.insert(
                "recovered_agent_id".to_string(),
                serde_json::json!(agent_id),
            );
        }
    });
    let mut data = serde_json::json!({
        "run_id": run_id,
        "worker_id": runtime_worker_id(project, phase),
    });
    if let Some(agent_id) = cleared_agent_id {
        data["cleared_agent_id"] = serde_json::json!(agent_id);
    }
    append_runtime_event(
        runtime,
        "worker.recovered",
        &format!("{} recovered", spec.role),
        "A previous process left this worker marked running. Captain is relaunching it with a fresh sub-agent.",
        "captain",
        phase,
        "ready",
        data,
    );
}

pub(crate) fn clear_stale_runtime_worker_agent(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
) -> Result<Option<String>, String> {
    let agent_name = runtime_worker_agent_name(project, spec);
    let Some(entry) = state.kernel.registry.find_by_name(&agent_name) else {
        return Ok(None);
    };
    if !matches_runtime_worker_agent(&entry, project, spec) {
        return Err(stale_worker_agent_collision_error(&agent_name, spec.phase));
    }

    let agent_id = entry.id;
    state.kernel.kill_agent(agent_id).map_err(|error| {
        stale_worker_agent_cleanup_error(spec.phase, &agent_id.to_string(), &error.to_string())
    })?;
    Ok(Some(agent_id.to_string()))
}

fn matches_runtime_worker_agent(
    entry: &AgentEntry,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
) -> bool {
    let tags_match = agent_entry_has_tag(entry, "project-runtime")
        && agent_entry_has_tag(entry, &format!("project:{}", project.slug))
        && agent_entry_has_tag(entry, &format!("phase:{}", spec.phase));
    let metadata_match = entry
        .manifest
        .metadata
        .get("project_id")
        .and_then(|v| v.as_str())
        == Some(project.id.as_str())
        && entry
            .manifest
            .metadata
            .get("project_slug")
            .and_then(|v| v.as_str())
            == Some(project.slug.as_str())
        && entry
            .manifest
            .metadata
            .get("runtime_phase")
            .and_then(|v| v.as_str())
            == Some(spec.phase);
    tags_match || metadata_match
}

fn agent_entry_has_tag(entry: &AgentEntry, tag: &str) -> bool {
    entry.tags.iter().any(|candidate| candidate == tag)
}

fn stale_worker_agent_collision_error(agent_name: &str, phase: &str) -> String {
    format!(
        "stale {phase} worker recovery found existing agent '{agent_name}', but it is not owned by this project runtime; manual review is required"
    )
}

fn stale_worker_agent_cleanup_error(phase: &str, agent_id: &str, error: &str) -> String {
    format!("failed to clear stale {phase} worker agent {agent_id}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_runtime_worker_manifest::runtime_worker_manifest_for_project;
    use crate::project_runtime_workers::RUNTIME_WORKER_SPECS;
    use captain_kernel::CaptainKernel;
    use captain_memory::project::ProjectStatus;
    use captain_types::agent::{AgentManifest, ModelConfig};
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;

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

    fn model() -> ModelConfig {
        ModelConfig {
            provider: "codex".to_string(),
            model: "gpt-5.4-mini".to_string(),
            system_prompt: "worker prompt".to_string(),
            ..Default::default()
        }
    }

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "codex".to_string(),
                model: "gpt-5.4-mini".to_string(),
                api_key_env: String::new(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();
        let state = AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        };
        (tmp, Arc::new(state))
    }

    #[test]
    fn mark_runtime_worker_recovered_sets_worker_ready_and_event() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let mut runtime = serde_json::json!({
            "workers": [
                { "id": "demo-build", "phase": "build", "status": "running", "agent_id": "stale-agent-1" }
            ],
            "timeline": []
        });

        mark_runtime_worker_recovered(&mut runtime, &project, spec, "run-1", Some("stale-agent-1"));

        assert_eq!(runtime["workers"][0]["status"], "ready");
        assert!(runtime["workers"][0]["agent_id"].is_null());
        assert_eq!(runtime["workers"][0]["recovered_from_stale_run"], true);
        assert_eq!(runtime["workers"][0]["recovered_agent_id"], "stale-agent-1");
        let event = runtime["timeline"].as_array().unwrap().last().unwrap();
        assert_eq!(event["kind"], "worker.recovered");
        assert_eq!(event["title"], "builder recovered");
        assert_eq!(event["actor"], "captain");
        assert_eq!(event["phase"], "build");
        assert_eq!(event["status"], "ready");
        assert_eq!(event["data"]["run_id"], "run-1");
        assert_eq!(event["data"]["worker_id"], "demo-build");
        assert_eq!(event["data"]["cleared_agent_id"], "stale-agent-1");
    }

    #[test]
    fn mark_runtime_worker_recovered_initializes_missing_worker_store() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[5];
        let mut runtime = serde_json::json!({});

        mark_runtime_worker_recovered(&mut runtime, &project, spec, "run-2", None);

        let worker = runtime["workers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|worker| worker["phase"] == "verify")
            .unwrap();
        assert_eq!(worker["status"], "ready");
        assert_eq!(worker["recovered_from_stale_run"], true);
        assert_eq!(runtime["timeline"][0]["kind"], "worker.recovered");
        assert_eq!(runtime["timeline"][0]["data"]["worker_id"], "demo-verify");
        assert!(runtime["timeline"][0]["data"]
            .get("cleared_agent_id")
            .is_none());
    }

    #[test]
    fn clear_stale_runtime_worker_agent_removes_matching_runtime_worker() {
        let (_tmp, state) = test_state();
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[0];
        let manifest = runtime_worker_manifest_for_project(
            &project,
            spec,
            model(),
            vec!["file_read".to_string()],
            state
                .kernel
                .config
                .effective_workspaces_dir()
                .join("demo-observe"),
            None,
        );
        let agent_name = manifest.name.clone();
        let agent_id = state.kernel.spawn_agent(manifest).unwrap();

        assert!(state.kernel.registry.find_by_name(&agent_name).is_some());
        let cleared =
            clear_stale_runtime_worker_agent(&state, &project, spec).expect("stale agent cleared");

        assert_eq!(cleared, Some(agent_id.to_string()));
        assert!(state.kernel.registry.find_by_name(&agent_name).is_none());
        assert!(state.kernel.memory.load_agent(agent_id).unwrap().is_none());
    }

    #[test]
    fn clear_stale_runtime_worker_agent_refuses_foreign_name_collision() {
        let (_tmp, state) = test_state();
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[0];
        let agent_name = runtime_worker_agent_name(&project, spec);
        let manifest = AgentManifest {
            name: agent_name.clone(),
            module: "builtin:chat".to_string(),
            ..Default::default()
        };
        let agent_id = state.kernel.spawn_agent(manifest).unwrap();

        let error = clear_stale_runtime_worker_agent(&state, &project, spec).unwrap_err();

        assert!(error.contains("manual review is required"));
        assert!(state.kernel.registry.get(agent_id).is_some());
    }
}

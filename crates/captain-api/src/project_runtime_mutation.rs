use crate::project_lifecycle::{is_valid_lifecycle_phase, set_lifecycle_phase};
use crate::project_metadata::metadata_set_runtime;
use crate::project_runtime_checkpoints::latest_checkpoint_runtime;
use crate::project_runtime_defaults::project_session_id;
use crate::project_runtime_events::{new_runtime_timeline_events, runtime_timeline_event_ids};
use crate::project_runtime_state::project_runtime_state_for_project;
use crate::project_runtime_workers::recompute_runtime_parallelism;
use crate::routes::AppState;
use captain_memory::project;
use captain_types::agent::AgentId;
use captain_types::event::{Event, EventPayload, EventTarget};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

static PROJECT_RUNTIME_STATE_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

pub(crate) fn project_runtime_json(
    state: &AppState,
    project: &project::Project,
    status_override: Option<&str>,
) -> Value {
    let checkpoint_runtime = if project
        .metadata
        .get("runtime")
        .map(|value| value.is_object())
        .unwrap_or(false)
    {
        None
    } else {
        latest_checkpoint_runtime(state, project)
    };
    project_runtime_state_for_project(
        project,
        status_override,
        checkpoint_runtime,
        captain_manager_json(state),
    )
}

pub(crate) async fn update_project_runtime_state<F>(
    state: &Arc<AppState>,
    project_id: &str,
    mutate: F,
) -> Result<project::Project, String>
where
    F: FnOnce(&mut Value, &project::Project) + Send,
{
    let (updated, runtime_status, phase, new_events) = {
        let _guard = PROJECT_RUNTIME_STATE_LOCK.lock().await;
        let project = state
            .kernel
            .memory
            .project_get(project_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("project '{project_id}' not found"))?;
        let mut runtime = project_runtime_json(state, &project, None);
        let before_event_ids = runtime_event_ids_before_mutation(&project, &runtime);
        mutate(&mut runtime, &project);
        runtime_update_for_project(state, &project, runtime, before_event_ids)?
    };

    persist_project_runtime_events(state, &updated, &new_events);
    set_active_project_if_runtime_available(state, &updated, &runtime_status);
    publish_project_runtime_updated_event(state, &updated, &runtime_status, &phase).await;
    Ok(updated)
}

pub(crate) fn captain_agent_id(state: &AppState) -> Option<String> {
    state
        .kernel
        .registry
        .list()
        .into_iter()
        .find(|entry| entry.name.eq_ignore_ascii_case("captain"))
        .or_else(|| state.kernel.registry.list().into_iter().next())
        .map(|entry| entry.id.to_string())
}

pub(crate) fn captain_manager_json(state: &AppState) -> Value {
    let default_model = state.kernel.effective_default_model();
    let captain = state
        .kernel
        .registry
        .list()
        .into_iter()
        .find(|entry| entry.name.eq_ignore_ascii_case("captain"))
        .or_else(|| state.kernel.registry.list().into_iter().next());
    if let Some(entry) = captain {
        let provider = if entry.manifest.model.provider.is_empty()
            || entry.manifest.model.provider == "default"
        {
            default_model.provider.clone()
        } else {
            entry.manifest.model.provider.clone()
        };
        let model =
            if entry.manifest.model.model.is_empty() || entry.manifest.model.model == "default" {
                default_model.model.clone()
            } else {
                entry.manifest.model.model.clone()
            };
        serde_json::json!({
            "id": entry.id.to_string(),
            "name": entry.name,
            "provider": provider,
            "model": model,
            "role": "project_manager",
        })
    } else {
        serde_json::json!({
            "id": Value::Null,
            "name": "captain",
            "provider": default_model.provider,
            "model": default_model.model,
            "role": "project_manager",
        })
    }
}

fn runtime_update_for_project(
    state: &AppState,
    project: &project::Project,
    mut runtime: Value,
    before_event_ids: HashSet<String>,
) -> Result<(project::Project, String, String, Vec<Value>), String> {
    recompute_runtime_parallelism(&mut runtime);
    let new_events = new_runtime_timeline_events(&before_event_ids, &runtime);
    let (phase, runtime_status) = runtime_phase_and_status(&runtime);
    let mut metadata = set_lifecycle_phase(project.metadata.clone(), &phase);
    metadata_set_runtime(&mut metadata, runtime);
    let updated = state
        .kernel
        .memory
        .project_update(
            &project.id,
            project::ProjectPatch {
                status: runtime_status_to_project_status(&runtime_status),
                metadata: Some(metadata),
                ..Default::default()
            },
        )
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("project '{}' not found", project.id))?;
    Ok((updated, runtime_status, phase, new_events))
}

fn runtime_event_ids_before_mutation(
    project: &project::Project,
    runtime: &Value,
) -> HashSet<String> {
    if project
        .metadata
        .get("runtime")
        .map(|value| value.is_object())
        .unwrap_or(false)
    {
        runtime_timeline_event_ids(runtime)
    } else {
        HashSet::new()
    }
}

fn runtime_phase_and_status(runtime: &Value) -> (String, String) {
    let phase = runtime
        .get("current_phase")
        .and_then(|value| value.as_str())
        .filter(|phase| is_valid_lifecycle_phase(phase))
        .unwrap_or("observe")
        .to_string();
    let runtime_status = runtime
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("ready")
        .to_string();
    (phase, runtime_status)
}

fn runtime_status_to_project_status(status: &str) -> Option<project::ProjectStatus> {
    match status {
        "paused" => Some(project::ProjectStatus::Paused),
        "done" => Some(project::ProjectStatus::Done),
        "running" | "ready" | "blocked" => Some(project::ProjectStatus::Active),
        _ => None,
    }
}

fn persist_project_runtime_events(state: &AppState, project: &project::Project, events: &[Value]) {
    if events.is_empty() {
        return;
    }
    let session_id = project_session_id(project);
    for event in events {
        let payload = serde_json::json!({
            "project_id": project.id,
            "project_slug": project.slug,
            "project_name": project.name,
            "event": event,
        });
        if let Err(err) =
            state
                .kernel
                .memory
                .append_session_event(&session_id, "project_runtime_event", &payload)
        {
            tracing::warn!(
                project_id = %project.id,
                session_id = %session_id,
                error = %err,
                "failed to persist project runtime event"
            );
        }
    }
}

fn set_active_project_if_runtime_available(
    state: &AppState,
    project: &project::Project,
    runtime_status: &str,
) {
    if matches!(runtime_status, "running" | "ready") {
        if let Some(agent_id) = captain_agent_id(state) {
            if let Some(reg) = captain_runtime::active_project::global() {
                reg.set(agent_id, project.slug.clone());
            }
        }
    }
}

async fn publish_project_runtime_updated_event(
    state: &AppState,
    project: &project::Project,
    runtime_status: &str,
    phase: &str,
) {
    if let Ok(bytes) = serde_json::to_vec(&serde_json::json!({
        "event": "project.runtime.updated",
        "project_id": project.id,
        "slug": project.slug,
        "status": runtime_status,
        "phase": phase,
    })) {
        state
            .kernel
            .publish_event(Event::new(
                AgentId::new(),
                EventTarget::Broadcast,
                EventPayload::Custom(bytes),
            ))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
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

    fn create_project(state: &AppState) -> project::Project {
        state
            .kernel
            .memory
            .project_create(project::NewProject {
                name: "Demo Project".to_string(),
                slug: "demo-project".to_string(),
                goal: "Ship safely".to_string(),
                deadline: None,
            })
            .unwrap()
    }

    #[test]
    fn runtime_status_to_project_status_matches_operator_lifecycle() {
        assert_eq!(
            runtime_status_to_project_status("paused"),
            Some(project::ProjectStatus::Paused)
        );
        assert_eq!(
            runtime_status_to_project_status("done"),
            Some(project::ProjectStatus::Done)
        );
        assert_eq!(
            runtime_status_to_project_status("running"),
            Some(project::ProjectStatus::Active)
        );
        assert_eq!(runtime_status_to_project_status("mystery"), None);
    }

    #[test]
    fn runtime_phase_and_status_rejects_unknown_phase() {
        let runtime = serde_json::json!({
            "current_phase": "unknown",
            "status": "blocked",
        });

        assert_eq!(
            runtime_phase_and_status(&runtime),
            ("observe".to_string(), "blocked".to_string())
        );
    }

    #[tokio::test]
    async fn update_project_runtime_state_persists_status_and_lifecycle() {
        let (_tmp, state) = test_state();
        let project = create_project(&state);

        let updated = update_project_runtime_state(&state, &project.id, |runtime, _project| {
            runtime["status"] = serde_json::json!("paused");
            runtime["current_phase"] = serde_json::json!("verify");
            runtime["timeline"] = serde_json::json!([
                {"id": "evt-1", "ts": "2026-05-25T00:00:00Z", "title": "Paused"}
            ]);
        })
        .await
        .unwrap();

        assert_eq!(updated.status, project::ProjectStatus::Paused);
        assert_eq!(updated.metadata["lifecycle"]["current_phase"], "verify");
        assert_eq!(updated.metadata["runtime"]["status"], "paused");
        assert_eq!(updated.metadata["runtime"]["current_phase"], "verify");
    }

    #[test]
    fn captain_manager_json_projects_current_manager_identity() {
        let (_tmp, state) = test_state();

        let manager = captain_manager_json(&state);

        assert!(manager["id"].as_str().is_some_and(|id| !id.is_empty()));
        assert!(manager["name"]
            .as_str()
            .is_some_and(|name| !name.is_empty()));
        assert_eq!(manager["provider"], "ollama");
        assert_eq!(manager["model"], "test-model");
        assert_eq!(manager["role"], "project_manager");
    }
}

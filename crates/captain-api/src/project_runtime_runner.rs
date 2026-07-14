use crate::project_lifecycle::runtime_progress_for_phase;
use crate::project_lookup_input::normalize_project_lookup_key;
use crate::project_runtime_checkpoints::{append_runtime_checkpoint, trim_runtime_text};
use crate::project_runtime_defaults::project_session_id;
use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_mutation::{project_runtime_json, update_project_runtime_state};
use crate::project_runtime_orchestrator::{
    deactivate_runtime_orchestrator, mark_runtime_completed, mark_runtime_dispatch_started,
    runtime_run_id,
};
use crate::project_runtime_worker_phase_finish::{
    finish_failed_project_worker_phase, finish_successful_project_worker_phase,
};
use crate::project_runtime_worker_phase_start::{
    prepare_project_worker_phase_start, ProjectWorkerPhaseStart,
};
use crate::project_runtime_worker_spawn::start_project_runtime_worker_agent;
use crate::project_runtime_worker_turn::run_project_worker_turn;
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use captain_memory::project;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};

static PROJECT_RUNTIME_RUNS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub(crate) fn project_runtime_is_running(project_id: &str) -> bool {
    PROJECT_RUNTIME_RUNS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(project_id)
}

fn project_runtime_mark_started(project_id: &str) -> bool {
    let mut guard = PROJECT_RUNTIME_RUNS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.insert(project_id.to_string())
}

fn project_runtime_mark_finished(project_id: &str) {
    let mut guard = PROJECT_RUNTIME_RUNS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard.remove(project_id);
}

fn resolve_project(
    state: &AppState,
    id_or_slug: &str,
) -> Result<Option<project::Project>, captain_types::error::CaptainError> {
    match state.kernel.memory.project_get(id_or_slug) {
        Ok(Some(project)) => Ok(Some(project)),
        Ok(None) => state.kernel.memory.project_find_by_slug(id_or_slug),
        Err(e) => Err(e),
    }
}

pub(crate) fn spawn_project_runtime_if_needed(state: Arc<AppState>, id_or_slug: String) {
    let id_or_slug = match normalize_project_lookup_key(&id_or_slug) {
        Ok(id_or_slug) => id_or_slug,
        Err(_) => {
            tracing::warn!("project runtime start skipped: invalid project identifier");
            return;
        }
    };
    let project = match resolve_project(&state, &id_or_slug) {
        Ok(Some(project)) => project,
        Ok(None) => {
            tracing::warn!("project runtime start skipped: project not found");
            return;
        }
        Err(e) => {
            tracing::warn!(error = %safe_project_storage_error(&e.to_string()), "project runtime start skipped");
            return;
        }
    };
    if !project_runtime_mark_started(&project.id) {
        return;
    }
    let project_id = project.id.clone();
    let runtime = project_runtime_json(&state, &project, None);
    let run_id = runtime_run_id(&runtime).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    tokio::spawn(async move {
        let result =
            run_project_runtime_v2(state.clone(), project_id.clone(), run_id.clone()).await;
        project_runtime_mark_finished(&project_id);
        if let Err(e) = result {
            let detail = trim_runtime_text(&e, 900);
            let _ = update_project_runtime_state(&state, &project_id, |runtime, _project| {
                let phase = runtime
                    .get("current_phase")
                    .and_then(|v| v.as_str())
                    .unwrap_or("observe")
                    .to_string();
                runtime["status"] = serde_json::json!("blocked");
                runtime["progress"] =
                    serde_json::json!(runtime_progress_for_phase(&phase, "paused"));
                deactivate_runtime_orchestrator(runtime, "blocked");
                append_runtime_event(
                    runtime,
                    "orchestrator.blocked",
                    "Autonomous run blocked",
                    &detail,
                    "captain",
                    &phase,
                    "blocked",
                    serde_json::json!({ "run_id": run_id.clone() }),
                );
            })
            .await;
        }
    });
}

async fn run_project_runtime_v2(
    state: Arc<AppState>,
    project_id: String,
    run_id: String,
) -> Result<(), String> {
    update_project_runtime_state(&state, &project_id, |runtime, _project| {
        mark_runtime_dispatch_started(runtime, &run_id);
    })
    .await?;

    let observe =
        run_project_worker_phase(state.clone(), project_id.clone(), run_id.clone(), "observe");
    let think =
        run_project_worker_phase(state.clone(), project_id.clone(), run_id.clone(), "think");
    let (observe_result, think_result) = tokio::join!(observe, think);
    if !observe_result? || !think_result? {
        return Ok(());
    }

    for phase in ["plan", "build", "execute", "verify", "learn"] {
        if !run_project_worker_phase(state.clone(), project_id.clone(), run_id.clone(), phase)
            .await?
        {
            return Ok(());
        }
    }

    let updated = update_project_runtime_state(&state, &project_id, |runtime, project| {
        mark_runtime_completed(runtime, &run_id, &project.id, &project.slug);
    })
    .await?;

    let runtime = project_runtime_json(&state, &updated, None);
    append_runtime_checkpoint(
        &state,
        &updated,
        &runtime,
        &run_id,
        project_session_id(&updated),
    );
    crate::project_ask::expire_project_asks_for_run(&project_id);
    Ok(())
}

async fn run_project_worker_phase(
    state: Arc<AppState>,
    project_id: String,
    run_id: String,
    phase: &'static str,
) -> Result<bool, String> {
    let phase_start =
        prepare_project_worker_phase_start(&state, &project_id, &run_id, phase).await?;
    let phase_start = match phase_start {
        ProjectWorkerPhaseStart::Waiting => return Ok(false),
        ProjectWorkerPhaseStart::SkippedDone => return Ok(true),
        ProjectWorkerPhaseStart::Ready(phase_start) => phase_start,
    };
    let worker = start_project_runtime_worker_agent(
        &state,
        &phase_start.project,
        phase_start.spec,
        &run_id,
        phase,
        &phase_start.runtime_snapshot,
    )
    .await?;
    let result = run_project_worker_turn(
        state.clone(),
        phase_start.project.clone(),
        phase_start.spec,
        &run_id,
        phase,
        worker.agent_id,
        worker.prompt,
    )
    .await;

    match result {
        Ok(result) => {
            finish_successful_project_worker_phase(
                &state,
                &phase_start.project,
                phase_start.spec,
                &run_id,
                phase,
                worker.agent_id,
                result,
                &phase_start.runtime_snapshot,
            )
            .await
        }
        Err(error) => {
            finish_failed_project_worker_phase(
                &state,
                &phase_start.project,
                phase_start.spec,
                &run_id,
                phase,
                worker.agent_id,
                &error.to_string(),
            )
            .await
        }
    }
}

#[cfg(test)]
#[path = "project_runtime_runner_tests.rs"]
mod project_runtime_runner_tests;

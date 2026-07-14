use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_mutation::update_project_runtime_state;
use crate::project_runtime_stream_ask::record_project_worker_ask_user;
use crate::project_runtime_stream_events::project_worker_stream_event;
use crate::project_runtime_workers::RuntimeWorkerSpec;
use crate::routes::AppState;
use captain_memory::project;
use captain_runtime::kernel_handle::KernelHandle;
use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::AgentId;
use chrono::Utc;
use std::sync::Arc;

pub(crate) async fn run_project_worker_turn(
    state: Arc<AppState>,
    project: project::Project,
    spec: &'static RuntimeWorkerSpec,
    run_id: &str,
    phase: &'static str,
    agent_id: AgentId,
    prompt: String,
) -> Result<captain_runtime::agent_loop::AgentLoopResult, String> {
    let handle: Arc<dyn KernelHandle> = state.kernel.clone();
    let (mut rx, join, user_input_tx) = state
        .kernel
        .send_message_streaming(
            agent_id,
            &prompt,
            Some(handle),
            Some(format!("project:{}", project.slug)),
            Some(format!("Project {}", project.name)),
            None,
            Some("project".to_string()),
        )
        .map_err(|e| format!("{e}"))?;

    while let Some(event) = rx.recv().await {
        if let Err(e) = record_project_stream_event(
            &state,
            &project,
            spec,
            run_id,
            phase,
            agent_id,
            event,
            user_input_tx.clone(),
        )
        .await
        {
            tracing::warn!(
                project_id = %project.id,
                phase = phase,
                agent_id = %agent_id,
                "project stream event recording failed: {e}"
            );
        }
    }

    join.await
        .map_err(|e| format!("project worker task join failed: {e}"))?
        .map_err(|e| format!("{e}"))
}

#[allow(clippy::too_many_arguments)]
async fn record_project_stream_event(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    phase: &str,
    agent_id: AgentId,
    event: StreamEvent,
    user_input_tx: tokio::sync::mpsc::Sender<String>,
) -> Result<(), String> {
    match event {
        StreamEvent::AskUser { question, options } => {
            record_project_worker_ask_user(
                state,
                project,
                spec,
                run_id,
                phase,
                agent_id,
                question,
                options,
                user_input_tx,
            )
            .await
        }
        other => {
            let Some(event) =
                project_worker_stream_event(project, spec, run_id, phase, &agent_id, other)
            else {
                return Ok(());
            };
            append_project_stream_event(
                state,
                project,
                event.kind,
                &event.title,
                &event.detail,
                &agent_id.to_string(),
                phase,
                event.status,
                event.data,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn append_project_stream_event(
    state: &Arc<AppState>,
    project: &project::Project,
    kind: &str,
    title: &str,
    detail: &str,
    actor: &str,
    phase: &str,
    status: &str,
    data: serde_json::Value,
) -> Result<(), String> {
    update_project_runtime_state(state, &project.id, |runtime, _project| {
        append_runtime_event(runtime, kind, title, detail, actor, phase, status, data);
        runtime["updated_at"] = serde_json::json!(Utc::now().to_rfc3339());
    })
    .await
    .map(|_| ())
}

#[cfg(test)]
#[path = "project_runtime_worker_turn_tests.rs"]
mod project_runtime_worker_turn_tests;

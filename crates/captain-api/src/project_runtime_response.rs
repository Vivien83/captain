use crate::project_lifecycle::lifecycle_from_metadata;
use crate::project_metadata::{project_source_from_metadata, project_workspace_from_metadata};
use crate::project_runtime_defaults::project_session_id;
use crate::project_runtime_events::merged_runtime_transcript_events;
use crate::project_runtime_mutation::project_runtime_json;
use crate::project_runtime_status::project_runtime_operator_status;
use crate::project_runtime_view as runtime_view;
use crate::routes::AppState;
use captain_kernel::goals::GoalStatus;
use captain_memory::project;

pub(crate) const PROJECT_RUNTIME_TRANSCRIPT_LIMIT: usize = 10_000;

pub(crate) fn enrich_project(state: &AppState, project: project::Project) -> serde_json::Value {
    let goals = state
        .kernel
        .goal_store
        .list_for_project(&project.id, &project.slug);
    let active_goal_count = goals
        .iter()
        .filter(|goal| goal.status == GoalStatus::Active)
        .count();
    let lifecycle =
        runtime_view::project_lifecycle_view(&lifecycle_from_metadata(&project.metadata));
    let phase = lifecycle
        .get("current_phase")
        .and_then(|v| v.as_str())
        .unwrap_or("observe")
        .to_string();
    let mut value = runtime_view::project_runtime_project_view(&project);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("goal_count".to_string(), serde_json::json!(goals.len()));
        obj.insert(
            "active_goal_count".to_string(),
            serde_json::json!(active_goal_count),
        );
        obj.insert("lifecycle_phase".to_string(), serde_json::json!(phase));
        obj.insert("lifecycle".to_string(), lifecycle);
        let source =
            runtime_view::project_source_view(&project_source_from_metadata(&project.metadata));
        let workspace = runtime_view::project_workspace_view(&project_workspace_from_metadata(
            &project.metadata,
        ));
        let source_type = source
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        obj.insert("source".to_string(), source);
        obj.insert("source_type".to_string(), serde_json::json!(source_type));
        obj.insert("workspace".to_string(), workspace);
        obj.insert(
            "runtime".to_string(),
            runtime_view::project_runtime_view(&project_runtime_json(state, &project, None)),
        );
    }
    value
}

pub(crate) fn project_runtime_response(
    state: &AppState,
    project: &project::Project,
) -> serde_json::Value {
    project_runtime_response_with_limit(state, project, PROJECT_RUNTIME_TRANSCRIPT_LIMIT)
}

pub(crate) fn project_runtime_response_with_limit(
    state: &AppState,
    project: &project::Project,
    transcript_limit: usize,
) -> serde_json::Value {
    let runtime = project_runtime_json(state, project, None);
    let transcript =
        project_runtime_transcript_with_limit(state, project, &runtime, transcript_limit);
    let operator_status = project_runtime_operator_status(
        project,
        &runtime,
        crate::project_runtime_runner::project_runtime_is_running(&project.id),
    );
    serde_json::json!({
        "project": runtime_view::project_runtime_project_view(project),
        "runtime": runtime_view::project_runtime_view(&runtime),
        "operator_status": operator_status,
        "transcript": transcript,
        "session_id": project_session_id(project),
    })
}

pub(crate) fn project_runtime_transcript_limit(requested: Option<usize>) -> usize {
    requested
        .unwrap_or(PROJECT_RUNTIME_TRANSCRIPT_LIMIT)
        .clamp(1, PROJECT_RUNTIME_TRANSCRIPT_LIMIT)
}

pub(crate) fn project_runtime_transcript_with_limit(
    state: &AppState,
    project: &project::Project,
    runtime: &serde_json::Value,
    limit: usize,
) -> serde_json::Value {
    let session_id = project_session_id(project);
    let event_type = "project_runtime_event";
    let stored_count = state
        .kernel
        .memory
        .count_session_events_by_type(&session_id, event_type)
        .unwrap_or(0);
    let query = captain_memory::event_log::RangeQuery {
        session_id: session_id.clone(),
        from_ts: None,
        to_ts: None,
        limit: Some(limit),
    };
    let mut stored_events = Vec::new();
    if let Ok(rows) = state
        .kernel
        .memory
        .read_session_events_tail_by_type(&query, event_type)
    {
        for row in rows {
            let Some(event) = row.payload.get("event").cloned().filter(|v| v.is_object()) else {
                continue;
            };
            stored_events.push(event);
        }
    }
    let (events, merged_count) = merged_runtime_transcript_events(stored_events, runtime, limit);
    let count = events.len();
    serde_json::json!({
        "session_id": session_id,
        "events": runtime_view::safe_runtime_events(events),
        "count": count,
        "stored_count": stored_count,
        "limit": limit,
        "truncated": stored_count as usize > limit || merged_count > limit,
    })
}

#[cfg(test)]
#[path = "project_runtime_query_tests.rs"]
mod project_runtime_query_tests;

#[cfg(test)]
#[path = "project_runtime_response_tests.rs"]
mod project_runtime_response_tests;

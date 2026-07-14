use crate::project_lookup_input::normalize_project_lookup_key;
use crate::project_runtime_status::project_runtime_operator_status;
use crate::project_tool_request_decision::ToolRequestDecision;
use crate::project_tool_request_runtime::{
    apply_project_tool_request_decision, first_pending_tool_request_phase, normalize_phase,
    normalize_tools, pending_tool_request_tools, valid_phase,
};
use crate::project_tool_request_view::{
    project_tool_request_json_error as json_error, project_tool_request_success_view,
    safe_project_tool_request_runtime_error, safe_project_tool_request_storage_error,
};
use crate::routes::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_memory::project;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

type ToolRequestHttpError = (StatusCode, Json<Value>);

#[derive(Debug, Deserialize)]
pub struct ProjectToolRequestDecisionReq {
    pub phase: Option<String>,
    pub decision: String,
    pub tools: Option<Vec<String>>,
    pub reason: Option<String>,
}

struct ProjectToolRequestUpdate {
    metadata: Value,
    phase: String,
    decision: ToolRequestDecision,
    tools: Vec<String>,
}

pub async fn respond_project_tool_request(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    Json(req): Json<ProjectToolRequestDecisionReq>,
) -> impl IntoResponse {
    let id_or_slug = match normalize_project_lookup_key(&id_or_slug) {
        Ok(id_or_slug) => id_or_slug,
        Err(error) => {
            return json_error(StatusCode::BAD_REQUEST, "invalid_project_identifier", error)
        }
    };
    let project = match resolve_project_for_tool_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let update = match prepare_project_tool_request_update(&project, req) {
        Ok(update) => update,
        Err(response) => return response,
    };
    persist_project_tool_request_update(&state, &project, update).await
}

fn resolve_project_for_tool_request(
    state: &AppState,
    id_or_slug: &str,
) -> Result<project::Project, ToolRequestHttpError> {
    match resolve_project(state, id_or_slug) {
        Ok(Some(project)) => Ok(project),
        Ok(None) => Err(json_error(
            StatusCode::NOT_FOUND,
            "project_not_found",
            "Project could not be found",
        )),
        Err(e) => Err(json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "project_lookup_failed",
            safe_project_tool_request_storage_error(&e),
        )),
    }
}

fn prepare_project_tool_request_update(
    project: &project::Project,
    req: ProjectToolRequestDecisionReq,
) -> Result<ProjectToolRequestUpdate, ToolRequestHttpError> {
    let decision = parse_tool_request_decision(&req.decision)?;
    let mut metadata = project.metadata.clone();
    let runtime = runtime_for_tool_request(&mut metadata)?;
    let phase = select_tool_request_phase(runtime, req.phase.as_deref())?;
    let tools = requested_tools_for_decision(runtime, &phase, req.tools, decision)?;
    apply_project_tool_request_decision(runtime, &phase, decision, &tools, req.reason.as_deref())
        .map_err(|error| {
        json_error(
            StatusCode::CONFLICT,
            "project_tool_request_not_pending",
            safe_project_tool_request_runtime_error(&error),
        )
    })?;
    Ok(ProjectToolRequestUpdate {
        metadata,
        phase,
        decision,
        tools,
    })
}

fn parse_tool_request_decision(value: &str) -> Result<ToolRequestDecision, ToolRequestHttpError> {
    match ToolRequestDecision::parse(value) {
        Some(decision) => Ok(decision),
        None => Err(json_error(
            StatusCode::BAD_REQUEST,
            "invalid_decision",
            "decision must be approve, allow-once, allow, or deny",
        )),
    }
}

fn runtime_for_tool_request(metadata: &mut Value) -> Result<&mut Value, ToolRequestHttpError> {
    metadata
        .get_mut("runtime")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            json_error(
                StatusCode::CONFLICT,
                "project_runtime_missing",
                "project has no runtime tool request state",
            )
        })
}

fn select_tool_request_phase(
    runtime: &Value,
    requested_phase: Option<&str>,
) -> Result<String, ToolRequestHttpError> {
    match requested_phase
        .map(normalize_phase)
        .or_else(|| first_pending_tool_request_phase(runtime))
    {
        Some(phase) if valid_phase(&phase) => Ok(phase),
        Some(_phase) => Err(json_error(
            StatusCode::BAD_REQUEST,
            "invalid_phase",
            "Unknown project runtime phase",
        )),
        None => Err(json_error(
            StatusCode::CONFLICT,
            "project_tool_request_not_pending",
            "no pending project tool request found",
        )),
    }
}

fn requested_tools_for_decision(
    runtime: &Value,
    phase: &str,
    requested_tools: Option<Vec<String>>,
    decision: ToolRequestDecision,
) -> Result<Vec<String>, ToolRequestHttpError> {
    let existing_tools = pending_tool_request_tools(runtime, phase);
    let tools = normalize_tools(requested_tools.unwrap_or(existing_tools));
    if decision == ToolRequestDecision::Approve && tools.is_empty() {
        return Err(json_error(
            StatusCode::CONFLICT,
            "project_tool_request_empty",
            "approval requires at least one requested tool",
        ));
    }
    Ok(tools)
}

async fn persist_project_tool_request_update(
    state: &AppState,
    project: &project::Project,
    update: ProjectToolRequestUpdate,
) -> (StatusCode, Json<serde_json::Value>) {
    match state.kernel.memory.project_update(
        &project.id,
        project::ProjectPatch {
            status: Some(project::ProjectStatus::Active),
            metadata: Some(update.metadata),
            ..Default::default()
        },
    ) {
        Ok(Some(updated)) => {
            let runtime = updated
                .metadata
                .get("runtime")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let operator_status = project_runtime_operator_status(
                &updated,
                &runtime,
                crate::project_runtime_runner::project_runtime_is_running(&updated.id),
            );
            (
                StatusCode::OK,
                Json(project_tool_request_success_view(
                    &updated,
                    operator_status,
                    &update.phase,
                    update.decision.as_str(),
                    &update.tools,
                    update.decision == ToolRequestDecision::Approve,
                )),
            )
        }
        Ok(None) => json_error(
            StatusCode::NOT_FOUND,
            "project_not_found",
            "Project could not be found",
        ),
        Err(e) => json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "project_update_failed",
            safe_project_tool_request_storage_error(&e.to_string()),
        ),
    }
}

fn resolve_project(state: &AppState, id_or_slug: &str) -> Result<Option<project::Project>, String> {
    state
        .kernel
        .memory
        .project_get(id_or_slug)
        .map_err(|e| e.to_string())
        .and_then(|found| {
            if found.is_some() {
                Ok(found)
            } else {
                state
                    .kernel
                    .memory
                    .project_find_by_slug(id_or_slug)
                    .map_err(|e| e.to_string())
            }
        })
}

#[cfg(test)]
#[path = "project_tool_request_routes_tests.rs"]
mod project_tool_request_routes_tests;

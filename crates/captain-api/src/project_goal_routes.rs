use crate::project_goal_input::{
    normalize_project_goal_create_id, normalize_project_goal_create_name,
    normalize_project_goal_description, normalize_project_goal_lookup_id,
    normalize_project_goal_recovery_command, normalize_project_goal_required_check_command,
    normalize_project_goal_update_check_command, normalize_project_goal_update_description,
    normalize_project_goal_update_name, PROJECT_GOAL_NOT_FOUND_ERROR,
};
use crate::project_goal_runtime::{add_project_goal, build_project_goal, spawn_project_goal_loop};
use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_resume_view as resume_view;
use crate::project_storage_error::safe_project_storage_error;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_kernel::goals::{EscalationTarget, Goal, GoalStatus};
use captain_memory::project;
use captain_types::agent::AgentId;
use captain_types::event::{Event, EventPayload, EventTarget};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct CreateProjectGoalReq {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub check_command: String,
    #[serde(default)]
    pub recovery_command: Option<String>,
    #[serde(default)]
    pub interval_secs: Option<u64>,
    #[serde(default)]
    pub escalation_threshold: Option<u32>,
    #[serde(default)]
    pub max_llm_calls_per_hour: Option<u32>,
    #[serde(default)]
    pub escalation_channel: Option<EscalationTarget>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateProjectGoalReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub check_command: Option<String>,
    #[serde(default)]
    pub recovery_command: Option<String>,
    #[serde(default)]
    pub interval_secs: Option<u64>,
    #[serde(default)]
    pub escalation_threshold: Option<u32>,
    #[serde(default)]
    pub max_llm_calls_per_hour: Option<u32>,
}

pub async fn list_project_goals(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let goals = state
        .kernel
        .goal_store
        .list_for_project(&project.id, &project.slug);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "project": crate::project_runtime_response::enrich_project(&state, project),
            "goals": resume_view::goal_list_view(
                serde_json::to_value(goals).unwrap_or(serde_json::Value::Null),
            ),
        })),
    )
}

pub async fn create_project_goal(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    Json(req): Json<CreateProjectGoalReq>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let id = match normalize_project_goal_create_id(req.id) {
        Ok(id) => id,
        Err(error) => return bad_request(error),
    };
    let name = match normalize_project_goal_create_name(req.name) {
        Ok(name) => name,
        Err(error) => return bad_request(error),
    };
    let description = match normalize_project_goal_description(req.description) {
        Ok(description) => description,
        Err(error) => return bad_request(error),
    };
    let check_command = match normalize_project_goal_required_check_command(req.check_command) {
        Ok(check_command) => check_command,
        Err(error) => return bad_request(error),
    };
    let recovery_command = match normalize_project_goal_recovery_command(req.recovery_command) {
        Ok(recovery_command) => recovery_command,
        Err(error) => return bad_request(error),
    };
    let goal = build_project_goal(
        &state,
        &project,
        id,
        name,
        description,
        check_command,
        recovery_command,
        req.interval_secs,
        req.escalation_threshold,
        req.max_llm_calls_per_hour,
        req.escalation_channel,
    );
    match add_project_goal(&state, goal) {
        Ok(goal) => {
            publish_project_event(
                &state,
                serde_json::json!({
                    "event": "project.goal.created",
                    "project_id": project.id,
                    "slug": project.slug,
                    "goal_id": goal.id,
                }),
            )
            .await;
            (
                StatusCode::CREATED,
                Json(resume_view::goal_item_view(
                    serde_json::to_value(goal).unwrap_or(serde_json::Value::Null),
                )),
            )
        }
        Err(error) => bad_request(error),
    }
}

pub async fn update_project_goal(
    State(state): State<Arc<AppState>>,
    Path((id_or_slug, goal_id)): Path<(String, String)>,
    Json(req): Json<UpdateProjectGoalReq>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let goal_id = match normalize_project_goal_lookup_id(goal_id) {
        Ok(goal_id) => goal_id,
        Err(error) => return bad_request(error),
    };
    let Some(mut goal) = state.kernel.goal_store.get(&goal_id) else {
        return not_found(PROJECT_GOAL_NOT_FOUND_ERROR);
    };
    if !goal_belongs_to_project(&goal, &project) {
        return not_found(PROJECT_GOAL_NOT_FOUND_ERROR);
    }
    let was_active = goal.status == GoalStatus::Active;
    if let Err(error) = apply_project_goal_update(&mut goal, req) {
        return bad_request(error);
    }
    goal.project_id = Some(project.id.clone());
    goal.project_slug = Some(project.slug.clone());
    match state.kernel.goal_store.update(goal) {
        Ok(Some(updated)) => {
            if !was_active && updated.status == GoalStatus::Active {
                spawn_project_goal_loop(&state, updated.id.clone());
            }
            publish_project_event(
                &state,
                serde_json::json!({
                    "event": "project.goal.updated",
                    "project_id": project.id,
                    "slug": project.slug,
                    "goal_id": goal_id,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(resume_view::goal_item_view(
                    serde_json::to_value(updated).unwrap_or(serde_json::Value::Null),
                )),
            )
        }
        Ok(None) => not_found(PROJECT_GOAL_NOT_FOUND_ERROR),
        Err(error) => bad_request(safe_project_storage_error(&error.to_string())),
    }
}

pub async fn pause_project_goal(
    State(state): State<Arc<AppState>>,
    Path((id_or_slug, goal_id)): Path<(String, String)>,
) -> impl IntoResponse {
    set_project_goal_status(state, id_or_slug, goal_id, GoalStatus::Paused).await
}

pub async fn resume_project_goal(
    State(state): State<Arc<AppState>>,
    Path((id_or_slug, goal_id)): Path<(String, String)>,
) -> impl IntoResponse {
    set_project_goal_status(state, id_or_slug, goal_id, GoalStatus::Active).await
}

pub async fn delete_project_goal(
    State(state): State<Arc<AppState>>,
    Path((id_or_slug, goal_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let goal_id = match normalize_project_goal_lookup_id(goal_id) {
        Ok(goal_id) => goal_id,
        Err(error) => return bad_request(error),
    };
    let Some(goal) = state.kernel.goal_store.get(&goal_id) else {
        return not_found(PROJECT_GOAL_NOT_FOUND_ERROR);
    };
    if !goal_belongs_to_project(&goal, &project) {
        return not_found(PROJECT_GOAL_NOT_FOUND_ERROR);
    }
    match state.kernel.goal_store.remove(&goal_id) {
        Ok(Some(_)) => {
            publish_project_event(
                &state,
                serde_json::json!({
                    "event": "project.goal.deleted",
                    "project_id": project.id,
                    "slug": project.slug,
                    "goal_id": goal_id,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "deleted", "goal_id": goal_id })),
            )
        }
        Ok(None) => not_found(PROJECT_GOAL_NOT_FOUND_ERROR),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

async fn set_project_goal_status(
    state: Arc<AppState>,
    id_or_slug: String,
    goal_id: String,
    status: GoalStatus,
) -> (StatusCode, Json<serde_json::Value>) {
    let project = match resolve_project_for_request(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let goal_id = match normalize_project_goal_lookup_id(goal_id) {
        Ok(goal_id) => goal_id,
        Err(error) => return bad_request(error),
    };
    let Some(goal) = state.kernel.goal_store.get(&goal_id) else {
        return not_found(PROJECT_GOAL_NOT_FOUND_ERROR);
    };
    if !goal_belongs_to_project(&goal, &project) {
        return not_found(PROJECT_GOAL_NOT_FOUND_ERROR);
    }
    let was_active = goal.status == GoalStatus::Active;
    match state.kernel.goal_store.set_status(&goal_id, status) {
        Ok(true) => {
            let updated = state.kernel.goal_store.get(&goal_id).unwrap_or(goal);
            if status == GoalStatus::Active && !was_active {
                spawn_project_goal_loop(&state, goal_id.clone());
            }
            publish_project_event(
                &state,
                serde_json::json!({
                    "event": "project.goal.status",
                    "project_id": project.id,
                    "slug": project.slug,
                    "goal_id": goal_id,
                    "status": updated.status,
                }),
            )
            .await;
            (
                StatusCode::OK,
                Json(resume_view::goal_item_view(
                    serde_json::to_value(updated).unwrap_or(serde_json::Value::Null),
                )),
            )
        }
        Ok(false) => not_found(PROJECT_GOAL_NOT_FOUND_ERROR),
        Err(error) => server_error(safe_project_storage_error(&error.to_string())),
    }
}

fn apply_project_goal_update(goal: &mut Goal, req: UpdateProjectGoalReq) -> Result<(), String> {
    let recovery_command_was_set = req.recovery_command.is_some();
    let name = normalize_project_goal_update_name(req.name).map_err(str::to_string)?;
    let description =
        normalize_project_goal_update_description(req.description).map_err(str::to_string)?;
    let check_command =
        normalize_project_goal_update_check_command(req.check_command).map_err(str::to_string)?;
    let recovery_command =
        normalize_project_goal_recovery_command(req.recovery_command).map_err(str::to_string)?;

    if let Some(name) = name {
        goal.name = name;
    }
    if let Some(description) = description {
        goal.description = description;
    }
    if let Some(check_command) = check_command {
        if check_command != goal.check_command {
            goal.check_command = check_command;
            goal.consecutive_fails = 0;
            goal.last_check_ts = None;
            goal.escalated_at = None;
            goal.status = GoalStatus::Active;
        }
    }
    if recovery_command_was_set {
        goal.recovery_command = recovery_command;
    }
    if let Some(interval_secs) = req.interval_secs {
        goal.interval_secs = interval_secs;
    }
    if let Some(escalation_threshold) = req.escalation_threshold {
        goal.escalation_threshold = escalation_threshold;
    }
    if let Some(max_llm_calls_per_hour) = req.max_llm_calls_per_hour {
        goal.max_llm_calls_per_hour = max_llm_calls_per_hour;
    }
    goal.updated_at = Utc::now();
    goal.validate().map_err(|error| error.to_string())
}

fn goal_belongs_to_project(goal: &Goal, project: &project::Project) -> bool {
    goal.project_id.as_deref() == Some(project.id.as_str())
        || goal.project_slug.as_deref() == Some(project.slug.as_str())
}

fn resolve_project_for_request(
    state: &AppState,
    id_or_slug: &str,
) -> Result<project::Project, (StatusCode, Json<serde_json::Value>)> {
    let id_or_slug = normalize_project_lookup_key(id_or_slug).map_err(bad_request)?;
    match resolve_project(state, &id_or_slug) {
        Ok(Some(project)) => Ok(project),
        Ok(None) => Err(not_found(PROJECT_LOOKUP_NOT_FOUND_ERROR)),
        Err(error) => Err(server_error(safe_project_storage_error(&error))),
    }
}

fn resolve_project(state: &AppState, id_or_slug: &str) -> Result<Option<project::Project>, String> {
    state
        .kernel
        .memory
        .project_get(id_or_slug)
        .map_err(|error| error.to_string())
        .and_then(|found| {
            if found.is_some() {
                Ok(found)
            } else {
                state
                    .kernel
                    .memory
                    .project_find_by_slug(id_or_slug)
                    .map_err(|error| error.to_string())
            }
        })
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg.into() })),
    )
}

fn not_found(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": msg.into() })),
    )
}

fn server_error(msg: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": msg.into() })),
    )
}

async fn publish_project_event(state: &AppState, payload: serde_json::Value) {
    if let Ok(bytes) = serde_json::to_vec(&payload) {
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
#[path = "project_goal_routes_tests.rs"]
mod project_goal_routes_tests;

#[cfg(test)]
#[path = "project_goal_input_tests.rs"]
mod project_goal_input_tests;

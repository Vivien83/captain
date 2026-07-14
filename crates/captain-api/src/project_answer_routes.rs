use crate::project_ask::ProjectAskAnswerReceipt;
use crate::project_ask_answering::record_project_ask_answer_runtime;
use crate::project_lookup_input::{normalize_project_lookup_key, PROJECT_LOOKUP_NOT_FOUND_ERROR};
use crate::project_runtime_status::project_runtime_operator_status;
use crate::project_runtime_view as runtime_view;
use crate::routes::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::project;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

type ProjectAnswerHttpResponse = (StatusCode, Json<Value>);

#[derive(Debug, Deserialize)]
pub struct AnswerProjectAskReq {
    pub ask_id: String,
    pub answer: String,
}

enum ProjectAnswerActiveDelivery {
    Delivered(ProjectAskAnswerReceipt),
    Failed(String),
}

pub async fn answer_project_ask(
    State(state): State<Arc<AppState>>,
    Path(id_or_slug): Path<String>,
    Json(req): Json<AnswerProjectAskReq>,
) -> impl IntoResponse {
    let id_or_slug = match normalize_project_lookup_key(&id_or_slug) {
        Ok(id_or_slug) => id_or_slug,
        Err(error) => {
            return project_answer_json_error(
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "error": error }),
            )
        }
    };
    let project = match resolve_project_for_answer(&state, &id_or_slug) {
        Ok(project) => project,
        Err(response) => return response,
    };
    let active_delivery = deliver_project_answer_to_active_worker(&project.id, &req).await;
    finalize_project_answer_response(&state, &project, &req, active_delivery)
}

fn resolve_project_for_answer(
    state: &AppState,
    id_or_slug: &str,
) -> Result<project::Project, ProjectAnswerHttpResponse> {
    match resolve_project(state, id_or_slug) {
        Ok(Some(project)) => Ok(project),
        Ok(None) => Err(project_answer_json_error(
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": PROJECT_LOOKUP_NOT_FOUND_ERROR }),
        )),
        Err(e) => Err(project_answer_json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({ "error": safe_project_answer_storage_error(&e) }),
        )),
    }
}

async fn deliver_project_answer_to_active_worker(
    project_id: &str,
    req: &AnswerProjectAskReq,
) -> ProjectAnswerActiveDelivery {
    match crate::project_ask::answer_project_ask_for_project_with_receipt(
        project_id,
        &req.ask_id,
        &req.answer,
    )
    .await
    {
        Ok(receipt) => ProjectAnswerActiveDelivery::Delivered(receipt),
        Err(error) => ProjectAnswerActiveDelivery::Failed(error),
    }
}

fn finalize_project_answer_response(
    state: &AppState,
    project: &project::Project,
    req: &AnswerProjectAskReq,
    active_delivery: ProjectAnswerActiveDelivery,
) -> ProjectAnswerHttpResponse {
    match active_delivery {
        ProjectAnswerActiveDelivery::Delivered(receipt) => {
            finalize_delivered_project_answer(state, project, receipt)
        }
        ProjectAnswerActiveDelivery::Failed(active_error) => {
            finalize_resume_project_answer(state, project, req, &active_error)
        }
    }
}

fn finalize_delivered_project_answer(
    state: &AppState,
    project: &project::Project,
    receipt: ProjectAskAnswerReceipt,
) -> ProjectAnswerHttpResponse {
    match record_project_ask_answer_runtime(
        state.kernel.memory.as_ref(),
        Some(&project.id),
        &receipt.ask_id,
        &receipt.answer,
        "delivered_to_active_worker",
    ) {
        Ok((updated, _)) => {
            project_answer_ok_view(&updated, &receipt.ask_id, true, false, None, None)
        }
        Err(e) => project_answer_ok_view(
            project,
            &receipt.ask_id,
            true,
            false,
            Some(safe_project_answer_runtime_warning(&e)),
            None,
        ),
    }
}

fn finalize_resume_project_answer(
    state: &AppState,
    project: &project::Project,
    req: &AnswerProjectAskReq,
    active_error: &str,
) -> ProjectAnswerHttpResponse {
    match record_project_ask_answer_runtime(
        state.kernel.memory.as_ref(),
        Some(&project.id),
        &req.ask_id,
        &req.answer,
        "recorded_for_resume",
    ) {
        Ok((updated, receipt)) => project_answer_ok_view(
            &updated,
            &receipt.ask_id,
            false,
            true,
            None,
            Some(active_error),
        ),
        Err(runtime_error) => project_answer_json_error(
            StatusCode::CONFLICT,
            serde_json::json!({
                "ok": false,
                "project_id": project.id,
                "error": "project_question_not_pending",
                "active_worker_error": safe_project_answer_active_error(active_error),
                "runtime_error": safe_project_answer_runtime_error(&runtime_error),
            }),
        ),
    }
}

fn project_answer_success_view(
    project: &project::Project,
    ask_id: &str,
    delivered_to_active_worker: bool,
    runtime_resume_pending: bool,
    warning: Option<String>,
    active_worker_error: Option<&str>,
) -> serde_json::Value {
    let (runtime, operator_status) = project_runtime_payload(project);
    let mut view = serde_json::json!({
        "ok": true,
        "project_id": project.id,
        "project": project_answer_project_view(project),
        "ask_id": safe_project_answer_id(ask_id),
        "delivered_to_active_worker": delivered_to_active_worker,
        "runtime_resume_pending": runtime_resume_pending,
        "runtime": runtime,
        "operator_status": operator_status,
        "warning": warning
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    });
    if let Some(error) = active_worker_error {
        if let Some(obj) = view.as_object_mut() {
            obj.insert(
                "active_worker_error".to_string(),
                serde_json::json!(safe_project_answer_active_error(error)),
            );
        }
    }
    view
}

fn project_answer_ok_view(
    project: &project::Project,
    ask_id: &str,
    delivered_to_active_worker: bool,
    runtime_resume_pending: bool,
    warning: Option<String>,
    active_worker_error: Option<&str>,
) -> ProjectAnswerHttpResponse {
    (
        StatusCode::OK,
        Json(project_answer_success_view(
            project,
            ask_id,
            delivered_to_active_worker,
            runtime_resume_pending,
            warning,
            active_worker_error,
        )),
    )
}

fn project_answer_json_error(status: StatusCode, body: Value) -> ProjectAnswerHttpResponse {
    (status, Json(body))
}

fn project_runtime_payload(project: &project::Project) -> (serde_json::Value, serde_json::Value) {
    let raw_runtime = project
        .metadata
        .get("runtime")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let operator_status = project_runtime_operator_status(
        project,
        &raw_runtime,
        crate::project_runtime_runner::project_runtime_is_running(&project.id),
    );
    let runtime = runtime_view::project_runtime_view(&raw_runtime);
    (runtime, operator_status)
}

fn project_answer_project_view(project: &project::Project) -> serde_json::Value {
    runtime_view::project_runtime_project_view(project)
}

fn safe_project_answer_id(ask_id: &str) -> String {
    ask_id.chars().take(120).collect()
}

fn safe_project_answer_storage_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("not found") {
        return "Project could not be found".to_string();
    }
    "Project lookup failed; verify project storage availability".to_string()
}

fn safe_project_answer_runtime_warning(error: &str) -> String {
    let safe = safe_project_answer_runtime_error(error);
    format!("Answer delivered to active worker, but runtime state was not updated: {safe}")
}

fn safe_project_answer_active_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("plus active") || lower.contains("no longer active") {
        return "Active project question is no longer waiting".to_string();
    }
    if lower.contains("ambiguous") || lower.contains("use more characters") {
        return "Project question id is ambiguous; use a longer ask_id".to_string();
    }
    "Active worker delivery failed; answer may need runtime resume".to_string()
}

fn safe_project_answer_runtime_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("not found") {
        return "Project could not be found".to_string();
    }
    if lower.contains("no persisted project question matches") {
        return "No pending persisted project question matches this ask_id".to_string();
    }
    if lower.contains("persisted project questions match") || lower.contains("use more characters")
    {
        return "Project question id is ambiguous; use a longer ask_id".to_string();
    }
    "Project question state could not be updated; verify project storage availability".to_string()
}

fn resolve_project(state: &AppState, id_or_slug: &str) -> Result<Option<project::Project>, String> {
    state
        .kernel
        .memory
        .project_get(id_or_slug)
        .map_err(|e| format!("{e}"))
        .and_then(|found| {
            if found.is_some() {
                Ok(found)
            } else {
                state
                    .kernel
                    .memory
                    .project_find_by_slug(id_or_slug)
                    .map_err(|e| format!("{e}"))
            }
        })
}

#[cfg(test)]
#[path = "project_answer_routes_tests.rs"]
mod project_answer_routes_tests;

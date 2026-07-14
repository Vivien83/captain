use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::{agent::AgentId, scheduler::CronJobId};
use std::{collections::HashMap, sync::Arc};

/// GET /api/cron/jobs - List all cron jobs, optionally filtered by agent_id.
pub async fn list_cron_jobs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let jobs = if let Some(agent_id_str) = params.get("agent_id") {
        match uuid::Uuid::parse_str(agent_id_str) {
            Ok(uuid) => state.kernel.cron_scheduler.list_jobs(AgentId(uuid)),
            Err(_) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "Invalid agent_id"})),
                );
            }
        }
    } else {
        state.kernel.cron_scheduler.list_all_jobs()
    };

    let all_meta = state.kernel.cron_scheduler.list_all_jobs_with_meta();
    let meta_map: HashMap<_, _> = all_meta.iter().map(|meta| (meta.job.id, meta)).collect();
    let total = jobs.len();
    let jobs_json: Vec<serde_json::Value> = jobs
        .into_iter()
        .map(|job| {
            let mut value = serde_json::to_value(&job).unwrap_or_default();
            if let Some(meta) = meta_map.get(&job.id) {
                if let serde_json::Value::Object(ref mut object) = value {
                    object.insert(
                        "last_status".to_string(),
                        serde_json::json!(meta.last_status),
                    );
                    object.insert(
                        "run_history".to_string(),
                        serde_json::to_value(&meta.run_history).unwrap_or_default(),
                    );
                    object.insert(
                        "consecutive_errors".to_string(),
                        serde_json::json!(meta.consecutive_errors),
                    );
                    object.insert(
                        "last_delivery_error".to_string(),
                        serde_json::json!(meta.last_delivery_error),
                    );
                    object.insert(
                        "dead_letters".to_string(),
                        serde_json::to_value(&meta.dead_letters).unwrap_or_default(),
                    );
                    object.insert(
                        "redelivery_queue".to_string(),
                        serde_json::to_value(&meta.redelivery_queue).unwrap_or_default(),
                    );
                }
            }
            value
        })
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({"jobs": jobs_json, "total": total})),
    )
}

/// POST /api/cron/jobs - Create a new cron job.
pub async fn create_cron_job(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = body["agent_id"].as_str().unwrap_or("");
    match state.kernel.cron_create(agent_id, body.clone()).await {
        Ok(result) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"result": result})),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// DELETE /api/cron/jobs/{id} - Delete a cron job.
pub async fn delete_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(job_id) => job_id,
        Err(response) => return response,
    };
    match state.kernel.cron_scheduler.remove_job(job_id) {
        Ok(_) => {
            let _ = state.kernel.cron_scheduler.persist();
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "deleted"})),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// PUT /api/cron/jobs/{id} - Update a cron job while preserving its ID/history.
pub async fn update_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(mut body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(job_id) => job_id,
        Err(response) => return response,
    };
    let Some(job) = state.kernel.cron_scheduler.get_job(job_id) else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Job not found"})),
        );
    };

    if let serde_json::Value::Object(ref mut object) = body {
        object.insert("job_id".to_string(), serde_json::json!(id));
    } else {
        body = serde_json::json!({ "job_id": id });
    }

    match state
        .kernel
        .cron_update(&job.agent_id.to_string(), body.clone())
        .await
    {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!({"result": result}))),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e})),
        ),
    }
}

/// PUT /api/cron/jobs/{id}/enable - Enable or disable a cron job.
pub async fn toggle_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(job_id) => job_id,
        Err(response) => return response,
    };
    let enabled = body["enabled"].as_bool().unwrap_or(true);
    match state.kernel.cron_scheduler.set_enabled(job_id, enabled) {
        Ok(()) => {
            let _ = state.kernel.cron_scheduler.persist();
            (
                StatusCode::OK,
                Json(serde_json::json!({"id": id, "enabled": enabled})),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/cron/jobs/{id}/status - Get status of a specific cron job.
pub async fn cron_job_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(job_id) => job_id,
        Err(response) => return response,
    };
    match state.kernel.cron_scheduler.get_meta(job_id) {
        Some(meta) => (
            StatusCode::OK,
            Json(serde_json::to_value(&meta).unwrap_or_default()),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Job not found"})),
        ),
    }
}

/// POST /api/cron/jobs/:id/run - Manually trigger a cron job now.
pub async fn run_cron_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let job_id = match parse_cron_job_id(&id) {
        Ok(job_id) => job_id,
        Err(response) => return response,
    };
    let job = match state.kernel.cron_scheduler.get_job(job_id) {
        Some(job) => job,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Job not found"})),
            );
        }
    };

    let agent_id = job.agent_id;
    if state.kernel.registry.get(agent_id).is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Target agent not found"})),
        );
    }

    let message = format!(
        "[CRON MANUAL RUN \u{2014} job '{}'] IMPORTANT: This is a manual execution of an existing recurring cron job. \
         Do NOT create, modify, or duplicate any cron/schedule. Just execute the task and report the result.\n\n{}",
        job.name,
        cron_base_message(&job)
    );

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_with_handle(agent_id, &message, Some(kernel_handle), None, None)
        .await
    {
        Ok(result) => {
            state.kernel.cron_scheduler.record_success(job_id);
            let _ = state.kernel.cron_scheduler.persist();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "completed",
                    "job_id": id,
                    "agent_id": agent_id.to_string(),
                    "response": result.response,
                })),
            )
        }
        Err(e) => {
            let err_msg = format!("{e}");
            state.kernel.cron_scheduler.record_failure(job_id, &err_msg);
            let _ = state.kernel.cron_scheduler.persist();
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "status": "failed",
                    "job_id": id,
                    "error": err_msg,
                })),
            )
        }
    }
}

fn parse_cron_job_id(id: &str) -> Result<CronJobId, (StatusCode, Json<serde_json::Value>)> {
    uuid::Uuid::parse_str(id).map(CronJobId).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid job ID"})),
        )
    })
}

fn cron_base_message(job: &captain_types::scheduler::CronJob) -> String {
    match &job.action {
        captain_types::scheduler::CronAction::AgentTurn { message, .. } => message.clone(),
        captain_types::scheduler::CronAction::SystemEvent { text } => text.clone(),
        captain_types::scheduler::CronAction::WorkflowRun {
            workflow_id, input, ..
        } => {
            format!(
                "Run workflow {} with input: {}",
                workflow_id,
                input.as_deref().unwrap_or("")
            )
        }
        captain_types::scheduler::CronAction::InlineWorkflow { steps } => {
            format!(
                "[Scheduled task '{}' inline workflow] {} steps",
                job.name,
                steps.len()
            )
        }
    }
}

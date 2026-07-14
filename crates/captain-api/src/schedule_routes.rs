use crate::state::AppState;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::AgentId;
use std::sync::Arc;

const SCHEDULES_KEY: &str = "__captain_schedules";

fn schedule_shared_agent_id() -> AgentId {
    AgentId(uuid::Uuid::from_bytes([
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
    ]))
}

/// GET /api/schedules - List all cron-based scheduled jobs.
pub async fn list_schedules(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agent_id = schedule_shared_agent_id();
    match state.kernel.memory.structured_get(agent_id, SCHEDULES_KEY) {
        Ok(Some(serde_json::Value::Array(arr))) => {
            let total = arr.len();
            Json(serde_json::json!({"schedules": arr, "total": total}))
        }
        Ok(_) => Json(serde_json::json!({"schedules": [], "total": 0})),
        Err(e) => {
            tracing::warn!("Failed to load schedules: {e}");
            Json(serde_json::json!({"schedules": [], "total": 0, "error": format!("{e}")}))
        }
    }
}

/// POST /api/schedules - Create a new cron-based scheduled job.
pub async fn create_schedule(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match req["name"].as_str() {
        Some(name) if !name.is_empty() => name.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'name' field"})),
            );
        }
    };

    let cron = match req["cron"].as_str() {
        Some(cron) if !cron.is_empty() => cron.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing 'cron' field"})),
            );
        }
    };

    if cron.split_whitespace().count() != 5 {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Invalid cron expression: must have 5 fields (min hour dom mon dow)"}),
            ),
        );
    }

    let agent_id_str = req["agent_id"].as_str().unwrap_or("").to_string();
    if agent_id_str.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: agent_id"})),
        );
    }

    let agent_exists = if let Ok(agent_id) = agent_id_str.parse::<AgentId>() {
        state.kernel.registry.get(agent_id).is_some()
    } else {
        state
            .kernel
            .registry
            .list()
            .iter()
            .any(|agent| agent.name == agent_id_str)
    };
    if !agent_exists {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Agent not found: {agent_id_str}")})),
        );
    }

    let entry = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "name": name,
        "cron": cron,
        "agent_id": agent_id_str,
        "message": req["message"].as_str().unwrap_or("").to_string(),
        "enabled": req.get("enabled").and_then(|value| value.as_bool()).unwrap_or(true),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "last_run": null,
        "run_count": 0,
    });

    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    schedules.push(entry.clone());
    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        tracing::warn!("Failed to save schedule: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save schedule: {e}")})),
        );
    }

    (StatusCode::CREATED, Json(entry))
}

/// PUT /api/schedules/:id - Update a scheduled job.
pub async fn update_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let mut found = false;
    for schedule in schedules.iter_mut() {
        if schedule["id"].as_str() == Some(&id) {
            found = true;
            apply_schedule_update(schedule, &req);
            if let Some(cron) = req.get("cron").and_then(|value| value.as_str()) {
                if cron.split_whitespace().count() != 5 {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(serde_json::json!({"error": "Invalid cron expression"})),
                    );
                }
            }
            break;
        }
    }

    if !found {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Schedule not found"})),
        );
    }

    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to update schedule: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "updated", "schedule_id": id})),
    )
}

/// DELETE /api/schedules/:id - Remove a scheduled job.
pub async fn delete_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let before = schedules.len();
    schedules.retain(|schedule| schedule["id"].as_str() != Some(&id));

    if schedules.len() == before {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Schedule not found"})),
        );
    }

    if let Err(e) = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to delete schedule: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed", "schedule_id": id})),
    )
}

/// POST /api/schedules/:id/run - Manually run a scheduled job now.
pub async fn run_schedule(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let shared_id = schedule_shared_agent_id();
    let schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };

    let schedule = match schedules
        .iter()
        .find(|schedule| schedule["id"].as_str() == Some(&id))
    {
        Some(schedule) => schedule.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Schedule not found"})),
            );
        }
    };

    let target_agent = match resolve_schedule_agent(&state, &schedule) {
        Some(agent) => agent,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(
                    serde_json::json!({"error": "No target agent found. Specify an agent_id or start an agent first."}),
                ),
            );
        }
    };

    update_schedule_run_count(&state, shared_id, &id);

    let name = schedule["name"].as_str().unwrap_or("(unnamed)");
    let message = schedule["message"]
        .as_str()
        .unwrap_or("Scheduled task triggered manually.");
    let run_message = if message.is_empty() {
        format!("[Scheduled task '{}' triggered manually]", name)
    } else {
        message.to_string()
    };

    let kernel_handle: Arc<dyn KernelHandle> = state.kernel.clone() as Arc<dyn KernelHandle>;
    match state
        .kernel
        .send_message_with_handle(target_agent, &run_message, Some(kernel_handle), None, None)
        .await
    {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "completed",
                "schedule_id": id,
                "agent_id": target_agent.to_string(),
                "response": result.response,
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "failed",
                "schedule_id": id,
                "error": format!("{e}"),
            })),
        ),
    }
}

fn apply_schedule_update(schedule: &mut serde_json::Value, req: &serde_json::Value) {
    if let Some(enabled) = req.get("enabled").and_then(|value| value.as_bool()) {
        schedule["enabled"] = serde_json::Value::Bool(enabled);
    }
    for field in ["name", "cron", "agent_id", "message"] {
        if let Some(value) = req.get(field).and_then(|value| value.as_str()) {
            schedule[field] = serde_json::Value::String(value.to_string());
        }
    }
}

fn resolve_schedule_agent(state: &AppState, schedule: &serde_json::Value) -> Option<AgentId> {
    let agent_id_str = schedule["agent_id"].as_str().unwrap_or("");
    if agent_id_str.is_empty() {
        return None;
    }
    if let Ok(agent_id) = agent_id_str.parse::<AgentId>() {
        return state
            .kernel
            .registry
            .get(agent_id)
            .is_some()
            .then_some(agent_id);
    }
    state
        .kernel
        .registry
        .list()
        .iter()
        .find(|agent| agent.name == agent_id_str)
        .map(|agent| agent.id)
}

fn update_schedule_run_count(state: &AppState, shared_id: AgentId, id: &str) {
    let mut schedules: Vec<serde_json::Value> =
        match state.kernel.memory.structured_get(shared_id, SCHEDULES_KEY) {
            Ok(Some(serde_json::Value::Array(arr))) => arr,
            _ => Vec::new(),
        };
    for schedule in schedules.iter_mut() {
        if schedule["id"].as_str() == Some(id) {
            schedule["last_run"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
            let count = schedule["run_count"].as_u64().unwrap_or(0);
            schedule["run_count"] = serde_json::json!(count + 1);
            break;
        }
    }
    let _ = state.kernel.memory.structured_set(
        shared_id,
        SCHEDULES_KEY,
        serde_json::Value::Array(schedules),
    );
}

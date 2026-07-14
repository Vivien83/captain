//! Workflow route handlers.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_kernel::workflow::{
    ErrorMode, StepAgent, StepMode, Workflow, WorkflowId, WorkflowRun, WorkflowStep,
};
use std::sync::Arc;

type JsonResponse = (StatusCode, Json<serde_json::Value>);

fn parse_workflow_id(id: &str) -> Result<WorkflowId, JsonResponse> {
    id.parse().map(WorkflowId).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid workflow ID"})),
        )
    })
}

fn parse_workflow_steps(req: &serde_json::Value) -> Result<Vec<WorkflowStep>, JsonResponse> {
    let steps_json = req["steps"].as_array().ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing 'steps' array"})),
        )
    })?;

    let mut steps = Vec::new();
    for step in steps_json {
        let step_name = step["name"].as_str().unwrap_or("step").to_string();
        let agent = if let Some(id) = step["agent_id"].as_str() {
            StepAgent::ById { id: id.to_string() }
        } else if let Some(name) = step["agent_name"].as_str() {
            StepAgent::ByName {
                name: name.to_string(),
            }
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": format!("Step '{}' needs 'agent_id' or 'agent_name'", step_name)}),
                ),
            ));
        };

        let mode = match step["mode"].as_str().unwrap_or("sequential") {
            "fan_out" => StepMode::FanOut,
            "collect" => StepMode::Collect,
            "conditional" => StepMode::Conditional {
                condition: step["condition"].as_str().unwrap_or("").to_string(),
            },
            "loop" => StepMode::Loop {
                max_iterations: step["max_iterations"].as_u64().unwrap_or(5) as u32,
                until: step["until"].as_str().unwrap_or("").to_string(),
            },
            _ => StepMode::Sequential,
        };

        let error_mode = match step["error_mode"].as_str().unwrap_or("fail") {
            "skip" => ErrorMode::Skip,
            "retry" => ErrorMode::Retry {
                max_retries: step["max_retries"].as_u64().unwrap_or(3) as u32,
            },
            _ => ErrorMode::Fail,
        };

        steps.push(WorkflowStep {
            name: step_name,
            agent,
            prompt_template: step["prompt"].as_str().unwrap_or("{{input}}").to_string(),
            mode,
            timeout_secs: step["timeout_secs"].as_u64().unwrap_or(120),
            error_mode,
            output_var: step["output_var"].as_str().map(String::from),
        });
    }

    Ok(steps)
}

/// POST /api/workflows - Register a new workflow.
pub async fn create_workflow(
    State(state): State<Arc<AppState>>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let steps = match parse_workflow_steps(&req) {
        Ok(steps) => steps,
        Err(response) => return response,
    };
    let workflow = Workflow {
        id: WorkflowId::new(),
        name: req["name"].as_str().unwrap_or("unnamed").to_string(),
        description: req["description"].as_str().unwrap_or("").to_string(),
        steps,
        graph: None,
        created_at: chrono::Utc::now(),
    };

    let id = state.kernel.register_workflow(workflow).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({"workflow_id": id.to_string()})),
    )
}

/// GET /api/workflows - List all workflows.
pub async fn list_workflows(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let workflows = state.kernel.workflows.list_workflows().await;
    let list: Vec<serde_json::Value> = workflows
        .iter()
        .map(|workflow| {
            serde_json::json!({
                "id": workflow.id.to_string(),
                "name": workflow.name,
                "description": workflow.description,
                "steps": workflow.steps.len(),
                "created_at": workflow.created_at.to_rfc3339(),
            })
        })
        .collect();
    Json(list)
}

/// POST /api/workflows/:id/run - Execute a workflow.
pub async fn run_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = match parse_workflow_id(&id) {
        Ok(workflow_id) => workflow_id,
        Err(response) => return response,
    };
    let input = req["input"].as_str().unwrap_or("").to_string();

    match state.kernel.run_workflow(workflow_id, input).await {
        Ok((run_id, output)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": run_id.to_string(),
                "output": output,
                "status": "completed",
            })),
        ),
        Err(error) => {
            tracing::warn!("Workflow run failed for {id}: {error}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": "Workflow execution failed"})),
            )
        }
    }
}

/// GET /api/workflows/:id/runs - List runs for a workflow.
pub async fn list_workflow_runs(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = match parse_workflow_id(&id) {
        Ok(workflow_id) => workflow_id,
        Err(response) => return response,
    };
    let runs = runs_for_workflow(state.kernel.workflows.list_runs(None).await, workflow_id);
    let list: Vec<serde_json::Value> = runs
        .iter()
        .map(|run| {
            serde_json::json!({
                "id": run.id.to_string(),
                "workflow_name": run.workflow_name,
                "state": serde_json::to_value(&run.state).unwrap_or_default(),
                "steps_completed": run.step_results.len(),
                "started_at": run.started_at.to_rfc3339(),
                "completed_at": run.completed_at.map(|t| t.to_rfc3339()),
                "output": run.output,
                "error": run.error,
            })
        })
        .collect();
    (StatusCode::OK, Json(serde_json::Value::Array(list)))
}

fn runs_for_workflow(mut runs: Vec<WorkflowRun>, workflow_id: WorkflowId) -> Vec<WorkflowRun> {
    runs.retain(|run| run.workflow_id == workflow_id);
    runs.sort_by(|left, right| {
        right
            .started_at
            .cmp(&left.started_at)
            .then_with(|| right.id.to_string().cmp(&left.id.to_string()))
    });
    runs
}

/// GET /api/workflows/:id - Get a single workflow by ID.
pub async fn get_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = match parse_workflow_id(&id) {
        Ok(workflow_id) => workflow_id,
        Err(response) => return response,
    };

    match state.kernel.workflows.get_workflow(workflow_id).await {
        Some(workflow) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": workflow.id.to_string(),
                "name": workflow.name,
                "description": workflow.description,
                "steps": workflow.steps,
                "created_at": workflow.created_at.to_rfc3339(),
            })),
        ),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Workflow not found"})),
        ),
    }
}

/// PUT /api/workflows/:id - Update a workflow definition.
pub async fn update_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> impl IntoResponse {
    let workflow_id = match parse_workflow_id(&id) {
        Ok(workflow_id) => workflow_id,
        Err(response) => return response,
    };
    let steps = match parse_workflow_steps(&req) {
        Ok(steps) => steps,
        Err(response) => return response,
    };

    let updated = Workflow {
        id: workflow_id,
        name: req["name"].as_str().unwrap_or("unnamed").to_string(),
        description: req["description"].as_str().unwrap_or("").to_string(),
        steps,
        graph: None,
        created_at: chrono::Utc::now(),
    };

    if state
        .kernel
        .workflows
        .update_workflow(workflow_id, updated)
        .await
    {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "updated", "workflow_id": id})),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Workflow not found"})),
        )
    }
}

/// DELETE /api/workflows/:id - Delete a workflow definition.
pub async fn delete_workflow(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let workflow_id = match parse_workflow_id(&id) {
        Ok(workflow_id) => workflow_id,
        Err(response) => return response,
    };

    if state.kernel.workflows.remove_workflow(workflow_id).await {
        (
            StatusCode::OK,
            Json(serde_json::json!({"status": "removed", "workflow_id": id})),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "Workflow not found"})),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_kernel::workflow::{WorkflowRunId, WorkflowRunState};
    use chrono::{Duration, Utc};

    fn workflow_run(workflow_id: WorkflowId, started_at: chrono::DateTime<Utc>) -> WorkflowRun {
        WorkflowRun {
            id: WorkflowRunId::new(),
            workflow_id,
            workflow_name: "workflow".to_string(),
            input: "input".to_string(),
            state: WorkflowRunState::Completed,
            step_results: Vec::new(),
            output: Some("done".to_string()),
            error: None,
            started_at,
            completed_at: Some(started_at),
        }
    }

    #[test]
    fn workflow_run_history_is_scoped_and_newest_first() {
        let wanted = WorkflowId::new();
        let other = WorkflowId::new();
        let now = Utc::now();
        let older = workflow_run(wanted, now - Duration::minutes(2));
        let newer = workflow_run(wanted, now - Duration::minutes(1));
        let unrelated = workflow_run(other, now);

        let runs = runs_for_workflow(vec![older.clone(), unrelated, newer.clone()], wanted);

        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].id, newer.id);
        assert_eq!(runs[1].id, older.id);
        assert!(runs.iter().all(|run| run.workflow_id == wanted));
    }

    #[test]
    fn workflow_run_history_rejects_invalid_workflow_ids() {
        assert!(parse_workflow_id("not-a-workflow-id").is_err());
    }
}

//! Usage and budget route handlers.

use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use captain_types::agent::AgentId;
use std::sync::Arc;

/// GET /api/usage - Get per-agent usage statistics.
pub async fn usage_stats(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .iter()
        .map(|entry| {
            let (tokens, tool_calls) = state.kernel.scheduler.get_usage(entry.id).unwrap_or((0, 0));
            serde_json::json!({
                "agent_id": entry.id.to_string(),
                "name": entry.name,
                "total_tokens": tokens,
                "tool_calls": tool_calls,
            })
        })
        .collect();

    Json(serde_json::json!({"agents": agents}))
}

/// GET /api/usage/summary - Get overall usage summary from UsageStore.
pub async fn usage_summary(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.usage().query_summary(None) {
        Ok(summary) => Json(serde_json::json!({
            "total_input_tokens": summary.total_input_tokens,
            "total_output_tokens": summary.total_output_tokens,
            "total_cost_usd": summary.total_cost_usd,
            "call_count": summary.call_count,
            "total_tool_calls": summary.total_tool_calls,
        })),
        Err(_) => Json(serde_json::json!({
            "total_input_tokens": 0,
            "total_output_tokens": 0,
            "total_cost_usd": 0.0,
            "call_count": 0,
            "total_tool_calls": 0,
        })),
    }
}

/// GET /api/usage/by-model - Get usage grouped by model.
pub async fn usage_by_model(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.kernel.memory.usage().query_by_model() {
        Ok(models) => {
            let list: Vec<serde_json::Value> = models
                .iter()
                .map(|model| {
                    serde_json::json!({
                        "model": model.model,
                        "total_cost_usd": model.total_cost_usd,
                        "total_input_tokens": model.total_input_tokens,
                        "total_output_tokens": model.total_output_tokens,
                        "call_count": model.call_count,
                    })
                })
                .collect();
            Json(serde_json::json!({"models": list}))
        }
        Err(_) => Json(serde_json::json!({"models": []})),
    }
}

/// GET /api/usage/daily - Get daily usage breakdown for the last 7 days.
pub async fn usage_daily(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let days = state.kernel.memory.usage().query_daily_breakdown(7);
    let today_cost = state.kernel.memory.usage().query_today_cost();
    let first_event = state.kernel.memory.usage().query_first_event_date();

    let days_list = match days {
        Ok(days) => days
            .iter()
            .map(|day| {
                serde_json::json!({
                    "date": day.date,
                    "cost_usd": day.cost_usd,
                    "tokens": day.tokens,
                    "calls": day.calls,
                })
            })
            .collect::<Vec<_>>(),
        Err(_) => vec![],
    };

    Json(serde_json::json!({
        "days": days_list,
        "today_cost_usd": today_cost.unwrap_or(0.0),
        "first_event_date": first_event.unwrap_or(None),
    }))
}

/// GET /api/budget - Current budget status.
pub async fn budget_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let budget = state.kernel.budget_config();
    let status = state.kernel.metering.budget_status(&budget);
    let mut payload = serde_json::to_value(&status).unwrap_or_default();
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "provider_subscriptions".to_string(),
            crate::provider_quota_status::build_provider_subscription_status(
                state.kernel.memory.provider_quotas(),
            ),
        );
    }
    Json(payload)
}

/// PUT /api/budget - Update global budget limits and persist config.toml.
pub async fn update_budget(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let patch = match BudgetPatch::parse(&body) {
        Ok(patch) => patch,
        Err(message) => return bad_request(message).into_response(),
    };
    match state
        .kernel
        .update_budget_config(|budget| patch.apply(budget))
    {
        Ok(_) => budget_status(State(state)).await.into_response(),
        Err(captain_kernel::BudgetConfigUpdateError::Validation(message)) => {
            bad_request(message).into_response()
        }
        Err(captain_kernel::BudgetConfigUpdateError::Persistence(error)) => {
            tracing::error!(%error, "Failed to persist global budget update");
            #[cfg(test)]
            let message = format!("Failed to persist budget configuration: {error}");
            #[cfg(not(test))]
            let message = "Failed to persist budget configuration";
            internal_error(message).into_response()
        }
    }
}

#[derive(Default)]
struct BudgetPatch {
    max_hourly_usd: Option<f64>,
    max_daily_usd: Option<f64>,
    max_monthly_usd: Option<f64>,
    alert_threshold: Option<f64>,
    default_max_llm_tokens_per_hour: Option<u64>,
}

impl BudgetPatch {
    fn parse(body: &serde_json::Value) -> Result<Self, String> {
        let object = body
            .as_object()
            .ok_or_else(|| "Budget update must be a JSON object".to_string())?;
        let patch = Self {
            max_hourly_usd: optional_f64(object, "max_hourly_usd")?,
            max_daily_usd: optional_f64(object, "max_daily_usd")?,
            max_monthly_usd: optional_f64(object, "max_monthly_usd")?,
            alert_threshold: optional_f64(object, "alert_threshold")?,
            default_max_llm_tokens_per_hour: optional_u64(
                object,
                "default_max_llm_tokens_per_hour",
            )?,
        };
        if patch.is_empty() {
            return Err("Provide at least one budget field".to_string());
        }
        Ok(patch)
    }

    fn is_empty(&self) -> bool {
        self.max_hourly_usd.is_none()
            && self.max_daily_usd.is_none()
            && self.max_monthly_usd.is_none()
            && self.alert_threshold.is_none()
            && self.default_max_llm_tokens_per_hour.is_none()
    }

    fn apply(&self, budget: &mut captain_types::config::BudgetConfig) {
        if let Some(value) = self.max_hourly_usd {
            budget.max_hourly_usd = value;
        }
        if let Some(value) = self.max_daily_usd {
            budget.max_daily_usd = value;
        }
        if let Some(value) = self.max_monthly_usd {
            budget.max_monthly_usd = value;
        }
        if let Some(value) = self.alert_threshold {
            budget.alert_threshold = value;
        }
        if let Some(value) = self.default_max_llm_tokens_per_hour {
            budget.default_max_llm_tokens_per_hour = value;
        }
    }
}

fn optional_f64(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<f64>, String> {
    object
        .get(key)
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| format!("{key} must be a JSON number"))
        })
        .transpose()
}

fn optional_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<u64>, String> {
    object
        .get(key)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| format!("{key} must be an unsigned JSON integer"))
        })
        .transpose()
}

/// GET /api/budget/agents/{id} - Per-agent budget/quota status.
pub async fn agent_budget_status(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let entry = match state.kernel.registry.get(agent_id) {
        Some(entry) => entry,
        None => return not_found("Agent not found"),
    };

    let quota = &entry.manifest.resources;
    let usage_store = captain_memory::usage::UsageStore::new(state.kernel.memory.usage_conn());
    let hourly = usage_store.query_hourly(agent_id).unwrap_or(0.0);
    let daily = usage_store.query_daily(agent_id).unwrap_or(0.0);
    let monthly = usage_store.query_monthly(agent_id).unwrap_or(0.0);
    let token_usage = state.kernel.scheduler.get_hourly_usage(agent_id);
    let tokens_used = token_usage
        .as_ref()
        .map(|usage| usage.total_tokens)
        .unwrap_or(0);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "agent_id": agent_id.to_string(),
            "agent_name": entry.name,
            "hourly": {
                "spend": hourly,
                "limit": quota.max_cost_per_hour_usd,
                "pct": ratio(hourly, quota.max_cost_per_hour_usd),
            },
            "daily": {
                "spend": daily,
                "limit": quota.max_cost_per_day_usd,
                "pct": ratio(daily, quota.max_cost_per_day_usd),
            },
            "monthly": {
                "spend": monthly,
                "limit": quota.max_cost_per_month_usd,
                "pct": ratio(monthly, quota.max_cost_per_month_usd),
            },
            "tokens": {
                "used": tokens_used,
                "limit": quota.max_llm_tokens_per_hour,
                "window_seconds": 3600,
                "resets_at": token_usage.and_then(|usage| usage.resets_at),
                "pct": if quota.max_llm_tokens_per_hour > 0 {
                    tokens_used as f64 / quota.max_llm_tokens_per_hour as f64
                } else {
                    0.0
                },
            },
        })),
    )
}

/// GET /api/budget/agents - Per-agent cost ranking.
pub async fn agent_budget_ranking(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let usage_store = captain_memory::usage::UsageStore::new(state.kernel.memory.usage_conn());
    let agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .iter()
        .filter_map(|entry| {
            let hourly = usage_store.query_hourly(entry.id).unwrap_or(0.0);
            let daily = usage_store.query_daily(entry.id).unwrap_or(0.0);
            let monthly = usage_store.query_monthly(entry.id).unwrap_or(0.0);
            let hourly_tokens = state.kernel.scheduler.get_hourly_usage(entry.id);
            let tokens_used = hourly_tokens
                .as_ref()
                .map(|usage| usage.total_tokens)
                .unwrap_or(0);
            (hourly > 0.0 || daily > 0.0 || monthly > 0.0 || tokens_used > 0).then(|| {
                serde_json::json!({
                    "agent_id": entry.id.to_string(),
                    "name": entry.name,
                    "agent_name": entry.name,
                    "hourly_usd": hourly,
                    "daily_usd": daily,
                    "monthly_usd": monthly,
                    "daily_cost_usd": daily,
                    "tokens_used": tokens_used,
                    "tokens_reset_at": hourly_tokens.and_then(|usage| usage.resets_at),
                    "hourly_limit": entry.manifest.resources.max_cost_per_hour_usd,
                    "daily_limit": entry.manifest.resources.max_cost_per_day_usd,
                    "monthly_limit": entry.manifest.resources.max_cost_per_month_usd,
                    "max_llm_tokens_per_hour": entry.manifest.resources.max_llm_tokens_per_hour,
                })
            })
        })
        .collect();

    Json(serde_json::json!({"agents": agents, "total": agents.len()}))
}

/// PUT /api/budget/agents/{id} - Update per-agent budget limits.
pub async fn update_agent_budget(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let agent_id = match parse_agent_id(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let Some(object) = body.as_object() else {
        return bad_request("Agent budget update must be a JSON object");
    };
    let hourly = match optional_f64(object, "max_cost_per_hour_usd") {
        Ok(value) => value,
        Err(error) => return bad_request(error),
    };
    let daily = match optional_f64(object, "max_cost_per_day_usd") {
        Ok(value) => value,
        Err(error) => return bad_request(error),
    };
    let monthly = match optional_f64(object, "max_cost_per_month_usd") {
        Ok(value) => value,
        Err(error) => return bad_request(error),
    };
    let tokens = match optional_u64(object, "max_llm_tokens_per_hour") {
        Ok(value) => value,
        Err(error) => return bad_request(error),
    };

    if hourly.is_none() && daily.is_none() && monthly.is_none() && tokens.is_none() {
        return bad_request(
            "Provide at least one of: max_cost_per_hour_usd, max_cost_per_day_usd, max_cost_per_month_usd, max_llm_tokens_per_hour",
        );
    }

    match state
        .kernel
        .registry
        .update_resources(agent_id, hourly, daily, monthly, tokens)
    {
        Ok(()) => {
            if let Some(value) = tokens {
                state.kernel.scheduler.set_hourly_quota(agent_id, value);
            }
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                let _ = state.kernel.memory.save_agent(&entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "message": "Agent budget updated"})),
            )
        }
        Err(captain_types::error::CaptainError::InvalidInput(error)) => bad_request(error),
        Err(err) => not_found(format!("{err}")),
    }
}

fn parse_agent_id(id: &str) -> Result<AgentId, (StatusCode, Json<serde_json::Value>)> {
    id.parse().map_err(|_| bad_request("Invalid agent ID"))
}

fn ratio(spend: f64, limit: f64) -> f64 {
    if limit > 0.0 {
        spend / limit
    } else {
        0.0
    }
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({"error": message.into()})),
    )
}

fn not_found(message: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": message.into()})),
    )
}

fn internal_error(message: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": message.into()})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::put;
    use axum::Router;
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;
    use tower::ServiceExt;

    fn test_state() -> (tempfile::TempDir, Arc<AppState>) {
        let temporary = tempfile::tempdir().unwrap();
        let home_dir = temporary.path().join("home");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: temporary.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        std::fs::write(
            home_dir.join("config.toml"),
            toml::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();
        let state = Arc::new(AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        (temporary, state)
    }

    async fn put_budget(state: Arc<AppState>, body: &str) -> (StatusCode, String) {
        let response = Router::new()
            .route("/api/budget", put(update_budget))
            .with_state(state)
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/budget")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn budget_put_rejects_negative_non_finite_and_excessive_values() {
        let (_temporary, state) = test_state();
        for body in [
            r#"{"max_hourly_usd":-1}"#,
            r#"{"max_hourly_usd":NaN}"#,
            r#"{"max_hourly_usd":1e308}"#,
            r#"{"alert_threshold":1.1}"#,
        ] {
            let (status, response_body) = put_budget(Arc::clone(&state), body).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{body} must be rejected: {response_body}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn budget_reads_remain_coherent_during_concurrent_puts() {
        let (_temporary, state) = test_state();
        state
            .kernel
            .update_budget_config(|budget| {
                budget.max_hourly_usd = 1.0;
                budget.max_daily_usd = 10.0;
                budget.alert_threshold = 0.75;
            })
            .expect("initial durable budget update");
        let mut tasks = Vec::new();
        for index in 1..=24 {
            let writer_state = Arc::clone(&state);
            tasks.push(tokio::spawn(async move {
                let body = format!(
                    "{{\"max_hourly_usd\":{index},\"max_daily_usd\":{},\"alert_threshold\":0.75}}",
                    index * 10
                );
                let (status, response_body) = put_budget(writer_state, &body).await;
                assert_eq!(status, StatusCode::OK, "{response_body}");
            }));
            let reader_state = Arc::clone(&state);
            tasks.push(tokio::spawn(async move {
                for _ in 0..100 {
                    let snapshot = reader_state.kernel.budget_config();
                    snapshot.validate().unwrap();
                    if snapshot.max_hourly_usd > 0.0 {
                        assert_eq!(snapshot.max_daily_usd, snapshot.max_hourly_usd * 10.0);
                        assert_eq!(snapshot.alert_threshold, 0.75);
                    }
                    tokio::task::yield_now().await;
                }
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        let live = state.kernel.budget_config();
        let persisted: KernelConfig = toml::from_str(
            &std::fs::read_to_string(state.kernel.config.home_dir.join("config.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted.budget, live);
    }

    #[tokio::test]
    async fn agent_budget_update_validates_and_updates_enforced_token_quota() {
        let (_temporary, state) = test_state();
        let agent_id = state.kernel.registry.list()[0].id;

        let invalid = update_agent_budget(
            State(Arc::clone(&state)),
            Path(agent_id.to_string()),
            Json(serde_json::json!({"max_cost_per_hour_usd": -1.0})),
        )
        .await
        .into_response();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

        let updated = update_agent_budget(
            State(Arc::clone(&state)),
            Path(agent_id.to_string()),
            Json(serde_json::json!({"max_llm_tokens_per_hour": 1})),
        )
        .await
        .into_response();
        assert_eq!(updated.status(), StatusCode::OK);
        assert_eq!(
            state
                .kernel
                .registry
                .get(agent_id)
                .unwrap()
                .manifest
                .resources
                .max_llm_tokens_per_hour,
            1
        );
        assert_eq!(
            state.kernel.scheduler.token_headroom(agent_id),
            Some(1),
            "the scheduler must receive the quota published by the API"
        );
    }
}

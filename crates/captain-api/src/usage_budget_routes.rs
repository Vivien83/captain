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
    let status = state
        .kernel
        .metering
        .budget_status(&state.kernel.config.budget);
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
    let default_hourly_tokens = body["default_max_llm_tokens_per_hour"].as_u64();
    if default_hourly_tokens.is_some_and(|value| value > i64::MAX as u64) {
        return bad_request("default_max_llm_tokens_per_hour exceeds TOML integer range")
            .into_response();
    }
    let config_ptr = &state.kernel.config as *const captain_types::config::KernelConfig
        as *mut captain_types::config::KernelConfig;

    unsafe {
        if let Some(value) = body["max_hourly_usd"].as_f64() {
            (*config_ptr).budget.max_hourly_usd = value;
        }
        if let Some(value) = body["max_daily_usd"].as_f64() {
            (*config_ptr).budget.max_daily_usd = value;
        }
        if let Some(value) = body["max_monthly_usd"].as_f64() {
            (*config_ptr).budget.max_monthly_usd = value;
        }
        if let Some(value) = body["alert_threshold"].as_f64() {
            (*config_ptr).budget.alert_threshold = value.clamp(0.0, 1.0);
        }
        if let Some(value) = default_hourly_tokens {
            (*config_ptr).budget.default_max_llm_tokens_per_hour = value;
        }
    }

    persist_budget_config(&state);
    budget_status(State(state)).await.into_response()
}

fn persist_budget_config(state: &Arc<AppState>) {
    let config_path = state.kernel.config.home_dir.join("config.toml");
    let Ok(content) = std::fs::read_to_string(&config_path) else {
        return;
    };
    let Ok(mut doc) = content.parse::<toml::Value>() else {
        return;
    };
    let Some(root) = doc.as_table_mut() else {
        return;
    };
    let Some(budget_table) = root
        .entry("budget")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
    else {
        return;
    };
    update_persisted_budget_table(budget_table, &state.kernel.config.budget);
    if let Err(err) =
        captain_types::durable_fs::atomic_write(&config_path, doc.to_string().as_bytes())
    {
        tracing::warn!("Failed to persist budget to config.toml: {err}");
    }
}

fn update_persisted_budget_table(
    budget_table: &mut toml::map::Map<String, toml::Value>,
    budget: &captain_types::config::BudgetConfig,
) {
    budget_table.insert(
        "max_hourly_usd".into(),
        toml::Value::Float(budget.max_hourly_usd),
    );
    budget_table.insert(
        "max_daily_usd".into(),
        toml::Value::Float(budget.max_daily_usd),
    );
    budget_table.insert(
        "max_monthly_usd".into(),
        toml::Value::Float(budget.max_monthly_usd),
    );
    budget_table.insert(
        "alert_threshold".into(),
        toml::Value::Float(budget.alert_threshold),
    );
    budget_table.insert(
        "default_max_llm_tokens_per_hour".into(),
        toml::Value::Integer(
            i64::try_from(budget.default_max_llm_tokens_per_hour).unwrap_or(i64::MAX),
        ),
    );
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
    let hourly = body["max_cost_per_hour_usd"].as_f64();
    let daily = body["max_cost_per_day_usd"].as_f64();
    let monthly = body["max_cost_per_month_usd"].as_f64();
    let tokens = body["max_llm_tokens_per_hour"].as_u64();

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
            if let Some(entry) = state.kernel.registry.get(agent_id) {
                let _ = state.kernel.memory.save_agent(&entry);
            }
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "ok", "message": "Agent budget updated"})),
            )
        }
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

#[cfg(test)]
mod tests {
    use super::update_persisted_budget_table;
    use captain_types::config::BudgetConfig;

    #[test]
    fn persisted_budget_keeps_default_hourly_token_guard() {
        let budget = BudgetConfig {
            max_hourly_usd: 1.0,
            max_daily_usd: 5.0,
            max_monthly_usd: 25.0,
            alert_threshold: 0.75,
            default_max_llm_tokens_per_hour: 345_678,
        };
        let mut table = toml::map::Map::new();

        update_persisted_budget_table(&mut table, &budget);

        assert_eq!(
            table["default_max_llm_tokens_per_hour"].as_integer(),
            Some(345_678)
        );
        assert_eq!(table["max_hourly_usd"].as_float(), Some(1.0));
        assert_eq!(table["alert_threshold"].as_float(), Some(0.75));
    }
}

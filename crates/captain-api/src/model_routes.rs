use crate::state::AppState;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::{collections::HashMap, sync::Arc};

/// GET /api/models - List all models in the catalog.
pub async fn list_models(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let provider_filter = params.get("provider").map(|s| s.to_lowercase());
    let tier_filter = params.get("tier").map(|s| s.to_lowercase());
    let available_only = params
        .get("available")
        .map(|value| value == "true" || value == "1")
        .unwrap_or(false);

    let models: Vec<serde_json::Value> = catalog
        .list_models()
        .iter()
        .filter(|model| {
            if let Some(ref provider) = provider_filter {
                if model.provider.to_lowercase() != *provider {
                    return false;
                }
            }
            if let Some(ref tier) = tier_filter {
                if model.tier.to_string() != *tier {
                    return false;
                }
            }
            if available_only {
                let provider = catalog.get_provider(&model.provider);
                if let Some(provider) = provider {
                    if provider.auth_status == captain_types::model_catalog::AuthStatus::Missing {
                        return false;
                    }
                }
            }
            true
        })
        .map(|model| {
            let available = catalog
                .get_provider(&model.provider)
                .map(|provider| {
                    provider.auth_status != captain_types::model_catalog::AuthStatus::Missing
                })
                .unwrap_or(model.tier == captain_types::model_catalog::ModelTier::Custom);
            serde_json::json!({
                "id": model.id,
                "display_name": model.display_name,
                "provider": model.provider,
                "tier": model.tier,
                "context_window": model.context_window,
                "max_output_tokens": model.max_output_tokens,
                "input_cost_per_m": model.input_cost_per_m,
                "output_cost_per_m": model.output_cost_per_m,
                "supports_tools": model.supports_tools,
                "supports_vision": model.supports_vision,
                "supports_streaming": model.supports_streaming,
                "available": available,
            })
        })
        .collect();

    let total = catalog.list_models().len();
    let available_count = catalog.available_models().len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "models": models,
            "total": total,
            "available": available_count,
        })),
    )
}

/// GET /api/models/aliases - List all alias-to-model mappings.
pub async fn list_aliases(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let aliases = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .list_aliases()
        .clone();
    let entries: Vec<serde_json::Value> = aliases
        .iter()
        .map(|(alias, model_id)| {
            serde_json::json!({
                "alias": alias,
                "model_id": model_id,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "aliases": entries,
            "total": entries.len(),
        })),
    )
}

/// GET /api/models/{id} - Get a single model by ID or alias.
pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let catalog = state
        .kernel
        .model_catalog
        .read()
        .unwrap_or_else(|e| e.into_inner());
    match catalog.find_model(&id) {
        Some(model) => {
            let available = catalog
                .get_provider(&model.provider)
                .map(|provider| {
                    provider.auth_status != captain_types::model_catalog::AuthStatus::Missing
                })
                .unwrap_or(model.tier == captain_types::model_catalog::ModelTier::Custom);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": model.id,
                    "display_name": model.display_name,
                    "provider": model.provider,
                    "tier": model.tier,
                    "context_window": model.context_window,
                    "max_output_tokens": model.max_output_tokens,
                    "input_cost_per_m": model.input_cost_per_m,
                    "output_cost_per_m": model.output_cost_per_m,
                    "supports_tools": model.supports_tools,
                    "supports_vision": model.supports_vision,
                    "supports_streaming": model.supports_streaming,
                    "aliases": model.aliases,
                    "available": available,
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Model '{}' not found", id)})),
        ),
    }
}

/// GET /api/providers - List all providers with auth and local probe status.
pub async fn list_providers(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let provider_list: Vec<captain_types::model_catalog::ProviderInfo> = {
        let catalog = state
            .kernel
            .model_catalog
            .read()
            .unwrap_or_else(|e| e.into_inner());
        catalog.list_providers().to_vec()
    };

    let local_providers: Vec<(usize, String, String)> = provider_list
        .iter()
        .enumerate()
        .filter(|(_, provider)| !provider.key_required && !provider.base_url.is_empty())
        .map(|(index, provider)| (index, provider.id.clone(), provider.base_url.clone()))
        .collect();

    let cache = &state.provider_probe_cache;
    let probe_futures: Vec<_> = local_providers
        .iter()
        .map(|(_, id, url)| captain_runtime::provider_health::probe_provider_cached(id, url, cache))
        .collect();
    let probe_results = futures::future::join_all(probe_futures).await;

    let mut probe_map: HashMap<usize, captain_runtime::provider_health::ProbeResult> =
        HashMap::with_capacity(local_providers.len());
    for ((index, _, _), result) in local_providers.iter().zip(probe_results) {
        probe_map.insert(*index, result);
    }

    let mut providers: Vec<serde_json::Value> = Vec::with_capacity(provider_list.len());

    for (index, provider) in provider_list.iter().enumerate() {
        let mut entry = serde_json::json!({
            "id": provider.id,
            "display_name": provider.display_name,
            "auth_status": provider.auth_status,
            "model_count": provider.model_count,
            "key_required": provider.key_required,
            "api_key_env": provider.api_key_env,
            "base_url": provider.base_url,
        });

        if let Some(probe) = probe_map.remove(&index) {
            entry["is_local"] = serde_json::json!(true);
            entry["reachable"] = serde_json::json!(probe.reachable);
            entry["latency_ms"] = serde_json::json!(probe.latency_ms);
            if !probe.discovered_models.is_empty() {
                entry["discovered_models"] = serde_json::json!(probe.discovered_models);
                if let Ok(mut catalog) = state.kernel.model_catalog.write() {
                    catalog.merge_discovered_models(&provider.id, &probe.discovered_models);
                }
            }
            if let Some(error) = &probe.error {
                entry["error"] = serde_json::json!(error);
            }
        } else if !provider.key_required {
            entry["is_local"] = serde_json::json!(true);
        }

        providers.push(entry);
    }

    let total = providers.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "providers": providers,
            "total": total,
        })),
    )
}

/// POST /api/models/custom - Add a custom model to the catalog.
pub async fn add_custom_model(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let id = body
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
    let provider = body
        .get("provider")
        .and_then(|value| value.as_str())
        .unwrap_or("openrouter")
        .to_string();
    let context_window = body
        .get("context_window")
        .and_then(|value| value.as_u64())
        .unwrap_or(128_000);
    let max_output = body
        .get("max_output_tokens")
        .and_then(|value| value.as_u64())
        .unwrap_or(8_192);

    if id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing required field: id"})),
        );
    }

    let display = body
        .get("display_name")
        .and_then(|value| value.as_str())
        .unwrap_or(&id)
        .to_string();

    let entry = captain_types::model_catalog::ModelCatalogEntry {
        id: id.clone(),
        display_name: display,
        provider: provider.clone(),
        tier: captain_types::model_catalog::ModelTier::Custom,
        context_window,
        max_output_tokens: max_output,
        input_cost_per_m: body
            .get("input_cost_per_m")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0),
        output_cost_per_m: body
            .get("output_cost_per_m")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0),
        supports_tools: body
            .get("supports_tools")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        supports_vision: body
            .get("supports_vision")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        supports_streaming: body
            .get("supports_streaming")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        aliases: vec![],
    };

    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.add_custom_model(entry) {
        return (
            StatusCode::CONFLICT,
            Json(
                serde_json::json!({"error": format!("Model '{}' already exists for provider '{}'", id, provider)}),
            ),
        );
    }

    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "provider": provider,
            "status": "added"
        })),
    )
}

/// PATCH /api/models/{id}/pricing - Update pricing for any model.
pub async fn update_model_pricing(
    State(state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let input_cost = body
        .get("input_cost_per_m")
        .and_then(|value| value.as_f64());
    let output_cost = body
        .get("output_cost_per_m")
        .and_then(|value| value.as_f64());

    if input_cost.is_none() && output_cost.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Provide input_cost_per_m and/or output_cost_per_m"})),
        );
    }

    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());
    let (current_in, current_out) = catalog.pricing(&model_id).unwrap_or((1.0, 3.0));
    let new_in = input_cost.unwrap_or(current_in);
    let new_out = output_cost.unwrap_or(current_out);

    if catalog.update_pricing(&model_id, new_in, new_out) {
        tracing::info!(model = %model_id, input = new_in, output = new_out, "Model pricing updated");
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": model_id,
                "input_cost_per_m": new_in,
                "output_cost_per_m": new_out,
            })),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Model '{}' not found", model_id)})),
        )
    }
}

/// DELETE /api/models/custom/{id} - Remove a custom model.
pub async fn remove_custom_model(
    State(state): State<Arc<AppState>>,
    Path(model_id): Path<String>,
) -> impl IntoResponse {
    let mut catalog = state
        .kernel
        .model_catalog
        .write()
        .unwrap_or_else(|e| e.into_inner());

    if !catalog.remove_custom_model(&model_id) {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Custom model '{}' not found", model_id)})),
        );
    }

    let custom_path = state.kernel.config.home_dir.join("custom_models.json");
    if let Err(e) = catalog.save_custom_models(&custom_path) {
        tracing::warn!("Failed to persist custom models: {e}");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "removed"})),
    )
}

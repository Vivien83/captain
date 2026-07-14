use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

pub(crate) fn server_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// GET /api/hands - List all hand definitions.
pub async fn list_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let defs = state.kernel.hand_registry.list_definitions();
    let hands: Vec<serde_json::Value> = defs
        .iter()
        .map(|d| {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&d.id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&d.id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            serde_json::json!({
                "id": d.id,
                "name": d.name,
                "description": d.description,
                "category": d.category,
                "icon": d.icon,
                "tools": d.tools,
                "requirements_met": requirements_met,
                "active": active,
                "degraded": degraded,
                "requirements": reqs.iter().map(|(r, ok)| serde_json::json!({
                    "key": r.key,
                    "label": r.label,
                    "satisfied": ok,
                    "optional": r.optional,
                })).collect::<Vec<_>>(),
                "dashboard_metrics": d.dashboard.metrics.len(),
                "has_settings": !d.settings.is_empty(),
                "settings_count": d.settings.len(),
            })
        })
        .collect();

    Json(serde_json::json!({ "hands": hands, "total": hands.len() }))
}

/// GET /api/hands/active - List active hand instances.
pub async fn list_active_hands(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let instances = state.kernel.hand_registry.list_instances();
    let items: Vec<serde_json::Value> = instances
        .iter()
        .map(|i| {
            serde_json::json!({
                "instance_id": i.instance_id,
                "hand_id": i.hand_id,
                "status": format!("{}", i.status),
                "agent_id": i.agent_id.map(|a| a.to_string()),
                "agent_name": i.agent_name,
                "activated_at": i.activated_at.to_rfc3339(),
                "updated_at": i.updated_at.to_rfc3339(),
            })
        })
        .collect();

    Json(serde_json::json!({ "instances": items, "total": items.len() }))
}

/// GET /api/hands/{hand_id} - Get a single hand definition with requirements.
pub async fn get_hand(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&hand_id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            let settings_status = state
                .kernel
                .hand_registry
                .check_settings_availability(&hand_id)
                .unwrap_or_default();
            let default_model = state.kernel.effective_default_model();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "id": def.id,
                    "name": def.name,
                    "description": def.description,
                    "category": def.category,
                    "icon": def.icon,
                    "tools": def.tools,
                    "requirements_met": requirements_met,
                    "active": active,
                    "degraded": degraded,
                    "requirements": reqs.iter().map(|(r, ok)| {
                        let mut req_json = serde_json::json!({
                            "key": r.key,
                            "label": r.label,
                            "type": format!("{:?}", r.requirement_type),
                            "check_value": r.check_value,
                            "satisfied": ok,
                            "optional": r.optional,
                        });
                        if let Some(ref desc) = r.description {
                            req_json["description"] = serde_json::json!(desc);
                        }
                        if let Some(ref install) = r.install {
                            req_json["install"] = serde_json::to_value(install).unwrap_or_default();
                        }
                        req_json
                    }).collect::<Vec<_>>(),
                    "server_platform": server_platform(),
                    "agent": {
                        "name": def.agent.name,
                        "description": def.agent.description,
                        "provider": if def.agent.provider == "default" {
                            default_model.provider.as_str()
                        } else { &def.agent.provider },
                        "model": if def.agent.model == "default" {
                            default_model.model.as_str()
                        } else { &def.agent.model },
                    },
                    "dashboard": def.dashboard.metrics.iter().map(|m| serde_json::json!({
                        "label": m.label,
                        "memory_key": m.memory_key,
                        "format": m.format,
                    })).collect::<Vec<_>>(),
                    "settings": settings_status,
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
        ),
    }
}

/// POST /api/hands/{hand_id}/check-deps - Re-check dependency status for a hand.
pub async fn check_hand_deps(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(def) => {
            let reqs = state
                .kernel
                .hand_registry
                .check_requirements(&hand_id)
                .unwrap_or_default();
            let readiness = state.kernel.hand_registry.readiness(&hand_id);
            let requirements_met = readiness
                .as_ref()
                .map(|r| r.requirements_met)
                .unwrap_or(false);
            let active = readiness.as_ref().map(|r| r.active).unwrap_or(false);
            let degraded = readiness.as_ref().map(|r| r.degraded).unwrap_or(false);
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "hand_id": def.id,
                    "requirements_met": requirements_met,
                    "active": active,
                    "degraded": degraded,
                    "server_platform": server_platform(),
                    "requirements": reqs.iter().map(|(r, ok)| {
                        let mut req_json = serde_json::json!({
                            "key": r.key,
                            "label": r.label,
                            "type": format!("{:?}", r.requirement_type),
                            "check_value": r.check_value,
                            "satisfied": ok,
                            "optional": r.optional,
                        });
                        if let Some(ref desc) = r.description {
                            req_json["description"] = serde_json::json!(desc);
                        }
                        if let Some(ref install) = r.install {
                            req_json["install"] = serde_json::to_value(install).unwrap_or_default();
                        }
                        req_json
                    }).collect::<Vec<_>>(),
                })),
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_platform_returns_known_platform_name() {
        assert!(matches!(server_platform(), "macos" | "windows" | "linux"));
    }
}

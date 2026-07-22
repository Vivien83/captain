//! Installed-skill API and compatibility tombstone for SkillSynthesizer v3.13.

use crate::state::AppState;
use crate::types::{SkillInstallRequest, SkillUninstallRequest};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;

/// Compatibility tombstone for the retired v3.13 SkillSynthesizer.
///
/// Historical rows remain available as a read-only SQLite audit archive. They
/// cannot be promoted because they lack immutable staging and validation
/// evidence required by Skill Learning V2.
pub async fn retired_skill_synthesizer() -> impl IntoResponse {
    (
        StatusCode::GONE,
        Json(serde_json::json!({
            "error": "The v3.13 SkillSynthesizer is retired",
            "replacement": "/api/learning/workflows",
            "archived": true,
            "migration": "v32"
        })),
    )
}

pub async fn list_proposals(
    State(_state): State<Arc<AppState>>,
    Query(_params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    retired_skill_synthesizer().await
}

pub async fn list_patterns(
    State(_state): State<Arc<AppState>>,
    Query(_params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    retired_skill_synthesizer().await
}

pub async fn decide_proposal(
    State(_state): State<Arc<AppState>>,
    Path(_proposal_id): Path<String>,
    Json(_body): Json<serde_json::Value>,
) -> impl IntoResponse {
    retired_skill_synthesizer().await
}

pub async fn metrics(State(_state): State<Arc<AppState>>) -> impl IntoResponse {
    retired_skill_synthesizer().await
}

/// GET /api/skills - List available skills.
pub async fn list_skills(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let registry = state
        .kernel
        .skill_registry
        .read()
        .unwrap_or_else(|e| e.into_inner());

    let mut skills: Vec<serde_json::Value> = registry
        .list()
        .iter()
        .map(|s| {
            let governance = captain_skills::skill_governance_from_tags(&s.manifest.skill.tags);
            let source = match &s.manifest.source {
                Some(captain_skills::SkillSource::ClawHub { slug, version }) => {
                    serde_json::json!({"type": "clawhub", "slug": slug, "version": version})
                }
                Some(captain_skills::SkillSource::OpenClaw) => {
                    serde_json::json!({"type": "openclaw"})
                }
                Some(captain_skills::SkillSource::Bundled) => {
                    serde_json::json!({"type": "bundled"})
                }
                Some(captain_skills::SkillSource::Native) | None => {
                    serde_json::json!({"type": "local"})
                }
            };
            serde_json::json!({
                "name": s.manifest.skill.name,
                "description": s.manifest.skill.description,
                "version": s.manifest.skill.version,
                "author": s.manifest.skill.author,
                "runtime": format!("{:?}", s.manifest.runtime.runtime_type),
                "tools_count": s.manifest.tools.provided.len(),
                "tags": s.manifest.skill.tags,
                "governance": governance,
                "enabled": s.enabled,
                "source": source,
                "has_prompt_context": s.manifest.prompt_context.is_some(),
            })
        })
        .collect();

    let workspaces_dir = state
        .kernel
        .config
        .workspaces_dir
        .clone()
        .unwrap_or_else(|| state.kernel.config.home_dir.join("workspaces"));
    if let Ok(entries) = std::fs::read_dir(&workspaces_dir) {
        for entry in entries.flatten() {
            let skills_dir = entry.path().join("skills");
            if !skills_dir.is_dir() {
                continue;
            }
            if let Ok(skill_entries) = std::fs::read_dir(&skills_dir) {
                for skill_entry in skill_entries.flatten() {
                    let skill_path = skill_entry.path();
                    let skill_md = skill_path.join("SKILL.md");
                    if !skill_md.exists() {
                        continue;
                    }
                    let skill_name = skill_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    if skills
                        .iter()
                        .any(|s| s.get("name").and_then(|v| v.as_str()) == Some(&skill_name))
                    {
                        continue;
                    }
                    let workspace_name = entry.file_name().to_string_lossy().to_string();
                    let description = std::fs::read_to_string(&skill_md)
                        .ok()
                        .and_then(|content| {
                            content
                                .lines()
                                .find(|line| !line.starts_with('#') && !line.trim().is_empty())
                                .map(|line| line.trim().to_string())
                        })
                        .unwrap_or_default();
                    skills.push(serde_json::json!({
                        "name": skill_name,
                        "description": description,
                        "version": "custom",
                        "author": workspace_name,
                        "runtime": "Shell",
                        "tools_count": 0,
                        "tags": ["custom", "workspace"],
                        "governance": {
                            "generated": false,
                            "quarantined": false,
                            "locked": false
                        },
                        "enabled": true,
                        "source": { "type": "workspace", "agent": workspace_name },
                        "has_prompt_context": true,
                    }));
                }
            }
        }
    }

    let total = skills.len();
    Json(serde_json::json!({ "skills": skills, "total": total }))
}

/// POST /api/skills/install - Install a skill from Captain Marketplace.
pub async fn install_skill(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SkillInstallRequest>,
) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let config = captain_skills::marketplace::MarketplaceConfig::default();
    let client = captain_skills::marketplace::MarketplaceClient::new(config);

    match client.install(&req.name, &skills_dir).await {
        Ok(version) => {
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": req.name,
                    "version": version,
                })),
            )
        }
        Err(e) => {
            tracing::warn!("Skill install failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Install failed: {e}")})),
            )
        }
    }
}

/// POST /api/skills/uninstall - Uninstall a skill.
pub async fn uninstall_skill(
    State(state): State<Arc<AppState>>,
    Json(req): Json<SkillUninstallRequest>,
) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let mut registry = captain_skills::registry::SkillRegistry::new(skills_dir);
    let _ = registry.load_all();

    match registry.remove(&req.name) {
        Ok(()) => {
            state.kernel.reload_skills();
            (
                StatusCode::OK,
                Json(serde_json::json!({"status": "uninstalled", "name": req.name})),
            )
        }
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// GET /api/marketplace/search - Search the Captain Marketplace.
pub async fn marketplace_search(
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return Json(serde_json::json!({"results": [], "total": 0}));
    }

    let config = captain_skills::marketplace::MarketplaceConfig::default();
    let client = captain_skills::marketplace::MarketplaceClient::new(config);

    match client.search(&query).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "name": r.name,
                        "description": r.description,
                        "stars": r.stars,
                        "url": r.url,
                    })
                })
                .collect();
            Json(serde_json::json!({"results": items, "total": items.len()}))
        }
        Err(e) => {
            tracing::warn!("Marketplace search failed: {e}");
            Json(serde_json::json!({"results": [], "total": 0, "error": format!("{e}")}))
        }
    }
}

/// POST /api/skills/create - Create a local prompt-only skill.
pub async fn create_skill(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let name = match body["name"].as_str() {
        Some(name) if !name.trim().is_empty() => name.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "Missing or empty 'name' field"})),
            );
        }
    };

    if !name
        .chars()
        .all(|char| char.is_alphanumeric() || char == '-' || char == '_')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Skill name must contain only letters, numbers, hyphens, and underscores"}),
            ),
        );
    }

    let description = body["description"].as_str().unwrap_or("").to_string();
    let runtime = body["runtime"].as_str().unwrap_or("prompt_only");
    let prompt_context = body["prompt_context"].as_str().unwrap_or("").to_string();

    if runtime != "prompt_only" {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Only prompt_only skills can be created from the web UI"}),
            ),
        );
    }

    let skill_dir = state.kernel.config.home_dir.join("skills").join(&name);
    if skill_dir.exists() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": format!("Skill '{}' already exists", name)})),
        );
    }

    let toml_content = format!(
        "[skill]\nname = \"{}\"\ndescription = \"{}\"\nruntime = \"prompt_only\"\n\n[prompt]\ncontext = \"\"\"\n{}\n\"\"\"\n",
        name,
        description.replace('"', "\\\""),
        prompt_context
    );

    let toml_path = skill_dir.join("skill.toml");
    if let Err(e) = captain_types::durable_fs::atomic_write(&toml_path, toml_content.as_bytes()) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to persist skill.toml: {e}")})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "created",
            "name": name,
            "note": "Restart the daemon to load the new skill, or it will be available on next boot."
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn legacy_synthesizer_returns_actionable_gone_response() {
        let response = retired_skill_synthesizer().await.into_response();
        assert_eq!(response.status(), StatusCode::GONE);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["replacement"], "/api/learning/workflows");
        assert_eq!(value["archived"], true);
        assert_eq!(value["migration"], "v32");
    }
}

use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// GET /api/clawhub/search - Search ClawHub skills using vector/semantic search.
pub async fn clawhub_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let query = params.get("q").cloned().unwrap_or_default();
    if query.is_empty() {
        return (
            StatusCode::OK,
            Json(serde_json::json!({"items": [], "next_cursor": null})),
        );
    }

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let cache_key = format!("search:{}:{}", query, limit);
    if let Some(entry) = state.clawhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = captain_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.config.home_dir.join("skills");
    match client.search(&query, limit).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .results
                .iter()
                .map(|e| {
                    let installed = skills_dir.join(&e.slug).exists();
                    serde_json::json!({
                        "slug": e.slug,
                        "name": e.display_name,
                        "description": e.summary,
                        "version": e.version,
                        "score": e.score,
                        "updated_at": e.updated_at,
                        "installed": installed,
                    })
                })
                .collect();
            let resp = serde_json::json!({
                "items": items,
                "next_cursor": null,
            });
            state
                .clawhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("ClawHub search failed: {msg}");
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(serde_json::json!({"items": [], "next_cursor": null, "error": msg})),
            )
        }
    }
}

/// GET /api/clawhub/browse - Browse ClawHub skills by sort order.
pub async fn clawhub_browse(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let sort = match params.get("sort").map(|s| s.as_str()) {
        Some("downloads") => captain_skills::clawhub::ClawHubSort::Downloads,
        Some("stars") => captain_skills::clawhub::ClawHubSort::Stars,
        Some("updated") => captain_skills::clawhub::ClawHubSort::Updated,
        Some("rating") => captain_skills::clawhub::ClawHubSort::Rating,
        _ => captain_skills::clawhub::ClawHubSort::Trending,
    };

    let limit: u32 = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);

    let cursor = params.get("cursor").map(|s| s.as_str());

    let cache_key = format!("browse:{:?}:{}:{}", sort, limit, cursor.unwrap_or(""));
    if let Some(entry) = state.clawhub_cache.get(&cache_key) {
        if entry.0.elapsed().as_secs() < 120 {
            return (StatusCode::OK, Json(entry.1.clone()));
        }
    }

    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = captain_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.config.home_dir.join("skills");
    match client.browse(sort, limit, cursor).await {
        Ok(results) => {
            let items: Vec<serde_json::Value> = results
                .items
                .iter()
                .map(|entry| {
                    let mut json = clawhub_browse_entry_to_json(entry);
                    let installed = skills_dir.join(&entry.slug).exists();
                    json["installed"] = serde_json::json!(installed);
                    json
                })
                .collect();
            let resp = serde_json::json!({
                "items": items,
                "next_cursor": results.next_cursor,
            });
            state
                .clawhub_cache
                .insert(cache_key, (Instant::now(), resp.clone()));
            (StatusCode::OK, Json(resp))
        }
        Err(e) => {
            let msg = format!("{e}");
            tracing::warn!("ClawHub browse failed: {msg}");
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::OK
            };
            (
                status,
                Json(serde_json::json!({"items": [], "next_cursor": null, "error": msg})),
            )
        }
    }
}

/// GET /api/clawhub/skill/{slug} - Get detailed info about a ClawHub skill.
pub async fn clawhub_skill_detail(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = captain_skills::clawhub::ClawHubClient::new(cache_dir);

    let skills_dir = state.kernel.config.home_dir.join("skills");
    let is_installed = client.is_installed(&slug, &skills_dir);

    match client.get_skill(&slug).await {
        Ok(detail) => {
            let version = detail
                .latest_version
                .as_ref()
                .map(|v| v.version.as_str())
                .unwrap_or("");
            let author = detail
                .owner
                .as_ref()
                .map(|o| o.handle.as_str())
                .unwrap_or("");
            let author_name = detail
                .owner
                .as_ref()
                .map(|o| o.display_name.as_str())
                .unwrap_or("");
            let author_image = detail
                .owner
                .as_ref()
                .map(|o| o.image.as_str())
                .unwrap_or("");

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "slug": detail.skill.slug,
                    "name": detail.skill.display_name,
                    "description": detail.skill.summary,
                    "version": version,
                    "downloads": detail.skill.stats.downloads,
                    "stars": detail.skill.stats.stars,
                    "author": author,
                    "author_name": author_name,
                    "author_image": author_image,
                    "tags": detail.skill.tags,
                    "updated_at": detail.skill.updated_at,
                    "created_at": detail.skill.created_at,
                    "installed": is_installed,
                })),
            )
        }
        Err(e) => {
            let status = if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::NOT_FOUND
            };
            (status, Json(serde_json::json!({"error": format!("{e}")})))
        }
    }
}

/// GET /api/clawhub/skill/{slug}/code - Fetch the source code of a ClawHub skill.
pub async fn clawhub_skill_code(
    State(state): State<Arc<AppState>>,
    Path(slug): Path<String>,
) -> impl IntoResponse {
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = captain_skills::clawhub::ClawHubClient::new(cache_dir);

    let mut code = String::new();
    let mut filename = String::new();

    if let Ok(content) = client.get_file(&slug, "SKILL.md").await {
        code = content;
        filename = "SKILL.md".to_string();
    } else if let Ok(content) = client.get_file(&slug, "package.json").await {
        code = content;
        filename = "package.json".to_string();
    } else if let Ok(content) = client.get_file(&slug, "skill.toml").await {
        code = content;
        filename = "skill.toml".to_string();
    }

    if code.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "No source code found for this skill"})),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "slug": slug,
            "filename": filename,
            "code": code,
        })),
    )
}

/// POST /api/clawhub/install - Install a skill from ClawHub.
pub async fn clawhub_install(
    State(state): State<Arc<AppState>>,
    Json(req): Json<crate::types::ClawHubInstallRequest>,
) -> impl IntoResponse {
    let skills_dir = state.kernel.config.home_dir.join("skills");
    let cache_dir = state.kernel.config.home_dir.join(".cache").join("clawhub");
    let client = captain_skills::clawhub::ClawHubClient::new(cache_dir);

    if client.is_installed(&req.slug, &skills_dir) {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": format!("Skill '{}' is already installed", req.slug),
                "status": "already_installed",
            })),
        );
    }

    match client.install(&req.slug, &skills_dir).await {
        Ok(result) => {
            let warnings: Vec<serde_json::Value> = result
                .warnings
                .iter()
                .map(|w| {
                    serde_json::json!({
                        "severity": format!("{:?}", w.severity),
                        "message": w.message,
                    })
                })
                .collect();

            let translations: Vec<serde_json::Value> = result
                .tool_translations
                .iter()
                .map(|(from, to)| serde_json::json!({"from": from, "to": to}))
                .collect();

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "installed",
                    "name": result.skill_name,
                    "version": result.version,
                    "slug": result.slug,
                    "is_prompt_only": result.is_prompt_only,
                    "warnings": warnings,
                    "tool_translations": translations,
                })),
            )
        }
        Err(e) => {
            let msg = format!("{e}");
            let status = if matches!(e, captain_skills::SkillError::SecurityBlocked(_)) {
                StatusCode::FORBIDDEN
            } else if is_clawhub_rate_limit(&e) {
                StatusCode::TOO_MANY_REQUESTS
            } else if matches!(e, captain_skills::SkillError::Network(_)) {
                StatusCode::BAD_GATEWAY
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            tracing::warn!("ClawHub install failed: {msg}");
            (status, Json(serde_json::json!({"error": msg})))
        }
    }
}

fn is_clawhub_rate_limit(err: &captain_skills::SkillError) -> bool {
    matches!(err, captain_skills::SkillError::RateLimited(_))
}

fn clawhub_browse_entry_to_json(
    entry: &captain_skills::clawhub::ClawHubBrowseEntry,
) -> serde_json::Value {
    let version = captain_skills::clawhub::ClawHubClient::entry_version(entry);
    serde_json::json!({
        "slug": entry.slug,
        "name": entry.display_name,
        "description": entry.summary,
        "version": version,
        "downloads": entry.stats.downloads,
        "stars": entry.stats.stars,
        "updated_at": entry.updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_skills::clawhub::{ClawHubBrowseEntry, ClawHubStats, ClawHubVersionInfo};

    #[test]
    fn is_clawhub_rate_limit_detects_only_rate_limit_errors() {
        assert!(is_clawhub_rate_limit(
            &captain_skills::SkillError::RateLimited("retry later".to_string())
        ));
        assert!(!is_clawhub_rate_limit(
            &captain_skills::SkillError::Network("offline".to_string())
        ));
    }

    #[test]
    fn clawhub_browse_entry_json_flattens_frontend_fields() {
        let entry = ClawHubBrowseEntry {
            slug: "github-helper".to_string(),
            display_name: "GitHub Helper".to_string(),
            summary: "Useful GitHub workflow helpers".to_string(),
            tags: HashMap::new(),
            stats: ClawHubStats {
                downloads: 42,
                stars: 7,
                ..Default::default()
            },
            created_at: 1,
            updated_at: 2,
            latest_version: Some(ClawHubVersionInfo {
                version: "1.2.3".to_string(),
                ..Default::default()
            }),
        };

        let json = clawhub_browse_entry_to_json(&entry);

        assert_eq!(json["slug"], "github-helper");
        assert_eq!(json["name"], "GitHub Helper");
        assert_eq!(json["version"], "1.2.3");
        assert_eq!(json["downloads"], 42);
        assert_eq!(json["stars"], 7);
        assert_eq!(json["updated_at"], 2);
    }
}

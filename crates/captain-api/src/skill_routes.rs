//! REST surface for the v3.13 SkillSynthesizer API.
//!
//! - GET /api/skills/proposals - list pending drafts awaiting review
//! - GET /api/skills/patterns - recent detected patterns (debug view)
//! - POST /api/skills/proposals/{id}/decide - approve or deny
//! - GET /api/skills/metrics - counts for the sidebar

use crate::state::AppState;
use crate::types::{SkillInstallRequest, SkillUninstallRequest};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_memory::{skill_patterns, skill_proposals};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

fn server_error(msg: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({ "error": msg })),
    )
}

fn bad_request(msg: String) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(serde_json::json!({ "error": msg })),
    )
}

fn parse_limit(params: &HashMap<String, String>, default: usize, cap: usize) -> usize {
    params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
        .min(cap)
}

pub async fn list_proposals(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = parse_limit(&params, 50, 500);
    let conn = state.kernel.memory.usage_conn();
    let rows = {
        let guard = match conn.lock() {
            Ok(g) => g,
            Err(e) => return server_error(format!("sqlite poisoned: {e}")),
        };
        match skill_proposals::list_pending(&guard, limit) {
            Ok(r) => r,
            Err(e) => return server_error(e.to_string()),
        }
    };
    let pending = rows
        .into_iter()
        .map(|row| {
            let mut value = serde_json::to_value(row).unwrap_or_default();
            captain_runtime::skill_proposer::localize_skill_proposal_value(
                &mut value,
                &state.kernel.config.language,
            );
            value
        })
        .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(serde_json::json!({ "pending": pending })),
    )
}

pub async fn list_patterns(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = parse_limit(&params, 50, 500);
    let threshold = params
        .get("threshold")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1);
    let window_days = params
        .get("window_days")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(30);
    let conn = state.kernel.memory.usage_conn();
    let rows = {
        let guard = match conn.lock() {
            Ok(g) => g,
            Err(e) => return server_error(format!("sqlite poisoned: {e}")),
        };
        match skill_patterns::list_ready(&guard, threshold, window_days, limit) {
            Ok(r) => r,
            Err(e) => return server_error(e.to_string()),
        }
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({ "patterns": rows })),
    )
}

#[derive(Deserialize)]
pub struct DecideBody {
    pub approve: bool,
    #[serde(default)]
    pub decided_by: Option<String>,
    #[serde(default)]
    pub verification: Option<SkillProposalApprovalVerification>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillProposalApprovalVerification {
    #[serde(default)]
    pub schema_reviewed: bool,
    #[serde(default)]
    pub diff_reviewed: bool,
    #[serde(default)]
    pub tests_reviewed: bool,
    #[serde(default)]
    pub human_approved: bool,
}

impl SkillProposalApprovalVerification {
    fn complete(&self) -> bool {
        self.schema_reviewed && self.diff_reviewed && self.tests_reviewed && self.human_approved
    }
}

fn skill_proposal_decided_by_for_api(body: &DecideBody) -> Result<String, String> {
    let decided_by = body
        .decided_by
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("api");
    if !body.approve {
        return Ok(decided_by.to_string());
    }
    if !body.verification.as_ref().is_some_and(|v| v.complete()) {
        return Err(
            "approve=true requires verification.schema_reviewed, diff_reviewed, tests_reviewed and human_approved".to_string(),
        );
    }
    Ok(captain_runtime::kernel_handle::skill_proposal_approval_decider(decided_by))
}

pub async fn decide_proposal(
    State(state): State<Arc<AppState>>,
    Path(proposal_id): Path<String>,
    Json(body): Json<DecideBody>,
) -> impl IntoResponse {
    use captain_runtime::kernel_handle::KernelHandle;
    let kh: Arc<dyn KernelHandle> = Arc::clone(&state.kernel) as Arc<dyn KernelHandle>;
    let decided_by = match skill_proposal_decided_by_for_api(&body) {
        Ok(value) => value,
        Err(e) => return bad_request(e),
    };
    match kh
        .skill_proposal_decide(&proposal_id, body.approve, Some(decided_by.as_str()))
        .await
    {
        Ok(v) => (StatusCode::OK, Json(v)),
        Err(e) => {
            let lower = e.to_lowercase();
            if lower.contains("not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": e })),
                )
            } else if lower.contains("already decided") {
                (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({ "error": e })),
                )
            } else {
                bad_request(e)
            }
        }
    }
}

pub async fn metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let conn = state.kernel.memory.usage_conn();
    let guard = match conn.lock() {
        Ok(g) => g,
        Err(e) => return server_error(format!("sqlite poisoned: {e}")),
    };
    let pending = skill_proposals::list_pending(&guard, 10_000)
        .map(|v| v.len() as i64)
        .unwrap_or(0);
    let patterns_hot = skill_patterns::list_ready(&guard, 3, 7, 10_000)
        .map(|v| v.len() as i64)
        .unwrap_or(0);
    let total_patterns: i64 = guard
        .query_row("SELECT COUNT(*) FROM skill_patterns", [], |r| r.get(0))
        .unwrap_or(0);
    let total_approved: i64 = guard
        .query_row(
            "SELECT COUNT(*) FROM skill_proposals WHERE status = 'approved'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let total_denied: i64 = guard
        .query_row(
            "SELECT COUNT(*) FROM skill_proposals WHERE status = 'denied'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "pending": pending,
            "patterns_hot": patterns_hot,
            "total_patterns": total_patterns,
            "approved": total_approved,
            "denied": total_denied,
            "skills_mode": format!("{:?}", state.kernel.config.skills.mode).to_lowercase(),
            "skills_enabled": state.kernel.config.skills.enabled,
        })),
    )
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
    use super::{skill_proposal_decided_by_for_api, DecideBody, SkillProposalApprovalVerification};

    #[test]
    fn skill_proposal_api_approval_requires_complete_external_verification() {
        let body = DecideBody {
            approve: true,
            decided_by: Some("web".to_string()),
            verification: Some(SkillProposalApprovalVerification {
                schema_reviewed: true,
                diff_reviewed: true,
                tests_reviewed: false,
                human_approved: true,
            }),
        };

        let err = skill_proposal_decided_by_for_api(&body).unwrap_err();
        assert!(err.contains("schema_reviewed"));
        assert!(err.contains("human_approved"));
    }

    #[test]
    fn skill_proposal_api_approval_marks_decider_with_verification() {
        let body = DecideBody {
            approve: true,
            decided_by: Some("web".to_string()),
            verification: Some(SkillProposalApprovalVerification {
                schema_reviewed: true,
                diff_reviewed: true,
                tests_reviewed: true,
                human_approved: true,
            }),
        };

        let decided_by = skill_proposal_decided_by_for_api(&body).unwrap();
        assert_eq!(decided_by, "web:schema_diff_tests_human");
    }
}

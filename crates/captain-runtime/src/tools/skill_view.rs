//! Exact skill view for skill-first runtime routing.

use std::path::{Path, PathBuf};

use captain_skills::registry::SkillRegistry;
use captain_skills::InstalledSkill;

use super::skill_linked_files::{linked_files, path_has_traversal};
use super::skill_view_validation::skill_validation;

const MAX_SKILL_FILE_BYTES: u64 = 1_048_576;

const SKILL_VIEW_NO_REGISTRY_HINT: &str =
    "SkillRegistry is unavailable in this execution context. Retry skill_view from the main Captain runtime.";

const SKILL_VIEW_NOT_FOUND_HINT: &str =
    "No installed skill has this exact name. Call skill_search with broader keywords before proposing a new skill.";

pub fn view_skill(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let request = SkillViewRequest::parse(input)?;

    let Some(registry) = skill_registry else {
        return Ok(skill_view_status_response(
            &request.name,
            "unavailable",
            SKILL_VIEW_NO_REGISTRY_HINT,
        ));
    };

    let Some(skill) = registry.get(&request.name) else {
        return Ok(skill_view_status_response(
            &request.name,
            "not_found",
            SKILL_VIEW_NOT_FOUND_HINT,
        ));
    };

    let skill_root = skill_root(skill.path.as_path());
    if let Some(file_path) = request.file_path.as_deref() {
        return view_linked_skill_file(&request.name, skill_root.as_deref(), file_path);
    }

    Ok(serde_json::Value::Object(build_skill_view_response(
        &request,
        skill,
        skill_root.as_deref(),
    ))
    .to_string())
}

struct SkillViewRequest {
    name: String,
    max_context_chars: usize,
    include_context: bool,
    file_path: Option<String>,
}

impl SkillViewRequest {
    fn parse(input: &serde_json::Value) -> Result<Self, String> {
        let name = input
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            return Err("missing required field: name".to_string());
        }

        Ok(Self {
            name,
            max_context_chars: input
                .get("max_context_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(8_000)
                .clamp(500, 20_000) as usize,
            include_context: input
                .get("include_context")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            file_path: input
                .get("file_path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        })
    }
}

fn skill_view_status_response(name: &str, status: &str, hint: &str) -> String {
    serde_json::json!({
        "name": name,
        "status": status,
        "hint": hint,
    })
    .to_string()
}

fn build_skill_view_response(
    request: &SkillViewRequest,
    skill: &InstalledSkill,
    skill_root: Option<&Path>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut response = serde_json::Map::new();
    response.insert("name".to_string(), serde_json::json!(request.name));
    insert_skill_metadata(&mut response, skill);
    insert_skill_runtime(&mut response, skill);
    insert_skill_tools(&mut response, skill);
    insert_skill_file_context(&mut response, skill, skill_root);
    insert_prompt_context(&mut response, skill, request);
    response
}

fn insert_skill_metadata(
    response: &mut serde_json::Map<String, serde_json::Value>,
    skill: &InstalledSkill,
) {
    let manifest = &skill.manifest;
    let family_id = captain_skills::families::infer_skill_family(skill);
    let family = captain_skills::families::known_family(family_id)
        .expect("infer_skill_family must return a known family");

    response.insert(
        "status".to_string(),
        serde_json::json!(if skill.enabled { "enabled" } else { "disabled" }),
    );
    response.insert(
        "description".to_string(),
        serde_json::json!(manifest.skill.description),
    );
    response.insert(
        "version".to_string(),
        serde_json::json!(manifest.skill.version),
    );
    response.insert(
        "family".to_string(),
        serde_json::json!({
            "id": family.id,
            "label": family.label,
        }),
    );
    response.insert(
        "source".to_string(),
        serde_json::json!(skill_source_label(skill)),
    );
    response.insert(
        "file_backed".to_string(),
        serde_json::json!(skill.path != Path::new("<bundled>")),
    );
    response.insert(
        "tags".to_string(),
        serde_json::json!(manifest.skill.tags.clone()),
    );
    response.insert(
        "governance".to_string(),
        serde_json::json!(captain_skills::skill_governance_from_tags(
            &manifest.skill.tags
        )),
    );
}

fn insert_skill_runtime(
    response: &mut serde_json::Map<String, serde_json::Value>,
    skill: &InstalledSkill,
) {
    let manifest = &skill.manifest;
    response.insert(
        "runtime".to_string(),
        serde_json::json!({
            "type": manifest.runtime.runtime_type,
            "entry": manifest.runtime.entry,
        }),
    );
}

fn insert_skill_tools(
    response: &mut serde_json::Map<String, serde_json::Value>,
    skill: &InstalledSkill,
) {
    let manifest = &skill.manifest;
    response.insert(
        "required_tools".to_string(),
        serde_json::json!(manifest.requirements.tools.clone()),
    );
    response.insert(
        "provided_tools".to_string(),
        serde_json::json!(manifest
            .tools
            .provided
            .iter()
            .map(|tool| serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema,
            }))
            .collect::<Vec<_>>()),
    );
    response.insert(
        "usage".to_string(),
        serde_json::json!(
            "Follow this skill's workflow before inventing a fresh procedure. If the skill is stale after real use, create a skill_refinement_propose item with evidence."
        ),
    );
}

fn insert_skill_file_context(
    response: &mut serde_json::Map<String, serde_json::Value>,
    skill: &InstalledSkill,
    skill_root: Option<&Path>,
) {
    let linked_files = skill_root.map(linked_files).unwrap_or_default();
    response.insert(
        "validation".to_string(),
        skill_validation(skill, skill_root, &linked_files),
    );

    if skill.path != Path::new("<bundled>") && !linked_files.is_empty() {
        response.insert("linked_files".to_string(), serde_json::json!(linked_files));
        response.insert(
            "usage_hint".to_string(),
            serde_json::json!(
                "To inspect supporting files, call skill_view with file_path such as 'references/api.md', 'templates/example.md', 'scripts/check.py', or 'assets/config.json'."
            ),
        );
    }
}

fn insert_prompt_context(
    response: &mut serde_json::Map<String, serde_json::Value>,
    skill: &InstalledSkill,
    request: &SkillViewRequest,
) {
    if request.include_context {
        if let Some(ctx) = skill
            .manifest
            .prompt_context
            .as_deref()
            .filter(|ctx| !ctx.is_empty())
        {
            let context = truncate_context(ctx, request.max_context_chars);
            response.insert("content".to_string(), serde_json::json!(context.clone()));
            response.insert("prompt_context".to_string(), serde_json::json!(context));
        }
    }
}

fn skill_source_label(skill: &captain_skills::InstalledSkill) -> &'static str {
    match skill.manifest.source.as_ref() {
        Some(captain_skills::SkillSource::Bundled) => "bundled",
        Some(captain_skills::SkillSource::OpenClaw) => "openclaw",
        Some(captain_skills::SkillSource::ClawHub { .. }) => "clawhub",
        Some(captain_skills::SkillSource::Native) | None => "native",
    }
}

fn skill_root(path: &Path) -> Option<PathBuf> {
    if path == Path::new("<bundled>") {
        return None;
    }
    if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        path.parent().map(Path::to_path_buf)
    }
}

fn view_linked_skill_file(
    skill_name: &str,
    skill_root: Option<&Path>,
    file_path: &str,
) -> Result<String, String> {
    let (resolved, metadata) = match lookup_linked_skill_file(skill_name, skill_root, file_path)? {
        LinkedSkillFile::Ready { resolved, metadata } => (resolved, metadata),
        LinkedSkillFile::Response(value) => return Ok(value.to_string()),
    };

    if metadata.len() > MAX_SKILL_FILE_BYTES {
        return Ok(serde_json::json!({
            "status": "too_large",
            "name": skill_name,
            "file": file_path,
            "size_bytes": metadata.len(),
            "error": "Skill supporting file is too large to load in one tool call.",
        })
        .to_string());
    }

    let bytes = std::fs::read(&resolved).map_err(|e| format!("failed to read skill file: {e}"))?;
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(err) => {
            return Ok(serde_json::json!({
                "status": "binary",
                "name": skill_name,
                "file": file_path,
                "content": format!("[Binary file: {}, size: {} bytes]", resolved.file_name().and_then(|name| name.to_str()).unwrap_or("file"), err.as_bytes().len()),
                "is_binary": true,
            })
            .to_string());
        }
    };

    Ok(serde_json::json!({
        "status": "ok",
        "name": skill_name,
        "file": file_path,
        "content": content,
        "file_type": resolved.extension().and_then(|ext| ext.to_str()).unwrap_or(""),
    })
    .to_string())
}

enum LinkedSkillFile {
    Ready {
        resolved: PathBuf,
        metadata: std::fs::Metadata,
    },
    Response(serde_json::Value),
}

fn lookup_linked_skill_file(
    skill_name: &str,
    skill_root: Option<&Path>,
    file_path: &str,
) -> Result<LinkedSkillFile, String> {
    let Some(skill_root) = skill_root else {
        return Ok(LinkedSkillFile::Response(serde_json::json!({
            "status": "not_available",
            "name": skill_name,
            "error": "This skill has no file-backed directory to inspect.",
        })));
    };
    if path_has_traversal(file_path) {
        return Ok(LinkedSkillFile::Response(blocked_linked_file_response(
            skill_name,
            file_path,
            "Path traversal is not allowed.",
        )));
    }

    let target = skill_root.join(file_path);
    let root = skill_root
        .canonicalize()
        .map_err(|e| format!("failed to resolve skill directory: {e}"))?;
    if !target.exists() {
        return Ok(LinkedSkillFile::Response(serde_json::json!({
            "status": "not_found",
            "name": skill_name,
            "file": file_path,
            "error": format!("File '{file_path}' not found in skill '{skill_name}'."),
            "available_files": linked_files(skill_root),
            "hint": "Use one of the available file_path values.",
        })));
    }

    let resolved = target
        .canonicalize()
        .map_err(|e| format!("failed to resolve skill file: {e}"))?;
    if !resolved.starts_with(&root) {
        return Ok(LinkedSkillFile::Response(blocked_linked_file_response(
            skill_name,
            file_path,
            "Requested file escapes the skill directory.",
        )));
    }
    if !resolved.is_file() {
        return Ok(LinkedSkillFile::Response(serde_json::json!({
            "status": "not_file",
            "name": skill_name,
            "file": file_path,
            "error": "Requested path is not a file.",
        })));
    }

    let metadata = std::fs::metadata(&resolved).map_err(|e| format!("metadata failed: {e}"))?;
    Ok(LinkedSkillFile::Ready { resolved, metadata })
}

fn blocked_linked_file_response(
    skill_name: &str,
    file_path: &str,
    error: &str,
) -> serde_json::Value {
    serde_json::json!({
        "status": "blocked",
        "name": skill_name,
        "file": file_path,
        "error": error,
        "hint": "Use a relative path inside the skill directory.",
    })
}

fn truncate_context(ctx: &str, max_chars: usize) -> String {
    if ctx.len() <= max_chars {
        return ctx.to_string();
    }
    let mut end = max_chars.min(ctx.len());
    while end > 0 && !ctx.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = ctx[..end].trim_end().to_string();
    out.push_str("\n...[truncated]");
    out
}

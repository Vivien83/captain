//! Procedural skill discovery.

use captain_skills::{registry::SkillRegistry, InstalledSkill};
use std::collections::BTreeMap;

use super::{lexical_weighted_score, query_tokens, result_name, result_score, snippet_for_tokens};

const SKILL_SEARCH_NO_REGISTRY_HINT: &str =
    "SkillRegistry is unavailable in this execution context. Continue with captain_docs or \
     capability_search, then retry skill_search from the main Captain runtime.";

const SKILL_SEARCH_NO_MATCH_HINT: &str =
    "No installed skill matched. Try a broader family, call captain_docs for canonical tool \
     behaviour, or propose a new skill only if the workflow is genuinely reusable.";

fn skill_search_families(registry: Option<&SkillRegistry>) -> Vec<serde_json::Value> {
    let mut counts: BTreeMap<&'static str, usize> = captain_skills::families::SKILL_FAMILIES
        .iter()
        .map(|family| (family.id, 0usize))
        .collect();
    if let Some(registry) = registry {
        for skill in registry.list().into_iter().filter(|skill| skill.enabled) {
            let family = captain_skills::families::infer_skill_family(skill);
            *counts.entry(family).or_insert(0) += 1;
        }
    }
    captain_skills::families::SKILL_FAMILIES
        .iter()
        .map(|family| {
            serde_json::json!({
                "id": family.id,
                "label": family.label,
                "description": family.description,
                "count": counts.get(family.id).copied().unwrap_or(0),
            })
        })
        .collect()
}

fn skill_source_label(skill: &captain_skills::InstalledSkill) -> &'static str {
    match skill.manifest.source.as_ref() {
        Some(captain_skills::SkillSource::Bundled) => "bundled",
        Some(captain_skills::SkillSource::OpenClaw) => "openclaw",
        Some(captain_skills::SkillSource::ClawHub { .. }) => "clawhub",
        Some(captain_skills::SkillSource::Native) | None => "native",
    }
}

fn skill_is_file_backed(skill: &captain_skills::InstalledSkill) -> bool {
    skill.path != std::path::Path::new("<bundled>")
}

fn skill_context_excerpt(ctx: &str, tokens: &[String], max: usize) -> String {
    if tokens.is_empty() {
        let mut end = max.min(ctx.len());
        while end < ctx.len() && !ctx.is_char_boundary(end) {
            end += 1;
        }
        let mut out = ctx[..end].to_string();
        if end < ctx.len() {
            out.push_str("...");
        }
        out
    } else {
        snippet_for_tokens(ctx, tokens, max)
    }
}

struct SkillSearchRequest<'a> {
    query: &'a str,
    family_filter: Option<&'a str>,
    max_results: usize,
    include_context: bool,
    include_families: bool,
    exact_names: Option<Vec<String>>,
    tokens: Vec<String>,
}

pub fn search_skills(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let request = parse_skill_search_request(input)?;
    let response = skill_search_base_response(&request, skill_registry);
    let Some(registry) = skill_registry else {
        return Ok(skill_search_unavailable_response(response));
    };
    let results = skill_search_results(registry, &request);
    Ok(skill_search_finish_response(response, results, &request))
}

fn parse_skill_search_request(input: &serde_json::Value) -> Result<SkillSearchRequest<'_>, String> {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let family_filter = input
        .get("family")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    validate_skill_family_filter(family_filter)?;
    let max_results = input
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(8)
        .clamp(1, 30) as usize;
    let include_context = input
        .get("include_context")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let include_families = input
        .get("include_families")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let exact_names = query
        .strip_prefix("select:")
        .map(|rest| {
            rest.split(',')
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|names| !names.is_empty());
    let tokens = query_tokens(query);

    Ok(SkillSearchRequest {
        query,
        family_filter,
        max_results,
        include_context,
        include_families,
        exact_names,
        tokens,
    })
}

fn validate_skill_family_filter(family_filter: Option<&str>) -> Result<(), String> {
    if let Some(family) = family_filter {
        if captain_skills::families::known_family(family).is_none() {
            return Err(format!(
                "unknown skill family '{family}'. Known families: {}",
                known_skill_family_ids()
            ));
        }
    }
    Ok(())
}

fn known_skill_family_ids() -> String {
    captain_skills::families::SKILL_FAMILIES
        .iter()
        .map(|family| family.id)
        .collect::<Vec<_>>()
        .join(", ")
}

fn skill_search_base_response(
    request: &SkillSearchRequest<'_>,
    skill_registry: Option<&SkillRegistry>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut response = serde_json::Map::new();
    response.insert("query".to_string(), serde_json::json!(request.query));
    response.insert(
        "family".to_string(),
        serde_json::json!(request.family_filter),
    );
    if request.include_families {
        response.insert(
            "families".to_string(),
            serde_json::Value::Array(skill_search_families(skill_registry)),
        );
    }
    response
}

fn skill_search_unavailable_response(
    mut response: serde_json::Map<String, serde_json::Value>,
) -> String {
    response.insert("results".to_string(), serde_json::json!([]));
    response.insert("total".to_string(), serde_json::json!(0));
    response.insert(
        "hint".to_string(),
        serde_json::json!(SKILL_SEARCH_NO_REGISTRY_HINT),
    );
    serde_json::Value::Object(response).to_string()
}

fn skill_search_results(
    registry: &SkillRegistry,
    request: &SkillSearchRequest<'_>,
) -> Vec<serde_json::Value> {
    registry
        .list()
        .into_iter()
        .filter(|skill| skill.enabled)
        .filter_map(|skill| skill_search_result(skill, request))
        .collect()
}

fn skill_search_result(
    skill: &InstalledSkill,
    request: &SkillSearchRequest<'_>,
) -> Option<serde_json::Value> {
    let family_id = captain_skills::families::infer_skill_family(skill);
    if request
        .family_filter
        .is_some_and(|wanted| wanted != family_id)
    {
        return None;
    }
    let score = skill_search_score(skill, request);
    if score == 0 {
        return None;
    }
    Some(serde_json::Value::Object(skill_search_result_item(
        skill, family_id, score, request,
    )))
}

fn skill_search_score(skill: &InstalledSkill, request: &SkillSearchRequest<'_>) -> u32 {
    let name = skill.manifest.skill.name.as_str();
    if skill_exact_name_match(name, request) {
        return 1_000;
    }
    if request.query.is_empty() {
        return 25;
    }
    let tags = skill.manifest.skill.tags.join(" ");
    let required_tools = skill.manifest.requirements.tools.join(" ");
    let provided_tools = skill_search_provided_tools(skill).join(" ");
    let prompt_context = skill.manifest.prompt_context.as_deref().unwrap_or("");
    lexical_weighted_score(
        &request.tokens,
        &[
            (name, 4),
            (skill.manifest.skill.description.as_str(), 3),
            (tags.as_str(), 2),
            (required_tools.as_str(), 2),
            (provided_tools.as_str(), 2),
            (prompt_context, 1),
        ],
    )
}

fn skill_exact_name_match(name: &str, request: &SkillSearchRequest<'_>) -> bool {
    request
        .exact_names
        .as_ref()
        .is_some_and(|names| names.iter().any(|candidate| candidate == name))
}

fn skill_search_provided_tools(skill: &InstalledSkill) -> Vec<String> {
    skill
        .manifest
        .tools
        .provided
        .iter()
        .map(|tool| tool.name.clone())
        .collect()
}

fn skill_search_result_item(
    skill: &InstalledSkill,
    family_id: &str,
    score: u32,
    request: &SkillSearchRequest<'_>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut item = serde_json::Map::new();
    insert_skill_search_identity(&mut item, skill, family_id, score);
    insert_skill_search_lists(&mut item, skill);
    item.insert(
        "usage".to_string(),
        serde_json::json!(
            "Load this skill when the task matches the trigger. Follow the SKILL.md workflow; if it requires a hidden builtin, call capability_search/tool_search for the exact tool schema."
        ),
    );
    insert_skill_search_context(&mut item, skill, request);
    item
}

fn insert_skill_search_identity(
    item: &mut serde_json::Map<String, serde_json::Value>,
    skill: &InstalledSkill,
    family_id: &str,
    score: u32,
) {
    let family = captain_skills::families::known_family(family_id)
        .expect("infer_skill_family must return a known family");
    item.insert(
        "name".to_string(),
        serde_json::json!(skill.manifest.skill.name.as_str()),
    );
    item.insert(
        "description".to_string(),
        serde_json::json!(skill.manifest.skill.description.as_str()),
    );
    item.insert(
        "family".to_string(),
        serde_json::json!({"id": family.id, "label": family.label}),
    );
    item.insert("status".to_string(), serde_json::json!("enabled"));
    item.insert(
        "source".to_string(),
        serde_json::json!(skill_source_label(skill)),
    );
    item.insert(
        "file_backed".to_string(),
        serde_json::json!(skill_is_file_backed(skill)),
    );
    item.insert("score".to_string(), serde_json::json!(score));
}

fn insert_skill_search_lists(
    item: &mut serde_json::Map<String, serde_json::Value>,
    skill: &InstalledSkill,
) {
    item.insert(
        "tags".to_string(),
        serde_json::json!(skill.manifest.skill.tags.clone()),
    );
    item.insert(
        "governance".to_string(),
        serde_json::json!(captain_skills::skill_governance_from_tags(
            &skill.manifest.skill.tags
        )),
    );
    item.insert(
        "required_tools".to_string(),
        serde_json::json!(skill.manifest.requirements.tools.clone()),
    );
    item.insert(
        "provided_tools".to_string(),
        serde_json::json!(skill_search_provided_tools(skill)),
    );
}

fn insert_skill_search_context(
    item: &mut serde_json::Map<String, serde_json::Value>,
    skill: &InstalledSkill,
    request: &SkillSearchRequest<'_>,
) {
    let prompt_context = skill.manifest.prompt_context.as_deref().unwrap_or("");
    if request.include_context && !prompt_context.is_empty() {
        item.insert(
            "context_excerpt".to_string(),
            serde_json::json!(skill_context_excerpt(
                prompt_context,
                &request.tokens,
                1_200
            )),
        );
    }
}

fn skill_search_finish_response(
    mut response: serde_json::Map<String, serde_json::Value>,
    mut results: Vec<serde_json::Value>,
    request: &SkillSearchRequest<'_>,
) -> String {
    results.sort_by(|a, b| {
        result_score(b)
            .cmp(&result_score(a))
            .then_with(|| result_name(a).cmp(result_name(b)))
    });
    results.truncate(request.max_results);
    let total = results.len();
    response.insert("results".to_string(), serde_json::Value::Array(results));
    response.insert("total".to_string(), serde_json::json!(total));
    if total == 0 {
        response.insert(
            "hint".to_string(),
            serde_json::json!(SKILL_SEARCH_NO_MATCH_HINT),
        );
    } else if request.query.is_empty() && request.family_filter.is_none() {
        response.insert(
            "hint".to_string(),
            serde_json::json!(
                "Minimal skill index returned. Use skill_view with an exact name before applying a matching workflow."
            ),
        );
    }
    serde_json::Value::Object(response).to_string()
}

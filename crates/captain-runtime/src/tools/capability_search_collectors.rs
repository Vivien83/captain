use std::collections::HashSet;
use std::sync::Arc;

use captain_skills::{registry::SkillRegistry, SkillToolDef};
use captain_types::tool::ToolDefinition;

use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::tools::{
    lexical_tool_score, lexical_weighted_score, result_name, result_score, result_source,
    snippet_for_tokens, ToolRegistry,
};

use super::{source_enabled, CAPABILITY_SEARCH_NO_MATCH_HINT};

#[allow(clippy::too_many_arguments)]
fn capability_candidate(
    source: &str,
    name: String,
    description: String,
    score: u32,
    status: &str,
    usage: &str,
    input_schema: Option<serde_json::Value>,
    metadata: serde_json::Value,
    include_schemas: bool,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("source".to_string(), serde_json::json!(source));
    obj.insert("name".to_string(), serde_json::json!(name));
    obj.insert("description".to_string(), serde_json::json!(description));
    obj.insert("status".to_string(), serde_json::json!(status));
    obj.insert("usage".to_string(), serde_json::json!(usage));
    obj.insert("score".to_string(), serde_json::json!(score));
    if include_schemas {
        if let Some(schema) = input_schema {
            obj.insert("input_schema".to_string(), schema);
        }
    }
    if !metadata.is_null() {
        obj.insert("metadata".to_string(), metadata);
    }
    serde_json::Value::Object(obj)
}

fn exact_name_matches(exact_names: Option<&[String]>, candidate: &str) -> bool {
    exact_names.is_some_and(|names| names.iter().any(|name| name == candidate))
}

pub(super) fn collect_builtin_capabilities(
    results: &mut Vec<serde_json::Value>,
    exact_names: Option<&[String]>,
    tokens: &[String],
    include_schemas: bool,
    builtin_definitions: Vec<ToolDefinition>,
    is_core_tool: &impl Fn(&str) -> bool,
) {
    let registry = ToolRegistry::new(builtin_definitions);
    for tool in registry.discoverable_definitions() {
        let is_exact = exact_name_matches(exact_names, &tool.name);
        let score = if is_exact {
            1_000
        } else {
            lexical_tool_score(tokens, tool)
        };
        if score == 0 {
            continue;
        }
        let core = is_core_tool(&tool.name);
        let status = if core {
            "core_visible"
        } else {
            "deferred_builtin"
        };
        let usage = if core {
            "Call this builtin tool directly; it is part of the CORE visible tool set."
        } else {
            "Builtin tool outside CORE. This discovery result asks the runtime to surface it on the next turn; if you still need an exact schema, call tool_search with select:<name>."
        };
        results.push(capability_candidate(
            "builtin",
            tool.name.clone(),
            tool.description.clone(),
            score,
            status,
            usage,
            Some(tool.input_schema.clone()),
            serde_json::json!({
                "core": core,
                "surface": crate::surface_gates::tool_surface(&tool.name).unwrap_or("core"),
            }),
            include_schemas,
        ));
    }
}

fn collect_skill_tool_capabilities(
    results: &mut Vec<serde_json::Value>,
    exact_names: Option<&[String]>,
    tokens: &[String],
    include_schemas: bool,
    skill_name: &str,
    tools: &[SkillToolDef],
) {
    for tool in tools {
        let is_exact = exact_name_matches(exact_names, &tool.name);
        let score = if is_exact {
            950
        } else {
            lexical_weighted_score(
                tokens,
                &[
                    (tool.name.as_str(), 3),
                    (tool.description.as_str(), 1),
                    (skill_name, 1),
                ],
            )
        };
        if score == 0 {
            continue;
        }
        results.push(capability_candidate(
            "skill_tool",
            tool.name.clone(),
            tool.description.clone(),
            score,
            "installed",
            "Call this skill-provided tool directly. If the skill is prompt-only instead, follow its injected instructions or use skill_execute for .md capabilities.",
            Some(tool.input_schema.clone()),
            serde_json::json!({ "skill": skill_name }),
            include_schemas,
        ));
    }
}

pub(super) fn collect_skill_capabilities(
    results: &mut Vec<serde_json::Value>,
    source_notes: &mut Vec<serde_json::Value>,
    source_filter: &Option<HashSet<String>>,
    exact_names: Option<&[String]>,
    tokens: &[String],
    include_schemas: bool,
    skill_registry: Option<&SkillRegistry>,
) {
    match skill_registry {
        Some(registry) => {
            for skill in registry.list().into_iter().filter(|skill| skill.enabled) {
                let skill_name = &skill.manifest.skill.name;
                let skill_desc = &skill.manifest.skill.description;
                let is_exact = exact_name_matches(exact_names, skill_name);
                let score = if is_exact {
                    900
                } else {
                    lexical_weighted_score(
                        tokens,
                        &[(skill_name.as_str(), 3), (skill_desc.as_str(), 1)],
                    )
                };
                if score > 0 && source_enabled(source_filter, "skill") {
                    let provided: Vec<String> = skill
                        .manifest
                        .tools
                        .provided
                        .iter()
                        .map(|tool| tool.name.clone())
                        .collect();
                    let status = if provided.is_empty() {
                        "prompt_only"
                    } else {
                        "installed"
                    };
                    results.push(capability_candidate(
                        "skill",
                        skill_name.clone(),
                        skill_desc.clone(),
                        score,
                        status,
                        "Use the injected skill instructions when they match; call one of its provided tools when a concrete tool is listed.",
                        None,
                        serde_json::json!({ "provided_tools": provided }),
                        include_schemas,
                    ));
                }

                if source_enabled(source_filter, "skill_tool") {
                    collect_skill_tool_capabilities(
                        results,
                        exact_names,
                        tokens,
                        include_schemas,
                        skill_name,
                        &skill.manifest.tools.provided,
                    );
                }
            }
        }
        None => source_notes.push(serde_json::json!({
            "source": "skill",
            "status": "unavailable",
            "hint": "No SkillRegistry was provided in this execution context."
        })),
    }
}

pub(super) async fn collect_mcp_capabilities(
    results: &mut Vec<serde_json::Value>,
    source_notes: &mut Vec<serde_json::Value>,
    exact_names: Option<&[String]>,
    tokens: &[String],
    include_schemas: bool,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) {
    match mcp_connections {
        Some(mcp_connections) => {
            let conns = mcp_connections.lock().await;
            if conns.is_empty() {
                source_notes.push(serde_json::json!({
                    "source": "mcp",
                    "status": "none_connected",
                    "hint": "No MCP server is connected. Inspect config/integrations or use mcp_setup when available."
                }));
            }
            for conn in conns.iter() {
                let server = conn.name().to_string();
                for tool in conn.tools() {
                    let is_exact = exact_name_matches(exact_names, &tool.name);
                    let score = if is_exact {
                        950
                    } else {
                        lexical_weighted_score(
                            tokens,
                            &[
                                (tool.name.as_str(), 3),
                                (tool.description.as_str(), 1),
                                (server.as_str(), 1),
                            ],
                        )
                    };
                    if score == 0 {
                        continue;
                    }
                    results.push(capability_candidate(
                        "mcp_tool",
                        tool.name.clone(),
                        tool.description.clone(),
                        score,
                        "connected",
                        "Call this namespaced MCP tool directly. MCP tools are dynamic; if the server disappears, inspect Connected Tool Servers or integration config.",
                        Some(tool.input_schema.clone()),
                        serde_json::json!({ "server": server }),
                        include_schemas,
                    ));
                }
            }
        }
        None => source_notes.push(serde_json::json!({
            "source": "mcp",
            "status": "unavailable",
            "hint": "No MCP connection registry was provided in this execution context."
        })),
    }
}

pub(super) async fn collect_hand_capabilities(
    results: &mut Vec<serde_json::Value>,
    source_notes: &mut Vec<serde_json::Value>,
    exact_names: Option<&[String]>,
    tokens: &[String],
    include_schemas: bool,
    kernel: Option<&Arc<dyn KernelHandle>>,
) {
    match kernel {
        Some(kernel) => match kernel.hand_list().await {
            Ok(hands) => {
                for hand in hands {
                    let id = hand["id"].as_str().unwrap_or("").to_string();
                    let name = hand["name"].as_str().unwrap_or(&id).to_string();
                    let desc = hand["description"].as_str().unwrap_or("").to_string();
                    let tools_text = hand["tools"].to_string();
                    let is_exact = exact_names
                        .is_some_and(|names| names.iter().any(|n| n == &id || n == &name));
                    let score = if is_exact {
                        850
                    } else {
                        lexical_weighted_score(
                            tokens,
                            &[
                                (id.as_str(), 3),
                                (name.as_str(), 3),
                                (desc.as_str(), 1),
                                (tools_text.as_str(), 1),
                            ],
                        )
                    };
                    if score == 0 {
                        continue;
                    }
                    let status = hand["status"].as_str().unwrap_or("unknown").to_string();
                    let usage = if status.eq_ignore_ascii_case("available") {
                        "Activate this Hand with hand_activate when a specialized autonomous package is better than manual tool chaining."
                    } else {
                        "This Hand is already active or has state; inspect hand_status or delegate to the associated agent when appropriate."
                    };
                    results.push(capability_candidate(
                        "hand",
                        id,
                        desc,
                        score,
                        &status,
                        usage,
                        None,
                        serde_json::json!({ "name": name, "tools": hand["tools"].clone() }),
                        include_schemas,
                    ));
                }
            }
            Err(error) => source_notes.push(serde_json::json!({
                "source": "hand",
                "status": "error",
                "hint": error,
            })),
        },
        None => source_notes.push(serde_json::json!({
            "source": "hand",
            "status": "unavailable",
            "hint": "No kernel handle was provided, so Hands cannot be listed in this context."
        })),
    }
}

pub(super) fn collect_docs_capabilities(
    results: &mut Vec<serde_json::Value>,
    exact_names: Option<&[String]>,
    tokens: &[String],
    include_schemas: bool,
) {
    for (slug, body) in crate::captain_docs::FAMILY_DOCS {
        let is_exact = exact_name_matches(exact_names, slug);
        let score = if is_exact {
            800
        } else {
            lexical_weighted_score(tokens, &[(slug, 3), (body, 1)])
        };
        if score == 0 {
            continue;
        }
        results.push(capability_candidate(
            "docs_family",
            (*slug).to_string(),
            format!("Captain tool documentation family: {slug}"),
            score,
            "available",
            "Call captain_docs with this family to read the canonical contract, including generated Live Tool Schemas from the running registry.",
            Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "family": { "type": "string", "const": slug }
                },
                "required": ["query", "family"]
            })),
            serde_json::json!({ "snippet": snippet_for_tokens(body, tokens, 420) }),
            include_schemas,
        ));
    }
}

pub(super) fn finish_capability_response(
    query: &str,
    max_results: usize,
    searched_sources: Vec<&'static str>,
    mut results: Vec<serde_json::Value>,
    source_notes: Vec<serde_json::Value>,
) -> String {
    results.sort_by(|a, b| {
        result_score(b)
            .cmp(&result_score(a))
            .then_with(|| result_source(a).cmp(result_source(b)))
            .then_with(|| result_name(a).cmp(result_name(b)))
    });
    results.truncate(max_results);

    let has_results = !results.is_empty();
    let total = results.len();
    let mut response = serde_json::Map::new();
    response.insert("query".to_string(), serde_json::json!(query));
    response.insert("results".to_string(), serde_json::Value::Array(results));
    response.insert("total".to_string(), serde_json::json!(total));
    response.insert(
        "searched_sources".to_string(),
        serde_json::json!(searched_sources),
    );
    if !source_notes.is_empty() {
        response.insert(
            "source_notes".to_string(),
            serde_json::Value::Array(source_notes),
        );
    }
    if !has_results {
        response.insert(
            "hint".to_string(),
            serde_json::json!(CAPABILITY_SEARCH_NO_MATCH_HINT),
        );
    }
    serde_json::Value::Object(response).to_string()
}

//! Unified capability discovery across active builtins, skills, MCP, and docs.

use std::collections::HashSet;
use std::sync::Arc;

use captain_skills::registry::SkillRegistry;
use captain_types::tool::ToolDefinition;

use crate::kernel_handle::KernelHandle;
use crate::mcp;

use super::query_tokens;

#[path = "capability_search_collectors.rs"]
mod collectors;

use self::collectors::{
    collect_builtin_capabilities, collect_docs_capabilities, collect_hand_capabilities,
    collect_mcp_capabilities, collect_skill_capabilities, finish_capability_response,
};

const CAPABILITY_SEARCH_EMPTY_QUERY_HINT: &str =
    "Describe the capability you need, or use select:name1,name2 when exact active tool, skill, \
     MCP, or docs family names are already known.";

const CAPABILITY_SEARCH_NO_MATCH_HINT: &str =
    "No capability matched. Refine the query with concrete nouns/verbs, then check captain_docs \
     for builtin behaviour. If the need is an external integration, configure or inspect MCP; if \
     it is a repeatable workflow, consider scaffold_skill.";

fn source_enabled(filter: &Option<HashSet<String>>, source: &str) -> bool {
    let Some(filter) = filter else {
        return crate::surface_gates::source_is_discoverable_by_default(source);
    };
    filter.contains("all")
        || filter.contains(source)
        || (source == "skill_tool" && filter.contains("skill"))
        || (source == "mcp_tool" && filter.contains("mcp"))
        || (source == "docs_family" && filter.contains("docs"))
}

fn capability_searched_sources(filter: &Option<HashSet<String>>) -> Vec<&'static str> {
    ["builtin", "skill", "mcp", "hand", "docs"]
        .into_iter()
        .filter(|source| source_enabled(filter, source))
        .collect()
}

fn parse_capability_sources(input: &serde_json::Value) -> Option<HashSet<String>> {
    let sources = input.get("sources")?.as_array()?;
    let set: HashSet<String> = sources
        .iter()
        .filter_map(|value| value.as_str())
        .map(|source| match source.trim().to_lowercase().as_str() {
            "builtins" | "tools" => "builtin".to_string(),
            "skills" => "skill".to_string(),
            "mcp_tool" | "mcp_tools" => "mcp".to_string(),
            "hands" => "hand".to_string(),
            "doc" | "docs_family" | "families" => "docs".to_string(),
            other => other.to_string(),
        })
        .filter(|source| !source.is_empty())
        .collect();
    if set.is_empty() {
        None
    } else {
        Some(set)
    }
}

fn parse_exact_names(query: &str) -> Option<Vec<String>> {
    query.strip_prefix("select:").map(|rest| {
        rest.split(',')
            .map(|name| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect()
    })
}

pub async fn search_capabilities(
    input: &serde_json::Value,
    skill_registry: Option<&SkillRegistry>,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    builtin_definitions: Vec<ToolDefinition>,
    is_core_tool: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let query = input
        .get("query")
        .and_then(|value| value.as_str())
        .ok_or("missing 'query' parameter")?
        .trim();
    let max_results = input
        .get("max_results")
        .and_then(|value| value.as_u64())
        .unwrap_or(8)
        .clamp(1, 30) as usize;
    let include_schemas = input
        .get("include_schemas")
        .and_then(|value| value.as_bool())
        .unwrap_or(true);
    let source_filter = parse_capability_sources(input);
    let searched_sources = capability_searched_sources(&source_filter);

    if query.is_empty() {
        return Ok(serde_json::json!({
            "query": query,
            "results": [],
            "searched_sources": searched_sources,
            "hint": CAPABILITY_SEARCH_EMPTY_QUERY_HINT,
        })
        .to_string());
    }

    let exact_names = parse_exact_names(query);
    let exact_names = exact_names.as_deref();
    let tokens = query_tokens(query);
    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut source_notes: Vec<serde_json::Value> = Vec::new();

    if source_enabled(&source_filter, "builtin") {
        collect_builtin_capabilities(
            &mut results,
            exact_names,
            &tokens,
            include_schemas,
            builtin_definitions,
            &is_core_tool,
        );
    }

    if source_enabled(&source_filter, "skill") || source_enabled(&source_filter, "skill_tool") {
        collect_skill_capabilities(
            &mut results,
            &mut source_notes,
            &source_filter,
            exact_names,
            &tokens,
            include_schemas,
            skill_registry,
        );
    }

    if source_enabled(&source_filter, "mcp") || source_enabled(&source_filter, "mcp_tool") {
        collect_mcp_capabilities(
            &mut results,
            &mut source_notes,
            exact_names,
            &tokens,
            include_schemas,
            mcp_connections,
        )
        .await;
    }

    if source_enabled(&source_filter, "hand") {
        collect_hand_capabilities(
            &mut results,
            &mut source_notes,
            exact_names,
            &tokens,
            include_schemas,
            kernel,
        )
        .await;
    }

    if source_enabled(&source_filter, "docs") || source_enabled(&source_filter, "docs_family") {
        collect_docs_capabilities(&mut results, exact_names, &tokens, include_schemas);
    }

    Ok(finish_capability_response(
        query,
        max_results,
        searched_sources,
        results,
        source_notes,
    ))
}

use crate::llm_driver::CompletionResponse;
use captain_types::message::{ContentBlock, Message, StopReason};
use captain_types::tool::ToolDefinition;
use tracing::info;

/// Detect when the LLM claims to have performed an action without using tools.
pub(crate) fn phantom_action_detected(text: &str) -> bool {
    let lower = text.to_lowercase();
    let action_verbs = ["sent ", "posted ", "emailed ", "delivered ", "forwarded "];
    let channel_refs = [
        "telegram",
        "whatsapp",
        "slack",
        "discord",
        "email",
        "channel",
        "message sent",
        "successfully sent",
        "has been sent",
    ];
    let has_action = action_verbs.iter().any(|v| lower.contains(v));
    let has_channel = channel_refs.iter().any(|c| lower.contains(c));
    has_action && has_channel
}

pub(crate) fn capability_denial_should_retry(text: &str, visible_tools: &[ToolDefinition]) -> bool {
    if !visible_tools.iter().any(|t| t.name == "capability_search") {
        return false;
    }
    let lower = text.to_lowercase();
    [
        "je n'ai pas accès",
        "je n ai pas acces",
        "je n'ai pas l'outil",
        "je n ai pas l outil",
        "je ne peux pas accéder",
        "je ne peux pas acceder",
        "pas accès à",
        "pas acces a",
        "i don't have access",
        "i do not have access",
        "i don't have the tool",
        "i do not have the tool",
        "no access to",
        "can't access",
        "cannot access",
        "not available in my tools",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

pub(crate) fn capability_search_nudge() -> Message {
    Message::user(
        "[System: Before concluding that a capability/tool is unavailable, call \
         capability_search with concrete keywords. Captain keeps most domain \
         tools deferred to save tokens; absence from the visible CORE is not \
         absence of capability. If capability_search finds nothing, then explain \
         the limitation briefly.]"
            .to_string(),
    )
}

/// Unified error guidance injected after failed tool calls.
pub(crate) fn append_tool_error_guidance(tool_result_blocks: &mut Vec<ContentBlock>) {
    let has_error = tool_result_blocks
        .iter()
        .any(|block| matches!(block, ContentBlock::ToolResult { is_error: true, .. }));
    if !has_error {
        return;
    }

    tool_result_blocks.push(ContentBlock::Text {
        text: "[System: Tool call(s) failed. Do NOT invent results or take actions \
               based on failed outputs. For internal tools (memory_store, memory_recall, \
               knowledge_query, config_read, secret_read), continue naturally without \
               mentioning the error to the user. For user-facing tools (web_search, \
               shell_exec, file_read, web_fetch), briefly tell the user it failed.]"
            .to_string(),
        provider_metadata: None,
    });
}

/// Expand the visible tool list after `capability_search` / `tool_search`.
pub(crate) fn expand_visible_tools_from_discovery(
    visible_tools: &mut Vec<ToolDefinition>,
    available_tool_catalog: &[ToolDefinition],
    discovery_tool: &str,
    discovery_result: &str,
) -> usize {
    if !matches!(discovery_tool, "capability_search" | "tool_search") {
        return 0;
    }

    let Ok(value) = serde_json::from_str::<serde_json::Value>(discovery_result) else {
        return 0;
    };
    let Some(results) = value.get("results").and_then(|v| v.as_array()) else {
        return 0;
    };

    let mut wanted = Vec::new();
    for item in results {
        let Some(name) = item.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        let source = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let status = item.get("status").and_then(|v| v.as_str()).unwrap_or("");
        let is_deferred_builtin = discovery_tool == "tool_search"
            || (source == "builtin"
                && (status.contains("deferred")
                    || item
                        .pointer("/metadata/core")
                        .and_then(|v| v.as_bool())
                        .is_some_and(|core| !core)));
        let is_dynamic_tool = matches!(source, "skill_tool" | "mcp_tool");
        if (is_deferred_builtin || is_dynamic_tool) && !wanted.contains(&name) {
            wanted.push(name);
        }
    }

    if wanted.is_empty() {
        return 0;
    }

    let all = crate::tool_runner::builtin_tool_definitions();
    let available_registry = crate::tools::ToolRegistry::new(available_tool_catalog.to_vec());
    let builtin_registry = crate::tools::ToolRegistry::new(all);
    let mut added = 0;
    for name in wanted {
        if visible_tools.iter().any(|t| t.name == name) {
            continue;
        }
        if let Some(def) = available_registry
            .find_discoverable(name)
            .or_else(|| builtin_registry.find_discoverable(name))
        {
            visible_tools.push(def.clone());
            added += 1;
        }
    }
    added
}

pub(crate) fn expand_visible_tools_after_discovery_result(
    visible_tools: &mut Vec<ToolDefinition>,
    available_tool_catalog: &[ToolDefinition],
    discovery_tool: &str,
    discovery_result: &str,
    streaming: bool,
) -> usize {
    let added_tools = expand_visible_tools_from_discovery(
        visible_tools,
        available_tool_catalog,
        discovery_tool,
        discovery_result,
    );
    if added_tools == 0 {
        return 0;
    }

    if streaming {
        info!(
            tool = %discovery_tool,
            added_tools,
            visible_tools = visible_tools.len(),
            "Deferred builtin tools surfaced for next LLM turn (streaming)"
        );
    } else {
        info!(
            tool = %discovery_tool,
            added_tools,
            visible_tools = visible_tools.len(),
            "Deferred builtin tools surfaced for next LLM turn"
        );
    }

    added_tools
}

pub(crate) fn codex_missing_tool_call_should_retry(
    provider: &str,
    response: &CompletionResponse,
    available_tools: &[ToolDefinition],
) -> bool {
    if provider != "codex" && provider != "openai-codex" {
        return false;
    }
    if available_tools.is_empty()
        || !response.tool_calls.is_empty()
        || !matches!(
            response.stop_reason,
            StopReason::EndTurn | StopReason::StopSequence
        )
    {
        return false;
    }

    let text = response.text();
    let lower = text.to_lowercase();
    [
        "i will use",
        "i'll use",
        "i will call",
        "i'll call",
        "i will run",
        "i'll run",
        "let me run",
        "let me call",
        "i am going to use",
        "i'm going to use",
        "i'll inspect",
        "i will inspect",
        "i'll check",
        "i will check",
        "i'll look",
        "i will look",
        "let me inspect",
        "let me check",
        "let me look",
        "je vais utiliser",
        "je vais appeler",
        "je vais lancer",
        "je vais executer",
        "je vais exécuter",
        "je vais inspecter",
        "je vais verifier",
        "je vais vérifier",
        "je vais regarder",
        "laisse moi verifier",
        "laisse-moi verifier",
        "laisse moi vérifier",
        "laisse-moi vérifier",
        "je lance l outil",
        "j appelle l outil",
        "j utilise l outil",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

#[cfg(test)]
#[path = "agent_loop_tool_flow_tests.rs"]
mod tests;

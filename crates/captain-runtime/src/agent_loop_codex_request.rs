use captain_types::message::{ContentBlock, Message, MessageContent};
use captain_types::tool::ToolDefinition;

pub(crate) const CODEX_MAX_HISTORY_MESSAGES: usize = 8;
const CODEX_MIN_HISTORY_MESSAGES: usize = 4;
pub(crate) const CODEX_REQUEST_TARGET_TOKENS: usize = 16_000;

pub(crate) fn is_codex_provider(provider: &str) -> bool {
    matches!(provider, "codex" | "openai-codex")
}

pub(crate) fn history_limit_for_provider(provider: &str, default_limit: usize) -> usize {
    if is_codex_provider(provider) {
        CODEX_MAX_HISTORY_MESSAGES
    } else {
        default_limit
    }
}

pub(crate) fn initial_visible_tools_for_provider(
    available_tools: &[ToolDefinition],
    provider: &str,
) -> Vec<ToolDefinition> {
    if !is_codex_provider(provider) {
        return available_tools.to_vec();
    }

    available_tools
        .iter()
        .filter(|tool| crate::core_tools::is_core_tool(&tool.name))
        .cloned()
        .collect()
}

fn compact_tool_description_for_codex(name: &str, description: &str) -> String {
    match name {
        "capability_search" => {
            "Use only when the needed capability/tool/skill/Hand is uncertain or hidden; skip when an exact CORE tool is obvious."
                .to_string()
        }
        "tool_search" => {
            "Surface exact deferred builtin tool schemas by keyword or select:name.".to_string()
        }
        "captain_docs" => {
            "Read Captain docs/runtime changelog/tool behavior directly; no capability_search needed first."
                .to_string()
        }
        "ask_user" => "Ask one concise clarification when a user choice is required.".to_string(),
        "memory_save" => {
            "Persist a clear durable fact/preference/correction in MemPalace.".to_string()
        }
        "memory_recall" => {
            "Recall focused prior context by key/query; skip for obvious current-turn facts."
                .to_string()
        }
        "session_recall" => {
            "Search past session checkpoints when the user refers to earlier work.".to_string()
        }
        "system_time" => "Get current local time/date/timezone.".to_string(),
        _ => crate::agent_loop_context::prompt_cap_chars(
            description
                .split(['.', '\n'])
                .next()
                .unwrap_or(description)
                .trim(),
            180,
        ),
    }
}

fn compact_schema_descriptions_for_codex(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("description");
            map.remove("title");
            for child in map.values_mut() {
                compact_schema_descriptions_for_codex(child);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                compact_schema_descriptions_for_codex(child);
            }
        }
        _ => {}
    }
}

pub(crate) fn request_tools_for_provider(
    visible_tools: &[ToolDefinition],
    provider: &str,
) -> Vec<ToolDefinition> {
    if !is_codex_provider(provider) {
        return visible_tools.to_vec();
    }

    visible_tools
        .iter()
        .map(|tool| {
            let mut compact = tool.clone();
            compact.description = compact_tool_description_for_codex(&tool.name, &tool.description);
            compact_schema_descriptions_for_codex(&mut compact.input_schema);
            compact
        })
        .collect()
}

fn is_canonical_context_message(message: &Message) -> bool {
    match &message.content {
        MessageContent::Text(text) => text.starts_with("[Contexte memoire"),
        MessageContent::Blocks(blocks) => blocks.iter().any(|block| {
            matches!(block, ContentBlock::Text { text, .. } if text.starts_with("[Contexte memoire"))
        }),
    }
}

pub(crate) fn trim_oldest_context_messages(messages: &mut Vec<Message>, limit: usize) -> usize {
    if limit == 0 || messages.len() <= limit {
        return 0;
    }

    let preserve_first = messages.first().is_some_and(is_canonical_context_message);
    if preserve_first && limit > 1 {
        let keep_tail = limit - 1;
        let drain_end = messages.len().saturating_sub(keep_tail);
        if drain_end <= 1 {
            return 0;
        }
        return messages.drain(1..drain_end).count();
    }

    let trim_count = messages.len() - limit;
    messages.drain(..trim_count).count()
}

pub(crate) fn apply_provider_request_context_economy(
    messages: &mut Vec<Message>,
    system_prompt: &str,
    visible_tools: &[ToolDefinition],
    provider: &str,
) -> usize {
    if !is_codex_provider(provider) {
        return 0;
    }

    let mut removed = trim_oldest_context_messages(messages, CODEX_MAX_HISTORY_MESSAGES);
    while messages.len() > CODEX_MIN_HISTORY_MESSAGES {
        let estimated = crate::compactor::estimate_token_count(
            messages,
            Some(system_prompt),
            Some(visible_tools),
        );
        if estimated <= CODEX_REQUEST_TARGET_TOKENS {
            break;
        }
        let before = messages.len();
        removed += trim_oldest_context_messages(messages, before - 1);
        if messages.len() == before {
            break;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_def(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            input_schema: serde_json::json!({}),
        }
    }

    #[test]
    fn codex_initial_visible_tools_keep_only_core_with_catalog_rehydration() {
        let tools = vec![
            tool_def("capability_search", "resolve"),
            ToolDefinition {
                name: "mcp_mempalace_mempalace_search".into(),
                description: "search memory palace".into(),
                input_schema: serde_json::json!({"type":"object"}),
            },
        ];

        let visible = initial_visible_tools_for_provider(&tools, "codex");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "capability_search");

        let visible_claude = initial_visible_tools_for_provider(&tools, "anthropic");
        assert_eq!(visible_claude.len(), 2);
    }

    #[test]
    fn codex_request_tools_compact_descriptions_without_removing_schema_contract() {
        let tools = vec![ToolDefinition {
            name: "capability_search".into(),
            description: format!(
                "Resolve user capability requests. {}",
                "This long documentation sentence should not be sent verbatim to Codex. ".repeat(8)
            ),
            input_schema: serde_json::json!({
                "type": "object",
                "title": "CapabilitySearch",
                "description": "Verbose root schema docs.",
                "properties": {
                    "query": {
                        "type": "string",
                        "title": "Query",
                        "description": "Verbose field docs."
                    }
                },
                "required": ["query"]
            }),
        }];

        let codex_tools = request_tools_for_provider(&tools, "codex");
        assert_eq!(codex_tools[0].name, "capability_search");
        assert!(codex_tools[0].description.contains("skip"));
        assert!(codex_tools[0].description.contains("CORE"));
        assert_eq!(codex_tools[0].input_schema["type"], "object");
        assert_eq!(
            codex_tools[0].input_schema["properties"]["query"]["type"],
            "string"
        );
        assert!(codex_tools[0].input_schema.get("description").is_none());
        assert!(codex_tools[0].input_schema.get("title").is_none());
        assert!(codex_tools[0].input_schema["properties"]["query"]
            .get("description")
            .is_none());
        assert!(codex_tools[0].description.len() < tools[0].description.len());

        let claude_tools = request_tools_for_provider(&tools, "anthropic");
        assert_eq!(claude_tools[0].description, tools[0].description);
        assert!(claude_tools[0].input_schema.get("description").is_some());
        assert!(claude_tools[0].input_schema["properties"]["query"]
            .get("description")
            .is_some());
    }

    #[test]
    fn codex_context_trim_preserves_canonical_and_recent_tail() {
        let mut messages = vec![Message::user(
            "[Contexte memoire - ce n'est pas une nouvelle demande utilisateur]\nsummary",
        )];
        for idx in 0..16 {
            messages.push(Message::user(format!("turn-{idx}")));
        }

        let removed = trim_oldest_context_messages(&mut messages, CODEX_MAX_HISTORY_MESSAGES);

        assert_eq!(removed, 17 - CODEX_MAX_HISTORY_MESSAGES);
        assert_eq!(messages.len(), CODEX_MAX_HISTORY_MESSAGES);
        assert!(is_canonical_context_message(&messages[0]));
        assert_eq!(messages.last().unwrap().content.text_content(), "turn-15");
        assert!(!messages
            .iter()
            .any(|m| m.content.text_content() == "turn-0"));
    }

    #[test]
    fn provider_request_context_economy_is_codex_only() {
        let mut messages = (0..14)
            .map(|idx| Message::user(format!("message-{idx} {}", "x".repeat(20_000))))
            .collect::<Vec<_>>();
        let mut claude_messages = messages.clone();

        let codex_removed =
            apply_provider_request_context_economy(&mut messages, "system", &[], "codex");
        let claude_removed = apply_provider_request_context_economy(
            &mut claude_messages,
            "system",
            &[],
            "anthropic",
        );

        assert!(codex_removed > 0);
        assert_eq!(claude_removed, 0);
        assert!(messages.len() <= CODEX_MAX_HISTORY_MESSAGES);
        assert_eq!(claude_messages.len(), 14);
    }

    #[test]
    fn history_limit_uses_codex_cap_only_for_codex_provider() {
        assert_eq!(
            history_limit_for_provider("codex", 20),
            CODEX_MAX_HISTORY_MESSAGES
        );
        assert_eq!(history_limit_for_provider("anthropic", 20), 20);
    }
}

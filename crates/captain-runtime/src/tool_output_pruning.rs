//! Pruning of old tool outputs before full LLM compaction.
//!
//! Two-phase context management inspired by opencode: before paying for a
//! full LLM summarization pass, truncate the content of old `ToolResult`
//! blocks that sit outside a reserved recent-token window. The tool call
//! structure (ids, names, error flags) is preserved so ToolUse/ToolResult
//! pairing stays valid; only the bulky content is replaced by a short
//! placeholder with a head preview.

use crate::str_utils::safe_truncate_str;
use captain_types::message::{ContentBlock, Message, MessageContent};
use std::collections::HashMap;

/// Tools whose results are never pruned, regardless of age or size:
/// content that is not re-derivable by rerunning the tool (the user's own
/// words) or that still steers behavior (skill instructions, final
/// deliverables). Mirrors opencode's `PRUNE_PROTECTED_TOOLS` (`["skill"]`)
/// extended with Captain-specific irreplaceable outputs.
pub const NEVER_PRUNE_TOOLS: &[&str] = &[
    "ask_user",
    "skill_execute",
    "skill_view",
    "document_create",
    "document_pipeline",
];

/// Estimated tokens of the most recent messages that pruning never touches.
pub const PRUNE_RESERVED_RECENT_TOKENS: usize = 40_000;
/// Minimum estimated tokens a pruning pass must free to be applied at all.
/// Below this, the cache churn is not worth the gain.
pub const PRUNE_MINIMUM_SAVINGS_TOKENS: usize = 20_000;
/// Tool results at or below this content size are never pruned.
const MIN_PRUNABLE_CONTENT_CHARS: usize = 600;
/// Head preview preserved from a pruned tool result.
const PRUNED_PREVIEW_CHARS: usize = 160;

/// Outcome of a pruning pass over a message history.
#[derive(Debug)]
pub struct PruneReport {
    /// Messages with old tool outputs truncated. Same length and order as
    /// the input; only `ToolResult` content strings may differ.
    pub messages: Vec<Message>,
    /// Number of tool results whose content was truncated.
    pub pruned_results: usize,
    /// Estimated tokens freed (chars/4 heuristic, same as the compactor).
    pub estimated_tokens_saved: usize,
}

/// Truncate the content of old `ToolResult` blocks outside the reserved
/// recent-token window.
///
/// Walks backward from the newest message accumulating estimated tokens;
/// every message inside the `reserved_recent_tokens` budget is protected.
/// Older tool results larger than a minimum size get their content replaced
/// by a placeholder. If the whole pass would free less than
/// [`PRUNE_MINIMUM_SAVINGS_TOKENS`], the input is returned unchanged.
pub fn prune_old_tool_outputs(messages: &[Message], reserved_recent_tokens: usize) -> PruneReport {
    let cutoff = recent_window_start(messages, reserved_recent_tokens);
    let names_by_id = tool_names_by_use_id(messages);
    let mut out = messages.to_vec();
    let mut pruned_results = 0usize;
    let mut chars_saved = 0usize;

    for msg in &mut out[..cutoff] {
        let MessageContent::Blocks(blocks) = &mut msg.content else {
            continue;
        };
        for block in blocks {
            let ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                content,
                ..
            } = block
            else {
                continue;
            };
            if content.len() <= MIN_PRUNABLE_CONTENT_CHARS {
                continue;
            }
            let resolved_name = if tool_name.is_empty() {
                names_by_id
                    .get(tool_use_id.as_str())
                    .copied()
                    .unwrap_or_default()
            } else {
                tool_name.as_str()
            };
            if NEVER_PRUNE_TOOLS.contains(&resolved_name) {
                continue;
            }
            let placeholder = pruned_placeholder(resolved_name, content);
            if placeholder.len() >= content.len() {
                continue;
            }
            chars_saved += content.len() - placeholder.len();
            *content = placeholder;
            pruned_results += 1;
        }
    }

    let estimated_tokens_saved = chars_saved / 4;
    if estimated_tokens_saved < PRUNE_MINIMUM_SAVINGS_TOKENS {
        return PruneReport {
            messages: messages.to_vec(),
            pruned_results: 0,
            estimated_tokens_saved: 0,
        };
    }

    PruneReport {
        messages: out,
        pruned_results,
        estimated_tokens_saved,
    }
}

/// Map of tool_use id → tool name, for legacy `ToolResult` blocks whose
/// `tool_name` field is empty.
fn tool_names_by_use_id(messages: &[Message]) -> HashMap<&str, &str> {
    let mut map = HashMap::new();
    for msg in messages {
        let MessageContent::Blocks(blocks) = &msg.content else {
            continue;
        };
        for block in blocks {
            if let ContentBlock::ToolUse { id, name, .. } = block {
                map.insert(id.as_str(), name.as_str());
            }
        }
    }
    map
}

/// Index of the first message inside the protected recent window.
/// Messages at `[..index]` are eligible for pruning. The message that
/// straddles the budget boundary is protected (conservative side).
fn recent_window_start(messages: &[Message], reserved_recent_tokens: usize) -> usize {
    let mut tokens = 0usize;
    for (idx, msg) in messages.iter().enumerate().rev() {
        tokens += msg.content.text_length() / 4 + 4;
        if tokens > reserved_recent_tokens {
            return idx;
        }
    }
    0
}

fn pruned_placeholder(tool_name: &str, content: &str) -> String {
    let label = if tool_name.is_empty() {
        "tool"
    } else {
        tool_name
    };
    format!(
        "[CAPTAIN CONTEXT ECONOMY: pruned old {label} result ({} chars) before compaction. \
         Rerun the tool if a missing detail matters.]\n{}",
        content.len(),
        safe_truncate_str(content, PRUNED_PREVIEW_CHARS)
    )
}

#[cfg(test)]
#[path = "tool_output_pruning_tests.rs"]
mod tests;

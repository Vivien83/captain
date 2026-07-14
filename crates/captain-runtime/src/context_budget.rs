//! Dynamic context budget for tool result truncation.
//!
//! Replaces the hardcoded MAX_TOOL_RESULT_CHARS with a two-layer system:
//! - Layer 1: Per-result cap based on context window size (30% of window)
//! - Layer 2: Context guard that scans all tool results before LLM calls
//!   and compacts oldest results when total exceeds 75% headroom.

use captain_types::message::{ContentBlock, Message, MessageContent};
use captain_types::tool::ToolDefinition;
use tracing::debug;

pub use crate::context_budget_compaction::compact_tool_result_for_context;
use crate::context_budget_compaction::truncate_to;

const COLD_TOOL_RESULT_CHARS: usize = 2_000;
const CODEX_COLD_TOOL_RESULT_CHARS: usize = 800;

struct ToolResultLoc {
    msg_idx: usize,
    block_idx: usize,
    char_len: usize,
}

/// Budget parameters derived from the model's context window.
#[derive(Debug, Clone)]
pub struct ContextBudget {
    /// Total context window size in tokens.
    pub context_window_tokens: usize,
    /// Estimated characters per token for tool results (denser content).
    pub tool_chars_per_token: f64,
    /// Estimated characters per token for general content.
    pub general_chars_per_token: f64,
    /// Optional hard cap for a single tool result. Used for token-expensive
    /// providers where "large context" still hurts product latency/cost.
    pub max_per_result_chars: Option<usize>,
    /// Optional hard cap for a single persisted result during context guard.
    pub max_single_result_chars: Option<usize>,
    /// Optional hard cap for all tool results in the current LLM call.
    pub max_total_tool_chars: Option<usize>,
    /// Optional cap used when replaying old tool results from prior turns.
    pub max_cold_tool_result_chars: Option<usize>,
}

impl ContextBudget {
    /// Create a new budget from a context window size.
    pub fn new(context_window_tokens: usize) -> Self {
        Self {
            context_window_tokens,
            tool_chars_per_token: 2.0,
            general_chars_per_token: 4.0,
            max_per_result_chars: None,
            max_single_result_chars: None,
            max_total_tool_chars: None,
            max_cold_tool_result_chars: None,
        }
    }

    /// Provider-specific economy mode for Codex/OpenAI subscription-backed
    /// calls. Codex can accept huge context windows, but replaying stale tool
    /// output on every turn is a product bug: latency and token counters explode
    /// without improving simple answers.
    pub fn codex_economy(context_window_tokens: usize) -> Self {
        Self {
            max_per_result_chars: Some(8_000),
            max_single_result_chars: Some(10_000),
            max_total_tool_chars: Some(6_000),
            max_cold_tool_result_chars: Some(CODEX_COLD_TOOL_RESULT_CHARS),
            ..Self::new(context_window_tokens)
        }
    }

    /// Per-result character cap: 30% of context window converted to chars.
    pub fn per_result_cap(&self) -> usize {
        let tokens_for_tool = (self.context_window_tokens as f64 * 0.30) as usize;
        let dynamic = (tokens_for_tool as f64 * self.tool_chars_per_token) as usize;
        self.max_per_result_chars
            .map(|cap| dynamic.min(cap))
            .unwrap_or(dynamic)
    }

    /// Single result absolute max: 50% of context window.
    pub fn single_result_max(&self) -> usize {
        let tokens = (self.context_window_tokens as f64 * 0.50) as usize;
        let dynamic = (tokens as f64 * self.tool_chars_per_token) as usize;
        self.max_single_result_chars
            .map(|cap| dynamic.min(cap))
            .unwrap_or(dynamic)
    }

    /// Total tool result headroom: 75% of context window in chars.
    pub fn total_tool_headroom_chars(&self) -> usize {
        let tokens = (self.context_window_tokens as f64 * 0.75) as usize;
        let dynamic = (tokens as f64 * self.tool_chars_per_token) as usize;
        self.max_total_tool_chars
            .map(|cap| dynamic.min(cap))
            .unwrap_or(dynamic)
    }

    pub fn cold_tool_result_cap(&self) -> usize {
        self.max_cold_tool_result_chars
            .unwrap_or(COLD_TOOL_RESULT_CHARS)
    }
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self::new(200_000)
    }
}

/// Layer 1: Truncate a single tool result dynamically based on context budget.
///
/// Breaks at newline boundaries when possible to avoid mid-line truncation.
pub fn truncate_tool_result_dynamic(content: &str, budget: &ContextBudget) -> String {
    let cap = budget.per_result_cap();
    if content.len() <= cap {
        return content.to_string();
    }

    // Find last newline before the cap to break cleanly (char-boundary safe)
    let mut safe_cap = cap.min(content.len());
    while safe_cap > 0 && !content.is_char_boundary(safe_cap) {
        safe_cap -= 1;
    }
    let mut search_start = safe_cap.saturating_sub(200);
    // Ensure search_start is a valid char boundary
    while search_start > 0 && !content.is_char_boundary(search_start) {
        search_start -= 1;
    }
    let mut break_point = content[search_start..safe_cap]
        .rfind('\n')
        .map(|pos| search_start + pos)
        .unwrap_or(safe_cap.saturating_sub(100));
    // Ensure break_point is also a char boundary
    while break_point > 0 && !content.is_char_boundary(break_point) {
        break_point -= 1;
    }

    format!(
        "{}\n\n[TRUNCATED: result was {} chars, showing first {} (budget: {}% of {}K context window)]",
        &content[..break_point],
        content.len(),
        break_point,
        30,
        budget.context_window_tokens / 1000
    )
}

/// Layer 2: Context guard — scan all tool_result blocks in the message history.
///
/// If total tool result content exceeds 75% of the context headroom,
/// compact oldest results first. Returns the number of results compacted.
pub fn apply_context_guard(
    messages: &mut [Message],
    budget: &ContextBudget,
    tools: &[ToolDefinition],
) -> usize {
    apply_context_guard_preserving_recent(messages, budget, tools, 0)
}

/// Context guard variant that can preserve the newest tool results from the
/// active tool loop. On Codex this lets us aggressively compact stale replay
/// while keeping the just-produced evidence rich enough for the final answer.
pub fn apply_context_guard_preserving_recent(
    messages: &mut [Message],
    budget: &ContextBudget,
    _tools: &[ToolDefinition],
    preserve_recent_tool_results: usize,
) -> usize {
    let headroom = budget.total_tool_headroom_chars();
    let (locations, mut total_chars) = collect_tool_result_locations(messages);

    if total_chars <= headroom {
        return 0;
    }

    debug_context_guard_pressure(total_chars, headroom, locations.len());

    let mut compacted =
        compact_oversized_tool_results(messages, budget, &locations, &mut total_chars);
    compacted += compact_cold_tool_results(
        messages,
        budget,
        &locations,
        &mut total_chars,
        headroom,
        preserve_recent_tool_results,
    );
    compacted
}

fn collect_tool_result_locations(messages: &[Message]) -> (Vec<ToolResultLoc>, usize) {
    let mut locations: Vec<ToolResultLoc> = Vec::new();
    let mut total_chars: usize = 0;

    for (msg_idx, msg) in messages.iter().enumerate() {
        if let MessageContent::Blocks(blocks) = &msg.content {
            for (block_idx, block) in blocks.iter().enumerate() {
                if let ContentBlock::ToolResult { content, .. } = block {
                    let len = content.len();
                    total_chars += len;
                    locations.push(ToolResultLoc {
                        msg_idx,
                        block_idx,
                        char_len: len,
                    });
                }
            }
        }
    }
    (locations, total_chars)
}

fn debug_context_guard_pressure(total_chars: usize, headroom: usize, results: usize) {
    debug!(
        total_chars,
        headroom, results, "Context guard: tool results exceed headroom, compacting oldest"
    );
}

fn compact_oversized_tool_results(
    messages: &mut [Message],
    budget: &ContextBudget,
    locations: &[ToolResultLoc],
    total_chars: &mut usize,
) -> usize {
    let single_max = budget.single_result_max();
    let mut compacted = 0;
    for loc in locations {
        if loc.char_len > single_max {
            if let Some((old_len, new_len)) =
                compact_tool_result_at(messages, loc, budget, single_max)
            {
                adjust_total_chars(total_chars, old_len, new_len);
                compacted += 1;
            }
        }
    }
    compacted
}

fn compact_cold_tool_results(
    messages: &mut [Message],
    budget: &ContextBudget,
    locations: &[ToolResultLoc],
    total_chars: &mut usize,
    headroom: usize,
    preserve_recent_tool_results: usize,
) -> usize {
    let compact_target = budget.cold_tool_result_cap();
    let protected_start = locations.len().saturating_sub(preserve_recent_tool_results);
    let mut compacted = 0;
    for (idx, loc) in locations.iter().enumerate() {
        if *total_chars <= headroom {
            break;
        }
        if idx >= protected_start {
            continue;
        }
        if loc.char_len <= compact_target {
            continue;
        }
        if let Some((old_len, new_len)) =
            compact_tool_result_at(messages, loc, budget, compact_target)
        {
            adjust_total_chars(total_chars, old_len, new_len);
            compacted += 1;
        }
    }
    compacted
}

fn compact_tool_result_at(
    messages: &mut [Message],
    loc: &ToolResultLoc,
    budget: &ContextBudget,
    target_chars: usize,
) -> Option<(usize, usize)> {
    if loc.msg_idx >= messages.len() {
        return None;
    }
    let MessageContent::Blocks(blocks) = &mut messages[loc.msg_idx].content else {
        return None;
    };
    if loc.block_idx >= blocks.len() {
        return None;
    }
    let ContentBlock::ToolResult {
        tool_name,
        content,
        is_error,
        ..
    } = &mut blocks[loc.block_idx]
    else {
        return None;
    };
    if content.len() <= target_chars {
        return None;
    }
    let old_len = content.len();
    let compact_body = compact_tool_result_for_context(tool_name, content, *is_error, budget);
    *content = truncate_to(&compact_body, target_chars);
    Some((old_len, content.len()))
}

fn adjust_total_chars(total_chars: &mut usize, old_len: usize, new_len: usize) {
    *total_chars -= old_len;
    *total_chars += new_len;
}

#[cfg(test)]
#[path = "context_budget_tests.rs"]
mod tests;

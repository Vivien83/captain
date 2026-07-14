//! Deterministic task checkpoint — no LLM involved.
//!
//! Extracts from a message history what a resuming turn needs to not lose
//! the thread of a long task: the last explicit user request, the tool
//! activity since that request, and whether the history ends mid-tool-run.
//! Used both for the periodic durable checkpoint (written to the structured
//! KV store well before compaction triggers) and as the authoritative
//! "# Demande active" content of the compaction handoff, replacing the
//! generic mid-tool-activity fallback note.

use crate::compaction_boundary::is_human_user_boundary;
use crate::compaction_handoff::ends_mid_tool_activity;
use crate::str_utils::safe_truncate_str;
use captain_types::message::{ContentBlock, Message, MessageContent};
use serde::{Deserialize, Serialize};

/// Maximum characters kept from the last user request.
const MAX_REQUEST_CHARS: usize = 600;
/// Maximum distinct tool names listed in the checkpoint.
const MAX_TOOL_ENTRIES: usize = 20;

/// Deterministic snapshot of the task in progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCheckpoint {
    /// Last explicit user request (human text, not a system injection),
    /// truncated. `None` when the history has no such message.
    pub last_user_request: Option<String>,
    /// Tool calls issued since that request, as "name xCount" in first-call
    /// order (e.g. "shell_exec x3").
    pub tool_calls_since: Vec<String>,
    /// True when the history ends on a ToolUse/ToolResult rather than a
    /// completed assistant reply.
    pub mid_tool_activity: bool,
}

/// Extract a checkpoint from a message history. Pure and deterministic.
pub fn extract_task_checkpoint(messages: &[Message]) -> TaskCheckpoint {
    let request_idx = last_explicit_user_index(messages);
    let last_user_request = request_idx.map(|idx| {
        safe_truncate_str(
            messages[idx].content.text_content().trim(),
            MAX_REQUEST_CHARS,
        )
        .to_string()
    });
    let since = match request_idx {
        Some(idx) => &messages[idx + 1..],
        None => messages,
    };

    TaskCheckpoint {
        last_user_request,
        tool_calls_since: tool_call_counts(since),
        mid_tool_activity: ends_mid_tool_activity(messages),
    }
}

/// Render the checkpoint as the "# Demande active" bullet(s) of a compaction
/// handoff. Always returns actionable content, falling back to a generic
/// warning when nothing could be extracted.
pub fn checkpoint_note(checkpoint: &TaskCheckpoint) -> String {
    let mut lines = Vec::new();

    match &checkpoint.last_user_request {
        Some(request) => lines.push(format!("- Demande utilisateur en cours: \"{request}\"")),
        None => lines.push(
            "- Demande utilisateur en cours non identifiee dans la partie compactee.".to_string(),
        ),
    }

    if !checkpoint.tool_calls_since.is_empty() {
        lines.push(format!(
            "- Travail effectue depuis cette demande: {}.",
            checkpoint.tool_calls_since.join(", ")
        ));
    }

    if checkpoint.mid_tool_activity {
        lines.push(
            "- Travail probablement encore en cours: la compaction est intervenue juste apres \
             un appel/resultat d'outil, pas apres une reponse terminee. Ne pas conclure qu'il \
             n'y a rien a faire sans verifier session_tool_call_summary et le dernier message \
             utilisateur apres ce handoff."
                .to_string(),
        );
    }

    lines.join("\n")
}

/// Index of the last explicit (human) user message: a non-empty user text
/// that is not a Captain system injection. Injected reference blocks all
/// start with '[' ("[Contexte memoire...]", "[CAPTAIN CONTEXT ECONOMY...]").
fn last_explicit_user_index(messages: &[Message]) -> Option<usize> {
    messages.iter().rposition(|msg| {
        is_human_user_boundary(msg) && !msg.content.text_content().trim_start().starts_with('[')
    })
}

/// Count ToolUse blocks by name, in first-call order.
fn tool_call_counts(messages: &[Message]) -> Vec<String> {
    let mut order: Vec<(String, usize)> = Vec::new();
    for msg in messages {
        let MessageContent::Blocks(blocks) = &msg.content else {
            continue;
        };
        for block in blocks {
            let ContentBlock::ToolUse { name, .. } = block else {
                continue;
            };
            if let Some(entry) = order.iter_mut().find(|(n, _)| n == name) {
                entry.1 += 1;
            } else if order.len() < MAX_TOOL_ENTRIES {
                order.push((name.clone(), 1));
            }
        }
    }
    order
        .into_iter()
        .map(|(name, count)| format!("{name} x{count}"))
        .collect()
}

#[cfg(test)]
#[path = "task_checkpoint_tests.rs"]
mod tests;

//! LLM-based session compaction.
//!
//! When a session's message count exceeds a threshold, the compactor
//! uses an LLM to summarize older messages into a concise summary,
//! keeping only the most recent messages intact. This prevents context
//! windows from growing unboundedly while preserving key information.
//!
//! Supports three summarization stages:
//! 1. Full single-pass summarization (fastest, best quality)
//! 2. Adaptive chunked summarization with merge (handles large histories)
//! 3. Minimal fallback without LLM (when summarization is unavailable)

use crate::compaction_boundary::coherent_recent_split;
use crate::compactor_summarization::{
    adaptive_chunk_size, summarize_in_chunks, summarize_messages,
};
use crate::tool_output_pruning::{prune_old_tool_outputs, PRUNE_RESERVED_RECENT_TOKENS};
use crate::{compaction_handoff, llm_driver::LlmDriver};
use captain_memory::session::Session;
use captain_types::message::Message;
use captain_types::tool::ToolDefinition;
use std::sync::Arc;
use tracing::{info, warn};

pub use crate::compactor_context::{
    format_context_report, generate_context_report, ContextBreakdown, ContextPressure,
    ContextReport,
};

/// Configuration for session compaction.
#[derive(Debug, Clone)]
pub struct CompactionConfig {
    /// Compact when session message count exceeds this.
    pub threshold: usize,
    /// Number of recent messages to keep verbatim (not summarized).
    pub keep_recent: usize,
    /// Maximum tokens for the summary generation.
    pub max_summary_tokens: u32,
    /// Base ratio of messages to process per chunk (0.0-1.0).
    pub base_chunk_ratio: f64,
    /// Minimum chunk ratio (floor for adaptive computation).
    pub min_chunk_ratio: f64,
    /// Safety margin multiplier for token estimation inaccuracy.
    pub safety_margin: f64,
    /// Overhead tokens reserved for summarization prompt itself.
    pub summarization_overhead_tokens: u32,
    /// Maximum input chars per summarization chunk.
    pub max_chunk_chars: usize,
    /// Maximum retry attempts for summarization.
    pub max_retries: u32,
    /// Trigger compaction when estimated tokens exceed this fraction of context_window_tokens.
    pub token_threshold_ratio: f64,
    /// Model context window size in tokens.
    pub context_window_tokens: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            threshold: 30,
            keep_recent: 10,
            max_summary_tokens: 1024,
            base_chunk_ratio: 0.4,
            min_chunk_ratio: 0.15,
            safety_margin: 1.2,
            summarization_overhead_tokens: 4096,
            max_chunk_chars: 80_000,
            max_retries: 3,
            token_threshold_ratio: 0.7,
            context_window_tokens: 200_000,
        }
    }
}

impl CompactionConfig {
    /// Product-economy profile for subscription-backed Codex.
    ///
    /// Codex accepts large windows, but replaying ~100k tokens for routine
    /// assistant turns is a bad product default. This profile compacts earlier,
    /// keeps fewer raw turns, and relies on the canonical summary plus memory
    /// recall to preserve quality.
    pub fn codex_economy() -> Self {
        Self {
            threshold: 14,
            keep_recent: 6,
            max_summary_tokens: 768,
            base_chunk_ratio: 0.30,
            min_chunk_ratio: 0.12,
            max_chunk_chars: 48_000,
            token_threshold_ratio: 0.22,
            ..Self::default()
        }
    }

    pub fn for_provider(provider: &str) -> Self {
        if matches!(provider, "codex" | "openai-codex") {
            Self::codex_economy()
        } else {
            Self::default()
        }
    }
}

/// Result of a compaction operation.
#[derive(Debug)]
pub struct CompactionResult {
    /// LLM-generated summary of the compacted messages.
    pub summary: String,
    /// Messages to keep (the most recent ones).
    pub kept_messages: Vec<Message>,
    /// Number of messages that were compacted (summarized).
    pub compacted_count: usize,
    /// Number of chunks used (1 = single-pass, >1 = chunked).
    pub chunks_used: u32,
    /// Whether fallback was used (LLM unavailable).
    pub used_fallback: bool,
    /// Number of old tool results whose content was pruned before summarization.
    pub pruned_tool_results: usize,
    /// True when pruning alone was enough: no LLM summarization ran, no
    /// summary was produced, and `kept_messages` is the full pruned history.
    pub pruned_only: bool,
}

/// Check whether a session needs compaction (message-count trigger).
pub fn needs_compaction(session: &Session, config: &CompactionConfig) -> bool {
    session.messages.len() > config.threshold
}

/// Estimate token count for a set of messages, optional system prompt, and tool definitions.
///
/// Uses the chars/4 heuristic - not exact, but good enough for budget gating.
pub fn estimate_token_count(
    messages: &[Message],
    system_prompt: Option<&str>,
    tools: Option<&[ToolDefinition]>,
) -> usize {
    let mut chars: usize = 0;

    if let Some(sp) = system_prompt {
        chars += sp.len();
    }

    for msg in messages {
        chars += msg.content.text_length();
        chars += 16;
    }

    if let Some(tool_defs) = tools {
        for tool in tool_defs {
            chars += tool.name.len() + tool.description.len();
            if let Ok(schema_str) = serde_json::to_string(&tool.input_schema) {
                chars += schema_str.len();
            }
        }
    }

    chars / 4
}

/// Check whether estimated tokens exceed the compaction threshold.
///
/// Returns true if `estimated_tokens > context_window * token_threshold_ratio`.
pub fn needs_compaction_by_tokens(estimated_tokens: usize, config: &CompactionConfig) -> bool {
    let threshold = (config.context_window_tokens as f64 * config.token_threshold_ratio) as usize;
    estimated_tokens > threshold
}

/// Compact a session by summarizing older messages with an LLM.
///
/// First prunes the content of old tool results outside the reserved recent
/// window (deterministic, no LLM). If pruning alone brings the session back
/// under the compaction thresholds, no LLM summarization runs at all and the
/// pruned history is returned as-is (`pruned_only`).
///
/// Otherwise takes all messages except the most recent `keep_recent` and uses
/// a multi-stage approach to produce a concise summary:
///
/// 1. **Full summarization**: tries to summarize all older messages in one pass
/// 2. **Chunked summarization**: splits into adaptive chunks, summarizes each,
///    then merges the chunk summaries
/// 3. **Minimal fallback**: if LLM is unavailable, produces a placeholder note
///
/// `non_message_overhead_tokens` is the estimated token cost of the system
/// prompt and tool definitions, needed to re-check the token threshold after
/// pruning on the same basis the caller used to trigger compaction.
///
/// Returns the summary, the kept messages, and metadata about the operation.
pub async fn compact_session(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    session: &Session,
    config: &CompactionConfig,
    non_message_overhead_tokens: usize,
) -> Result<CompactionResult, String> {
    let prune = prune_old_tool_outputs(&session.messages, PRUNE_RESERVED_RECENT_TOKENS);
    let messages = prune.messages;
    let msg_count = messages.len();

    let pruning_alone_sufficient = prune.pruned_results > 0 && {
        let estimated = estimate_token_count(&messages, None, None) + non_message_overhead_tokens;
        msg_count <= config.threshold && !needs_compaction_by_tokens(estimated, config)
    };

    if pruning_alone_sufficient || msg_count <= config.keep_recent {
        if prune.pruned_results > 0 {
            info!(
                pruned = prune.pruned_results,
                tokens_saved = prune.estimated_tokens_saved,
                llm_compaction_skipped = pruning_alone_sufficient,
                "Old tool outputs pruned; no LLM summarization this round"
            );
        }
        return Ok(CompactionResult {
            summary: String::new(),
            kept_messages: messages,
            compacted_count: 0,
            chunks_used: 0,
            used_fallback: false,
            pruned_tool_results: prune.pruned_results,
            pruned_only: prune.pruned_results > 0,
        });
    }

    if prune.pruned_results > 0 {
        info!(
            pruned = prune.pruned_results,
            tokens_saved = prune.estimated_tokens_saved,
            "Old tool outputs pruned before LLM summarization"
        );
    }

    let split_at = coherent_recent_split(&messages, config.keep_recent);
    let to_compact = &messages[..split_at];
    let kept = &messages[split_at..];

    info!(
        total = msg_count,
        compacting = to_compact.len(),
        keeping = kept.len(),
        "Compacting session messages"
    );

    let kept_messages = kept.to_vec();
    let compacted_count = to_compact.len();
    // Deterministic signal, independent of the LLM summarizer: was this cut
    // made mid-tool-activity rather than at a completed reply? If so, the
    // "# Demande active" section is overwritten below with the deterministic
    // task checkpoint (last user request + tool activity since), regardless
    // of what the summarizer wrote there.
    let active_note = compaction_handoff::ends_mid_tool_activity(to_compact).then(|| {
        crate::task_checkpoint::checkpoint_note(&crate::task_checkpoint::extract_task_checkpoint(
            to_compact,
        ))
    });

    match summarize_messages(driver.clone(), model, to_compact, config).await {
        Ok(summary) => {
            info!(
                summary_len = summary.len(),
                compacted = compacted_count,
                "Session compaction complete (single-pass)"
            );
            return Ok(CompactionResult {
                summary: compaction_handoff::enforce_active_task_note(
                    &summary,
                    active_note.as_deref(),
                ),
                kept_messages,
                compacted_count,
                chunks_used: 1,
                used_fallback: false,
                pruned_tool_results: prune.pruned_results,
                pruned_only: false,
            });
        }
        Err(e) => {
            warn!(error = %e, "Full summarization failed, trying chunked approach");
        }
    }

    match summarize_in_chunks(driver.clone(), model, to_compact, config).await {
        Ok(summary) => {
            let chunk_size = adaptive_chunk_size(to_compact, config);
            let num_chunks = (to_compact.len() as f64 / chunk_size as f64).ceil() as u32;

            info!(
                summary_len = summary.len(),
                compacted = compacted_count,
                chunks = num_chunks,
                "Session compaction complete (chunked)"
            );
            return Ok(CompactionResult {
                summary: compaction_handoff::enforce_active_task_note(
                    &summary,
                    active_note.as_deref(),
                ),
                kept_messages,
                compacted_count,
                chunks_used: num_chunks.max(1),
                used_fallback: false,
                pruned_tool_results: prune.pruned_results,
                pruned_only: false,
            });
        }
        Err(e) => {
            warn!(error = %e, "Chunked summarization failed, using minimal fallback");
        }
    }

    let minimal = compaction_handoff::enforce_active_task_note(
        &compaction_handoff::fallback_handoff_summary(to_compact.len(), kept_messages.len()),
        active_note.as_deref(),
    );

    warn!(
        compacted = compacted_count,
        "Using fallback compaction (no LLM summary)"
    );

    Ok(CompactionResult {
        summary: minimal,
        kept_messages,
        compacted_count,
        chunks_used: 0,
        used_fallback: true,
        pruned_tool_results: prune.pruned_results,
        pruned_only: false,
    })
}

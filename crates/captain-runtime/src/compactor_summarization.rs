//! LLM summarization helpers for session compaction.

use crate::compactor::CompactionConfig;
use crate::llm_driver::{CompletionRequest, LlmDriver};
use crate::str_utils::safe_truncate_str;
use crate::{compaction_handoff, compaction_handoff::HandoffLimits};
use captain_types::message::{ContentBlock, Message, MessageContent, Role};
use std::sync::Arc;
use tracing::{info, warn};

/// Compute adaptive chunk ratio based on average message size.
///
/// Shorter messages get larger chunks (more context per summary).
/// Longer messages get smaller chunks (each message has more info to summarize).
pub(crate) fn compute_adaptive_chunk_ratio(messages: &[Message], config: &CompactionConfig) -> f64 {
    if messages.is_empty() {
        return config.base_chunk_ratio;
    }

    let avg_len = messages
        .iter()
        .map(|m| m.content.text_length())
        .sum::<usize>() as f64
        / messages.len() as f64;

    let ratio = if avg_len > 1000.0 {
        config.min_chunk_ratio
    } else if avg_len > 500.0 {
        (config.base_chunk_ratio + config.min_chunk_ratio) / 2.0
    } else {
        config.base_chunk_ratio
    };

    ratio.clamp(config.min_chunk_ratio, config.base_chunk_ratio)
}

/// Check if a single message is oversized (> 50% of max_chunk_chars).
///
/// Oversized messages should be summarized individually rather than in chunks
/// to avoid exceeding context window limits.
pub(crate) fn is_oversized(message: &Message, config: &CompactionConfig) -> bool {
    message.content.text_length() > config.max_chunk_chars / 2
}

/// Build conversation text from a slice of messages (block-aware).
///
/// Handles all content block types: text, tool use, tool result, image, unknown.
/// Oversized messages are truncated inline with a marker.
pub(crate) fn build_conversation_text(messages: &[Message], config: &CompactionConfig) -> String {
    let mut conversation_text = String::new();

    for msg in messages {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::System => "System",
        };
        let oversized = is_oversized(msg, config);

        match &msg.content {
            MessageContent::Text(s) => {
                push_text_content(&mut conversation_text, role_label, s, oversized, config)
            }
            MessageContent::Blocks(blocks) => {
                push_block_content(
                    &mut conversation_text,
                    role_label,
                    blocks,
                    oversized,
                    config,
                );
            }
        }
    }

    conversation_text
}

fn push_text_content(
    conversation_text: &mut String,
    role_label: &str,
    text: &str,
    oversized: bool,
    config: &CompactionConfig,
) {
    if text.is_empty() {
        return;
    }
    if oversized {
        let limit = config.max_chunk_chars / 4;
        let truncated = if text.len() > limit {
            format!(
                "{}...[truncated from {} chars]",
                safe_truncate_str(text, limit),
                text.len()
            )
        } else {
            text.to_string()
        };
        conversation_text.push_str(&format!("{role_label}: {truncated}\n\n"));
    } else {
        conversation_text.push_str(&format!("{role_label}: {text}\n\n"));
    }
}

fn push_block_content(
    conversation_text: &mut String,
    role_label: &str,
    blocks: &[ContentBlock],
    oversized: bool,
    config: &CompactionConfig,
) {
    for block in blocks {
        match block {
            ContentBlock::Text { text, .. } => {
                push_text_content(conversation_text, role_label, text, oversized, config);
            }
            ContentBlock::ToolUse { name, input, .. } => {
                push_tool_use(conversation_text, name, input);
            }
            ContentBlock::ToolResult {
                content, is_error, ..
            } => {
                push_tool_result(conversation_text, content, *is_error);
            }
            ContentBlock::Image { media_type, .. } => {
                conversation_text.push_str(&format!("[Image: {media_type}]\n\n"));
            }
            ContentBlock::Thinking { .. } | ContentBlock::Unknown => {}
        }
    }
}

fn push_tool_use(conversation_text: &mut String, name: &str, input: &serde_json::Value) {
    let input_str = serde_json::to_string(input).unwrap_or_default();
    let input_preview = if input_str.len() > 200 {
        format!("{}...", safe_truncate_str(&input_str, 200))
    } else {
        input_str
    };
    conversation_text.push_str(&format!(
        "[Used tool '{name}' with params: {input_preview}]\n\n"
    ));
}

fn push_tool_result(conversation_text: &mut String, content: &str, is_error: bool) {
    let status = if is_error { "ERROR" } else { "OK" };
    let cleaned = crate::session_repair::strip_tool_result_details(content);
    let preview = if cleaned.len() > 2000 {
        format!("{}...", safe_truncate_str(&cleaned, 2000))
    } else {
        cleaned
    };
    conversation_text.push_str(&format!("[Tool result ({status}): {preview}]\n\n"));
}

/// Summarize a slice of messages using the LLM.
///
/// Builds the conversation text, applies chunking limits, and calls the LLM
/// with a summarization prompt. Retries on transient failures.
pub(crate) async fn summarize_messages(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    messages: &[Message],
    config: &CompactionConfig,
) -> Result<String, String> {
    let mut conversation_text = build_conversation_text(messages, config);
    let handoff_limits = compaction_handoff::handoff_limits(config);

    let effective_max = (config.max_chunk_chars as f64 / config.safety_margin) as usize;
    if conversation_text.len() > effective_max {
        conversation_text = truncate_conversation_tail(conversation_text, effective_max);
    }

    let summarize_prompt =
        compaction_handoff::handoff_user_prompt(&conversation_text, &handoff_limits);
    let request = summarization_request(model, summarize_prompt, config, &handoff_limits);

    let mut last_error = String::new();
    for attempt in 0..config.max_retries {
        match driver.complete(request.clone()).await {
            Ok(response) => {
                let summary = response.text();
                if summary.is_empty() {
                    last_error = "LLM returned empty summary".to_string();
                    warn!(attempt, "Empty summary from LLM, retrying");
                    continue;
                }
                return Ok(compaction_handoff::normalize_handoff_summary(
                    &summary,
                    &handoff_limits,
                ));
            }
            Err(e) => {
                last_error = format!("LLM summarization failed: {e}");
                if attempt + 1 < config.max_retries {
                    warn!(attempt, error = %e, "Summarization attempt failed, retrying");
                }
            }
        }
    }

    Err(last_error)
}

fn truncate_conversation_tail(conversation_text: String, effective_max: usize) -> String {
    let start = conversation_text.len() - effective_max;
    let safe_start = if conversation_text.is_char_boundary(start) {
        start
    } else {
        conversation_text[start..]
            .char_indices()
            .next()
            .map(|(i, _)| start + i)
            .unwrap_or(conversation_text.len())
    };
    conversation_text[safe_start..].to_string()
}

fn summarization_request(
    model: &str,
    prompt: String,
    config: &CompactionConfig,
    handoff_limits: &HandoffLimits,
) -> CompletionRequest {
    CompletionRequest {
        model: model.to_string(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Blocks(vec![ContentBlock::Text {
                text: prompt,
                provider_metadata: None,
            }]),
        }],
        tools: vec![],
        max_tokens: config.max_summary_tokens,
        temperature: 0.3,
        system: Some(compaction_handoff::handoff_system_prompt(handoff_limits)),
        thinking: None,
        tool_choice: None,
        cache_hints: crate::llm_driver::CacheHints::default(),
    }
}

/// Summarize messages in adaptive chunks, then merge the per-chunk summaries.
///
/// Splits messages into chunks based on adaptive ratio (accounting for message size),
/// summarizes each chunk independently, then merges all chunk summaries with a final
/// LLM call into one cohesive summary.
pub(crate) async fn summarize_in_chunks(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    messages: &[Message],
    config: &CompactionConfig,
) -> Result<String, String> {
    let handoff_limits = compaction_handoff::handoff_limits(config);
    let chunk_size = adaptive_chunk_size(messages, config);

    info!(
        total = messages.len(),
        chunk_size,
        chunk_ratio = compute_adaptive_chunk_ratio(messages, config),
        "Starting chunked summarization"
    );

    let summaries = summarize_chunks(driver.clone(), model, messages, config, chunk_size).await?;
    if summaries.len() == 1 {
        return Ok(summaries.into_iter().next().unwrap());
    }

    merge_chunk_summaries(driver, model, summaries, config, &handoff_limits).await
}

pub(crate) fn adaptive_chunk_size(messages: &[Message], config: &CompactionConfig) -> usize {
    let chunk_ratio = compute_adaptive_chunk_ratio(messages, config);
    let chunk_size = (messages.len() as f64 * chunk_ratio).ceil() as usize;
    chunk_size.max(5)
}

async fn summarize_chunks(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    messages: &[Message],
    config: &CompactionConfig,
    chunk_size: usize,
) -> Result<Vec<String>, String> {
    let mut summaries = Vec::new();
    let mut success_count = 0usize;
    let mut last_chunk_error = String::new();

    for (i, chunk) in messages.chunks(chunk_size).enumerate() {
        match summarize_messages(driver.clone(), model, chunk, config).await {
            Ok(summary) => {
                info!(chunk = i, summary_len = summary.len(), "Chunk summarized");
                summaries.push(summary);
                success_count += 1;
            }
            Err(e) => {
                warn!(chunk = i, error = %e, "Chunk summarization failed, skipping");
                last_chunk_error = e;
                summaries.push(format!(
                    "[Chunk {}: {} messages, summarization unavailable]",
                    i + 1,
                    chunk.len()
                ));
            }
        }
    }

    if success_count == 0 {
        return Err(format!(
            "All {} chunks failed to summarize: {last_chunk_error}",
            summaries.len()
        ));
    }
    if summaries.is_empty() {
        return Err("No chunks were summarized".to_string());
    }

    Ok(summaries)
}

async fn merge_chunk_summaries(
    driver: Arc<dyn LlmDriver>,
    model: &str,
    summaries: Vec<String>,
    config: &CompactionConfig,
    handoff_limits: &HandoffLimits,
) -> Result<String, String> {
    let merge_prompt = compaction_handoff::merge_handoff_user_prompt(&summaries, handoff_limits);
    let merge_request = summarization_request(model, merge_prompt, config, handoff_limits);

    match driver.complete(merge_request).await {
        Ok(response) => {
            let merged = response.text();
            if merged.is_empty() {
                Ok(normalize_joined_handoff_summaries(
                    &summaries,
                    handoff_limits,
                ))
            } else {
                Ok(compaction_handoff::normalize_handoff_summary(
                    &merged,
                    handoff_limits,
                ))
            }
        }
        Err(e) => {
            warn!(error = %e, "Merge summarization failed, concatenating chunks");
            Ok(normalize_joined_handoff_summaries(
                &summaries,
                handoff_limits,
            ))
        }
    }
}

fn normalize_joined_handoff_summaries(summaries: &[String], limits: &HandoffLimits) -> String {
    compaction_handoff::normalize_handoff_summary(&summaries.join("\n\n"), limits)
}

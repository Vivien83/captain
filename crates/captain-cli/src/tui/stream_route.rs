use super::screens::chat::{ChatState, PendingAskUser, Role};
use captain_runtime::llm_driver::StreamEvent;

pub(crate) fn apply_stream_event(chat: &mut ChatState, ev: StreamEvent) {
    match ev {
        StreamEvent::TextDelta { text } => {
            chat.thinking = false;
            if chat.active_tool.is_some() {
                chat.active_tool = None;
            }
            chat.append_stream(&text);
        }
        StreamEvent::ToolUseStart { id, name } => {
            flush_streaming_text(chat);
            chat.tool_start(&id, &name);
        }
        StreamEvent::ToolInputDelta { text } => {
            chat.tool_input_buf.push_str(&text);
        }
        StreamEvent::ToolUseEnd { id, name, input } => {
            let input_str = if !chat.tool_input_buf.is_empty() {
                std::mem::take(&mut chat.tool_input_buf)
            } else {
                serde_json::to_string(&input).unwrap_or_default()
            };
            chat.tool_use_end(&id, &name, &input_str);
        }
        StreamEvent::ContentComplete { usage, .. } => {
            chat.record_context_usage(usage.input_tokens, usage.output_tokens);
            chat.last_tokens = Some((usage.input_tokens, usage.output_tokens));
            chat.last_cached_input_tokens = usage.cached_input_tokens;
            chat.last_cache_creation_tokens = usage.cache_creation_tokens;
        }
        StreamEvent::PhaseChange { phase, detail } => {
            if phase == "tool_use" {
                if let Some(tool_name) = detail {
                    chat.tool_start("", &tool_name);
                }
            } else if phase == "thinking" {
                chat.thinking = true;
            } else if phase == "model_fallback" {
                if let Some(text) = detail.filter(|value| !value.trim().is_empty()) {
                    chat.push_message(Role::Agent, text);
                }
            }
        }
        StreamEvent::ThinkingDelta { text } => {
            chat.append_thinking(&text);
        }
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error,
        } => {
            chat.tool_result(&tool_use_id, &name, &result_preview, is_error);
        }
        StreamEvent::IntermediateMessage { content } => {
            flush_streaming_text(chat);
            chat.push_message(Role::Agent, content);
        }
        StreamEvent::AskUser { question, options } => {
            let options = options.unwrap_or_default();
            if options.is_empty() {
                // No predefined choices to render as a modal — keep the
                // existing plain-text behavior so the keyboard stays free
                // for typing the answer.
                chat.push_message(Role::Agent, format!("\u{2753} {question}"));
            } else {
                chat.pending_ask_user = Some(PendingAskUser { question, options });
            }
        }
        StreamEvent::UserResponse { .. } => {}
        StreamEvent::ToolOutputDelta {
            tool_use_id,
            stream,
            chunk,
        } => {
            chat.tool_output_delta(&tool_use_id, stream, &chunk);
        }
    }
}

fn flush_streaming_text(chat: &mut ChatState) {
    if !chat.streaming_text.is_empty() {
        let text = std::mem::take(&mut chat.streaming_text);
        chat.push_message(Role::Agent, text);
    }
}

#[cfg(test)]
#[path = "stream_route/tests.rs"]
mod tests;

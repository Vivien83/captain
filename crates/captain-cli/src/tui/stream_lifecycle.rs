use super::screens::chat::{ChatState, Role};
use captain_runtime::agent_loop::AgentLoopResult;

pub(crate) fn prepare_stream_start(chat: &mut ChatState) {
    chat.is_streaming = true;
    chat.thinking = true;
    chat.streaming_chars = 0;
    chat.last_tokens = None;
    chat.last_cached_input_tokens = 0;
    chat.last_cache_creation_tokens = 0;
    chat.last_cost_usd = None;
    chat.status_msg = None;
}

pub(crate) fn apply_stream_result(chat: &mut ChatState, result: Result<AgentLoopResult, String>) {
    chat.finalize_stream();
    match result {
        Ok(result) => apply_success(chat, result),
        Err(error) => {
            chat.status_msg = Some(format!("Error: {error}"));
        }
    }
}

fn apply_success(chat: &mut ChatState, result: AgentLoopResult) {
    if !result.response.is_empty()
        && chat.messages.last().map(|message| message.text.as_str()) != Some(&result.response)
    {
        chat.push_message(Role::Agent, result.response);
    }

    if result.total_usage.input_tokens > 0 || result.total_usage.output_tokens > 0 {
        chat.last_tokens = Some((
            result.total_usage.input_tokens,
            result.total_usage.output_tokens,
        ));
        chat.last_cached_input_tokens = result.total_usage.cached_input_tokens;
        chat.last_cache_creation_tokens = result.total_usage.cache_creation_tokens;
        chat.record_usage(
            result.total_usage.input_tokens,
            result.total_usage.output_tokens,
            result.total_usage.cached_input_tokens,
            result.total_usage.cache_creation_tokens,
            result.cost_usd.unwrap_or(0.0),
        );
    }

    chat.last_cost_usd = result.cost_usd;
}

#[cfg(test)]
#[path = "stream_lifecycle/tests.rs"]
mod tests;

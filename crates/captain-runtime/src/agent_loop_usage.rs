use crate::llm_driver::CompletionResponse;
use captain_types::message::TokenUsage;

pub(crate) fn record_response_usage(
    total_usage: &mut TokenUsage,
    response: &CompletionResponse,
    provider_name: &str,
) {
    let usage = response.usage;
    add_usage(total_usage, usage);
    log_prompt_cache_telemetry(provider_name, usage);
}

fn add_usage(total_usage: &mut TokenUsage, usage: TokenUsage) {
    total_usage.input_tokens += usage.input_tokens;
    total_usage.output_tokens += usage.output_tokens;
    total_usage.cached_input_tokens += usage.cached_input_tokens;
    total_usage.cache_creation_tokens += usage.cache_creation_tokens;
}

fn log_prompt_cache_telemetry(provider_name: &str, usage: TokenUsage) {
    if usage.cached_input_tokens == 0 && usage.cache_creation_tokens == 0 {
        return;
    }

    tracing::info!(
        provider = provider_name,
        input_tokens = usage.input_tokens,
        cached_input_tokens = usage.cached_input_tokens,
        cache_creation_tokens = usage.cache_creation_tokens,
        cache_hit_ratio = usage.cache_hit_ratio(),
        "prompt cache telemetry"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::{ContentBlock, StopReason};

    fn response_with_usage(usage: TokenUsage) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "ok".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage,
        }
    }

    #[test]
    fn record_response_usage_accumulates_all_token_fields() {
        let mut total = TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            cached_input_tokens: 2,
            cache_creation_tokens: 1,
        };
        let response = response_with_usage(TokenUsage {
            input_tokens: 30,
            output_tokens: 7,
            cached_input_tokens: 11,
            cache_creation_tokens: 13,
        });

        record_response_usage(&mut total, &response, "codex");

        assert_eq!(total.input_tokens, 40);
        assert_eq!(total.output_tokens, 12);
        assert_eq!(total.cached_input_tokens, 13);
        assert_eq!(total.cache_creation_tokens, 14);
    }

    #[test]
    fn record_response_usage_handles_no_cache_telemetry() {
        let mut total = TokenUsage::default();
        let response = response_with_usage(TokenUsage {
            input_tokens: 3,
            output_tokens: 4,
            ..Default::default()
        });

        record_response_usage(&mut total, &response, "anthropic");

        assert_eq!(total.input_tokens, 3);
        assert_eq!(total.output_tokens, 4);
        assert_eq!(total.cached_input_tokens, 0);
        assert_eq!(total.cache_creation_tokens, 0);
    }
}

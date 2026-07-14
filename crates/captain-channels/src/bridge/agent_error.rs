//! Operator-safe error messages for channel agent failures.

pub(crate) fn sanitize_agent_error(raw: &str) -> String {
    let lower = raw.to_lowercase();

    if lower.contains("rate limit")
        || lower.contains("rate_limit")
        || lower.contains("429")
        || lower.contains("too many requests")
        || lower.contains("resource_exhausted")
    {
        return "Rate limit reached, please try again later.".to_string();
    }

    if lower.contains("authentication")
        || lower.contains("unauthorized")
        || lower.contains("invalid api key")
        || lower.contains("invalid x-goog-api-key")
        || lower.contains("incorrect api key")
        || lower.contains("permission denied")
        || lower.contains("billing")
        || lower.contains("quota exceeded")
    {
        return "Service temporarily unavailable.".to_string();
    }

    if lower.contains("context length")
        || lower.contains("token limit")
        || lower.contains("too many tokens")
        || lower.contains("maximum context")
        || lower.contains("max_tokens")
        || lower.contains("context window")
    {
        return "Message too long, try a shorter request.".to_string();
    }

    if lower.contains("overloaded")
        || lower.contains("503")
        || lower.contains("502")
        || lower.contains("server error")
        || lower.contains("internal error")
    {
        return "The AI service is temporarily overloaded, please try again shortly.".to_string();
    }

    if lower.contains("timeout") || lower.contains("timed out") || lower.contains("deadline") {
        return "Request timed out, please try again.".to_string();
    }

    if lower.contains("model not found") || lower.contains("model_not_found") {
        return "The requested model is currently unavailable.".to_string();
    }

    let cleaned = raw
        .strip_prefix("LLM driver error: ")
        .or_else(|| raw.strip_prefix("Agent error: "))
        .unwrap_or(raw);

    if let Some(first_sentence_end) = cleaned.find(". ") {
        let first = &cleaned[..=first_sentence_end];
        if first.len() < cleaned.len() / 2 {
            return format!("Agent error: {first}");
        }
    }

    if cleaned.contains('{') || cleaned.len() > 200 {
        return "Something went wrong processing your request. Please try again.".to_string();
    }

    format!("Agent error: {cleaned}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_agent_error_maps_operator_categories() {
        assert_eq!(
            sanitize_agent_error("HTTP 429 rate_limit"),
            "Rate limit reached, please try again later."
        );
        assert_eq!(
            sanitize_agent_error("invalid x-goog-api-key"),
            "Service temporarily unavailable."
        );
        assert_eq!(
            sanitize_agent_error("maximum context window exceeded"),
            "Message too long, try a shorter request."
        );
        assert_eq!(
            sanitize_agent_error("request timed out while waiting"),
            "Request timed out, please try again."
        );
    }

    #[test]
    fn sanitize_agent_error_keeps_short_first_sentence() {
        assert_eq!(
            sanitize_agent_error("LLM driver error: Backend failed. Full debug payload follows."),
            "Agent error: Backend failed."
        );
    }

    #[test]
    fn sanitize_agent_error_hides_structured_or_long_payloads() {
        assert_eq!(
            sanitize_agent_error(r#"LLM driver error: {"secret":"value"}"#),
            "Something went wrong processing your request. Please try again."
        );
        assert_eq!(
            sanitize_agent_error(&"x".repeat(201)),
            "Something went wrong processing your request. Please try again."
        );
    }
}

//! Shared lexical search helpers for tool and skill discovery.

pub(crate) fn query_tokens(query: &str) -> Vec<String> {
    query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

pub(crate) fn lexical_weighted_score(query_tokens: &[String], fields: &[(&str, u32)]) -> u32 {
    let lowered: Vec<(String, u32)> = fields
        .iter()
        .map(|(text, weight)| (text.to_lowercase(), *weight))
        .collect();
    let mut score = 0u32;
    for tok in query_tokens {
        if tok.is_empty() {
            continue;
        }
        for (text, weight) in &lowered {
            if text.contains(tok) {
                score += *weight;
            }
        }
    }
    score
}

pub(crate) fn snippet_for_tokens(body: &str, tokens: &[String], max: usize) -> String {
    let body_lc = body.to_lowercase();
    let first_pos = tokens
        .iter()
        .filter_map(|token| body_lc.find(token))
        .min()
        .unwrap_or(0);
    let mut start = first_pos.saturating_sub(80);
    while start > 0 && !body.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (first_pos + max).min(body.len());
    while end < body.len() && !body.is_char_boundary(end) {
        end += 1;
    }
    let mut snippet = String::new();
    if start > 0 {
        snippet.push_str("...");
    }
    snippet.push_str(&body[start..end]);
    if end < body.len() {
        snippet.push_str("...");
    }
    snippet
}

pub(crate) fn result_score(value: &serde_json::Value) -> u32 {
    value["score"].as_u64().unwrap_or(0) as u32
}

pub(crate) fn result_name(value: &serde_json::Value) -> &str {
    value["name"].as_str().unwrap_or("")
}

pub(crate) fn result_source(value: &serde_json::Value) -> &str {
    value["source"].as_str().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_weighted_score_prefers_weighted_fields() {
        let tokens = query_tokens("browser click");
        let score = lexical_weighted_score(&tokens, &[("browser_click", 3), ("Click UI", 1)]);
        assert_eq!(score, 7);
    }

    #[test]
    fn snippet_for_tokens_clamps_to_char_boundaries() {
        let snippet = snippet_for_tokens("alpha beta gamma delta", &query_tokens("gamma"), 8);
        assert!(snippet.contains("gamma"));
    }
}

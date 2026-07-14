//! Shared helpers for LLM driver implementations.
//!
//! Currently hosts a single utility — [`parse_tool_input`] — that
//! protects every driver from a class of bug that crashed Captain in
//! production: a tool with no parameters (e.g. `system_time`) yields
//! an empty `arguments` / `input_json` string, the previous code did
//! `serde_json::from_str("").unwrap_or_default()` which returns
//! `Value::Null`, and Anthropic then refuses the next turn with
//! `messages.N.content.0.tool_use.input: Input should be a valid
//! dictionary` (HTTP 400). The agent loop classified the failure as
//! non-retryable Format and stopped the turn.

/// Parse a tool-call argument string into a `serde_json::Value`,
/// guaranteeing the result is a JSON object.
///
/// Behaviour:
/// - empty / whitespace-only input → `{}` (Anthropic expects a dict
///   even when the tool takes no args).
/// - valid JSON object → returned as-is.
/// - any other input (malformed JSON, valid JSON string / array /
///   number / null) → `{}`.
pub fn parse_tool_input(s: &str) -> serde_json::Value {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return serde_json::json!({});
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(v) if v.is_object() => v,
        _ => serde_json::json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_yields_empty_object() {
        let v = parse_tool_input("");
        assert!(v.is_object(), "empty input must produce an Object, got {v}");
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn whitespace_yields_empty_object() {
        assert!(parse_tool_input("   \n\t").is_object());
    }

    #[test]
    fn empty_braces_yield_empty_object() {
        let v = parse_tool_input("{}");
        assert!(v.is_object());
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn valid_object_is_preserved() {
        let v = parse_tool_input(r#"{"path":"/etc","limit":5}"#);
        assert_eq!(v["path"], "/etc");
        assert_eq!(v["limit"], 5);
    }

    #[test]
    fn malformed_json_falls_back_to_empty_object() {
        let v = parse_tool_input(r#"{"path":"#);
        assert!(v.is_object());
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn json_string_falls_back_to_empty_object() {
        // Must not propagate `Value::String` — Anthropic would reject it
        // exactly like Null. The whole point of the helper is to filter
        // non-object payloads out.
        let v = parse_tool_input(r#""just a string""#);
        assert!(v.is_object());
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn json_array_falls_back_to_empty_object() {
        let v = parse_tool_input("[1, 2, 3]");
        assert!(v.is_object());
    }

    #[test]
    fn json_null_falls_back_to_empty_object() {
        // Reproduces the exact crash recorded 2026-04-29 (#186): the
        // pre-fix code path produced `Value::Null` here, Anthropic
        // returned 400 on the next turn.
        let v = parse_tool_input("null");
        assert!(v.is_object(), "null must NOT propagate as Value::Null");
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn json_number_falls_back_to_empty_object() {
        assert!(parse_tool_input("42").is_object());
    }

    #[test]
    fn nested_object_is_preserved() {
        let v = parse_tool_input(r#"{"outer":{"inner":true}}"#);
        assert_eq!(v["outer"]["inner"], true);
    }
}

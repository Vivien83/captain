use super::*;

#[test]
fn sanitize_strips_legacy_placeholder() {
    // Older builds persisted "?/?" when the agent metadata fetch had not
    // returned yet. Reloading such a session must not keep the marker.
    assert_eq!(sanitize_model_label("?/?"), "");
}

#[test]
fn sanitize_preserves_real_label() {
    assert_eq!(
        sanitize_model_label("anthropic/claude-sonnet-4-6"),
        "anthropic/claude-sonnet-4-6"
    );
}

#[test]
fn sanitize_passes_empty_through() {
    assert_eq!(sanitize_model_label(""), "");
}

#[test]
fn sanitize_does_not_match_partial_unknown() {
    assert_eq!(sanitize_model_label("?/?-extra"), "?/?-extra");
    assert_eq!(sanitize_model_label("model/?"), "model/?");
}

#[test]
fn compose_model_label_returns_none_when_both_unknown() {
    assert!(compose_model_label("?", "?").is_none());
}

#[test]
fn compose_model_label_keeps_partial_information() {
    assert_eq!(
        compose_model_label("anthropic", "?"),
        Some("anthropic/?".to_string())
    );
    assert_eq!(
        compose_model_label("?", "claude-sonnet-4-6"),
        Some("?/claude-sonnet-4-6".to_string())
    );
}

#[test]
fn compose_model_label_passes_real_pair_through() {
    assert_eq!(
        compose_model_label("anthropic", "claude-sonnet-4-6"),
        Some("anthropic/claude-sonnet-4-6".to_string())
    );
}

#[test]
fn metadata_label_accepts_flat_agent_shape() {
    let body = serde_json::json!({
        "model_provider": "codex",
        "model_name": "gpt-5.5"
    });
    assert_eq!(
        model_label_from_agent_metadata(&body),
        Some("codex/gpt-5.5".to_string())
    );
}

#[test]
fn metadata_label_accepts_nested_agent_shape() {
    let body = serde_json::json!({
        "model": {
            "provider": "codex",
            "model": "gpt-5.5"
        }
    });
    assert_eq!(
        model_label_from_agent_metadata(&body),
        Some("codex/gpt-5.5".to_string())
    );
}

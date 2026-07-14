use super::*;

#[test]
fn test_detect_vision_provider_none() {
    let _ = detect_vision_provider();
}

#[test]
fn test_default_vision_models() {
    assert_eq!(default_vision_model("anthropic"), "claude-sonnet-4-6");
    assert_eq!(default_vision_model("openai"), "gpt-4o");
    assert_eq!(default_vision_model("gemini"), "gemini-2.5-flash");
    assert_eq!(default_vision_model("unknown"), "unknown");
}

#[test]
fn test_pick_vision_model_anthropic_small_batch() {
    assert_eq!(
        pick_vision_model("anthropic", Some(1)),
        "claude-haiku-4-5-20251001"
    );
    assert_eq!(
        pick_vision_model("anthropic", Some(5)),
        "claude-haiku-4-5-20251001"
    );
    assert_eq!(pick_vision_model("anthropic", Some(6)), "claude-sonnet-4-6");
    assert_eq!(
        pick_vision_model("anthropic", Some(30)),
        "claude-sonnet-4-6"
    );
    assert_eq!(pick_vision_model("anthropic", None), "claude-sonnet-4-6");
}

#[test]
fn test_pick_vision_model_other_providers_unchanged() {
    assert_eq!(pick_vision_model("openai", Some(1)), "gpt-4o");
    assert_eq!(pick_vision_model("openai", Some(30)), "gpt-4o");
    assert_eq!(pick_vision_model("openai", None), "gpt-4o");
    assert_eq!(pick_vision_model("gemini", Some(1)), "gemini-2.5-flash");
    assert_eq!(pick_vision_model("gemini", None), "gemini-2.5-flash");
    assert_eq!(pick_vision_model("unknown", Some(1)), "unknown");
}

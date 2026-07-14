use super::*;

fn basic_ctx() -> PromptContext {
    PromptContext {
        agent_name: "researcher".to_string(),
        agent_description: "Research agent".to_string(),
        base_system_prompt: "You are Researcher, a research agent.".to_string(),
        granted_tools: vec![
            "web_search".to_string(),
            "web_fetch".to_string(),
            "file_read".to_string(),
            "file_write".to_string(),
            "memory_save".to_string(),
            "memory_recall".to_string(),
        ],
        ..Default::default()
    }
}

/// v3.7f - Different model families have different known failure modes.
/// OpenAI/gpt/codex tend to narrate before acting; Gemini tends to refuse
/// ambiguously; Mimo is fine-tuned for tool calling and needs less
/// nudging. Conditional injection keeps the prompt dense with relevance.
#[test]
fn test_model_family_detection() {
    use crate::prompt_builder::ModelFamily;
    assert_eq!(detect_model_family("gpt-4"), ModelFamily::OpenAI);
    assert_eq!(detect_model_family("openai/gpt-5"), ModelFamily::OpenAI);
    assert_eq!(
        detect_model_family("openai/codex-mini"),
        ModelFamily::OpenAI
    );
    assert_eq!(
        detect_model_family("google/gemini-2.5-pro"),
        ModelFamily::Google
    );
    assert_eq!(detect_model_family("gemma-7b"), ModelFamily::Google);
    assert_eq!(
        detect_model_family("anthropic/claude-sonnet-4-6"),
        ModelFamily::Anthropic
    );
    assert_eq!(detect_model_family("xiaomi/mimo-v2-pro"), ModelFamily::Mimo);
    assert_eq!(
        detect_model_family("unknown/some-model"),
        ModelFamily::Other
    );
}

#[test]
fn test_openai_guidance_only_for_gpt() {
    let mut ctx = basic_ctx();
    ctx.model_family = Some(ModelFamily::OpenAI);
    let prompt = build_system_prompt(&ctx);
    assert!(
        prompt.contains("<openai_execution>"),
        "OpenAI guidance must appear for gpt/codex family"
    );

    let mut ctx2 = basic_ctx();
    ctx2.model_family = Some(ModelFamily::Anthropic);
    let prompt2 = build_system_prompt(&ctx2);
    assert!(
        !prompt2.contains("<openai_execution>"),
        "OpenAI guidance must NOT appear for Anthropic family"
    );
}

#[test]
fn test_model_family_none_falls_back_safely() {
    // No model_family = no family-specific guidance injected.
    let prompt = build_system_prompt(&basic_ctx());
    assert!(!prompt.contains("<openai_execution>"));
    assert!(!prompt.contains("<google_operational>"));
    assert!(!prompt.contains("<anthropic_tool>"));
    assert!(!prompt.contains("<mimo_tool>"));
}

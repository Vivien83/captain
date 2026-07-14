use super::*;

/// v3.7 gate 5 — full Captain-class prompt remains under 12k tokens
/// (approx 48k chars) to avoid prompt bloat. Run with `cargo test -- --nocapture`
/// to see the number.
#[test]
fn size_under_budget() {
    let ctx = PromptContext {
        agent_name: "Captain".to_string(),
        base_system_prompt: "You are Captain.".to_string(),
        granted_tools: crate::core_tools::CORE_TOOLS
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        soul_md: Some("Concise technical expert. Tutoies.".into()),
        user_md: Some("Name: Alex, EU/Paris".into()),
        memory_md: Some("Legacy workspace memory should be ignored.".into()),
        channel_type: Some("telegram".into()),
        model_family: Some(detect_model_family("anthropic/claude-sonnet-4-6")),
        current_date: Some("2026-04-21".into()),
        ..Default::default()
    };
    let prompt = build_system_prompt(&ctx);
    let chars = prompt.chars().count();
    let approx_tokens = chars / 4;
    eprintln!("PROMPT_SIZE chars={chars} approx_tokens={approx_tokens}");
    assert!(
        !prompt.contains("Legacy workspace memory"),
        "retired MEMORY.md content must not be prompt-injected"
    );
    assert!(
        approx_tokens < 12_000,
        "prompt exceeds 12k-token budget: {approx_tokens} tokens ({chars} chars)"
    );
}

#[test]
fn codex_economy_prompt_keeps_contracts_but_removes_heavy_sections() {
    let ctx = PromptContext {
        agent_name: "captain".to_string(),
        agent_description: "Principal local agent".to_string(),
        base_system_prompt: "Very long legacy Captain prompt. ".repeat(400),
        granted_tools: crate::core_tools::CORE_TOOLS
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
        model_family: Some(ModelFamily::OpenAI),
        prompt_profile: PromptProfile::CodexEconomy,
        current_date: Some("2026-05-06".to_string()),
        recalled_memories: vec![(
            "pref".to_string(),
            "The user wants critical analysis and no quality regression.".to_string(),
        )],
        ..Default::default()
    };

    let compact = build_system_prompt(&ctx);

    assert!(compact.contains("## Context Capsule"));
    assert!(compact.contains("capability_search"));
    assert!(compact.contains("tool_search"));
    assert!(compact.contains("captain_docs"));
    assert!(compact.contains("memory_save"));
    assert!(compact.contains("config.toml"));
    assert!(compact.contains("Runtime changelog"));
    assert!(compact.contains("call captain_docs directly"));
    assert!(
        !compact.contains("## Consciousness System"),
        "Codex economy prompt should not carry the heavy consciousness explainer"
    );
    assert!(
        !compact.contains("## Decision Tables"),
        "Codex economy prompt should replace long decision tables with compact discovery rules"
    );
    assert!(
        compact.chars().count() < 10_000,
        "Codex economy prompt should stay below roughly 2.5k tokens in this fixture"
    );
}

#[test]
fn codex_economy_cacheable_prefix_excludes_turn_date() {
    let base = PromptContext {
        agent_name: "captain".to_string(),
        agent_description: "Principal local agent".to_string(),
        base_system_prompt: "You are Captain.".to_string(),
        granted_tools: vec!["memory_save".to_string(), "capability_search".to_string()],
        prompt_profile: PromptProfile::CodexEconomy,
        ..Default::default()
    };
    let mut first = base.clone();
    first.current_date = Some("2026-06-20".to_string());
    let mut second = base;
    second.current_date = Some("2026-06-21".to_string());

    let first = build_system_prompt_with_cache(&first);
    let second = build_system_prompt_with_cache(&second);
    let first_prefix = &first.system_prompt[..first.cacheable_prefix_bytes.unwrap()];
    let second_prefix = &second.system_prompt[..second.cacheable_prefix_bytes.unwrap()];

    assert_eq!(first_prefix, second_prefix);
    assert!(first.system_prompt.contains("2026-06-20"));
    assert!(second.system_prompt.contains("2026-06-21"));
}

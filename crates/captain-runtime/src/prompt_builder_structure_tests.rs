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

#[test]
fn test_full_prompt_has_all_sections() {
    let prompt = build_system_prompt(&basic_ctx());
    assert!(prompt.contains("You are Researcher"));
    assert!(prompt.contains("## Tool Call Behavior"));
    assert!(prompt.contains("## Your Tools"));
    assert!(prompt.contains("## Memory"));
    assert!(prompt.contains("## User Profile"));
    assert!(prompt.contains("## Safety"));
    assert!(prompt.contains("## Operational Guidelines"));
}

#[test]
fn prompt_injects_configured_language_contract() {
    let ctx = PromptContext {
        configured_language: Some("fr".to_string()),
        ..basic_ctx()
    };
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("## Language Contract"));
    assert!(prompt.contains("French / français"));
    assert!(prompt.contains("from the first turn"));
}

#[test]
fn prompt_injects_vps_local_shell_context() {
    let ctx = PromptContext {
        deployment_profile: Some("vps".to_string()),
        ..basic_ctx()
    };
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("## Deployment Context"));
    assert!(prompt.contains("local execution environment"));
    assert!(prompt.contains("prefer local `shell_exec` first"));
    assert!(prompt.contains("another host"));
}

#[test]
fn lean_direct_prompt_omits_heavy_agent_surface() {
    let mut ctx = basic_ctx();
    ctx.recalled_memories = vec![("old".to_string(), "memory payload".to_string())];
    ctx.canonical_context = Some("large dynamic context".to_string());
    ctx.skill_summary = "skill list".to_string();
    ctx.mcp_summary = "mcp list".to_string();

    let built = build_lean_direct_system_prompt(&ctx);

    assert!(built.system_prompt.contains("## Direct Response Mode"));
    assert!(built.system_prompt.contains("You are Researcher"));
    assert!(!built.system_prompt.contains("## Your Tools"));
    assert!(!built.system_prompt.contains("## Memory"));
    assert!(!built.system_prompt.contains("skill list"));
    assert!(!built.system_prompt.contains("large dynamic context"));
    assert!(built.system_prompt.len() < 2_000);
    assert_eq!(
        built.cacheable_prefix_bytes,
        Some(built.system_prompt.len())
    );
}

#[test]
fn every_prompt_profile_exposes_exact_live_model_identity() {
    let mut ctx = basic_ctx();
    ctx.active_provider = Some("codex".to_string());
    ctx.active_model = Some("gpt-5.6-sol".to_string());
    ctx.peer_agents = vec![(
        "researcher-hand".to_string(),
        "Running".to_string(),
        "gpt-5.5".to_string(),
    )];

    let full = build_system_prompt_with_cache(&ctx);
    assert!(full.system_prompt.contains("## Runtime Identity"));
    assert!(full
        .system_prompt
        .contains("Active agent provider: `codex`"));
    assert!(full
        .system_prompt
        .contains("Active agent model: `gpt-5.6-sol`"));
    assert!(full
        .system_prompt
        .contains("Do not infer your identity from peer agents"));
    let full_prefix = &full.system_prompt[..full.cacheable_prefix_bytes.unwrap()];
    assert!(!full_prefix.contains("gpt-5.6-sol"));

    ctx.prompt_profile = PromptProfile::CodexEconomy;
    let compact = build_system_prompt_with_cache(&ctx);
    assert!(compact
        .system_prompt
        .contains("Active agent model: `gpt-5.6-sol`"));
    let compact_prefix = &compact.system_prompt[..compact.cacheable_prefix_bytes.unwrap()];
    assert!(!compact_prefix.contains("gpt-5.6-sol"));

    let lean = build_lean_direct_system_prompt(&ctx);
    assert!(lean
        .system_prompt
        .contains("Active agent model: `gpt-5.6-sol`"));
    assert!(lean
        .system_prompt
        .contains("separate from the Captain binary version"));
}

#[test]
fn test_section_ordering() {
    let prompt = build_system_prompt(&basic_ctx());
    let tool_behavior_pos = prompt.find("## Tool Call Behavior").unwrap();
    let tools_pos = prompt.find("## Your Tools").unwrap();
    let memory_pos = prompt.find("## Memory").unwrap();
    let safety_pos = prompt.find("## Safety").unwrap();
    let guidelines_pos = prompt.find("## Operational Guidelines").unwrap();

    assert!(tool_behavior_pos < tools_pos);
    assert!(tools_pos < memory_pos);
    assert!(memory_pos < safety_pos);
    assert!(safety_pos < guidelines_pos);
}

#[test]
fn test_subagent_omits_sections() {
    let mut ctx = basic_ctx();
    ctx.is_subagent = true;
    let prompt = build_system_prompt(&ctx);

    assert!(!prompt.contains("## Tool Call Behavior"));
    assert!(!prompt.contains("## User Profile"));
    assert!(!prompt.contains("## Channel"));
    assert!(!prompt.contains("## Safety"));
    // Subagents still get tools and guidelines
    assert!(prompt.contains("## Your Tools"));
    assert!(prompt.contains("## Operational Guidelines"));
    assert!(prompt.contains("## Memory"));
}

#[test]
fn test_empty_tools_no_section() {
    let ctx = PromptContext {
        agent_name: "test".to_string(),
        ..Default::default()
    };
    let prompt = build_system_prompt(&ctx);
    assert!(!prompt.contains("## Your Tools"));
}

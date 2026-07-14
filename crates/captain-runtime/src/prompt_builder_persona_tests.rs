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
fn test_persona_section_with_soul() {
    let mut ctx = basic_ctx();
    ctx.soul_md = Some("You are a pirate. Arr!".to_string());
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("## Persona"));
    assert!(prompt.contains("pirate"));
}

#[test]
fn test_persona_soul_capped_at_1000() {
    let long_soul = "x".repeat(2000);
    let section = build_persona_section(None, Some(&long_soul), None, None, None);
    assert!(section.contains("..."));
    // The raw soul content in the section should be at most 1003 chars (1000 + "...")
    assert!(section.len() < 1200);
}

#[test]
fn test_user_name_known() {
    let mut ctx = basic_ctx();
    ctx.user_name = Some("Alice".to_string());
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("Alice"));
    assert!(!prompt.contains("don't know the user's name"));
}

#[test]
fn test_user_name_unknown() {
    let ctx = basic_ctx();
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("don't know the user's name"));
}

#[test]
fn test_empty_base_prompt_generates_default_identity() {
    let ctx = PromptContext {
        agent_name: "helper".to_string(),
        agent_description: "A helpful agent".to_string(),
        ..Default::default()
    };
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("You are helper"));
    assert!(prompt.contains("A helpful agent"));
}

#[test]
fn test_workspace_in_persona() {
    let mut ctx = basic_ctx();
    ctx.workspace_path = Some("/home/user/project".to_string());
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("## Workspace"));
    assert!(prompt.contains("/home/user/project"));
}

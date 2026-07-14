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
fn test_skills_section_omitted_when_empty() {
    let ctx = basic_ctx();
    let prompt = build_system_prompt(&ctx);
    assert!(!prompt.contains("## Skills"));
}

#[test]
fn test_skills_section_present() {
    let mut ctx = basic_ctx();
    ctx.skill_summary = "- web-search: Search the web\n- git-expert: Git commands".to_string();
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("## Skills"));
    assert!(prompt.contains("web-search"));
}

#[test]
fn test_api_workflows_are_skill_first() {
    let mut ctx = basic_ctx();
    ctx.skill_summary = "- api-tester: API testing\n- openapi-expert: OpenAPI".to_string();
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("External API"));
    assert!(prompt.contains("skill_search"));
    assert!(prompt.contains("skill_view"));
    assert!(prompt.contains("OpenAPI"));
    assert!(prompt.contains("required parameters"));
}

#[test]
fn test_compact_prompt_keeps_api_workflow_rule() {
    let mut ctx = basic_ctx();
    ctx.prompt_profile = PromptProfile::CodexEconomy;
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("External API"));
    assert!(prompt.contains("skill_search"));
    assert!(prompt.contains("CLI --help"));
}

#[test]
fn test_mcp_section_omitted_when_empty() {
    let ctx = basic_ctx();
    let prompt = build_system_prompt(&ctx);
    assert!(!prompt.contains("## Connected Tool Servers"));
}

#[test]
fn test_mcp_section_present() {
    let mut ctx = basic_ctx();
    ctx.mcp_summary = "- github: 5 tools (search, create_issue, ...)".to_string();
    let prompt = build_system_prompt(&ctx);
    assert!(prompt.contains("## Connected Tool Servers (MCP)"));
    assert!(prompt.contains("github"));
}

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

/// v3.7e — USER, GRAPH and SOUL headers show a `[X% — N/Max chars]`
/// budget indicator so the LLM can reason about its own context surface
/// and self-regulate (trim, replace, or split) before tool errors fire.
#[test]
fn test_budget_indicators_present() {
    let mut ctx = basic_ctx();
    ctx.soul_md = Some("You are a concise technical expert.".to_string());
    ctx.user_md = Some("Name: Alice, EU/Paris".to_string());
    ctx.memory_md = Some("Legacy workspace memory must stay ignored".to_string());
    ctx.graph_md = Some("entity:alice knows_about Rust".to_string());
    let prompt = build_system_prompt(&ctx);

    let budget_re = regex_lite_matches(&prompt);
    assert!(
        budget_re >= 3,
        "need at least 3 budget indicators (SOUL, USER, GRAPH), got {budget_re}"
    );
    assert!(
        !prompt.contains("Legacy workspace memory"),
        "retired MEMORY.md content must not be prompt-injected"
    );
}

// Minimal manual scanner — we don't depend on the `regex` crate for tests.
// Counts patterns matching: `[` digit+ `%` ... `/` digit+ ` chars]`.
fn regex_lite_matches(s: &str) -> usize {
    let mut count = 0;
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // find the next ']' within 40 chars
            let end = (i + 1..=i + 80.min(bytes.len() - i))
                .find(|&j| j < bytes.len() && bytes[j] == b']');
            if let Some(e) = end {
                let chunk = &s[i..=e];
                if chunk.contains('%') && chunk.contains('/') && chunk.contains("chars") {
                    count += 1;
                    i = e + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    count
}

/// v3.7c — Wrap critical rules in XML pseudo-tag containers. Named
/// scopes resist attention dilution across a 10k-token prompt: the LLM
/// can retrieve `<destructive_ops>` faster than free prose.
#[test]
fn test_xml_containers_present() {
    let prompt = build_system_prompt(&basic_ctx());
    // TOOL_CALL_BEHAVIOR gets at least 2 pseudo-XML scopes
    assert!(
        prompt.contains("<mandatory_tool_use>") && prompt.contains("</mandatory_tool_use>"),
        "TOOL_CALL_BEHAVIOR must scope <mandatory_tool_use>"
    );
    assert!(
        prompt.contains("<act_dont_ask>") && prompt.contains("</act_dont_ask>"),
        "TOOL_CALL_BEHAVIOR must scope <act_dont_ask>"
    );
    assert!(
        prompt.contains("<failure_recovery>") && prompt.contains("</failure_recovery>"),
        "TOOL_CALL_BEHAVIOR must scope <failure_recovery>"
    );
    // SAFETY_SECTION scopes destructive-ops explicitly
    assert!(
        prompt.contains("<destructive_ops>") && prompt.contains("</destructive_ops>"),
        "SAFETY_SECTION must scope <destructive_ops>"
    );
}

/// v3.7b — DECISION_TABLES maps common questions to the exact tool that
/// should answer them. Arrow notation closes hallucination lanes: without
/// this, the LLM improvises ("I'll estimate the time" instead of running
/// `date`). Non-subagent only — subagents inherit context from parent.
#[test]
fn test_decision_tables_present_non_subagent() {
    let prompt = build_system_prompt(&basic_ctx());
    assert!(
        prompt.contains("## Decision Tables"),
        "decision tables must be injected for non-subagent prompts"
    );
    let arrows = prompt.matches('→').count();
    assert!(
        arrows >= 8,
        "decision tables need at least 8 → arrows, got {arrows}"
    );
}

#[test]
fn test_decision_tables_skipped_for_subagent() {
    let mut ctx = basic_ctx();
    ctx.is_subagent = true;
    let prompt = build_system_prompt(&ctx);
    assert!(
        !prompt.contains("## Decision Tables"),
        "decision tables must be omitted for subagents"
    );
}

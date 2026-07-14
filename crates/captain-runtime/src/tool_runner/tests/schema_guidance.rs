use super::*;

#[test]
fn test_memory_save_description_has_concrete_example() {
    let tools = builtin_tool_definitions();
    let save = tools
        .iter()
        .find(|t| t.name == "memory_save")
        .expect("memory_save must exist");
    let desc = &save.description;
    assert!(
        desc.contains("EXEMPLE") || desc.contains("Exemple") || desc.contains("EXAMPLE"),
        "description must include a concrete example"
    );
    for key in ["subject", "predicate", "object", "category"] {
        assert!(
            desc.contains(key),
            "description must mention required param '{key}'"
        );
    }
}

/// scaffold_skill is the agent's lever for self-extensibility, but it is
/// a durable behaviour change. The description must keep the model on the
/// controlled-improvement rail: visible proposal first, explicit approval
/// before writing a global reusable capability.
#[test]
fn test_scaffold_skill_description_has_controlled_guidance() {
    let tools = builtin_tool_definitions();
    let scaffold = tools
        .iter()
        .find(|t| t.name == "scaffold_skill")
        .expect("scaffold_skill must exist");
    let desc = &scaffold.description;
    assert!(
        desc.contains("approbation") && desc.contains("self_improvement_review"),
        "scaffold_skill must require visible controlled improvement guidance; got: {desc}"
    );
    assert!(
        desc.contains("EXEMPLE") || desc.contains("Exemple"),
        "scaffold_skill must show a concrete example"
    );
}

#[test]
fn test_skill_approval_tools_block_agent_self_approval_in_descriptions() {
    let tools = builtin_tool_definitions();
    for name in ["skill_proposal_decide", "skill_refinement_decide"] {
        let def = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} must exist"));
        let desc = &def.description;
        assert!(
            desc.contains("outil") && desc.contains("API/canal") && desc.contains("approve=false"),
            "{name} must steer tool calls away from positive self-approval; got: {desc}"
        );
    }
}

/// Proactive guidance: critical remote/credential tools must
/// tell the LLM *when* to reach for them spontaneously, not just *what*
/// they do. Without this, Sonnet defaults to ask_user instead of resolving
/// a familiar server name or fetching an API key from the vault.
#[test]
fn test_critical_tools_have_proactive_guidance() {
    let tools = builtin_tool_definitions();
    for name in ["ssh_exec", "secret_read"] {
        let def = tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} must exist in builtin tools"));
        let desc = &def.description;
        assert!(
            desc.contains("SPONTANÉMENT") || desc.contains("spontanément"),
            "{name} description must contain proactive guidance ('spontanément'); got: {desc}"
        );
    }
}

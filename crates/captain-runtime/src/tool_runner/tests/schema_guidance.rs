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
/// controlled-improvement rail: manual scaffolding only on explicit request;
/// observed workflows must stay in the durable V2 lifecycle.
#[test]
fn test_scaffold_skill_description_has_controlled_guidance() {
    let tools = builtin_tool_definitions();
    let scaffold = tools
        .iter()
        .find(|t| t.name == "scaffold_skill")
        .expect("scaffold_skill must exist");
    let desc = &scaffold.description;
    assert!(
        desc.contains("workflow_learning_list") && desc.contains("carte authentifiée"),
        "scaffold_skill must route observed workflows through Skill Learning V2; got: {desc}"
    );
    assert!(
        desc.contains("uniquement") && desc.contains("explicitement"),
        "scaffold_skill must require an explicit manual request"
    );
}

#[test]
fn test_skill_refinement_blocks_agent_self_approval_and_v2_has_no_decide_tool() {
    let tools = builtin_tool_definitions();
    let refinement = tools
        .iter()
        .find(|tool| tool.name == "skill_refinement_decide")
        .expect("skill_refinement_decide must exist");
    assert!(
        refinement.description.contains("outil")
            && refinement.description.contains("API/canal")
            && refinement.description.contains("approve=false")
    );
    assert!(tools
        .iter()
        .any(|tool| tool.name == "workflow_learning_list"));
    assert!(!tools
        .iter()
        .any(|tool| tool.name == "workflow_learning_decide"));
    assert!(!tools
        .iter()
        .any(|tool| tool.name == "skill_proposal_decide"));
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

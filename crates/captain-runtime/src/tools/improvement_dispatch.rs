//! Controlled-improvement and learning-review dispatch.

use std::sync::Arc;

use captain_skills::registry::SkillRegistry;

use crate::kernel_handle::KernelHandle;

use super::{
    tool_learning_review_decide, tool_learning_review_list, tool_self_improvement_review,
    tool_skill_proposal_decide, tool_skill_proposal_list, tool_skill_refinement_decide,
    tool_skill_refinement_list, tool_skill_refinement_propose, tool_skill_refinement_restore,
    tool_skill_refinement_update, tool_system_bug_list, tool_system_bug_report,
    tool_system_bug_update,
};

pub(crate) async fn dispatch_improvement_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    match tool_name {
        "self_improvement_review" => tool_self_improvement_review(input, kernel),
        "system_bug_report" => tool_system_bug_report(input, kernel),
        "system_bug_list" => tool_system_bug_list(input, kernel),
        "system_bug_update" => tool_system_bug_update(input, kernel),
        "learning_review_list" => tool_learning_review_list(input, kernel),
        "learning_review_decide" => {
            tool_learning_review_decide(input, kernel, caller_agent_id).await
        }
        "skill_proposal_list" => tool_skill_proposal_list(input, kernel),
        "skill_proposal_decide" => tool_skill_proposal_decide(input, kernel, caller_agent_id).await,
        "skill_refinement_propose" => tool_skill_refinement_propose(input, kernel, skill_registry),
        "skill_refinement_list" => tool_skill_refinement_list(input, kernel),
        "skill_refinement_decide" => tool_skill_refinement_decide(input, kernel, caller_agent_id),
        "skill_refinement_update" => tool_skill_refinement_update(input, kernel),
        "skill_refinement_restore" => tool_skill_refinement_restore(input, kernel, skill_registry),
        other => Err(format!("Unknown improvement tool: {other}")),
    }
}

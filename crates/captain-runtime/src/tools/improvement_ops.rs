use super::improvement_common::{
    block_agent_positive_skill_decision, public_safe_json_value, resolve_json_id_prefix,
    review_id_field,
};
use super::skill_refinement_ops::skill_refinement_snapshot;
use super::system_bug_ops::system_bug_snapshot;
use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::sync::Arc;

const REVIEW_LIST_LIMIT_MAX: usize = 50;

fn limit_review_items(items: serde_json::Value, limit: usize) -> serde_json::Value {
    match items {
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().take(limit).collect())
        }
        other => other,
    }
}

fn project_review_items(
    items: serde_json::Value,
    limit: usize,
    projector: fn(&serde_json::Value) -> serde_json::Value,
) -> serde_json::Value {
    match items {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .take(limit)
                .map(|item| projector(&item))
                .collect(),
        ),
        _ => serde_json::Value::Array(Vec::new()),
    }
}

fn insert_string_field(
    output: &mut serde_json::Map<String, serde_json::Value>,
    item: &serde_json::Value,
    field: &str,
) {
    if let Some(value) = item.get(field).and_then(|value| value.as_str()) {
        output.insert(
            field.to_string(),
            serde_json::Value::String(value.to_string()),
        );
    }
}

fn insert_json_field(
    output: &mut serde_json::Map<String, serde_json::Value>,
    item: &serde_json::Value,
    field: &str,
) {
    if let Some(value) = item.get(field) {
        output.insert(field.to_string(), value.clone());
    }
}

fn learning_review_item_output(item: &serde_json::Value) -> serde_json::Value {
    let mut output = serde_json::Map::new();
    for field in ["id", "wing", "room", "subject", "predicate", "object"] {
        insert_string_field(&mut output, item, field);
    }
    insert_json_field(&mut output, item, "confidence");
    serde_json::Value::Object(output)
}

fn skill_proposal_item_output(item: &serde_json::Value) -> serde_json::Value {
    let mut output = serde_json::Map::new();
    for field in [
        "id",
        "name",
        "description",
        "trigger_hint",
        "arg_schema_hint",
        "family",
    ] {
        insert_string_field(&mut output, item, field);
    }
    insert_json_field(&mut output, item, "tool_sequence");
    insert_json_field(&mut output, item, "confidence");
    serde_json::Value::Object(output)
}

fn queue_snapshot(
    result: Result<serde_json::Value, String>,
    tool_name: &str,
    limit: usize,
    projector: fn(&serde_json::Value) -> serde_json::Value,
) -> serde_json::Value {
    let snapshot = match result {
        Ok(items) => {
            let items =
                public_safe_json_value(project_review_items(items, limit, projector), tool_name);
            let count = items.as_array().map(|a| a.len()).unwrap_or(0);
            serde_json::json!({
                "status": "ok",
                "count": count,
                "items": items,
            })
        }
        Err(error) => serde_json::json!({
            "status": "unavailable",
            "count": 0,
            "error": error,
            "items": [],
        }),
    };
    public_safe_json_value(snapshot, tool_name)
}

fn learning_review_decision_output(id: &str, approve: bool) -> serde_json::Value {
    let mut output = serde_json::Map::new();
    output.insert(
        "status".to_string(),
        serde_json::Value::String(if approve { "committed" } else { "denied" }.to_string()),
    );
    output.insert("id".to_string(), serde_json::Value::String(id.to_string()));
    if approve {
        output.insert(
            "memory".to_string(),
            serde_json::json!({
                "available": true,
                "kind": "learning"
            }),
        );
    }
    public_safe_json_value(serde_json::Value::Object(output), "learning_review_decide")
}

fn skill_proposal_decision_output(id: &str, approve: bool) -> serde_json::Value {
    let mut output = serde_json::Map::new();
    output.insert(
        "status".to_string(),
        serde_json::Value::String(if approve { "approved" } else { "denied" }.to_string()),
    );
    output.insert("id".to_string(), serde_json::Value::String(id.to_string()));
    if approve {
        output.insert(
            "written".to_string(),
            serde_json::json!({
                "available": true,
                "kind": "generated_skill"
            }),
        );
    }
    public_safe_json_value(serde_json::Value::Object(output), "skill_proposal_decide")
}

fn resolve_skill_proposal_decision_id(
    kh: &Arc<dyn KernelHandle>,
    id_or_prefix: &str,
) -> Result<String, String> {
    let proposals = limit_review_items(
        kh.skill_proposal_list(REVIEW_LIST_LIMIT_MAX)?,
        REVIEW_LIST_LIMIT_MAX,
    );
    match resolve_json_id_prefix(&proposals, id_or_prefix, "Skill proposal") {
        Ok(id) => Ok(id),
        Err(error) if id_or_prefix.len() >= 32 && error == "Skill proposal id not found" => {
            Ok(id_or_prefix.to_string())
        }
        Err(error) => Err(error),
    }
}

fn review_list_limit(input: &serde_json::Value, default: usize) -> usize {
    input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, REVIEW_LIST_LIMIT_MAX as u64) as usize)
        .unwrap_or(default)
}

pub(crate) fn tool_self_improvement_review(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let limit = review_list_limit(input, 10);

    let learning_review = queue_snapshot(
        kh.learning_review_list(limit),
        "self_improvement_review",
        limit,
        learning_review_item_output,
    );
    let skill_proposals = queue_snapshot(
        kh.skill_proposal_list(limit),
        "self_improvement_review",
        limit,
        skill_proposal_item_output,
    );
    let system_bugs = system_bug_snapshot(kh, limit).unwrap_or_else(|error| {
        serde_json::json!({
            "status": "unavailable",
            "count": 0,
            "error": error,
            "open": [],
            "investigating": [],
        })
    });
    let skill_refinements = skill_refinement_snapshot(kh, limit).unwrap_or_else(|error| {
        serde_json::json!({
            "status": "unavailable",
            "count": 0,
            "error": error,
            "pending": [],
            "approved": [],
        })
    });

    let learning_count = learning_review["count"].as_u64().unwrap_or(0);
    let skill_count = skill_proposals["count"].as_u64().unwrap_or(0);
    let bug_count = system_bugs["count"].as_u64().unwrap_or(0);
    let refinement_count = skill_refinements["count"].as_u64().unwrap_or(0);
    let mut next_actions = Vec::new();
    if bug_count > 0 {
        next_actions.push(
            "Review pending system_bugs and decide whether to fix, report, or mark resolved with system_bug_update.",
        );
    }
    if refinement_count > 0 {
        next_actions.push(
            "Review skill_refinements: reject noisy proposals from tools, or approve only through an explicit human/API/channel decision; then patch minimally and mark applied after tests.",
        );
    }
    if learning_count > 0 {
        next_actions.push(
            "Review learning_review.items and call learning_review_decide(id, approve) only for generic, non-secret, durable learnings.",
        );
    }
    if skill_count > 0 {
        next_actions.push(
            "Review skill_proposals.items: tools may reject noisy generated skills, but approval must come from explicit human/API/channel review after external validation.",
        );
    }
    if next_actions.is_empty() {
        next_actions.push(
            "No pending reviewed items. If this turn taught a durable non-critical fact, use memory_save; if it revealed a repeatable workflow, surface a visible proposal first, then use scaffold_skill only after explicit approval or when the user asked for it.",
        );
    }

    let report = serde_json::json!({
        "status": "ok",
        "mode": "controlled_self_improvement",
        "visual_feedback_contract": {
            "auto_learning": "MemoryStored -> chat renders '🧠 memorise'",
            "approval_learning": "MemoryQueued -> chat renders '💭 apprentissage a valider'",
            "critical_changes": "skills/config/goals/global behaviour require explicit approval or a visible proposal before mutation",
            "visible_adaptation": "When a learning changes future behaviour, tell the user what changed and how Captain will act differently next time.",
            "preference_clarification": "If the preference or behavioural rule is ambiguous, ask one short question before storing or applying it."
        },
        "pending": {
            "learning_review": learning_review,
            "system_bugs": system_bugs,
            "skill_refinements": skill_refinements,
            "skill_proposals": skill_proposals
        },
        "decision_policy": [
            "Non-critical durable facts/preferences/lessons may be saved with memory_save; the user must see the chat feedback.",
            "Behaviour-changing learnings must include an explicit user-facing adaptation note: what changed, why, and the expected next-time behaviour.",
            "Skills, config changes, goals, routing, prompts and global behaviour are critical: inspect first, propose visibly, then require explicit external validation or a human/API/channel approval before mutation.",
            "Never store secrets, private infrastructure aliases, one-off paths, or raw credentials as learnings or generated skills.",
            "After any Security blocked error, switch to the vault/native integration/env_inject pattern instead of retrying the blocked sink."
        ],
        "next_actions": next_actions,
    });

    let report = public_safe_json_value(report, "self_improvement_review");
    Ok(serde_json::to_string_pretty(&report).unwrap_or_else(|_| report.to_string()))
}

pub(crate) fn tool_learning_review_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let limit = review_list_limit(input, 50);
    let items = public_safe_json_value(
        project_review_items(
            kh.learning_review_list(limit)?,
            limit,
            learning_review_item_output,
        ),
        "learning_review_list",
    );
    Ok(serde_json::to_string_pretty(&items).unwrap_or_else(|_| items.to_string()))
}

pub(crate) async fn tool_learning_review_decide(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = review_id_field(input, "id", "learning_review_decide")?;
    let approve = input["approve"]
        .as_bool()
        .ok_or("Missing 'approve' parameter (bool)")?;
    if approve {
        block_agent_positive_skill_decision("learning_review_decide", caller_agent_id)?;
    }
    let decided_by = caller_agent_id;
    kh.learning_review_decide(&id, approve, decided_by).await?;
    let res = learning_review_decision_output(&id, approve);
    Ok(serde_json::to_string_pretty(&res).unwrap_or_else(|_| res.to_string()))
}

pub(crate) fn tool_skill_proposal_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let limit = review_list_limit(input, 50);
    let items = public_safe_json_value(
        project_review_items(
            kh.skill_proposal_list(limit)?,
            limit,
            skill_proposal_item_output,
        ),
        "skill_proposal_list",
    );
    Ok(serde_json::to_string_pretty(&items).unwrap_or_else(|_| items.to_string()))
}

pub(crate) async fn tool_skill_proposal_decide(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id_or_prefix = review_id_field(input, "id", "skill_proposal_decide")?;
    let id = resolve_skill_proposal_decision_id(kh, &id_or_prefix)?;
    let approve = input["approve"]
        .as_bool()
        .ok_or("Missing 'approve' parameter (bool)")?;
    if approve {
        block_agent_positive_skill_decision("skill_proposal_decide", caller_agent_id)?;
    }
    kh.skill_proposal_decide(&id, approve, caller_agent_id)
        .await?;
    let res = skill_proposal_decision_output(&id, approve);
    Ok(serde_json::to_string_pretty(&res).unwrap_or_else(|_| res.to_string()))
}

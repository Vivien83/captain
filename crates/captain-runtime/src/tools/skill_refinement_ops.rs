use super::improvement_common::{
    block_agent_positive_skill_decision, enum_field, public_safe_text_field, push_note,
    resolve_registry_index, review_id_field,
};
use super::skill_refinement_output::{refinement_for_output, refinements_for_output};
use super::skill_refinement_snapshots::{
    copy_dir_recursive, create_skill_directory_snapshot, recorded_snapshot_path,
    trusted_skill_snapshot_path,
};
use crate::kernel_handle::KernelHandle;
use crate::tools::{current_origin_channel, require_kernel};
use captain_skills::registry::SkillRegistry;
use std::path::Path;
use std::sync::Arc;

pub(crate) const SKILL_REFINEMENTS_KEY: &str = "__captain_skill_refinement_registry";
const SKILL_REFINEMENT_RISKS: &[&str] = &["low", "medium", "high"];
const SKILL_REFINEMENT_STATUSES: &[&str] =
    &["pending", "approved", "denied", "applied", "restored"];

fn snapshot_skill_for_refinement(
    skill_registry: Option<&SkillRegistry>,
    skill: &str,
    refinement_id: &str,
) -> Result<serde_json::Value, String> {
    let registry = skill_registry.ok_or("skill registry unavailable")?;
    let installed = registry
        .get(skill)
        .ok_or_else(|| format!("skill '{skill}' not found in registry"))?;
    if installed.path == Path::new("<bundled>") {
        return Err("bundled skills are not file-backed".to_string());
    }
    create_skill_directory_snapshot(
        &installed.path,
        skill,
        refinement_id,
        "before-refinement-proposal",
    )
}

fn load_skill_refinements(kh: &Arc<dyn KernelHandle>) -> Result<Vec<serde_json::Value>, String> {
    match kh.memory_recall(SKILL_REFINEMENTS_KEY)? {
        Some(serde_json::Value::Array(items)) => Ok(items),
        Some(_) => Err("Skill refinement registry is corrupted: expected JSON array".to_string()),
        None => Ok(Vec::new()),
    }
}

fn store_skill_refinements(
    kh: &Arc<dyn KernelHandle>,
    items: Vec<serde_json::Value>,
) -> Result<(), String> {
    kh.memory_store(SKILL_REFINEMENTS_KEY, serde_json::Value::Array(items))
}

fn filter_skill_refinements(
    items: &[serde_json::Value],
    skill: Option<&str>,
    status: Option<&str>,
    risk: Option<&str>,
    limit: usize,
) -> Vec<serde_json::Value> {
    items
        .iter()
        .rev()
        .filter(|item| {
            skill.is_none_or(|s| item["skill"].as_str() == Some(s))
                && status.is_none_or(|s| item["status"].as_str() == Some(s))
                && risk.is_none_or(|r| item["risk"].as_str() == Some(r))
        })
        .take(limit)
        .cloned()
        .collect()
}

pub(crate) fn skill_refinement_snapshot(
    kh: &Arc<dyn KernelHandle>,
    limit: usize,
) -> Result<serde_json::Value, String> {
    let items = load_skill_refinements(kh)?;
    let pending = refinements_for_output(filter_skill_refinements(
        &items,
        None,
        Some("pending"),
        None,
        limit,
    ));
    let approved = refinements_for_output(filter_skill_refinements(
        &items,
        None,
        Some("approved"),
        None,
        limit,
    ));
    Ok(serde_json::json!({
        "status": "ok",
        "count": pending.len() + approved.len(),
        "pending": pending,
        "approved": approved,
    }))
}

pub(crate) fn tool_skill_refinement_propose(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let skill = public_safe_text_field(input, "skill", true, 120, "skill_refinement_propose")?
        .ok_or("Missing 'skill'")?;
    let finding = public_safe_text_field(input, "finding", true, 2000, "skill_refinement_propose")?
        .ok_or("Missing 'finding'")?;
    let suggested_change = public_safe_text_field(
        input,
        "suggested_change",
        true,
        2000,
        "skill_refinement_propose",
    )?
    .ok_or("Missing 'suggested_change'")?;
    let evidence =
        public_safe_text_field(input, "evidence", false, 2000, "skill_refinement_propose")?;
    let current_version = public_safe_text_field(
        input,
        "current_version",
        false,
        40,
        "skill_refinement_propose",
    )?;
    let proposed_version = public_safe_text_field(
        input,
        "proposed_version",
        false,
        40,
        "skill_refinement_propose",
    )?;
    let source = public_safe_text_field(input, "source", false, 80, "skill_refinement_propose")?
        .unwrap_or_else(|| "skill_use_review".to_string());
    let risk = enum_field(input, "risk", SKILL_REFINEMENT_RISKS, false, Some("medium"))?
        .unwrap_or_else(|| "medium".to_string());

    let mut items = load_skill_refinements(kh)?;
    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let snapshot = snapshot_skill_for_refinement(skill_registry, &skill, &id);
    let origin_channel =
        public_safe_text_field(input, "channel", false, 80, "skill_refinement_propose")?
            .or_else(current_origin_channel);
    let mut item = serde_json::json!({
        "id": id,
        "skill": skill,
        "finding": finding,
        "suggested_change": suggested_change,
        "evidence": evidence,
        "current_version": current_version,
        "proposed_version": proposed_version,
        "risk": risk,
        "status": "pending",
        "source": source,
        "origin_channel": origin_channel,
        "created_at": now,
        "updated_at": now,
        "notes": [],
    });
    match snapshot {
        Ok(snapshot) => {
            item["snapshot"] = snapshot;
        }
        Err(error) => {
            item["snapshot_error"] = serde_json::Value::String(error);
        }
    }
    items.push(item.clone());
    store_skill_refinements(kh, items)?;
    kh.publish_skill_refinement_queued(
        item["id"].as_str().unwrap_or_default(),
        item["skill"].as_str().unwrap_or_default(),
        item["finding"].as_str().unwrap_or_default(),
        item["suggested_change"].as_str().unwrap_or_default(),
        item["risk"].as_str().unwrap_or_default(),
        item["source"].as_str().unwrap_or_default(),
        item["origin_channel"].as_str(),
    );
    let output_refinement = refinement_for_output(&item);
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "proposed",
        "refinement": output_refinement.clone(),
        "decision_contract": "Visible proposal only. Do not edit the skill until an explicit human/API/channel approval is recorded. Tool calls may reject but cannot approve. File-backed skills include a pre-improvement snapshot for rollback."
    }))
    .unwrap_or_else(|_| output_refinement.to_string()))
}

pub(crate) fn tool_skill_refinement_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let skill = public_safe_text_field(input, "skill", false, 120, "skill_refinement_list")?;
    let status = enum_field(input, "status", SKILL_REFINEMENT_STATUSES, false, None)?;
    let risk = enum_field(input, "risk", SKILL_REFINEMENT_RISKS, false, None)?;
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 50) as usize)
        .unwrap_or(20);
    let items = load_skill_refinements(kh)?;
    let filtered = refinements_for_output(filter_skill_refinements(
        &items,
        skill.as_deref(),
        status.as_deref(),
        risk.as_deref(),
        limit,
    ));
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "ok",
        "count": filtered.len(),
        "items": filtered,
    }))
    .unwrap_or_else(|_| "[]".to_string()))
}

pub(crate) fn tool_skill_refinement_decide(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = review_id_field(input, "id", "skill_refinement_decide")?;
    let approve = input["approve"]
        .as_bool()
        .ok_or("Missing 'approve' boolean")?;
    let note = public_safe_text_field(input, "note", false, 2000, "skill_refinement_decide")?;
    let mut items = load_skill_refinements(kh)?;
    let index = resolve_registry_index(&items, &id, "Skill refinement")?;
    if approve {
        block_agent_positive_skill_decision("skill_refinement_decide", caller_agent_id)?;
    }
    let now = chrono::Utc::now().to_rfc3339();
    let refinement = items
        .get_mut(index)
        .and_then(|v| v.as_object_mut())
        .ok_or("Skill refinement registry is corrupted: item is not an object")?;
    let status = if approve { "approved" } else { "denied" };
    refinement.insert(
        "status".to_string(),
        serde_json::Value::String(status.to_string()),
    );
    refinement.insert(
        "updated_at".to_string(),
        serde_json::Value::String(now.clone()),
    );
    refinement.insert(
        "decided_at".to_string(),
        serde_json::Value::String(now.clone()),
    );
    if let Some(note) = note {
        push_note(refinement, &now, note);
    }
    let updated = refinement_for_output(&serde_json::Value::Object(refinement.clone()));
    store_skill_refinements(kh, items)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": status,
        "refinement": updated,
        "next_action": if approve {
            "If the user approved this critical change, inspect the skill file, apply the smallest patch, test it, then mention what changed."
        } else {
            "Do not mutate the skill for this proposal."
        }
    }))
    .unwrap_or_else(|_| "{}".to_string()))
}

pub(crate) fn tool_skill_refinement_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = review_id_field(input, "id", "skill_refinement_update")?;
    let status = enum_field(input, "status", SKILL_REFINEMENT_STATUSES, false, None)?;
    let risk = enum_field(input, "risk", SKILL_REFINEMENT_RISKS, false, None)?;
    let note = public_safe_text_field(input, "note", false, 2000, "skill_refinement_update")?;
    let proposed_version = public_safe_text_field(
        input,
        "proposed_version",
        false,
        40,
        "skill_refinement_update",
    )?;
    if status.is_none() && risk.is_none() && note.is_none() && proposed_version.is_none() {
        return Err("skill_refinement_update requires at least one patch field".to_string());
    }

    let mut items = load_skill_refinements(kh)?;
    let index = resolve_registry_index(&items, &id, "Skill refinement")?;
    let now = chrono::Utc::now().to_rfc3339();
    let refinement = items
        .get_mut(index)
        .and_then(|v| v.as_object_mut())
        .ok_or("Skill refinement registry is corrupted: item is not an object")?;
    if let Some(status) = status {
        refinement.insert("status".to_string(), serde_json::Value::String(status));
    }
    if let Some(risk) = risk {
        refinement.insert("risk".to_string(), serde_json::Value::String(risk));
    }
    if let Some(proposed_version) = proposed_version {
        refinement.insert(
            "proposed_version".to_string(),
            serde_json::Value::String(proposed_version),
        );
    }
    if let Some(note) = note {
        push_note(refinement, &now, note);
    }
    refinement.insert(
        "updated_at".to_string(),
        serde_json::Value::String(now.clone()),
    );
    let updated = refinement_for_output(&serde_json::Value::Object(refinement.clone()));
    store_skill_refinements(kh, items)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "updated",
        "refinement": updated,
    }))
    .unwrap_or_else(|_| "{}".to_string()))
}

pub(crate) fn tool_skill_refinement_restore(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    skill_registry: Option<&SkillRegistry>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let registry = skill_registry.ok_or("skill_refinement_restore requires skill registry")?;
    let id_prefix = review_id_field(input, "id", "skill_refinement_restore")?;
    let note = public_safe_text_field(input, "note", false, 2000, "skill_refinement_restore")?;

    let mut items = load_skill_refinements(kh)?;
    let index = resolve_registry_index(&items, &id_prefix, "Skill refinement")?;
    let now = chrono::Utc::now().to_rfc3339();
    let item = items
        .get(index)
        .ok_or("Skill refinement registry is corrupted: missing item")?
        .clone();
    let id = item["id"].as_str().unwrap_or(&id_prefix).to_string();
    let skill = item["skill"]
        .as_str()
        .ok_or("Skill refinement registry is corrupted: missing skill")?;
    let installed = registry
        .get(skill)
        .ok_or("Skill is not available in registry for restore")?;
    if installed.path == Path::new("<bundled>") {
        return Err("Bundled skills cannot be restored from a file snapshot".to_string());
    }
    let skill_path = installed.path.clone();
    let snapshot_path = recorded_snapshot_path(&item["snapshot"])?;
    if !trusted_skill_snapshot_path(&snapshot_path) {
        return Err("Snapshot path is missing or outside Captain's snapshot root".to_string());
    }
    let pre_restore_snapshot =
        create_skill_directory_snapshot(&skill_path, skill, &id, "before-restore")?;

    let restore_result = (|| -> Result<(), String> {
        std::fs::remove_dir_all(&skill_path)
            .map_err(|e| format!("remove current skill directory: {e}"))?;
        copy_dir_recursive(&snapshot_path, &skill_path)
            .map_err(|e| format!("restore snapshot: {e}"))
    })();

    if let Err(error) = restore_result {
        let _ = std::fs::remove_dir_all(&skill_path);
        if let Ok(pre_path) = recorded_snapshot_path(&pre_restore_snapshot) {
            let _ = copy_dir_recursive(&pre_path, &skill_path);
        }
        return Err(format!(
            "restore failed and attempted rollback from pre-restore backup: {error}"
        ));
    }

    let refinement = items
        .get_mut(index)
        .and_then(|v| v.as_object_mut())
        .ok_or("Skill refinement registry is corrupted: item is not an object")?;
    refinement.insert(
        "status".to_string(),
        serde_json::Value::String("restored".to_string()),
    );
    refinement.insert(
        "updated_at".to_string(),
        serde_json::Value::String(now.clone()),
    );
    refinement.insert(
        "restored_at".to_string(),
        serde_json::Value::String(now.clone()),
    );
    refinement.insert("restore_backup".to_string(), pre_restore_snapshot);
    let note_text = note.unwrap_or_else(|| "Restored from pre-improvement snapshot".to_string());
    push_note(refinement, &now, note_text);
    let updated = refinement_for_output(&serde_json::Value::Object(refinement.clone()));
    store_skill_refinements(kh, items)?;
    Ok(serde_json::to_string_pretty(&serde_json::json!({
        "status": "restored",
        "refinement": updated,
    }))
    .unwrap_or_else(|_| "{}".to_string()))
}

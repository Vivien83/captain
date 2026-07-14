use super::improvement_common::{
    enum_field, public_safe_json_value, public_safe_text_field, push_note, resolve_registry_index,
    review_id_field,
};
use crate::kernel_handle::KernelHandle;
use crate::tools::require_kernel;
use std::sync::Arc;

pub(crate) const SYSTEM_BUGS_KEY: &str = "__captain_system_bug_registry";
const SYSTEM_BUG_CATEGORIES: &[&str] = &[
    "tool",
    "scheduler",
    "channel",
    "memory",
    "security",
    "performance",
    "mcp",
    "skill",
    "docs",
    "ui",
    "unknown",
];
const SYSTEM_BUG_SEVERITIES: &[&str] = &["low", "medium", "high", "critical"];
const SYSTEM_BUG_STATUSES: &[&str] = &[
    "open",
    "investigating",
    "fixed",
    "wont_fix",
    "duplicate",
    "reported",
];

fn load_system_bugs(kh: &Arc<dyn KernelHandle>) -> Result<Vec<serde_json::Value>, String> {
    match kh.memory_recall(SYSTEM_BUGS_KEY)? {
        Some(serde_json::Value::Array(items)) => Ok(items),
        Some(_) => Err("System bug registry is corrupted: expected JSON array".to_string()),
        None => Ok(Vec::new()),
    }
}

fn store_system_bugs(
    kh: &Arc<dyn KernelHandle>,
    items: Vec<serde_json::Value>,
) -> Result<(), String> {
    let items = items
        .into_iter()
        .map(|item| public_safe_json_value(item, "system_bug_registry"))
        .collect();
    kh.memory_store(SYSTEM_BUGS_KEY, serde_json::Value::Array(items))
}

fn filter_system_bugs(
    items: &[serde_json::Value],
    status: Option<&str>,
    category: Option<&str>,
    severity: Option<&str>,
    limit: usize,
) -> Vec<serde_json::Value> {
    items
        .iter()
        .rev()
        .filter(|item| {
            status.is_none_or(|s| item["status"].as_str() == Some(s))
                && category.is_none_or(|c| item["category"].as_str() == Some(c))
                && severity.is_none_or(|s| item["severity"].as_str() == Some(s))
        })
        .take(limit)
        .cloned()
        .collect()
}

pub(crate) fn system_bug_snapshot(
    kh: &Arc<dyn KernelHandle>,
    limit: usize,
) -> Result<serde_json::Value, String> {
    let items = load_system_bugs(kh)?;
    let open = filter_system_bugs(&items, Some("open"), None, None, limit);
    let investigating = filter_system_bugs(&items, Some("investigating"), None, None, limit);
    let snapshot = serde_json::json!({
        "status": "ok",
        "count": open.len() + investigating.len(),
        "open": open,
        "investigating": investigating,
    });
    Ok(public_safe_json_value(snapshot, "system_bug_snapshot"))
}

pub(crate) fn tool_system_bug_report(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let title = public_safe_text_field(input, "title", true, 160, "system_bug_report")?
        .ok_or("Missing 'title'")?;
    let description =
        public_safe_text_field(input, "description", true, 2000, "system_bug_report")?
            .ok_or("Missing 'description'")?;
    let category = enum_field(
        input,
        "category",
        SYSTEM_BUG_CATEGORIES,
        true,
        Some("unknown"),
    )?
    .unwrap_or_else(|| "unknown".to_string());
    let severity = enum_field(
        input,
        "severity",
        SYSTEM_BUG_SEVERITIES,
        true,
        Some("medium"),
    )?
    .unwrap_or_else(|| "medium".to_string());
    let evidence = public_safe_text_field(input, "evidence", false, 2000, "system_bug_report")?;
    let suggested_fix =
        public_safe_text_field(input, "suggested_fix", false, 2000, "system_bug_report")?;
    let source = public_safe_text_field(input, "source", false, 80, "system_bug_report")?
        .unwrap_or_else(|| "self_review".to_string());

    let mut items = load_system_bugs(kh)?;
    let now = chrono::Utc::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();
    let item = serde_json::json!({
        "id": id,
        "title": title,
        "description": description,
        "category": category,
        "severity": severity,
        "status": "open",
        "evidence": evidence,
        "suggested_fix": suggested_fix,
        "source": source,
        "created_at": now,
        "updated_at": now,
        "notes": [],
    });
    items.push(item.clone());
    store_system_bugs(kh, items)?;
    let output = public_safe_json_value(
        serde_json::json!({
            "status": "reported",
            "bug": item,
            "next_actions": [
                "Use system_bug_list before related future fixes to avoid rediscovering the same issue.",
                "If the fix is safe and user-approved, implement it; otherwise mark status=reported after external handoff."
            ]
        }),
        "system_bug_report",
    );
    Ok(serde_json::to_string_pretty(&output).unwrap_or_else(|_| item.to_string()))
}

pub(crate) fn tool_system_bug_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let status = enum_field(input, "status", SYSTEM_BUG_STATUSES, false, None)?;
    let category = enum_field(input, "category", SYSTEM_BUG_CATEGORIES, false, None)?;
    let severity = enum_field(input, "severity", SYSTEM_BUG_SEVERITIES, false, None)?;
    let limit = input
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v.clamp(1, 50) as usize)
        .unwrap_or(20);
    let items = load_system_bugs(kh)?;
    let filtered = filter_system_bugs(
        &items,
        status.as_deref(),
        category.as_deref(),
        severity.as_deref(),
        limit,
    );
    let output = public_safe_json_value(
        serde_json::json!({
            "status": "ok",
            "count": filtered.len(),
            "items": filtered,
        }),
        "system_bug_list",
    );
    Ok(serde_json::to_string_pretty(&output).unwrap_or_else(|_| "[]".to_string()))
}

pub(crate) fn tool_system_bug_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = review_id_field(input, "id", "system_bug_update")?;
    let status = enum_field(input, "status", SYSTEM_BUG_STATUSES, false, None)?;
    let category = enum_field(input, "category", SYSTEM_BUG_CATEGORIES, false, None)?;
    let severity = enum_field(input, "severity", SYSTEM_BUG_SEVERITIES, false, None)?;
    let note = public_safe_text_field(input, "note", false, 2000, "system_bug_update")?;
    let suggested_fix =
        public_safe_text_field(input, "suggested_fix", false, 2000, "system_bug_update")?;
    if status.is_none()
        && category.is_none()
        && severity.is_none()
        && note.is_none()
        && suggested_fix.is_none()
    {
        return Err("system_bug_update requires at least one patch field".to_string());
    }

    let mut items = load_system_bugs(kh)?;
    let index = resolve_registry_index(&items, &id, "System bug")?;
    let now = chrono::Utc::now().to_rfc3339();
    let bug = items
        .get_mut(index)
        .and_then(|v| v.as_object_mut())
        .ok_or("System bug registry is corrupted: item is not an object")?;
    if let Some(status) = status {
        bug.insert("status".to_string(), serde_json::Value::String(status));
    }
    if let Some(category) = category {
        bug.insert("category".to_string(), serde_json::Value::String(category));
    }
    if let Some(severity) = severity {
        bug.insert("severity".to_string(), serde_json::Value::String(severity));
    }
    if let Some(suggested_fix) = suggested_fix {
        bug.insert(
            "suggested_fix".to_string(),
            serde_json::Value::String(suggested_fix),
        );
    }
    if let Some(note) = note {
        push_note(bug, &now, note);
    }
    bug.insert(
        "updated_at".to_string(),
        serde_json::Value::String(now.clone()),
    );
    let updated = serde_json::Value::Object(bug.clone());
    store_system_bugs(kh, items)?;
    let output = public_safe_json_value(
        serde_json::json!({
            "status": "updated",
            "bug": updated,
        }),
        "system_bug_update",
    );
    Ok(serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string()))
}

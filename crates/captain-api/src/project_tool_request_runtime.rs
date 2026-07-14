use crate::project_tool_request_decision::ToolRequestDecision;
use crate::project_tool_request_view::safe_project_tool_request_tool;
use chrono::Utc;

const TIMELINE_LIMIT: usize = 120;

pub(crate) fn normalize_phase(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(crate) fn valid_phase(phase: &str) -> bool {
    matches!(
        phase,
        "observe" | "think" | "plan" | "build" | "execute" | "verify" | "learn"
    )
}

pub(crate) fn normalize_tools(tools: Vec<String>) -> Vec<String> {
    let mut tools: Vec<String> = tools
        .into_iter()
        .map(|tool| safe_project_tool_request_tool(&tool))
        .filter(|tool| !tool.is_empty())
        .collect();
    tools.sort();
    tools.dedup();
    tools
}

pub(crate) fn first_pending_tool_request_phase(runtime: &serde_json::Value) -> Option<String> {
    if let Some(results) = runtime["worker_results"].as_object() {
        for (phase, result) in results {
            if result
                .get("tool_request")
                .map(tool_request_is_pending)
                .unwrap_or(false)
            {
                return Some(phase.to_string());
            }
        }
    }
    runtime["workers"].as_array().and_then(|workers| {
        workers.iter().find_map(|worker| {
            let pending = worker
                .get("tool_request")
                .map(tool_request_is_pending)
                .unwrap_or(false);
            if pending {
                worker["phase"].as_str().map(str::to_string)
            } else {
                None
            }
        })
    })
}

pub(crate) fn pending_tool_request_tools(runtime: &serde_json::Value, phase: &str) -> Vec<String> {
    if let Some(tools) = runtime
        .pointer(&format!("/worker_results/{phase}/tool_request/tools"))
        .and_then(|value| value.as_array())
    {
        return normalize_tools(
            tools
                .iter()
                .filter_map(|tool| tool.as_str().map(str::to_string))
                .collect(),
        );
    }
    for worker in runtime["workers"].as_array().into_iter().flatten() {
        if worker["phase"].as_str() != Some(phase) {
            continue;
        }
        if let Some(tools) = worker
            .pointer("/tool_request/tools")
            .and_then(|value| value.as_array())
        {
            return normalize_tools(
                tools
                    .iter()
                    .filter_map(|tool| tool.as_str().map(str::to_string))
                    .collect(),
            );
        }
    }
    Vec::new()
}

fn tool_request_is_pending(request: &serde_json::Value) -> bool {
    request["status"]
        .as_str()
        .map(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "pending" | "pending_captain_decision" | "pending_operator" | "open"
            )
        })
        .unwrap_or(true)
}

pub(crate) fn apply_project_tool_request_decision(
    runtime: &mut serde_json::Value,
    phase: &str,
    decision: ToolRequestDecision,
    tools: &[String],
    reason: Option<&str>,
) -> Result<(), String> {
    let mut found = false;
    let now = Utc::now().to_rfc3339();
    update_result_tool_request(runtime, phase, decision, tools, reason, &now, &mut found);
    update_worker_tool_request(runtime, phase, decision, tools, reason, &now, &mut found);
    if !found {
        return Err(format!("no pending tool request found for phase {phase}"));
    }
    if decision == ToolRequestDecision::Approve {
        mark_runtime_ready_for_tool_resume(runtime, phase, tools, reason, &now);
    } else {
        clear_resume_pending_for_phase(runtime, phase);
        runtime["status"] = serde_json::json!("blocked");
        runtime["current_phase"] = serde_json::json!(phase);
    }
    append_decision_event(runtime, phase, decision, tools, reason, &now);
    Ok(())
}

fn update_result_tool_request(
    runtime: &mut serde_json::Value,
    phase: &str,
    decision: ToolRequestDecision,
    tools: &[String],
    reason: Option<&str>,
    now: &str,
    found: &mut bool,
) {
    let Some(result) = runtime
        .pointer_mut(&format!("/worker_results/{phase}"))
        .and_then(|value| value.as_object_mut())
    else {
        return;
    };
    let Some(request) = result.get_mut("tool_request") else {
        return;
    };
    if !tool_request_is_pending(request) {
        return;
    }
    *found = true;
    mark_request_decision(request, decision, tools, reason, now);
    if decision == ToolRequestDecision::Approve {
        result.insert("status".to_string(), serde_json::json!("ready"));
        result.insert("blocked".to_string(), serde_json::json!(false));
        result.insert(
            "resume_pending".to_string(),
            serde_json::json!({"reason": "tool_request_approved", "phase": phase}),
        );
    }
}

fn update_worker_tool_request(
    runtime: &mut serde_json::Value,
    phase: &str,
    decision: ToolRequestDecision,
    tools: &[String],
    reason: Option<&str>,
    now: &str,
    found: &mut bool,
) {
    for worker in runtime["workers"].as_array_mut().into_iter().flatten() {
        if worker["phase"].as_str() != Some(phase) {
            continue;
        }
        let Some(obj) = worker.as_object_mut() else {
            continue;
        };
        let pending = obj
            .get("tool_request")
            .map(tool_request_is_pending)
            .unwrap_or(false);
        if !pending {
            continue;
        }
        *found = true;
        if let Some(request) = obj.get_mut("tool_request") {
            mark_request_decision(request, decision, tools, reason, now);
        }
        if decision == ToolRequestDecision::Approve {
            obj.insert("status".to_string(), serde_json::json!("ready"));
            obj.insert("approved_tools".to_string(), serde_json::json!(tools));
            obj.insert(
                "resume_pending".to_string(),
                serde_json::json!({"reason": "tool_request_approved", "phase": phase}),
            );
            obj.remove("error");
        }
    }
}

fn mark_request_decision(
    request: &mut serde_json::Value,
    decision: ToolRequestDecision,
    tools: &[String],
    reason: Option<&str>,
    now: &str,
) {
    if let Some(obj) = request.as_object_mut() {
        obj.insert(
            "status".to_string(),
            serde_json::json!(decision.as_status()),
        );
        obj.insert("decided_at".to_string(), serde_json::json!(now));
        obj.insert("decided_by".to_string(), serde_json::json!("operator"));
        obj.insert("tools".to_string(), serde_json::json!(tools));
        if let Some(reason) = reason.map(str::trim).filter(|reason| !reason.is_empty()) {
            obj.insert("decision_reason".to_string(), serde_json::json!(reason));
        }
    }
}

fn mark_runtime_ready_for_tool_resume(
    runtime: &mut serde_json::Value,
    phase: &str,
    tools: &[String],
    reason: Option<&str>,
    now: &str,
) {
    runtime["status"] = serde_json::json!("ready");
    runtime["current_phase"] = serde_json::json!(phase);
    runtime["resume_pending"] = serde_json::json!({
        "reason": "tool_request_approved",
        "phase": phase,
        "tools": tools,
        "operator_reason": reason.unwrap_or(""),
        "marked_at": now,
    });
    runtime["control"] = serde_json::json!({"paused": false, "takeover": false});
}

fn clear_resume_pending_for_phase(runtime: &mut serde_json::Value, phase: &str) {
    let same_phase = runtime
        .pointer("/resume_pending/phase")
        .and_then(|value| value.as_str())
        == Some(phase);
    if same_phase {
        if let Some(obj) = runtime.as_object_mut() {
            obj.remove("resume_pending");
        }
    }
}

fn append_decision_event(
    runtime: &mut serde_json::Value,
    phase: &str,
    decision: ToolRequestDecision,
    tools: &[String],
    reason: Option<&str>,
    now: &str,
) {
    if !runtime["timeline"].is_array() {
        runtime["timeline"] = serde_json::json!([]);
    }
    let detail = reason
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| decision.default_detail());
    let event = serde_json::json!({
        "id": uuid::Uuid::new_v4().to_string(),
        "ts": now,
        "kind": format!("worker.tool_request.{}", decision.as_status()),
        "title": format!("Tool request {}", decision.as_status()),
        "detail": detail,
        "actor": "operator",
        "phase": phase,
        "status": decision.as_status(),
        "data": {
            "phase": phase,
            "decision": decision.as_str(),
            "tools": tools,
        },
    });
    if let Some(items) = runtime["timeline"].as_array_mut() {
        items.push(event);
        if items.len() > TIMELINE_LIMIT {
            let drain = items.len() - TIMELINE_LIMIT;
            items.drain(0..drain);
        }
    }
}

#[cfg(test)]
#[path = "project_tool_request_runtime_tests.rs"]
mod project_tool_request_runtime_tests;

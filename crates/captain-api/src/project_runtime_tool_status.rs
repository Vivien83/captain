use serde_json::{json, Value};

pub(crate) fn pending_tool_request(runtime: &Value) -> Option<Value> {
    if let Some(results) = runtime
        .get("worker_results")
        .and_then(|value| value.as_object())
    {
        for (phase, result) in results {
            if let Some(request) = result.get("tool_request") {
                if tool_request_is_pending(request) {
                    return Some(normalized_tool_request(
                        phase,
                        request,
                        result,
                        "worker_results",
                    ));
                }
            }
        }
    }

    if let Some(workers) = runtime.get("workers").and_then(|value| value.as_array()) {
        for worker in workers {
            if let Some(request) = worker.get("tool_request") {
                if tool_request_is_pending(request) {
                    let phase = worker
                        .get("phase")
                        .and_then(|value| value.as_str())
                        .unwrap_or("unknown");
                    return Some(normalized_tool_request(phase, request, worker, "workers"));
                }
            }
        }
    }

    None
}

pub(crate) fn denied_tool_request(runtime: &Value, current_phase: &str) -> Option<Value> {
    tool_request_with_status(runtime, Some(current_phase), "denied")
        .or_else(|| tool_request_with_status(runtime, None, "denied"))
}

pub(crate) fn prepare_denied_tool_request_retry(runtime: &mut Value, phase: &str) -> bool {
    if denied_tool_request(runtime, phase).is_none() {
        return false;
    }
    let mut prepared = false;
    if let Some(result) = runtime
        .pointer_mut(&format!("/worker_results/{phase}"))
        .and_then(|value| value.as_object_mut())
    {
        result.insert("status".to_string(), json!("ready"));
        result.insert("blocked".to_string(), json!(false));
        result.insert("retry_after_denied_tool_request".to_string(), json!(true));
        prepared = true;
    }
    if let Some(workers) = runtime
        .get_mut("workers")
        .and_then(|value| value.as_array_mut())
    {
        for worker in workers {
            if worker.get("phase").and_then(|value| value.as_str()) != Some(phase) {
                continue;
            }
            if let Some(obj) = worker.as_object_mut() {
                obj.insert("status".to_string(), json!("ready"));
                obj.insert("retry_after_denied_tool_request".to_string(), json!(true));
                obj.remove("error");
                prepared = true;
            }
        }
    }
    prepared
}

pub(crate) fn approved_tools_for_phase(runtime: &Value, phase: &str) -> Vec<String> {
    let mut tools = Vec::new();
    collect_approved_request_tools(
        runtime.pointer(&format!("/worker_results/{phase}/tool_request")),
        &mut tools,
    );
    if let Some(workers) = runtime.get("workers").and_then(|value| value.as_array()) {
        for worker in workers {
            if worker.get("phase").and_then(|value| value.as_str()) != Some(phase) {
                continue;
            }
            collect_approved_request_tools(worker.get("tool_request"), &mut tools);
            collect_string_array(worker.get("approved_tools"), &mut tools);
        }
    }
    tools.sort();
    tools.dedup();
    tools
}

pub(crate) fn tool_decisions_context(runtime: &Value, phase: &str) -> String {
    let mut lines = Vec::new();
    let approved = approved_tools_for_phase(runtime, phase);
    if !approved.is_empty() {
        lines.push(format!(
            "- Approved extra tools for this phase: {}.",
            approved.join(", ")
        ));
    }
    if let Some(request) = denied_tool_request(runtime, phase) {
        let tools = tool_request_tools_label(Some(&request));
        let reason = request
            .get("decision_reason")
            .and_then(|value| value.as_str())
            .or_else(|| request.get("reason").and_then(|value| value.as_str()))
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("No operator reason recorded.");
        lines.push(format!(
            "- Denied tools for this phase: {tools}. Operator reason: {reason}. Do NOT request those tools again in this phase; choose another path or return the smallest manual next action."
        ));
    }
    if lines.is_empty() {
        "No previous tool approval or denial for this phase.".to_string()
    } else {
        lines.join("\n")
    }
}

pub(crate) fn repeated_denied_tool_request(runtime: &Value, phase: &str, request: Value) -> Value {
    let Some(previous) = denied_tool_request(runtime, phase) else {
        return request;
    };
    let requested_tools = tool_names(request.get("tools"));
    let denied_tools = tool_names(previous.get("tools"));
    let repeated_tools: Vec<String> = requested_tools
        .iter()
        .filter(|tool| {
            let needle = tool.to_ascii_lowercase();
            denied_tools
                .iter()
                .any(|denied| denied.eq_ignore_ascii_case(&needle))
        })
        .cloned()
        .collect();
    if repeated_tools.is_empty() {
        return request;
    }

    let previous_reason = previous
        .get("decision_reason")
        .and_then(|value| value.as_str())
        .or_else(|| previous.get("reason").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("No operator reason recorded.")
        .to_string();
    json!({
        "tools": request.get("tools").cloned().unwrap_or_else(|| json!([])),
        "reason": request
            .get("reason")
            .cloned()
            .unwrap_or_else(|| json!("Worker repeated a previously denied tool request.")),
        "status": "denied",
        "repeat_of_denied_tool_request": true,
        "repeated_denied_tools": repeated_tools,
        "decision_reason": format!(
            "Repeated request for tools already denied by the operator. Previous operator reason: {previous_reason}"
        ),
        "previous_decision_reason": previous_reason,
        "previous_denied_tool_request": previous,
    })
}

pub(crate) fn response_declares_blocked(response: &str) -> bool {
    response
        .lines()
        .take(12)
        .any(|line| line.trim().eq_ignore_ascii_case("status: blocked"))
}

pub(crate) fn extract_runtime_tool_request(summary: &str) -> Option<Value> {
    let mut requested = Vec::new();
    let mut reason = None;
    for line in summary.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower
            .strip_prefix("tool_request:")
            .or_else(|| lower.strip_prefix("tool request:"))
        {
            let original_rest = &trimmed[trimmed.len() - rest.len()..];
            requested.extend(
                original_rest
                    .split([',', ';'])
                    .map(|tool| tool.trim().trim_matches('`').trim())
                    .filter(|tool| !tool.is_empty())
                    .map(str::to_string),
            );
        } else if let Some(rest) = lower.strip_prefix("reason:") {
            let original_rest = trimmed[trimmed.len() - rest.len()..].trim();
            if !original_rest.is_empty() {
                reason = Some(original_rest.to_string());
            }
        }
    }
    requested.sort();
    requested.dedup();
    if requested.is_empty() {
        return None;
    }
    Some(json!({
        "tools": requested,
        "reason": reason.unwrap_or_else(|| "Worker requested an additional tool outside its current allowlist.".to_string()),
        "status": "pending_captain_decision",
    }))
}

pub(crate) fn tool_request_tools_label(request: Option<&Value>) -> String {
    let Some(tools) = request
        .and_then(|value| value.get("tools"))
        .and_then(|value| value.as_array())
    else {
        return "additional tools".to_string();
    };
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|tool| tool.as_str())
        .filter(|tool| !tool.trim().is_empty())
        .take(4)
        .collect();
    if names.is_empty() {
        "additional tools".to_string()
    } else {
        names.join(", ")
    }
}

fn tool_names(value: Option<&Value>) -> Vec<String> {
    let mut tools = Vec::new();
    collect_string_array(value, &mut tools);
    tools.sort_by_key(|tool| tool.to_ascii_lowercase());
    tools.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    tools
}

fn collect_approved_request_tools(request: Option<&Value>, tools: &mut Vec<String>) {
    let Some(request) = request else {
        return;
    };
    let approved = request
        .get("status")
        .and_then(|value| value.as_str())
        .map(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "approved" | "allow-once"
            )
        })
        .unwrap_or(false);
    if approved {
        collect_string_array(request.get("tools"), tools);
    }
}

fn collect_string_array(value: Option<&Value>, out: &mut Vec<String>) {
    if let Some(items) = value.and_then(|value| value.as_array()) {
        for item in items {
            if let Some(tool) = item.as_str().map(str::trim).filter(|tool| !tool.is_empty()) {
                out.push(tool.to_string());
            }
        }
    }
}

fn tool_request_with_status(
    runtime: &Value,
    phase_filter: Option<&str>,
    wanted: &str,
) -> Option<Value> {
    if let Some(results) = runtime
        .get("worker_results")
        .and_then(|value| value.as_object())
    {
        for (phase, result) in results {
            if phase_filter.map(|filter| filter != phase).unwrap_or(false) {
                continue;
            }
            if let Some(request) = result.get("tool_request") {
                if tool_request_has_status(request, wanted) {
                    return Some(normalized_tool_request(
                        phase,
                        request,
                        result,
                        "worker_results",
                    ));
                }
            }
        }
    }

    if let Some(workers) = runtime.get("workers").and_then(|value| value.as_array()) {
        for worker in workers {
            let phase = worker
                .get("phase")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            if phase_filter.map(|filter| filter != phase).unwrap_or(false) {
                continue;
            }
            if let Some(request) = worker.get("tool_request") {
                if tool_request_has_status(request, wanted) {
                    return Some(normalized_tool_request(phase, request, worker, "workers"));
                }
            }
        }
    }

    None
}

fn tool_request_is_pending(request: &Value) -> bool {
    request
        .get("status")
        .and_then(|value| value.as_str())
        .map(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "pending" | "pending_captain_decision" | "pending_operator" | "open"
            )
        })
        .unwrap_or(true)
}

fn tool_request_has_status(request: &Value, wanted: &str) -> bool {
    request
        .get("status")
        .and_then(|value| value.as_str())
        .map(|status| status.eq_ignore_ascii_case(wanted))
        .unwrap_or(false)
}

fn normalized_tool_request(phase: &str, request: &Value, carrier: &Value, source: &str) -> Value {
    json!({
        "phase": phase,
        "worker_id": carrier
            .get("id")
            .or_else(|| carrier.get("worker_id"))
            .cloned()
            .unwrap_or(Value::Null),
        "worker_role": carrier.get("role").cloned().unwrap_or(Value::Null),
        "agent_id": carrier.get("agent_id").cloned().unwrap_or(Value::Null),
        "tools": request.get("tools").cloned().unwrap_or_else(|| json!([])),
        "reason": request
            .get("reason")
            .cloned()
            .unwrap_or_else(|| json!("Worker requested an additional tool outside its current allowlist.")),
        "status": request
            .get("status")
            .cloned()
            .unwrap_or_else(|| json!("pending_captain_decision")),
        "decision_reason": request.get("decision_reason").cloned().unwrap_or(Value::Null),
        "decided_at": request.get("decided_at").cloned().unwrap_or(Value::Null),
        "decided_by": request.get("decided_by").cloned().unwrap_or(Value::Null),
        "repeat_of_denied_tool_request": request
            .get("repeat_of_denied_tool_request")
            .cloned()
            .unwrap_or(Value::Null),
        "repeated_denied_tools": request
            .get("repeated_denied_tools")
            .cloned()
            .unwrap_or(Value::Null),
        "previous_decision_reason": request
            .get("previous_decision_reason")
            .cloned()
            .unwrap_or(Value::Null),
        "source": source,
    })
}

#[cfg(test)]
#[path = "project_runtime_tool_status_tests.rs"]
mod project_runtime_tool_status_tests;

use crate::project_lifecycle::runtime_progress_for_phase;
use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_orchestrator::deactivate_runtime_orchestrator;
use crate::project_runtime_tool_status::{
    extract_runtime_tool_request, repeated_denied_tool_request, response_declares_blocked,
};
use crate::project_runtime_worker_summary::runtime_worker_summary;
use crate::project_runtime_workers::{runtime_worker_id, upsert_runtime_worker, RuntimeWorkerSpec};
use captain_memory::project;
use captain_runtime::agent_loop::AgentLoopResult;
use chrono::Utc;
use serde_json::Value;

pub(crate) struct RuntimeWorkerTurnOutcome {
    pub(crate) summary: String,
    pub(crate) blocked: bool,
    pub(crate) final_status: &'static str,
    tool_request: Option<Value>,
    repeated_denied_tool_request: bool,
}

pub(crate) fn runtime_worker_turn_outcome(
    spec: &RuntimeWorkerSpec,
    result: &AgentLoopResult,
    runtime_snapshot: &Value,
) -> RuntimeWorkerTurnOutcome {
    let summary =
        runtime_worker_summary(spec.role, spec.phase, &result.response, &result.tool_calls);
    let blocked =
        response_declares_blocked(&result.response) || response_declares_blocked(&summary);
    let tool_request = if blocked {
        extract_runtime_tool_request(&summary)
            .map(|request| repeated_denied_tool_request(runtime_snapshot, spec.phase, request))
    } else {
        None
    };
    let repeated_denied_tool_request = tool_request
        .as_ref()
        .and_then(|request| {
            request
                .get("repeat_of_denied_tool_request")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(false);
    RuntimeWorkerTurnOutcome {
        summary,
        blocked,
        final_status: if blocked { "blocked" } else { "done" },
        tool_request,
        repeated_denied_tool_request,
    }
}

pub(crate) fn mark_runtime_worker_turn_result(
    runtime: &mut Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    agent_id: &str,
    result: &AgentLoopResult,
    outcome: &RuntimeWorkerTurnOutcome,
) {
    let phase = spec.phase;
    mark_runtime_worker_record(runtime, project, spec, agent_id, result, outcome);
    write_worker_result(runtime, phase, agent_id, result, outcome);
    if let Some(request) = outcome.tool_request.clone() {
        write_worker_tool_request(runtime, project, spec, run_id, agent_id, request, outcome);
    }
    append_worker_result_event(runtime, project, spec, run_id, agent_id, result, outcome);
    if outcome.blocked {
        mark_runtime_blocked_by_worker(runtime, phase);
    }
}

fn mark_runtime_worker_record(
    runtime: &mut Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    agent_id: &str,
    result: &AgentLoopResult,
    outcome: &RuntimeWorkerTurnOutcome,
) {
    upsert_runtime_worker(runtime, project, spec, |worker| {
        worker.insert(
            "status".to_string(),
            serde_json::json!(outcome.final_status),
        );
        worker.insert("agent_id".to_string(), serde_json::json!(agent_id));
        worker.insert(
            "completed_at".to_string(),
            serde_json::json!(Utc::now().to_rfc3339()),
        );
        worker.insert(
            "summary".to_string(),
            serde_json::json!(outcome.summary.clone()),
        );
        worker.insert(
            "usage".to_string(),
            serde_json::to_value(result.total_usage).unwrap_or_default(),
        );
        worker.insert(
            "iterations".to_string(),
            serde_json::json!(result.iterations),
        );
        worker.insert(
            "tool_calls".to_string(),
            serde_json::json!(result.tool_calls.len()),
        );
        let tool_decisions = worker_tool_decisions(&result.tool_calls);
        if tool_decisions.is_empty() {
            worker.remove("tool_decisions");
        } else {
            worker.insert(
                "tool_decisions".to_string(),
                serde_json::json!(tool_decisions),
            );
        }
        if let Some(cost) = result.cost_usd {
            worker.insert("cost_usd".to_string(), serde_json::json!(cost));
        }
        if let Some(request) = outcome.tool_request.clone() {
            worker.insert("tool_request".to_string(), request);
        } else {
            worker.remove("tool_request");
        }
    });
}

fn write_worker_result(
    runtime: &mut Value,
    phase: &str,
    agent_id: &str,
    result: &AgentLoopResult,
    outcome: &RuntimeWorkerTurnOutcome,
) {
    if !runtime
        .get("worker_results")
        .map(|v| v.is_object())
        .unwrap_or(false)
    {
        runtime["worker_results"] = serde_json::json!({});
    }
    runtime["worker_results"][phase] = serde_json::json!({
        "agent_id": agent_id,
        "status": outcome.final_status,
        "summary": outcome.summary.clone(),
        "blocked": outcome.blocked,
        "tool_calls": result.tool_calls.len(),
        "tool_decisions": worker_tool_decisions(&result.tool_calls),
        "cost_usd": result.cost_usd,
    });
}

fn write_worker_tool_request(
    runtime: &mut Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    agent_id: &str,
    request: Value,
    outcome: &RuntimeWorkerTurnOutcome,
) {
    runtime["worker_results"][spec.phase]["tool_request"] = request.clone();
    append_tool_request_event(
        runtime,
        project,
        spec,
        run_id,
        agent_id,
        request,
        outcome.repeated_denied_tool_request,
    );
}

fn append_worker_result_event(
    runtime: &mut Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    agent_id: &str,
    result: &AgentLoopResult,
    outcome: &RuntimeWorkerTurnOutcome,
) {
    let phase = spec.phase;
    let event_detail = runtime
        .pointer(&format!("/worker_results/{phase}/summary"))
        .and_then(|v| v.as_str())
        .unwrap_or("Worker finished.")
        .to_string();
    append_runtime_event(
        runtime,
        if outcome.blocked {
            "worker.blocked"
        } else {
            "worker.completed"
        },
        &format!(
            "{} {}",
            spec.role,
            if outcome.blocked {
                "blocked"
            } else {
                "completed"
            }
        ),
        &event_detail,
        agent_id,
        phase,
        outcome.final_status,
        serde_json::json!({
            "run_id": run_id,
            "worker_id": runtime_worker_id(project, phase),
            "agent_id": agent_id,
            "iterations": result.iterations,
            "tool_decisions": worker_tool_decisions(&result.tool_calls),
            "cost_usd": result.cost_usd,
        }),
    );
}

fn worker_tool_decisions(tool_calls: &[captain_runtime::agent_loop::ToolCallRecord]) -> Vec<Value> {
    tool_calls
        .iter()
        .take(12)
        .map(|call| {
            serde_json::json!({
                "tool": call.tool_name,
                "reason": call.reason,
                "status": if call.is_error { "error" } else { "ok" },
                "duration_ms": call.duration_ms,
            })
        })
        .collect()
}

fn mark_runtime_blocked_by_worker(runtime: &mut Value, phase: &str) {
    runtime["status"] = serde_json::json!("blocked");
    runtime["progress"] = serde_json::json!(runtime_progress_for_phase(phase, "paused"));
    deactivate_runtime_orchestrator(runtime, "blocked");
}

fn append_tool_request_event(
    runtime: &mut Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    agent_id: &str,
    request: Value,
    repeated_denied: bool,
) {
    append_runtime_event(
        runtime,
        if repeated_denied {
            "worker.tool_request.denied_repeat"
        } else {
            "worker.tool_request"
        },
        &format!(
            "{} {}",
            spec.role,
            if repeated_denied {
                "repeated a denied tool request"
            } else {
                "requested a tool"
            }
        ),
        request
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("Worker needs a tool outside its current allowlist."),
        agent_id,
        spec.phase,
        if repeated_denied {
            "denied"
        } else {
            "pending_captain_decision"
        },
        serde_json::json!({
            "run_id": run_id,
            "worker_id": runtime_worker_id(project, spec.phase),
            "agent_id": agent_id,
            "tool_request": request,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_runtime_workers::RUNTIME_WORKER_SPECS;
    use captain_memory::project::ProjectStatus;
    use captain_runtime::agent_loop::{AgentLoopResult, ToolCallRecord};
    use captain_types::message::{ReplyDirectives, TokenUsage};

    fn project() -> project::Project {
        project::Project {
            id: "project-1".to_string(),
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Ship safely".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata: serde_json::json!({}),
        }
    }

    fn agent_result(response: &str) -> AgentLoopResult {
        AgentLoopResult {
            response: response.to_string(),
            total_usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
            iterations: 3,
            cost_usd: Some(0.25),
            silent: false,
            directives: ReplyDirectives::default(),
            tool_calls: vec![ToolCallRecord {
                tool_name: "shell_exec".to_string(),
                reason: "Run the build command.".to_string(),
                is_error: false,
                duration_ms: 7,
                input_summary: "cargo test".to_string(),
                output_summary: "ok".to_string(),
            }],
        }
    }

    #[test]
    fn mark_runtime_worker_turn_result_records_completion() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let worker_id = crate::project_runtime_workers::runtime_worker_id(&project, spec.phase);
        let result = agent_result("STATUS: complete\nSUMMARY: Build done.");
        let outcome = runtime_worker_turn_outcome(spec, &result, &serde_json::json!({}));
        let mut runtime = serde_json::json!({
            "workers": [{
                "id": worker_id,
                "phase": "build",
                "status": "blocked",
                "tool_request": {"status": "pending_captain_decision"}
            }],
            "timeline": []
        });

        mark_runtime_worker_turn_result(
            &mut runtime,
            &project,
            spec,
            "run-1",
            "agent-1",
            &result,
            &outcome,
        );

        let worker = runtime["workers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|worker| worker["phase"] == "build")
            .unwrap();
        assert_eq!(worker["status"], "done");
        assert_eq!(worker["agent_id"], "agent-1");
        assert_eq!(worker["summary"], serde_json::json!(outcome.summary));
        assert_eq!(worker["iterations"], 3);
        assert_eq!(worker["tool_calls"], 1);
        assert_eq!(worker["cost_usd"], 0.25);
        assert_eq!(worker["tool_decisions"][0]["tool"], "shell_exec");
        assert_eq!(
            worker["tool_decisions"][0]["reason"],
            "Run the build command."
        );
        assert_eq!(worker["tool_decisions"][0]["status"], "ok");
        assert_eq!(worker["tool_decisions"][0]["duration_ms"], 7);
        assert!(worker.get("tool_request").is_none());
        assert_eq!(runtime["worker_results"]["build"]["blocked"], false);
        assert_eq!(
            runtime["worker_results"]["build"]["tool_decisions"][0]["reason"],
            "Run the build command."
        );
        assert_eq!(runtime["worker_results"]["build"]["cost_usd"], 0.25);
        assert!(runtime["worker_results"]["build"]
            .get("tool_request")
            .is_none());
        assert_eq!(runtime["timeline"][0]["kind"], "worker.completed");
    }

    #[test]
    fn mark_runtime_worker_turn_result_records_blocker_and_tool_request() {
        let project = project();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let result = agent_result(
            "STATUS: blocked\nTOOL_REQUEST: shell_exec\nREASON: Need to run the build.",
        );
        let snapshot = serde_json::json!({
            "worker_results": {
                "build": {
                    "tool_request": {
                        "status": "denied",
                        "tools": ["shell_exec"],
                        "decision_reason": "Not needed yet."
                    }
                }
            },
            "orchestrator": { "active": true, "run_id": "run-2" }
        });
        let outcome = runtime_worker_turn_outcome(spec, &result, &snapshot);
        let mut runtime = serde_json::json!({
            "orchestrator": { "active": true, "run_id": "run-2" },
            "timeline": []
        });

        mark_runtime_worker_turn_result(
            &mut runtime,
            &project,
            spec,
            "run-2",
            "agent-2",
            &result,
            &outcome,
        );

        assert!(outcome.blocked);
        assert_eq!(runtime["status"], "blocked");
        assert_eq!(runtime["orchestrator"]["active"], false);
        assert_eq!(runtime["worker_results"]["build"]["status"], "blocked");
        assert_eq!(
            runtime["worker_results"]["build"]["tool_request"]["repeat_of_denied_tool_request"],
            true
        );
        assert_eq!(
            runtime["timeline"][0]["kind"],
            "worker.tool_request.denied_repeat"
        );
        assert_eq!(runtime["timeline"][0]["status"], "denied");
        assert_eq!(runtime["timeline"][1]["kind"], "worker.blocked");
    }
}

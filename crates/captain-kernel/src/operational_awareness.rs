//! Compact operational awareness for agent prompt injection.

use crate::goals::{Goal, GoalStatus};
use crate::graph_memory::GraphMemory;
use crate::supervisor::SupervisorHealth;
use captain_memory::project::Project;
use serde_json::Value;

const MAX_GOALS_IN_PROMPT: usize = 3;
const MAX_GOAL_LABEL_CHARS: usize = 80;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProjectAwarenessSignals {
    pub waiting_for_user: usize,
    pub tool_request_pending: usize,
    pub tool_request_denied: usize,
    pub repeated_tool_denials: usize,
    pub resume_ready: usize,
    pub active_or_stale: usize,
    pub blocked: usize,
    pub failed: usize,
}

impl ProjectAwarenessSignals {
    fn has_attention(self) -> bool {
        self.waiting_for_user > 0
            || self.tool_request_pending > 0
            || self.tool_request_denied > 0
            || self.resume_ready > 0
            || self.active_or_stale > 0
            || self.blocked > 0
            || self.failed > 0
    }
}

pub fn project_awareness_from_projects(projects: &[Project]) -> ProjectAwarenessSignals {
    let mut signals = ProjectAwarenessSignals::default();
    for project in projects {
        let Some(runtime) = project
            .metadata
            .get("runtime")
            .filter(|value| value.is_object())
        else {
            continue;
        };
        let status = runtime_string(runtime, "status", "ready").to_ascii_lowercase();
        if pending_question_count(runtime) > 0 {
            signals.waiting_for_user += 1;
        } else if runtime_has_pending_tool_request(runtime) {
            signals.tool_request_pending += 1;
        } else if runtime
            .get("resume_pending")
            .map(|v| !v.is_null())
            .unwrap_or(false)
        {
            signals.resume_ready += 1;
        } else if let Some(request) = runtime_denied_tool_request(runtime) {
            signals.tool_request_denied += 1;
            if request
                .get("repeat_of_denied_tool_request")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                signals.repeated_tool_denials += 1;
            }
        } else if runtime_declares_active(runtime) {
            signals.active_or_stale += 1;
        } else if status == "blocked" {
            signals.blocked += 1;
        } else if status == "failed" {
            signals.failed += 1;
        }
    }
    signals
}

pub fn build_operational_awareness_prompt(
    graph_memory: &GraphMemory,
    goals: &[Goal],
    supervisor: &SupervisorHealth,
    projects: ProjectAwarenessSignals,
) -> String {
    let mood = graph_memory.get_mood();
    let user_state = graph_memory.get_user_state();
    let (prediction_accuracy, _, prediction_total) = graph_memory.prediction_accuracy();
    let active_goals = goal_labels(goals, GoalStatus::Active);
    let escalated_goals = goal_labels(goals, GoalStatus::Escalated);

    format_awareness(AwarenessSnapshot {
        supervisor,
        active_goals,
        escalated_goals,
        queued_thoughts: graph_memory.queued_thought_count(),
        confidence: mood.confidence,
        error_rate: mood.error_rate,
        user_mode: user_state.mode,
        user_frustration: user_state.frustration,
        prediction_accuracy,
        prediction_total,
        projects,
    })
}

struct AwarenessSnapshot<'a> {
    supervisor: &'a SupervisorHealth,
    active_goals: Vec<String>,
    escalated_goals: Vec<String>,
    queued_thoughts: usize,
    confidence: f64,
    error_rate: f64,
    user_mode: String,
    user_frustration: f64,
    prediction_accuracy: f64,
    prediction_total: usize,
    projects: ProjectAwarenessSignals,
}

fn format_awareness(snapshot: AwarenessSnapshot<'_>) -> String {
    let mut signals = Vec::new();
    let mut state = "steady";

    if snapshot.supervisor.is_shutting_down {
        state = "critical";
        signals.push("supervisor shutdown requested".to_string());
    }
    if snapshot.supervisor.panic_count > 0 {
        state = raise_state(state, "warn");
        signals.push(format!(
            "supervisor panics since daemon start: {}",
            snapshot.supervisor.panic_count
        ));
    }
    if snapshot.supervisor.restart_count > 0 {
        state = raise_state(state, "watch");
        signals.push(format!(
            "supervisor restarts: {}",
            snapshot.supervisor.restart_count
        ));
    }
    if !snapshot.escalated_goals.is_empty() {
        state = raise_state(state, "warn");
        signals.push(format!(
            "escalated goals: {}",
            snapshot.escalated_goals.join("; ")
        ));
    }
    if !snapshot.active_goals.is_empty() {
        state = raise_state(state, "watch");
        signals.push(format!(
            "active goals: {}",
            snapshot.active_goals.join("; ")
        ));
    }
    if snapshot.error_rate >= 0.35 {
        state = raise_state(state, "warn");
        signals.push(format!("recent error rate: {:.2}", snapshot.error_rate));
    }
    if snapshot.user_frustration >= 0.6 {
        state = raise_state(state, "warn");
        signals.push(format!(
            "user mode: {}, frustration {:.2}",
            snapshot.user_mode, snapshot.user_frustration
        ));
    }
    if snapshot.queued_thoughts > 0 {
        state = raise_state(state, "watch");
        signals.push(format!(
            "queued graph thoughts: {}",
            snapshot.queued_thoughts
        ));
    }
    if snapshot.prediction_total >= 3 && snapshot.prediction_accuracy < 0.5 {
        state = raise_state(state, "watch");
        signals.push(format!(
            "prediction accuracy low: {:.2}",
            snapshot.prediction_accuracy
        ));
    }
    append_project_signals(snapshot.projects, &mut state, &mut signals);

    if state == "steady" && signals.is_empty() {
        return String::new();
    }

    let mut out = format!(
        "[OPERATIONAL AWARENESS]\nState: {state} (confidence {:.2}).\nSignals:\n",
        snapshot.confidence
    );
    for signal in signals {
        out.push_str("- ");
        out.push_str(&signal);
        out.push('\n');
    }
    out.push_str(
        "Use this as runtime telemetry only: prioritize blockers, avoid repeating failed loops, and mention it only when it changes the next action.",
    );
    out
}

fn append_project_signals(
    projects: ProjectAwarenessSignals,
    state: &mut &'static str,
    signals: &mut Vec<String>,
) {
    if !projects.has_attention() {
        return;
    }
    if projects.waiting_for_user > 0 {
        *state = raise_state(state, "warn");
        signals.push(format!(
            "project questions waiting for user: {}",
            projects.waiting_for_user
        ));
    }
    if projects.tool_request_pending > 0 {
        *state = raise_state(state, "warn");
        signals.push(format!(
            "project tool requests pending: {}",
            projects.tool_request_pending
        ));
    }
    if projects.repeated_tool_denials > 0 {
        *state = raise_state(state, "warn");
        signals.push(format!(
            "project repeated denied tools: {}",
            projects.repeated_tool_denials
        ));
    } else if projects.tool_request_denied > 0 {
        *state = raise_state(state, "warn");
        signals.push(format!(
            "project denied tool requests: {}",
            projects.tool_request_denied
        ));
    }
    if projects.blocked > 0 {
        *state = raise_state(state, "warn");
        signals.push(format!("project phases blocked: {}", projects.blocked));
    }
    if projects.failed > 0 {
        *state = raise_state(state, "warn");
        signals.push(format!("project phases failed: {}", projects.failed));
    }
    if projects.resume_ready > 0 {
        *state = raise_state(state, "watch");
        signals.push(format!(
            "project runs ready to resume: {}",
            projects.resume_ready
        ));
    }
    if projects.active_or_stale > 0 {
        *state = raise_state(state, "watch");
        signals.push(format!(
            "project runtimes marked active: {}",
            projects.active_or_stale
        ));
    }
}

fn goal_labels(goals: &[Goal], status: GoalStatus) -> Vec<String> {
    goals
        .iter()
        .filter(|goal| goal.status == status)
        .take(MAX_GOALS_IN_PROMPT)
        .map(|goal| {
            let mut label = if goal.name.trim().is_empty() {
                goal.id.clone()
            } else {
                format!("{} ({})", goal.name.trim(), goal.id)
            };
            truncate_chars(&mut label, MAX_GOAL_LABEL_CHARS);
            label
        })
        .collect()
}

fn raise_state(current: &'static str, candidate: &'static str) -> &'static str {
    if severity(candidate) > severity(current) {
        candidate
    } else {
        current
    }
}

fn severity(state: &str) -> u8 {
    match state {
        "critical" => 3,
        "warn" => 2,
        "watch" => 1,
        _ => 0,
    }
}

fn truncate_chars(value: &mut String, max_chars: usize) {
    if value.chars().count() <= max_chars {
        return;
    }
    *value = value.chars().take(max_chars.saturating_sub(3)).collect();
    value.push_str("...");
}

fn runtime_string(runtime: &Value, key: &str, fallback: &str) -> String {
    runtime
        .get(key)
        .and_then(|value| value.as_str())
        .unwrap_or(fallback)
        .to_string()
}

fn pending_question_count(runtime: &Value) -> usize {
    runtime
        .get("user_questions")
        .and_then(|value| value.as_array())
        .map(|questions| {
            questions
                .iter()
                .filter(|question| {
                    question
                        .get("status")
                        .and_then(|value| value.as_str())
                        .unwrap_or("pending")
                        .eq_ignore_ascii_case("pending")
                        && question
                            .get("ask_id")
                            .and_then(|value| value.as_str())
                            .map(|ask_id| !ask_id.trim().is_empty())
                            .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0)
}

fn runtime_has_pending_tool_request(runtime: &Value) -> bool {
    runtime_tool_request(runtime, |request| {
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
    })
    .is_some()
}

fn runtime_denied_tool_request(runtime: &Value) -> Option<&Value> {
    runtime_tool_request(runtime, |request| {
        request
            .get("status")
            .and_then(|value| value.as_str())
            .map(|status| status.eq_ignore_ascii_case("denied"))
            .unwrap_or(false)
    })
}

fn runtime_tool_request(
    runtime: &Value,
    mut predicate: impl FnMut(&Value) -> bool,
) -> Option<&Value> {
    if let Some(results) = runtime
        .get("worker_results")
        .and_then(|value| value.as_object())
    {
        for result in results.values() {
            if let Some(request) = result
                .get("tool_request")
                .filter(|request| predicate(request))
            {
                return Some(request);
            }
        }
    }
    if let Some(workers) = runtime.get("workers").and_then(|value| value.as_array()) {
        for worker in workers {
            if let Some(request) = worker
                .get("tool_request")
                .filter(|request| predicate(request))
            {
                return Some(request);
            }
        }
    }
    None
}

fn runtime_declares_active(runtime: &Value) -> bool {
    runtime
        .get("status")
        .and_then(|value| value.as_str())
        .map(|status| status.eq_ignore_ascii_case("running"))
        .unwrap_or(false)
        || runtime
            .pointer("/orchestrator/active")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
}

#[cfg(test)]
#[path = "operational_awareness_tests.rs"]
mod operational_awareness_tests;

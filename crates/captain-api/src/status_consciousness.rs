//! Operational consciousness summary for daemon status.

use captain_kernel::{graph_memory::TemporalPattern, CaptainKernel};

pub(crate) fn build_consciousness_status(
    kernel: &CaptainKernel,
    active_runs: usize,
    active_processes: usize,
    active_goals: usize,
    escalated_goals: usize,
    project_attention: &[serde_json::Value],
) -> serde_json::Value {
    let mood = kernel.graph_memory.get_mood();
    let user_state = kernel.graph_memory.get_user_state();
    let (prediction_accuracy, prediction_correct, prediction_total) =
        kernel.graph_memory.prediction_accuracy();
    let queued_thoughts = kernel.graph_memory.queued_thought_count();
    let patterns = kernel.graph_memory.detect_patterns(3);
    let supervisor = kernel.supervisor.health();
    let active_work = active_runs + active_processes;
    let projects = ProjectAttentionSignals::from_items(project_attention);
    let runtime = RuntimeAttentionSignals {
        shutting_down: supervisor.is_shutting_down,
        panic_count: supervisor.panic_count,
        restart_count: supervisor.restart_count,
        error_rate: mood.error_rate,
        user_frustration: user_state.frustration,
        queued_thoughts,
        active_work,
        active_goals,
        escalated_goals,
        prediction_accuracy,
        prediction_total,
        pattern_count: patterns.len(),
    };
    let attention = classify_operational_attention(AttentionInput::from_runtime(runtime, projects));

    serde_json::json!({
        "state": attention.state,
        "confidence": mood.confidence,
        "error_rate": mood.error_rate,
        "streak": mood.streak,
        "prediction_accuracy": prediction_accuracy,
        "prediction_correct": prediction_correct,
        "prediction_total": prediction_total,
        "queued_thoughts": queued_thoughts,
        "active_work": active_work,
        "active_goals": active_goals,
        "escalated_goals": escalated_goals,
        "projects": projects.to_json(),
        "user_mode": user_state.mode,
        "user_frustration": user_state.frustration,
        "supervisor": {
            "shutting_down": supervisor.is_shutting_down,
            "failure_count": supervisor.failure_count,
            "panic_count": supervisor.panic_count,
            "restart_count": supervisor.restart_count,
        },
        "patterns": pattern_status_json(&patterns),
        "signals": attention.signals,
        "operator_actions": attention.operator_actions,
    })
}

fn pattern_status_json(patterns: &[TemporalPattern]) -> Vec<serde_json::Value> {
    patterns
        .iter()
        .take(3)
        .map(|pattern| {
            serde_json::json!({
                "action": pattern.action,
                "hour": pattern.hour,
                "weekday": pattern.weekday,
                "occurrences": pattern.occurrences,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Default)]
struct ProjectAttentionSignals {
    total: usize,
    waiting_for_user: usize,
    tool_request_pending: usize,
    tool_request_denied: usize,
    repeated_tool_denials: usize,
    resume_ready: usize,
    stale_active: usize,
    blocked: usize,
    failed: usize,
}

impl ProjectAttentionSignals {
    fn from_items(items: &[serde_json::Value]) -> Self {
        let mut signals = Self {
            total: items.len(),
            ..Self::default()
        };
        for item in items {
            match item["state"].as_str().unwrap_or_default() {
                "waiting_for_user" => signals.waiting_for_user += 1,
                "tool_request_pending" => signals.tool_request_pending += 1,
                "tool_request_denied" => {
                    signals.tool_request_denied += 1;
                    if item
                        .pointer("/denied_tool_request/repeat_of_denied_tool_request")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    {
                        signals.repeated_tool_denials += 1;
                    }
                }
                "resume_ready" => signals.resume_ready += 1,
                "stale_active" => signals.stale_active += 1,
                "blocked" => signals.blocked += 1,
                "failed" => signals.failed += 1,
                _ => {}
            }
        }
        signals
    }

    fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "attention": self.total,
            "waiting_for_user": self.waiting_for_user,
            "tool_request_pending": self.tool_request_pending,
            "tool_request_denied": self.tool_request_denied,
            "repeated_tool_denials": self.repeated_tool_denials,
            "resume_ready": self.resume_ready,
            "stale_active": self.stale_active,
            "blocked": self.blocked,
            "failed": self.failed,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct RuntimeAttentionSignals {
    shutting_down: bool,
    panic_count: u64,
    restart_count: u64,
    error_rate: f64,
    user_frustration: f64,
    queued_thoughts: usize,
    active_work: usize,
    active_goals: usize,
    escalated_goals: usize,
    prediction_accuracy: f64,
    prediction_total: usize,
    pattern_count: usize,
}

struct AttentionInput {
    shutting_down: bool,
    panic_count: u64,
    restart_count: u64,
    error_rate: f64,
    user_frustration: f64,
    queued_thoughts: usize,
    active_work: usize,
    active_goals: usize,
    escalated_goals: usize,
    prediction_accuracy: f64,
    prediction_total: usize,
    pattern_count: usize,
    project_attention_count: usize,
    project_waiting_for_user: usize,
    project_tool_request_pending: usize,
    project_tool_request_denied: usize,
    project_repeated_tool_denials: usize,
    project_resume_ready: usize,
    project_stale_active: usize,
    project_blocked: usize,
    project_failed: usize,
}

impl AttentionInput {
    fn from_runtime(runtime: RuntimeAttentionSignals, projects: ProjectAttentionSignals) -> Self {
        Self {
            shutting_down: runtime.shutting_down,
            panic_count: runtime.panic_count,
            restart_count: runtime.restart_count,
            error_rate: runtime.error_rate,
            user_frustration: runtime.user_frustration,
            queued_thoughts: runtime.queued_thoughts,
            active_work: runtime.active_work,
            active_goals: runtime.active_goals,
            escalated_goals: runtime.escalated_goals,
            prediction_accuracy: runtime.prediction_accuracy,
            prediction_total: runtime.prediction_total,
            pattern_count: runtime.pattern_count,
            project_attention_count: projects.total,
            project_waiting_for_user: projects.waiting_for_user,
            project_tool_request_pending: projects.tool_request_pending,
            project_tool_request_denied: projects.tool_request_denied,
            project_repeated_tool_denials: projects.repeated_tool_denials,
            project_resume_ready: projects.resume_ready,
            project_stale_active: projects.stale_active,
            project_blocked: projects.blocked,
            project_failed: projects.failed,
        }
    }
}

struct AttentionSummary {
    state: &'static str,
    signals: Vec<String>,
    operator_actions: Vec<String>,
}

fn classify_operational_attention(input: AttentionInput) -> AttentionSummary {
    if input.shutting_down {
        return AttentionBuilder::critical(
            "supervisor_shutdown_requested",
            "Wait for shutdown or restart Captain if it stalls.",
        );
    }

    let mut builder = AttentionBuilder::default();
    add_supervisor_attention(&mut builder, &input);
    add_goal_and_runtime_attention(&mut builder, &input);
    add_project_attention(&mut builder, &input);
    add_prediction_attention(&mut builder, &input);
    builder.finish()
}

#[derive(Default)]
struct AttentionBuilder {
    signals: Vec<String>,
    operator_actions: Vec<String>,
    warn: bool,
    watch: bool,
}

impl AttentionBuilder {
    fn critical(signal: &str, action: &str) -> AttentionSummary {
        AttentionSummary {
            state: "critical",
            signals: vec![signal.to_string()],
            operator_actions: vec![action.to_string()],
        }
    }

    fn warn(&mut self, signal: String, action: Option<&str>) {
        self.warn = true;
        self.signals.push(signal);
        if let Some(action) = action {
            self.operator_actions.push(action.to_string());
        }
    }

    fn watch(&mut self, signal: String) {
        self.watch = true;
        self.signals.push(signal);
    }

    fn finish(self) -> AttentionSummary {
        AttentionSummary {
            state: if self.warn {
                "warn"
            } else if self.watch {
                "watch"
            } else {
                "steady"
            },
            signals: self.signals,
            operator_actions: self.operator_actions,
        }
    }
}

fn add_supervisor_attention(builder: &mut AttentionBuilder, input: &AttentionInput) {
    if input.panic_count > 0 {
        builder.warn(
            format!("supervisor_panics_since_start:{}", input.panic_count),
            Some("Inspect recent logs before starting new long work."),
        );
    }
    if input.restart_count > 0 {
        builder.watch(format!("supervisor_restarts:{}", input.restart_count));
    }
}

fn add_goal_and_runtime_attention(builder: &mut AttentionBuilder, input: &AttentionInput) {
    if input.escalated_goals > 0 {
        builder.warn(
            format!("goals_escalated:{}", input.escalated_goals),
            Some("Review escalated goals and unblock or pause them."),
        );
    }
    if input.error_rate >= 0.35 {
        builder.warn(
            format!("error_rate:{:.2}", input.error_rate),
            Some("Prefer smaller steps until recent failures drop."),
        );
    }
    if input.user_frustration >= 0.6 {
        builder.warn(
            format!("user_frustration:{:.2}", input.user_frustration),
            Some("Keep the next response short and concrete."),
        );
    }
    if input.queued_thoughts > 0 {
        builder.watch(format!("queued_thoughts:{}", input.queued_thoughts));
    }
    if input.active_work > 0 {
        builder.watch(format!("active_work:{}", input.active_work));
    }
    if input.active_goals > 0 {
        builder.watch(format!("active_goals:{}", input.active_goals));
    }
}

fn add_project_attention(builder: &mut AttentionBuilder, input: &AttentionInput) {
    add_project_blocking_attention(builder, input);
    if input.project_resume_ready > 0 {
        builder.watch(format!(
            "project_resume_ready:{}",
            input.project_resume_ready
        ));
        builder
            .operator_actions
            .push("Resume projects with stored answers or approved tools.".to_string());
    }
    if has_only_generic_project_attention(input) {
        builder.watch(format!(
            "project_attention:{}",
            input.project_attention_count
        ));
    }
}

fn add_project_blocking_attention(builder: &mut AttentionBuilder, input: &AttentionInput) {
    if input.project_waiting_for_user > 0 {
        builder.warn(
            format!(
                "project_waiting_for_user:{}",
                input.project_waiting_for_user
            ),
            Some("Answer pending project questions."),
        );
    }
    if input.project_tool_request_pending > 0 {
        builder.warn(
            format!(
                "project_tool_requests_pending:{}",
                input.project_tool_request_pending
            ),
            Some("Approve or deny pending project tool requests."),
        );
    }
    if input.project_repeated_tool_denials > 0 {
        builder.warn(
            format!(
                "project_repeated_tool_denials:{}",
                input.project_repeated_tool_denials
            ),
            Some("Review repeated denied project tools and choose another path."),
        );
    } else if input.project_tool_request_denied > 0 {
        builder.warn(
            format!(
                "project_tool_requests_denied:{}",
                input.project_tool_request_denied
            ),
            Some("Resume denied project phases with an alternate plan."),
        );
    }
    if input.project_stale_active > 0 {
        builder.warn(
            format!("project_stale_active:{}", input.project_stale_active),
            Some("Resume or pause stale project runtimes."),
        );
    }
    if input.project_blocked > 0 {
        builder.warn(
            format!("project_blocked:{}", input.project_blocked),
            Some("Review blocked project phases before adding new work."),
        );
    }
    if input.project_failed > 0 {
        builder.warn(
            format!("project_failed:{}", input.project_failed),
            Some("Inspect failed project runtimes and decide retry or rollback."),
        );
    }
}

fn has_only_generic_project_attention(input: &AttentionInput) -> bool {
    input.project_attention_count > 0
        && input.project_waiting_for_user == 0
        && input.project_tool_request_pending == 0
        && input.project_tool_request_denied == 0
        && input.project_resume_ready == 0
        && input.project_stale_active == 0
        && input.project_blocked == 0
        && input.project_failed == 0
}

fn add_prediction_attention(builder: &mut AttentionBuilder, input: &AttentionInput) {
    if input.prediction_total >= 3 && input.prediction_accuracy < 0.5 {
        builder.watch(format!(
            "prediction_accuracy_low:{:.2}",
            input.prediction_accuracy
        ));
    }
    if input.pattern_count > 0 {
        builder.watch(format!("temporal_patterns:{}", input.pattern_count));
    }
}

#[cfg(test)]
#[path = "status_consciousness_tests.rs"]
mod status_consciousness_tests;

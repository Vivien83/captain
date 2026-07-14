use crate::project_runtime_checkpoints::trim_runtime_text;
use crate::project_runtime_workers::{runtime_worker_id, RuntimeWorkerSpec};
use captain_memory::project;
use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::AgentId;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectWorkerStreamEvent {
    pub(crate) kind: &'static str,
    pub(crate) title: String,
    pub(crate) detail: String,
    pub(crate) status: &'static str,
    pub(crate) data: serde_json::Value,
}

struct ProjectWorkerStreamContext<'a> {
    run_id: &'a str,
    worker_id: String,
    agent_id: String,
}

impl<'a> ProjectWorkerStreamContext<'a> {
    fn new(project: &project::Project, run_id: &'a str, phase: &str, agent_id: &AgentId) -> Self {
        Self {
            run_id,
            worker_id: runtime_worker_id(project, phase),
            agent_id: agent_id.to_string(),
        }
    }
}

pub(crate) fn project_worker_stream_event(
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    phase: &str,
    agent_id: &AgentId,
    event: StreamEvent,
) -> Option<ProjectWorkerStreamEvent> {
    let ctx = ProjectWorkerStreamContext::new(project, run_id, phase, agent_id);
    match event {
        StreamEvent::PhaseChange {
            phase: loop_phase,
            detail,
        } => Some(worker_loop_phase_event(spec, &ctx, loop_phase, detail)),
        StreamEvent::ToolUseStart { id, name } => {
            Some(worker_tool_start_event(spec, &ctx, id, name))
        }
        StreamEvent::ToolUseEnd { id, name, input } => {
            Some(worker_tool_input_event(spec, &ctx, id, name, input))
        }
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error,
        } => Some(worker_tool_result_event(
            spec,
            &ctx,
            tool_use_id,
            name,
            result_preview,
            is_error,
        )),
        StreamEvent::ToolOutputDelta { stream, chunk, .. } if stream != "stdout" => {
            Some(worker_tool_output_event(spec, &ctx, stream, chunk))
        }
        StreamEvent::IntermediateMessage { content } => {
            Some(worker_note_event(spec, &ctx, content))
        }
        _ => None,
    }
}

fn worker_loop_phase_event(
    spec: &RuntimeWorkerSpec,
    ctx: &ProjectWorkerStreamContext<'_>,
    loop_phase: String,
    detail: Option<String>,
) -> ProjectWorkerStreamEvent {
    let detail_text = detail
        .as_deref()
        .map(|detail| format!("Worker loop phase: {loop_phase} ({detail})"))
        .unwrap_or_else(|| format!("Worker loop phase: {loop_phase}"));
    ProjectWorkerStreamEvent {
        kind: "worker.loop_phase",
        title: format!("{} loop: {}", spec.role, loop_phase),
        detail: detail_text,
        status: "running",
        data: serde_json::json!({
            "run_id": ctx.run_id,
            "worker_id": &ctx.worker_id,
            "agent_id": &ctx.agent_id,
            "loop_phase": loop_phase,
            "detail": detail,
        }),
    }
}

fn worker_tool_start_event(
    spec: &RuntimeWorkerSpec,
    ctx: &ProjectWorkerStreamContext<'_>,
    id: String,
    name: String,
) -> ProjectWorkerStreamEvent {
    ProjectWorkerStreamEvent {
        kind: "worker.tool_started",
        title: format!("{} used {}", spec.role, name),
        detail: "The project worker started a tool call.".to_string(),
        status: "running",
        data: serde_json::json!({
            "run_id": ctx.run_id,
            "worker_id": &ctx.worker_id,
            "agent_id": &ctx.agent_id,
            "tool_use_id": id,
            "tool": name,
        }),
    }
}

fn worker_tool_input_event(
    spec: &RuntimeWorkerSpec,
    ctx: &ProjectWorkerStreamContext<'_>,
    id: String,
    name: String,
    input: serde_json::Value,
) -> ProjectWorkerStreamEvent {
    ProjectWorkerStreamEvent {
        kind: "worker.tool_input",
        title: format!("{} prepared {}", spec.role, name),
        detail: trim_runtime_text(&input.to_string(), 420),
        status: "running",
        data: serde_json::json!({
            "run_id": ctx.run_id,
            "worker_id": &ctx.worker_id,
            "agent_id": &ctx.agent_id,
            "tool_use_id": id,
            "tool": name,
        }),
    }
}

fn worker_tool_result_event(
    spec: &RuntimeWorkerSpec,
    ctx: &ProjectWorkerStreamContext<'_>,
    tool_use_id: String,
    name: String,
    result_preview: String,
    is_error: bool,
) -> ProjectWorkerStreamEvent {
    ProjectWorkerStreamEvent {
        kind: "worker.tool_finished",
        title: format!("{} finished {}", spec.role, name),
        detail: trim_runtime_text(&result_preview, 700),
        status: if is_error { "error" } else { "running" },
        data: serde_json::json!({
            "run_id": ctx.run_id,
            "worker_id": &ctx.worker_id,
            "agent_id": &ctx.agent_id,
            "tool_use_id": tool_use_id,
            "tool": name,
            "is_error": is_error,
        }),
    }
}

fn worker_tool_output_event(
    spec: &RuntimeWorkerSpec,
    ctx: &ProjectWorkerStreamContext<'_>,
    stream: &str,
    chunk: String,
) -> ProjectWorkerStreamEvent {
    ProjectWorkerStreamEvent {
        kind: "worker.tool_output",
        title: format!("{} tool {}", spec.role, stream),
        detail: trim_runtime_text(&chunk, 700),
        status: "running",
        data: serde_json::json!({
            "run_id": ctx.run_id,
            "worker_id": &ctx.worker_id,
            "agent_id": &ctx.agent_id,
            "stream": stream,
        }),
    }
}

fn worker_note_event(
    spec: &RuntimeWorkerSpec,
    ctx: &ProjectWorkerStreamContext<'_>,
    content: String,
) -> ProjectWorkerStreamEvent {
    ProjectWorkerStreamEvent {
        kind: "worker.note",
        title: format!("{} update", spec.role),
        detail: trim_runtime_text(&content, 1100),
        status: "running",
        data: serde_json::json!({
            "run_id": ctx.run_id,
            "worker_id": &ctx.worker_id,
            "agent_id": &ctx.agent_id,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::project::ProjectStatus;
    use captain_types::agent::ToolProfile;

    fn project_fixture() -> project::Project {
        project::Project {
            id: "project-1".to_string(),
            slug: "alpha".to_string(),
            name: "Alpha".to_string(),
            goal: "Ship alpha".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            metadata: serde_json::json!({}),
            created_at: 1_779_660_000_000,
            updated_at: 1_779_660_000_000,
        }
    }

    fn worker_spec() -> RuntimeWorkerSpec {
        RuntimeWorkerSpec {
            phase: "build",
            role: "builder",
            task: "Build the slice.",
            mode: "gated",
            dependencies: &["plan"],
            profile: ToolProfile::Coding,
        }
    }

    fn agent_id() -> AgentId {
        "00000000-0000-0000-0000-000000000001".parse().unwrap()
    }

    #[test]
    fn worker_stream_phase_change_keeps_loop_detail_and_runtime_ids() {
        let event = project_worker_stream_event(
            &project_fixture(),
            &worker_spec(),
            "run-1",
            "build",
            &agent_id(),
            StreamEvent::PhaseChange {
                phase: "thinking".to_string(),
                detail: Some("reviewing files".to_string()),
            },
        )
        .unwrap();

        assert_eq!(event.kind, "worker.loop_phase");
        assert_eq!(event.title, "builder loop: thinking");
        assert_eq!(
            event.detail,
            "Worker loop phase: thinking (reviewing files)"
        );
        assert_eq!(event.status, "running");
        assert_eq!(event.data["run_id"], "run-1");
        assert_eq!(event.data["worker_id"], "alpha-build");
        assert_eq!(event.data["loop_phase"], "thinking");
    }

    #[test]
    fn worker_stream_tool_result_marks_errors_without_losing_tool_id() {
        let event = project_worker_stream_event(
            &project_fixture(),
            &worker_spec(),
            "run-1",
            "verify",
            &agent_id(),
            StreamEvent::ToolExecutionResult {
                tool_use_id: "tool-1".to_string(),
                name: "cargo test".to_string(),
                result_preview: "failed".to_string(),
                is_error: true,
            },
        )
        .unwrap();

        assert_eq!(event.kind, "worker.tool_finished");
        assert_eq!(event.status, "error");
        assert_eq!(event.title, "builder finished cargo test");
        assert_eq!(event.detail, "failed");
        assert_eq!(event.data["tool_use_id"], "tool-1");
        assert_eq!(event.data["tool"], "cargo test");
        assert_eq!(event.data["is_error"], true);
    }

    #[test]
    fn worker_stream_ignores_stdout_tool_deltas() {
        let event = project_worker_stream_event(
            &project_fixture(),
            &worker_spec(),
            "run-1",
            "execute",
            &agent_id(),
            StreamEvent::ToolOutputDelta {
                tool_use_id: "tool-1".to_string(),
                stream: "stdout",
                chunk: "line".to_string(),
            },
        );

        assert!(event.is_none());
    }

    #[test]
    fn worker_stream_keeps_stderr_tool_deltas_bounded() {
        let event = project_worker_stream_event(
            &project_fixture(),
            &worker_spec(),
            "run-1",
            "execute",
            &agent_id(),
            StreamEvent::ToolOutputDelta {
                tool_use_id: "tool-1".to_string(),
                stream: "stderr",
                chunk: "x".repeat(900),
            },
        )
        .unwrap();

        assert_eq!(event.kind, "worker.tool_output");
        assert_eq!(event.title, "builder tool stderr");
        assert_eq!(event.data["stream"], "stderr");
        assert!(event.detail.len() <= 703);
    }
}

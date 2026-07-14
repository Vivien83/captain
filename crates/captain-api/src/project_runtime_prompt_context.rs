use crate::project_runtime_asks::runtime_user_questions_context;
use crate::project_runtime_checkpoints::trim_runtime_text;
use crate::project_runtime_tool_status::tool_decisions_context;
use crate::project_runtime_worker_tools::runtime_worker_authorized_tools_for_runtime;
use crate::project_runtime_workers::RuntimeWorkerSpec;
use captain_kernel::goals::{Goal, GoalStatus};
use captain_memory::project;

#[derive(Debug, Clone, Copy)]
pub(crate) struct RuntimeWorkerPromptParts<'a> {
    pub phase: &'a str,
    pub role: &'a str,
    pub task: &'a str,
    pub project_name: &'a str,
    pub project_slug: &'a str,
    pub project_goal: &'a str,
    pub workspace: &'a str,
    pub authorized_tools: &'a str,
    pub criteria: &'a str,
    pub project_goals: &'a str,
    pub goal_gate: &'a str,
    pub prior: &'a str,
    pub user_questions: &'a str,
    pub tool_decisions: &'a str,
}

pub(crate) fn acceptance_criteria_context(metadata: &serde_json::Value) -> String {
    metadata
        .pointer("/launch/acceptance_criteria")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| format!("- {item}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "- No explicit acceptance criteria recorded.".to_string())
}

pub(crate) fn format_project_goals_for_prompt(goals: &[Goal]) -> String {
    if goals.is_empty() {
        return "- No project goals/check commands are registered.".to_string();
    }
    goals
        .iter()
        .take(12)
        .enumerate()
        .map(|(idx, goal)| {
            let mut lines = vec![
                format!(
                    "{}. [{}] {} ({})",
                    idx + 1,
                    goal.id,
                    goal.name,
                    goal_status_label(goal.status)
                ),
                format!(
                    "   Description: {}",
                    trim_runtime_text(&goal.description, 500)
                ),
                format!("   Check command: {}", goal.check_command),
            ];
            if let Some(recovery) = goal.recovery_command.as_deref() {
                lines.push(format!("   Recovery command: {recovery}"));
            }
            if let Some(last) = goal.recent_checks.back() {
                lines.push(format!(
                    "   Last check: {} ({})",
                    if last.ok { "ok" } else { "failed" },
                    last.ts.to_rfc3339()
                ));
            }
            lines.join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn runtime_goal_gate_instruction(phase: &str) -> &'static str {
    match phase {
        "build" => {
            "- Align produced files and entrypoints with registered project goal check commands when practical.\n"
        }
        "execute" => {
            "- Run or rehearse the registered project goal check commands when they are safe and local; record exact command outcomes.\n"
        }
        "verify" => {
            "- Verification is not complete until every active project goal check command has passed, or the exact failed command/result is recorded as a blocker.\n"
        }
        "learn" => {
            "- Capture durable learnings from project goal check outcomes without duplicating existing memory or skills.\n"
        }
        _ => "",
    }
}

pub(crate) fn runtime_worker_system_prompt_for_tools(
    phase: &str,
    authorized_tools: &[String],
) -> String {
    let tools = authorized_tools.join(", ");
    format!(
        "You are a Captain project runtime sub-agent for the {phase} phase.\n\
         You are not the project manager: Captain coordinates the full run and you own only this phase.\n\
         Authorized tools: {tools}.\n\
         Work pragmatically inside the provided project workspace, preserve unrelated user changes, and do not claim completion without evidence.\n\
         Use only the authorized tools. If a required tool is missing, do not improvise a workaround outside your scope: return STATUS: blocked with TOOL_REQUEST and REASON so Captain can approve or deny the extension.\n\
         Return a concise handoff with these headings exactly: STATUS, SUMMARY, ACTIONS, FILES, VERIFY, NEXT, LEARN.\n\
         Use STATUS: blocked if you hit a real blocker."
    )
}

pub(crate) fn runtime_worker_prompt_text(parts: RuntimeWorkerPromptParts<'_>) -> String {
    format!(
        "Project runtime phase: {phase}\n\
         Role: {role}\n\
         Task: {task}\n\
         Project: {name} ({slug})\n\
         Goal: {goal}\n\
         Workspace: {workspace}\n\
         Authorized tools: {authorized_tools}\n\
         Acceptance criteria:\n{criteria}\n\n\
         Project goals and check commands:\n{project_goals}\n\n\
         Prior phase context:\n{prior}\n\n\
         User questions and answers:\n{user_questions}\n\n\
         Tool approval decisions:\n{tool_decisions}\n\n\
         Instructions:\n\
         - Execute only the {phase} responsibility.\n\
         {goal_gate}\
         - Use only the authorized tools listed above. If you need another tool, stop with STATUS: blocked and include TOOL_REQUEST: <tool name> and REASON: <why Captain should approve it>.\n\
         - When the right capability is unclear, call capability_search or tool_search before claiming it is unavailable.\n\
         - Use tools when needed; for code work, inspect before editing and verify after editing.\n\
         - If dependencies, credentials, or environment prevent completion, use STATUS: blocked and explain the smallest next action.\n\
         - Keep the handoff short but operationally useful for Captain and the user.\n\
         - End with this exact handoff block, without raw tool-call transcripts:\n\
           STATUS: complete|blocked\n\
           SUMMARY: <2-5 concise sentences>\n\
           CHANGED_FILES: <files changed, or none>\n\
           VERIFY: <checks run and result>\n\
           NEXT: <smallest next action, or none>",
        phase = parts.phase,
        role = parts.role,
        task = parts.task,
        name = parts.project_name,
        slug = parts.project_slug,
        goal = parts.project_goal,
        workspace = parts.workspace,
        authorized_tools = parts.authorized_tools,
        criteria = parts.criteria,
        project_goals = parts.project_goals,
        goal_gate = parts.goal_gate,
        prior = parts.prior,
        user_questions = parts.user_questions,
        tool_decisions = parts.tool_decisions,
    )
}

pub(crate) fn runtime_worker_prompt_for_project(
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    runtime: &serde_json::Value,
    workspace_path: &str,
    project_goals: &str,
) -> String {
    let authorized_tools =
        runtime_worker_authorized_tools_for_runtime(&spec.profile, spec.phase, runtime).join(", ");
    let criteria = acceptance_criteria_context(&project.metadata);
    let goal_gate = runtime_goal_gate_instruction(spec.phase);
    let prior = runtime_dependency_context(runtime, spec.phase);
    let user_questions = runtime_user_questions_context(runtime, spec.phase);
    let tool_decisions = tool_decisions_context(runtime, spec.phase);
    runtime_worker_prompt_text(RuntimeWorkerPromptParts {
        phase: spec.phase,
        role: spec.role,
        task: spec.task,
        project_name: &project.name,
        project_slug: &project.slug,
        project_goal: &project.goal,
        workspace: workspace_path,
        authorized_tools: &authorized_tools,
        criteria: &criteria,
        project_goals,
        goal_gate,
        prior: &prior,
        user_questions: &user_questions,
        tool_decisions: &tool_decisions,
    })
}

pub(crate) fn runtime_dependency_context(
    runtime: &serde_json::Value,
    current_phase: &str,
) -> String {
    let mut sections = Vec::new();
    if let Some(workers) = runtime.get("workers").and_then(|value| value.as_array()) {
        for worker in workers {
            let phase = worker
                .get("phase")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if phase == current_phase {
                continue;
            }
            let status = worker
                .get("status")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if !matches!(status, "done" | "blocked" | "failed") {
                continue;
            }
            let summary = worker
                .get("summary")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim();
            if summary.is_empty() {
                continue;
            }
            sections.push(format!(
                "[{} / {}]\n{}",
                phase,
                status,
                trim_runtime_text(summary, 900)
            ));
        }
    }
    if sections.is_empty() {
        "No prior completed phase context yet.".to_string()
    } else {
        sections.join("\n\n")
    }
}

fn goal_status_label(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Active => "active",
        GoalStatus::Paused => "paused",
        GoalStatus::Escalated => "escalated",
    }
}

#[cfg(test)]
#[path = "project_runtime_prompt_context_tests.rs"]
mod project_runtime_prompt_context_tests;

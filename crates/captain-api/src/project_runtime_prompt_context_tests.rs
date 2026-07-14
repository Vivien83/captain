use super::*;
use captain_kernel::goals::{Goal, GoalStatus};
use captain_memory::project::{self, ProjectStatus};

fn project() -> project::Project {
    project::Project {
        id: "project-1".to_string(),
        name: "Demo Project".to_string(),
        slug: "demo-project".to_string(),
        goal: "Ship a working demo".to_string(),
        status: ProjectStatus::Active,
        deadline: None,
        created_at: 0,
        updated_at: 0,
        metadata: serde_json::json!({
            "launch": {
                "acceptance_criteria": ["smoke passes"]
            }
        }),
    }
}

fn goal(id: &str, status: GoalStatus) -> Goal {
    Goal {
        id: id.to_string(),
        name: "Calculator smoke".to_string(),
        description: "Keep the CLI calculator smoke test passing.".to_string(),
        project_id: Some("project-1".to_string()),
        project_slug: Some("release-smoke-calculator".to_string()),
        status,
        interval_secs: 300,
        check_command: "test -f main.py && python3 main.py".to_string(),
        recovery_command: Some("python3 recover.py".to_string()),
        escalation_threshold: 1,
        max_llm_calls_per_hour: 20,
        escalation_channel: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        last_check_ts: None,
        consecutive_fails: 0,
        escalated_at: None,
        recent_checks: std::collections::VecDeque::new(),
        llm_call_log: Vec::new(),
        suggestions: Vec::new(),
    }
}

#[test]
fn acceptance_criteria_context_formats_items_and_fallback() {
    let metadata = serde_json::json!({
        "launch": {
            "acceptance_criteria": [" smoke passes ", "", 42, "docs updated"]
        }
    });

    assert_eq!(
        acceptance_criteria_context(&metadata),
        "- smoke passes\n- docs updated"
    );
    assert_eq!(
        acceptance_criteria_context(&serde_json::json!({})),
        "- No explicit acceptance criteria recorded."
    );
}

#[test]
fn project_goals_prompt_includes_check_commands_and_recovery() {
    let context = format_project_goals_for_prompt(&[goal("goal-1", GoalStatus::Escalated)]);
    assert!(context.contains("[goal-1] Calculator smoke (escalated)"));
    assert!(context.contains("Check command: test -f main.py && python3 main.py"));
    assert!(context.contains("Recovery command: python3 recover.py"));
}

#[test]
fn project_goals_prompt_has_empty_fallback() {
    assert_eq!(
        format_project_goals_for_prompt(&[]),
        "- No project goals/check commands are registered."
    );
}

#[test]
fn runtime_goal_gate_instruction_marks_verify_as_hard_gate() {
    assert!(runtime_goal_gate_instruction("verify")
        .contains("every active project goal check command has passed"));
    assert_eq!(runtime_goal_gate_instruction("observe"), "");
}

#[test]
fn runtime_worker_system_prompt_lists_tools_and_tool_request_contract() {
    let prompt = runtime_worker_system_prompt_for_tools(
        "build",
        &["file_read".to_string(), "shell_exec".to_string()],
    );

    assert!(prompt.contains("build phase"));
    assert!(prompt.contains("Authorized tools: file_read, shell_exec."));
    assert!(prompt.contains("TOOL_REQUEST"));
    assert!(prompt.contains("STATUS, SUMMARY, ACTIONS, FILES, VERIFY, NEXT, LEARN"));
}

#[test]
fn runtime_worker_system_prompt_keeps_worker_scope_narrow() {
    let prompt = runtime_worker_system_prompt_for_tools("verify", &["cargo_test".to_string()]);

    assert!(prompt.contains("You are not the project manager"));
    assert!(prompt.contains("Use only the authorized tools"));
    assert!(prompt.contains("do not claim completion without evidence"));
}

#[test]
fn runtime_worker_prompt_text_includes_context_sections_and_handoff_contract() {
    let prompt = runtime_worker_prompt_text(RuntimeWorkerPromptParts {
        phase: "build",
        role: "Builder",
        task: "Create the project files",
        project_name: "Demo Project",
        project_slug: "demo-project",
        project_goal: "Ship a working demo",
        workspace: "/tmp/demo-project",
        authorized_tools: "file_read, apply_patch",
        criteria: "- smoke passes",
        project_goals: "1. [goal-1] Smoke (active)\n   Check command: cargo test",
        goal_gate: runtime_goal_gate_instruction("build"),
        prior: "[observe / done]\nRepo inspected.",
        user_questions: "- [ask-1] build/answered: Which runtime? | answer: Rust",
        tool_decisions: "- Approved extra tools for this phase: cargo_test.",
    });

    assert!(prompt.contains("Project runtime phase: build"));
    assert!(prompt.contains("Role: Builder"));
    assert!(prompt.contains("Task: Create the project files"));
    assert!(prompt.contains("Project: Demo Project (demo-project)"));
    assert!(prompt.contains("Goal: Ship a working demo"));
    assert!(prompt.contains("Workspace: /tmp/demo-project"));
    assert!(prompt.contains("Authorized tools: file_read, apply_patch"));
    assert!(prompt.contains("Acceptance criteria:\n- smoke passes"));
    assert!(prompt.contains("Project goals and check commands:\n1. [goal-1]"));
    assert!(prompt.contains("Prior phase context:\n[observe / done]\nRepo inspected."));
    assert!(prompt.contains("User questions and answers:\n- [ask-1]"));
    assert!(prompt.contains("Tool approval decisions:\n- Approved extra tools"));
    assert!(prompt.contains("STATUS: complete|blocked"));
    assert!(prompt.contains("CHANGED_FILES: <files changed, or none>"));
}

#[test]
fn runtime_worker_prompt_text_preserves_operational_blocker_rules() {
    let prompt = runtime_worker_prompt_text(RuntimeWorkerPromptParts {
        phase: "verify",
        role: "Verifier",
        task: "Run checks",
        project_name: "Demo",
        project_slug: "demo",
        project_goal: "Release confidently",
        workspace: "/workspace/demo",
        authorized_tools: "shell_exec",
        criteria: "- all checks pass",
        project_goals: "- No project goals/check commands are registered.",
        goal_gate: runtime_goal_gate_instruction("verify"),
        prior: "No prior completed phase context yet.",
        user_questions: "Current phase has a pending user question.",
        tool_decisions: "No previous tool approval or denial for this phase.",
    });

    assert!(prompt.contains("every active project goal check command has passed"));
    assert!(prompt.contains("TOOL_REQUEST: <tool name>"));
    assert!(prompt.contains("capability_search or tool_search"));
    assert!(prompt.contains("inspect before editing and verify after editing"));
    assert!(prompt.contains("smallest next action"));
}

#[test]
fn runtime_worker_prompt_for_project_assembles_runtime_context() {
    let project = project();
    let spec = &crate::project_runtime_workers::RUNTIME_WORKER_SPECS[3];
    let runtime = serde_json::json!({
        "workers": [
            {"phase": "observe", "status": "done", "summary": "Repo inspected."}
        ],
        "worker_results": {
            "build": {
                "tool_request": {
                    "status": "approved",
                    "tools": ["extra_tool"]
                }
            }
        },
        "user_questions": [
            {
                "ask_id": "ask-123456",
                "phase": "build",
                "status": "answered",
                "question": "Which runtime?",
                "answer": "Rust"
            }
        ]
    });

    let prompt = runtime_worker_prompt_for_project(
        &project,
        spec,
        &runtime,
        "/workspace/demo",
        "1. [goal-1] Smoke (active)\n   Check command: cargo test",
    );

    assert!(prompt.contains("Project runtime phase: build"));
    assert!(prompt.contains("Project: Demo Project (demo-project)"));
    assert!(prompt.contains("Workspace: /workspace/demo"));
    assert!(prompt.contains("Authorized tools:"));
    assert!(prompt.contains("extra_tool"));
    assert!(prompt.contains("Acceptance criteria:\n- smoke passes"));
    assert!(prompt.contains("Project goals and check commands:\n1. [goal-1]"));
    assert!(prompt.contains("Prior phase context:\n[observe / done]\nRepo inspected."));
    assert!(prompt.contains("User questions and answers:\n- [ask-123"));
    assert!(prompt.contains("Which runtime?"));
    assert!(prompt.contains("answer: Rust"));
    assert!(prompt.contains("Tool approval decisions:\n- Approved extra tools"));
}

#[test]
fn runtime_dependency_context_keeps_prior_terminal_worker_summaries() {
    let runtime = serde_json::json!({
        "workers": [
            {"phase": "observe", "status": "done", "summary": "Repo inspected."},
            {"phase": "build", "status": "running", "summary": "Should be ignored."},
            {"phase": "verify", "status": "blocked", "summary": "Tests need credentials."}
        ]
    });

    let context = runtime_dependency_context(&runtime, "build");
    assert!(context.contains("[observe / done]\nRepo inspected."));
    assert!(context.contains("[verify / blocked]\nTests need credentials."));
    assert!(!context.contains("Should be ignored"));
}

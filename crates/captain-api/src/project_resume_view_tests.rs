use super::*;

#[test]
fn resume_context_views_omit_raw_handoff_payloads() {
    let checkpoint = checkpoint_view(json!({
        "id": "checkpoint-1",
        "session_id": "session-1",
        "summary": "Ready for verification.",
        "state": {"secret": "checkpoint-state-secret"},
        "project_id": "project-secret"
    }));
    let tasks = task_list_view(json!([{
        "id": "task-1",
        "title": "Verify",
        "status": "blocked-secret-status",
        "description": "task-description-secret",
        "assignee_agent_id": "agent-secret",
        "metadata": {"secret": "task-metadata-secret"}
    }]));
    let goals = goal_list_view(json!([{
        "id": "goal-1",
        "name": "Release guard",
        "status": "active",
        "description": "goal-description-secret",
        "check_command": "echo goal-command-secret",
        "recovery_command": "echo recovery-secret",
        "recent_checks": [{"output": "goal-output-secret"}],
        "suggestions": ["goal-suggestion-secret"]
    }]));
    let progress = milestone_progress_view(json!({
        "total": 2,
        "completed": 1,
        "missed": 0,
        "pct": 0.5,
        "deliverables": ["deliverable-secret"]
    }));
    let milestone = milestone_item_view(json!({
        "id": "milestone-1",
        "project_id": "project-1",
        "name": "First delivery",
        "status": "completed",
        "deliverables": ["milestone-deliverable-secret"],
        "metadata": {"secret": "milestone-metadata-secret"}
    }));
    let checkpoint_history = checkpoint_list_view(json!([{
        "id": "checkpoint-history-1",
        "summary": "History row",
        "state": {"secret": "checkpoint-history-secret"}
    }]));

    assert_eq!(checkpoint["summary"], "Ready for verification.");
    assert!(checkpoint.get("state").is_none());
    assert_eq!(tasks[0]["status"], "todo");
    assert!(tasks[0].get("description").is_none());
    assert_eq!(goals[0]["status"], "active");
    assert_eq!(goals[0]["check_command_configured"], true);
    assert_eq!(goals[0]["recovery_command_configured"], true);
    assert!(goals[0].get("check_command").is_none());
    assert!(goals[0].get("recovery_command").is_none());
    assert_eq!(milestone["deliverable_count"], 1);
    assert!(milestone.get("deliverables").is_none());
    assert_eq!(progress["pct"], 0.5);
    assert!(progress.get("deliverables").is_none());
    assert!(checkpoint_history[0].get("state").is_none());

    let encoded = serde_json::to_string(&json!({
        "checkpoint": checkpoint,
        "tasks": tasks,
        "goals": goals,
        "milestone": milestone,
        "progress": progress,
        "checkpoint_history": checkpoint_history
    }))
    .unwrap();
    for forbidden in [
        "checkpoint-state-secret",
        "project-secret",
        "task-description-secret",
        "agent-secret",
        "task-metadata-secret",
        "goal-description-secret",
        "goal-command-secret",
        "recovery-secret",
        "goal-output-secret",
        "goal-suggestion-secret",
        "milestone-deliverable-secret",
        "milestone-metadata-secret",
        "deliverable-secret",
        "checkpoint-history-secret",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn resume_context_views_bound_text_and_list_lengths() {
    let long = "x".repeat(700);
    let checkpoint = checkpoint_view(json!({
        "id": "checkpoint-2",
        "summary": long.clone()
    }));
    let summary = checkpoint["summary"].as_str().unwrap();
    assert_eq!(summary.chars().count(), 600);
    assert!(summary.ends_with("..."));

    let tasks = task_list_view(Value::Array(
        (0..55)
            .map(|index| {
                json!({
                    "id": format!("task-{index}"),
                    "title": long.clone(),
                    "status": "doing"
                })
            })
            .collect(),
    ));
    let tasks = tasks.as_array().unwrap();
    assert_eq!(tasks.len(), 50);
    assert_eq!(tasks[0]["title"].as_str().unwrap().chars().count(), 180);
    assert_eq!(tasks[0]["status"], "doing");
}

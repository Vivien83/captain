use super::*;

#[test]
fn project_update_text_fields_trim_and_reject_empty_or_huge_values() {
    assert_eq!(
        normalize_project_update_name(Some("  Demo  ".to_string())).unwrap(),
        Some("Demo".to_string())
    );
    assert_eq!(
        normalize_project_update_goal(Some("  Ship safely  ".to_string())).unwrap(),
        Some("Ship safely".to_string())
    );
    assert_eq!(
        normalize_project_update_goal(Some("   ".to_string())),
        Err("goal cannot be empty")
    );
    assert_eq!(
        normalize_project_update_name(Some("n".repeat(PROJECT_NAME_LIMIT + 1))),
        Err("name is too long")
    );
}

#[test]
fn project_create_fields_trim_and_reject_invalid_values() {
    assert_eq!(
        normalize_project_create_name("  Demo  ".to_string()).unwrap(),
        "Demo"
    );
    assert_eq!(
        normalize_project_create_goal("  Ship safely  ".to_string()).unwrap(),
        "Ship safely"
    );
    assert_eq!(
        normalize_project_create_goal("   ".to_string()).unwrap(),
        ""
    );
    assert_eq!(
        normalize_project_slug(" demo-1 ".to_string()).unwrap(),
        "demo-1"
    );
    assert_eq!(
        normalize_project_create_name("   ".to_string()),
        Err("name cannot be empty")
    );
    assert_eq!(
        normalize_project_create_goal("x".repeat(PROJECT_GOAL_LIMIT + 1)),
        Err("goal is too long")
    );
    assert_eq!(
        normalize_project_slug("Invalid-/Users/example-ghp_secret".to_string()),
        Err(PROJECT_SLUG_ERROR)
    );
}

#[test]
fn project_update_status_error_does_not_echo_input() {
    let secret = "invalid-status-/Users/example/private-ghp_secret".to_string();

    assert_eq!(
        normalize_project_update_status(Some(secret)),
        Err(PROJECT_STATUS_ERROR)
    );
}

#[test]
fn project_task_status_error_does_not_echo_input() {
    let secret = "invalid-task-status-/Users/example/private-ghp_secret".to_string();

    assert_eq!(
        normalize_project_task_update_status(Some(secret)),
        Err(PROJECT_TASK_STATUS_ERROR)
    );
    assert_eq!(
        normalize_project_task_update_status(Some(" review ".to_string())).unwrap(),
        Some(TaskStatus::Review)
    );
}

#[test]
fn project_task_text_fields_trim_and_reject_empty_or_huge_values() {
    assert_eq!(
        normalize_project_task_title("  Review launch  ".to_string()).unwrap(),
        "Review launch"
    );
    assert_eq!(
        normalize_project_task_description("  Check status  ".to_string()).unwrap(),
        "Check status"
    );
    assert_eq!(
        normalize_project_task_description("   ".to_string()).unwrap(),
        ""
    );
    assert_eq!(
        normalize_project_task_title("   ".to_string()),
        Err(PROJECT_TASK_TITLE_EMPTY_ERROR)
    );
    assert_eq!(
        normalize_project_task_update_title(Some("x".repeat(PROJECT_TASK_TITLE_LIMIT + 1))),
        Err(PROJECT_TASK_TITLE_LONG_ERROR)
    );
    assert_eq!(
        normalize_project_task_update_description(Some(
            "x".repeat(PROJECT_TASK_DESCRIPTION_LIMIT + 1)
        )),
        Err(PROJECT_TASK_DESCRIPTION_LONG_ERROR)
    );
}

#[test]
fn project_milestone_fields_trim_drop_empty_and_reject_huge_values() {
    assert_eq!(
        normalize_project_milestone_name("  Beta launch  ".to_string()).unwrap(),
        "Beta launch"
    );
    assert_eq!(
        normalize_project_milestone_deliverables(vec![
            " docs ".to_string(),
            "   ".to_string(),
            "smoke".to_string(),
        ])
        .unwrap(),
        vec!["docs".to_string(), "smoke".to_string()]
    );
    assert_eq!(
        normalize_project_milestone_name("   ".to_string()),
        Err(PROJECT_MILESTONE_NAME_EMPTY_ERROR)
    );
    assert_eq!(
        normalize_project_milestone_name("x".repeat(PROJECT_MILESTONE_NAME_LIMIT + 1)),
        Err(PROJECT_MILESTONE_NAME_LONG_ERROR)
    );
    assert_eq!(
        normalize_project_milestone_deliverables(vec![
            "x".repeat(PROJECT_MILESTONE_DELIVERABLE_LIMIT + 1)
        ]),
        Err(PROJECT_MILESTONE_DELIVERABLE_LONG_ERROR)
    );
    assert_eq!(
        normalize_project_milestone_deliverables(vec![
            "item".to_string();
            PROJECT_MILESTONE_DELIVERABLES_LIMIT + 1
        ]),
        Err(PROJECT_MILESTONE_DELIVERABLES_COUNT_ERROR)
    );
}

#[test]
fn project_checkpoint_fields_trim_and_reject_state_payloads() {
    assert_eq!(
        normalize_project_checkpoint_summary("  reached verify  ".to_string()).unwrap(),
        "reached verify"
    );
    assert_eq!(
        normalize_project_checkpoint_session_id(Some(" session-1 ".to_string())).unwrap(),
        Some("session-1".to_string())
    );
    assert_eq!(
        normalize_project_checkpoint_session_id(Some("   ".to_string())).unwrap(),
        None
    );
    assert_eq!(
        normalize_project_checkpoint_summary("   ".to_string()),
        Err(PROJECT_CHECKPOINT_SUMMARY_EMPTY_ERROR)
    );
    assert_eq!(
        normalize_project_checkpoint_summary("x".repeat(PROJECT_CHECKPOINT_SUMMARY_LIMIT + 1)),
        Err(PROJECT_CHECKPOINT_SUMMARY_LONG_ERROR)
    );
    assert_eq!(
        normalize_project_checkpoint_session_id(Some(
            "x".repeat(PROJECT_CHECKPOINT_SESSION_ID_LIMIT + 1)
        )),
        Err(PROJECT_CHECKPOINT_SESSION_ID_LONG_ERROR)
    );
    assert!(!rejects_project_checkpoint_state(&Value::Null));
    assert!(!rejects_project_checkpoint_state(&serde_json::json!({})));
    assert!(rejects_project_checkpoint_state(&serde_json::json!({
        "path": "/Users/example/private",
        "token": "ghp_secret"
    })));
}

#[test]
fn project_lifecycle_phase_error_does_not_echo_input() {
    let secret = "invalid-phase-/Users/example/private-ghp_secret".to_string();

    assert_eq!(
        normalize_project_lifecycle_phase(secret),
        Err(PROJECT_LIFECYCLE_PHASE_ERROR)
    );
    assert_eq!(
        normalize_project_lifecycle_phase(" BUILD ".to_string()).unwrap(),
        "build"
    );
}

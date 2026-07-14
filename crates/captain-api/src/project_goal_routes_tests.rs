use super::*;
use std::collections::VecDeque;

#[test]
fn project_goal_update_edits_fields_and_resets_changed_check_state() {
    let mut goal = Goal {
        id: "goal-1".to_string(),
        name: "Old smoke".to_string(),
        description: "old".to_string(),
        project_id: Some("project-1".to_string()),
        project_slug: Some("demo".to_string()),
        status: GoalStatus::Active,
        interval_secs: 300,
        check_command: "python3 old.py".to_string(),
        recovery_command: Some("echo recover".to_string()),
        escalation_threshold: 3,
        max_llm_calls_per_hour: 20,
        escalation_channel: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_check_ts: Some(Utc::now()),
        consecutive_fails: 2,
        escalated_at: None,
        recent_checks: VecDeque::new(),
        llm_call_log: Vec::new(),
        suggestions: Vec::new(),
    };

    apply_project_goal_update(
        &mut goal,
        UpdateProjectGoalReq {
            name: Some("New smoke".to_string()),
            description: Some("new description".to_string()),
            check_command: Some("python3 main.py --add 1 2".to_string()),
            recovery_command: Some(String::new()),
            interval_secs: Some(120),
            escalation_threshold: Some(2),
            max_llm_calls_per_hour: Some(10),
        },
    )
    .unwrap();

    assert_eq!(goal.name, "New smoke");
    assert_eq!(goal.description, "new description");
    assert_eq!(goal.check_command, "python3 main.py --add 1 2");
    assert_eq!(goal.recovery_command, None);
    assert_eq!(goal.interval_secs, 120);
    assert_eq!(goal.escalation_threshold, 2);
    assert_eq!(goal.max_llm_calls_per_hour, 10);
    assert_eq!(goal.consecutive_fails, 0);
    assert!(goal.last_check_ts.is_none());
    assert_eq!(goal.status, GoalStatus::Active);
    assert!(goal.escalated_at.is_none());
}

#[test]
fn project_goal_update_rejects_blank_check_command() {
    let mut goal = Goal {
        id: "goal-1".to_string(),
        name: "Smoke".to_string(),
        description: "desc".to_string(),
        project_id: Some("project-1".to_string()),
        project_slug: Some("demo".to_string()),
        status: GoalStatus::Active,
        interval_secs: 300,
        check_command: "python3 main.py".to_string(),
        recovery_command: None,
        escalation_threshold: 3,
        max_llm_calls_per_hour: 20,
        escalation_channel: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_check_ts: None,
        consecutive_fails: 0,
        escalated_at: None,
        recent_checks: VecDeque::new(),
        llm_call_log: Vec::new(),
        suggestions: Vec::new(),
    };

    let result = apply_project_goal_update(
        &mut goal,
        UpdateProjectGoalReq {
            name: None,
            description: None,
            check_command: Some(" ".to_string()),
            recovery_command: None,
            interval_secs: None,
            escalation_threshold: None,
            max_llm_calls_per_hour: None,
        },
    );

    assert!(result.unwrap_err().contains("check_command"));
    assert_eq!(goal.check_command, "python3 main.py");
}

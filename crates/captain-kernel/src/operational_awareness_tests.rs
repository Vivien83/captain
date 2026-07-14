use super::*;
use captain_memory::project::ProjectStatus;
use chrono::Utc;
use std::collections::VecDeque;

fn supervisor() -> SupervisorHealth {
    SupervisorHealth {
        is_shutting_down: false,
        failure_count: 0,
        panic_count: 0,
        restart_count: 0,
    }
}

fn snapshot<'a>(supervisor: &'a SupervisorHealth) -> AwarenessSnapshot<'a> {
    AwarenessSnapshot {
        supervisor,
        active_goals: Vec::new(),
        escalated_goals: Vec::new(),
        queued_thoughts: 0,
        confidence: 0.7,
        error_rate: 0.0,
        user_mode: "explore".to_string(),
        user_frustration: 0.0,
        prediction_accuracy: 1.0,
        prediction_total: 0,
        projects: ProjectAwarenessSignals::default(),
    }
}

fn goal(id: &str, status: GoalStatus) -> Goal {
    Goal {
        id: id.to_string(),
        name: format!("Goal {id}"),
        description: String::new(),
        project_id: None,
        project_slug: None,
        status,
        interval_secs: crate::goals::MIN_INTERVAL_SECS,
        check_command: "true".to_string(),
        recovery_command: None,
        escalation_threshold: 1,
        max_llm_calls_per_hour: 1,
        escalation_channel: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_check_ts: None,
        consecutive_fails: 0,
        escalated_at: None,
        recent_checks: VecDeque::new(),
        llm_call_log: Vec::new(),
        suggestions: Vec::new(),
    }
}

fn project(metadata: serde_json::Value) -> Project {
    Project {
        id: "p1".to_string(),
        name: "Project".to_string(),
        slug: "project".to_string(),
        goal: "Ship".to_string(),
        status: ProjectStatus::Active,
        deadline: None,
        created_at: 0,
        updated_at: 0,
        metadata,
    }
}

#[test]
fn format_awareness_returns_empty_when_steady() {
    let health = supervisor();
    assert!(format_awareness(snapshot(&health)).is_empty());
}

#[test]
fn format_awareness_includes_active_goals_as_watch_signal() {
    let health = supervisor();
    let active = goal_labels(&[goal("uptime", GoalStatus::Active)], GoalStatus::Active);
    let prompt = format_awareness(AwarenessSnapshot {
        active_goals: active,
        confidence: 0.6,
        ..snapshot(&health)
    });

    assert!(prompt.contains("State: watch"));
    assert!(prompt.contains("active goals: Goal uptime (uptime)"));
}

#[test]
fn format_awareness_escalates_supervisor_and_goal_failures() {
    let health = SupervisorHealth {
        panic_count: 1,
        ..supervisor()
    };
    let escalated = goal_labels(
        &[goal("deploy", GoalStatus::Escalated)],
        GoalStatus::Escalated,
    );
    let prompt = format_awareness(AwarenessSnapshot {
        supervisor: &health,
        escalated_goals: escalated,
        confidence: 0.4,
        error_rate: 0.4,
        user_mode: "debug".to_string(),
        ..snapshot(&health)
    });

    assert!(prompt.contains("State: warn"));
    assert!(prompt.contains("supervisor panics since daemon start: 1"));
    assert!(prompt.contains("escalated goals: Goal deploy (deploy)"));
    assert!(prompt.contains("recent error rate: 0.40"));
}

#[test]
fn recoverable_failure_count_does_not_create_a_permanent_prompt_warning() {
    let health = SupervisorHealth {
        failure_count: 8,
        ..supervisor()
    };

    assert!(format_awareness(snapshot(&health)).is_empty());
}

#[test]
fn format_awareness_marks_shutdown_as_critical() {
    let health = SupervisorHealth {
        is_shutting_down: true,
        ..supervisor()
    };
    let prompt = format_awareness(AwarenessSnapshot {
        supervisor: &health,
        confidence: 0.5,
        ..snapshot(&health)
    });

    assert!(prompt.contains("State: critical"));
    assert!(prompt.contains("supervisor shutdown requested"));
}

#[test]
fn format_awareness_includes_project_attention() {
    let health = supervisor();
    let prompt = format_awareness(AwarenessSnapshot {
        projects: ProjectAwarenessSignals {
            waiting_for_user: 1,
            repeated_tool_denials: 1,
            resume_ready: 1,
            ..ProjectAwarenessSignals::default()
        },
        ..snapshot(&health)
    });

    assert!(prompt.contains("State: warn"));
    assert!(prompt.contains("project questions waiting for user: 1"));
    assert!(prompt.contains("project repeated denied tools: 1"));
    assert!(prompt.contains("project runs ready to resume: 1"));
}

#[test]
fn project_awareness_extracts_runtime_attention() {
    let projects = vec![
        project(serde_json::json!({
            "runtime": {
                "status": "blocked",
                "user_questions": [{"ask_id": "q1", "status": "pending"}]
            }
        })),
        project(serde_json::json!({
            "runtime": {
                "worker_results": {
                    "build": {
                        "tool_request": {
                            "status": "denied",
                            "repeat_of_denied_tool_request": true
                        }
                    }
                }
            }
        })),
        project(serde_json::json!({
            "runtime": {"resume_pending": {"reason": "project_ask_answered"}}
        })),
    ];

    let signals = project_awareness_from_projects(&projects);

    assert_eq!(signals.waiting_for_user, 1);
    assert_eq!(signals.tool_request_denied, 1);
    assert_eq!(signals.repeated_tool_denials, 1);
    assert_eq!(signals.resume_ready, 1);
}

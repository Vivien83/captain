use super::*;

fn input() -> AttentionInput {
    AttentionInput {
        shutting_down: false,
        panic_count: 0,
        restart_count: 0,
        error_rate: 0.0,
        user_frustration: 0.0,
        queued_thoughts: 0,
        active_work: 0,
        active_goals: 0,
        escalated_goals: 0,
        prediction_accuracy: 1.0,
        prediction_total: 0,
        pattern_count: 0,
        project_attention_count: 0,
        project_waiting_for_user: 0,
        project_tool_request_pending: 0,
        project_tool_request_denied: 0,
        project_repeated_tool_denials: 0,
        project_resume_ready: 0,
        project_stale_active: 0,
        project_blocked: 0,
        project_failed: 0,
    }
}

#[test]
fn attention_is_critical_when_supervisor_is_shutting_down() {
    let summary = classify_operational_attention(AttentionInput {
        shutting_down: true,
        ..input()
    });

    assert_eq!(summary.state, "critical");
    assert_eq!(summary.signals, vec!["supervisor_shutdown_requested"]);
    assert!(!summary.operator_actions.is_empty());
}

#[test]
fn attention_warns_on_escalated_goals_and_errors() {
    let summary = classify_operational_attention(AttentionInput {
        escalated_goals: 1,
        error_rate: 0.4,
        ..input()
    });

    assert_eq!(summary.state, "warn");
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "goals_escalated:1"));
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "error_rate:0.40"));
}

#[test]
fn attention_watches_active_or_queued_runtime_signals() {
    let summary = classify_operational_attention(AttentionInput {
        queued_thoughts: 2,
        active_work: 1,
        ..input()
    });

    assert_eq!(summary.state, "watch");
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "queued_thoughts:2"));
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "active_work:1"));
}

#[test]
fn attention_stays_steady_for_clean_runtime() {
    let summary = classify_operational_attention(input());

    assert_eq!(summary.state, "steady");
    assert!(summary.signals.is_empty());
    assert!(summary.operator_actions.is_empty());
}

#[test]
fn attention_warns_only_for_true_panics_and_names_the_counter_scope() {
    let summary = classify_operational_attention(AttentionInput {
        panic_count: 2,
        ..input()
    });

    assert_eq!(summary.state, "warn");
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "supervisor_panics_since_start:2"));
}

#[test]
fn project_attention_warns_for_pending_tool_requests() {
    let summary = classify_operational_attention(AttentionInput {
        project_attention_count: 1,
        project_tool_request_pending: 1,
        ..input()
    });

    assert_eq!(summary.state, "warn");
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "project_tool_requests_pending:1"));
    assert!(summary
        .operator_actions
        .iter()
        .any(|action| action.contains("Approve or deny")));
}

#[test]
fn project_attention_warns_for_pending_user_answers() {
    let summary = classify_operational_attention(AttentionInput {
        project_attention_count: 1,
        project_waiting_for_user: 1,
        ..input()
    });

    assert_eq!(summary.state, "warn");
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "project_waiting_for_user:1"));
    assert!(summary
        .operator_actions
        .iter()
        .any(|action| action.contains("pending project questions")));
}

#[test]
fn project_attention_tracks_repeated_tool_denials() {
    let items = vec![serde_json::json!({
        "state": "tool_request_denied",
        "denied_tool_request": {
            "repeat_of_denied_tool_request": true
        }
    })];

    let signals = ProjectAttentionSignals::from_items(&items);

    assert_eq!(signals.total, 1);
    assert_eq!(signals.tool_request_denied, 1);
    assert_eq!(signals.repeated_tool_denials, 1);
}

#[test]
fn project_attention_watches_generic_attention_without_blockers() {
    let summary = classify_operational_attention(AttentionInput {
        project_attention_count: 2,
        ..input()
    });

    assert_eq!(summary.state, "watch");
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "project_attention:2"));
}

#[test]
fn prediction_attention_watches_low_accuracy_and_patterns() {
    let summary = classify_operational_attention(AttentionInput {
        prediction_accuracy: 0.25,
        prediction_total: 4,
        pattern_count: 3,
        ..input()
    });

    assert_eq!(summary.state, "watch");
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "prediction_accuracy_low:0.25"));
    assert!(summary
        .signals
        .iter()
        .any(|signal| signal == "temporal_patterns:3"));
}

#[test]
fn attention_input_from_runtime_projects_project_counts() {
    let projects = ProjectAttentionSignals {
        total: 7,
        waiting_for_user: 1,
        tool_request_pending: 2,
        tool_request_denied: 3,
        repeated_tool_denials: 1,
        resume_ready: 1,
        stale_active: 1,
        blocked: 1,
        failed: 1,
    };

    let input = AttentionInput::from_runtime(
        RuntimeAttentionSignals {
            shutting_down: true,
            panic_count: 1,
            restart_count: 2,
            error_rate: 0.4,
            user_frustration: 0.7,
            queued_thoughts: 3,
            active_work: 4,
            active_goals: 5,
            escalated_goals: 6,
            prediction_accuracy: 0.25,
            prediction_total: 8,
            pattern_count: 9,
        },
        projects,
    );

    assert!(input.shutting_down);
    assert_eq!(input.panic_count, 1);
    assert_eq!(input.restart_count, 2);
    assert_eq!(input.queued_thoughts, 3);
    assert_eq!(input.active_work, 4);
    assert_eq!(input.active_goals, 5);
    assert_eq!(input.escalated_goals, 6);
    assert_eq!(input.prediction_total, 8);
    assert_eq!(input.pattern_count, 9);
    assert_eq!(input.project_attention_count, 7);
    assert_eq!(input.project_waiting_for_user, 1);
    assert_eq!(input.project_tool_request_pending, 2);
    assert_eq!(input.project_tool_request_denied, 3);
    assert_eq!(input.project_repeated_tool_denials, 1);
    assert_eq!(input.project_resume_ready, 1);
    assert_eq!(input.project_stale_active, 1);
    assert_eq!(input.project_blocked, 1);
    assert_eq!(input.project_failed, 1);
}

#[test]
fn pattern_status_json_preserves_optional_weekday_and_limit() {
    let patterns = vec![
        TemporalPattern {
            action: "first".to_string(),
            hour: 8,
            weekday: Some(2),
            occurrences: 4,
            last_seen: 10,
        },
        TemporalPattern {
            action: "daily".to_string(),
            hour: 9,
            weekday: None,
            occurrences: 5,
            last_seen: 11,
        },
        TemporalPattern {
            action: "third".to_string(),
            hour: 10,
            weekday: Some(4),
            occurrences: 6,
            last_seen: 12,
        },
        TemporalPattern {
            action: "hidden".to_string(),
            hour: 11,
            weekday: Some(5),
            occurrences: 7,
            last_seen: 13,
        },
    ];

    let json = pattern_status_json(&patterns);

    assert_eq!(json.len(), 3);
    assert_eq!(json[0]["action"], "first");
    assert_eq!(json[0]["weekday"], 2);
    assert_eq!(json[1]["action"], "daily");
    assert!(json[1]["weekday"].is_null());
    assert_eq!(json[2]["occurrences"], 6);
}

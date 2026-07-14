use super::*;

#[test]
fn status_message_reports_daemon_mode_and_agent() {
    let msg = status_message(
        StatusSnapshot::Daemon {
            base_url: "http://127.0.0.1:50051",
            agent_name: Some("Captain"),
        },
        Lang::En,
    );

    assert_eq!(msg, "Mode: daemon (http://127.0.0.1:50051)\nAgent: Captain");
}

#[test]
fn status_message_reports_inprocess_counts_and_disconnected() {
    let msg = status_message(
        StatusSnapshot::InProcess {
            agent_count: 3,
            agent_name: Some("Local"),
        },
        Lang::En,
    );
    assert_eq!(msg, "Mode: in-process\nAgents: 3\nAgent: Local");

    assert_eq!(
        status_message(StatusSnapshot::Disconnected, Lang::En),
        "Mode: disconnected"
    );
}

#[test]
fn status_message_uses_french_labels() {
    let msg = status_message(
        StatusSnapshot::Daemon {
            base_url: "http://daemon",
            agent_name: None,
        },
        Lang::Fr,
    );

    assert_eq!(msg, "Mode : daemon (http://daemon)");
}

#[test]
fn daemon_session_lines_limit_and_default_missing_fields() {
    let body = serde_json::json!({"sessions": [
        {"session_id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "label": "Captain audit", "message_count": 2, "updated_at": "today"},
        {"session_id": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "label": "Inventory health", "message_count": 3, "updated_at": "yesterday"},
        {}
    ]});

    assert_eq!(
        daemon_session_lines(&body, 2),
        vec![
            "Captain audit [aaaaaaaa] \u{2014} 2 msg \u{2014} today".to_string(),
            "Inventory health [bbbbbbbb] \u{2014} 3 msg \u{2014} yesterday".to_string(),
        ]
    );
    assert!(daemon_session_lines(&serde_json::json!({"sessions": []}), 10).is_empty());
}

#[test]
fn daemon_agent_lines_format_defaults_and_inprocess_line_matches_legacy_shape() {
    let body = serde_json::json!([
        {"name": "captain", "state": "Ready", "model_name": "gpt-5"},
        {}
    ]);

    assert_eq!(
        daemon_agent_lines(&body),
        vec!["captain [Ready] gpt-5".to_string(), "? [?] ?".to_string(),]
    );
    assert_eq!(
        inprocess_agent_line("local", "Running", "openai", "gpt-5"),
        "local [Running] openai/gpt-5"
    );
}

#[test]
fn list_message_uses_empty_fallback_or_joined_lines() {
    assert_eq!(list_message(Vec::new(), "empty"), "empty");
    assert_eq!(
        list_message(vec!["one".to_string(), "two".to_string()], "empty"),
        "one\ntwo"
    );
}

#[test]
fn session_and_agent_fallbacks_preserve_hermes_i18n_text() {
    assert_eq!(sessions_not_connected_message(Lang::En), "Not connected.");
    assert_eq!(
        sessions_list_message(Vec::new(), Lang::Fr),
        "Aucune session récente."
    );
    assert_eq!(
        agents_list_message(Vec::new(), Lang::En),
        "No agents running."
    );
}

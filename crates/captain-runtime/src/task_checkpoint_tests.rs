use super::*;
use captain_types::message::Role;

fn tool_use(name: &str, id: &str) -> Message {
    Message {
        role: Role::Assistant,
        content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
            id: id.to_string(),
            name: name.to_string(),
            input: serde_json::json!({}),
            provider_metadata: None,
        }]),
    }
}

fn tool_result(id: &str, name: &str) -> Message {
    Message {
        role: Role::User,
        content: MessageContent::Blocks(vec![ContentBlock::ToolResult {
            tool_use_id: id.to_string(),
            tool_name: name.to_string(),
            content: "ok".to_string(),
            is_error: false,
        }]),
    }
}

#[test]
fn extracts_last_user_request_and_tool_activity() {
    let messages = vec![
        Message::user("old request"),
        Message::assistant("done"),
        Message::user("audit the release and fix the failures"),
        tool_use("shell_exec", "c1"),
        tool_result("c1", "shell_exec"),
        tool_use("shell_exec", "c2"),
        tool_result("c2", "shell_exec"),
        tool_use("file_write", "c3"),
    ];

    let cp = extract_task_checkpoint(&messages);

    assert_eq!(
        cp.last_user_request.as_deref(),
        Some("audit the release and fix the failures")
    );
    assert_eq!(cp.tool_calls_since, vec!["shell_exec x2", "file_write x1"]);
    assert!(cp.mid_tool_activity);
}

#[test]
fn system_injections_are_not_user_requests() {
    let messages = vec![
        Message::user("real request"),
        Message::assistant("working"),
        Message::user("[Contexte memoire - reference de compaction]\nvieux resume"),
        tool_use("shell_exec", "c1"),
    ];

    let cp = extract_task_checkpoint(&messages);

    assert_eq!(cp.last_user_request.as_deref(), Some("real request"));
    // Tool calls are counted from the real request, through the injection.
    assert_eq!(cp.tool_calls_since, vec!["shell_exec x1"]);
}

#[test]
fn no_user_message_yields_none_and_counts_everything() {
    let messages = vec![
        tool_use("web_search", "c1"),
        tool_result("c1", "web_search"),
        Message::assistant("summary done"),
    ];

    let cp = extract_task_checkpoint(&messages);

    assert!(cp.last_user_request.is_none());
    assert_eq!(cp.tool_calls_since, vec!["web_search x1"]);
    assert!(!cp.mid_tool_activity);
}

#[test]
fn long_user_request_is_truncated() {
    let messages = vec![Message::user("x".repeat(5_000))];
    let cp = extract_task_checkpoint(&messages);
    assert!(cp.last_user_request.unwrap().len() <= 600);
}

#[test]
fn note_contains_request_activity_and_mid_tool_warning() {
    let cp = TaskCheckpoint {
        last_user_request: Some("deploy the fix".to_string()),
        tool_calls_since: vec!["shell_exec x3".to_string()],
        mid_tool_activity: true,
    };

    let note = checkpoint_note(&cp);

    assert!(note.contains("\"deploy the fix\""));
    assert!(note.contains("shell_exec x3"));
    assert!(note.contains("session_tool_call_summary"));
}

#[test]
fn note_falls_back_when_nothing_extracted() {
    let cp = TaskCheckpoint {
        last_user_request: None,
        tool_calls_since: Vec::new(),
        mid_tool_activity: false,
    };

    let note = checkpoint_note(&cp);

    assert!(note.contains("non identifiee"));
}

#[test]
fn checkpoint_round_trips_through_json() {
    let cp = TaskCheckpoint {
        last_user_request: Some("keep going".to_string()),
        tool_calls_since: vec!["file_read x2".to_string()],
        mid_tool_activity: true,
    };

    let value = serde_json::to_value(&cp).unwrap();
    let back: TaskCheckpoint = serde_json::from_value(value).unwrap();

    assert_eq!(back.last_user_request.as_deref(), Some("keep going"));
    assert_eq!(back.tool_calls_since, vec!["file_read x2"]);
    assert!(back.mid_tool_activity);
}

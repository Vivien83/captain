use super::*;

fn progress(
    message: &str,
    frame_index: Option<usize>,
    frames_total: Option<usize>,
) -> tool_runner::ToolProgressEvent {
    tool_runner::ToolProgressEvent {
        tool_use_id: "tool_1".to_string(),
        message: message.to_string(),
        frame_index,
        frames_total,
    }
}

#[test]
fn exec_tool_classifier_covers_terminal_style_tools() {
    assert!(is_exec_tool("shell_exec"));
    assert!(is_exec_tool("docker_run"));
    assert!(is_exec_tool("process_write"));
    assert!(!is_exec_tool("file_read"));
}

#[test]
fn tool_progress_chunk_prefers_message() {
    let chunk = format_tool_progress_chunk(&progress("  rendering frame  ", Some(1), Some(3)));
    assert_eq!(chunk, "rendering frame\n");
}

#[test]
fn tool_progress_chunk_formats_frame_fallback() {
    let chunk = format_tool_progress_chunk(&progress("", Some(1), Some(3)));
    assert_eq!(chunk, "Progression: 2/3\n");
}

#[test]
fn tool_progress_chunk_is_empty_without_message_or_total() {
    assert_eq!(format_tool_progress_chunk(&progress("", Some(1), None)), "");
}

#[test]
fn tool_timeout_constant_matches_legacy_guardrail() {
    assert_eq!(TOOL_TIMEOUT_SECS, 120);
}

#[test]
fn explicit_shell_timeout_disables_outer_tool_wall() {
    let timeout = tool_timeout_guard_secs(
        "shell_exec",
        &serde_json::json!({"command": "cargo build", "timeout_seconds": 1_000}),
        None,
    );
    assert_eq!(timeout, None);
}

#[test]
fn explicit_package_timeout_disables_outer_tool_wall() {
    let timeout = tool_timeout_guard_secs(
        "cargo",
        &serde_json::json!({"subcommand": "test", "timeout_seconds": 1_000}),
        None,
    );
    assert_eq!(timeout, None);
}

#[test]
fn capspec_uses_its_own_durable_deadline() {
    let timeout = tool_timeout_guard_secs("cap_project_release", &serde_json::json!({}), None);
    assert_eq!(timeout, None);
}

#[test]
fn default_tool_timeout_keeps_outer_guard() {
    let timeout = tool_timeout_guard_secs(
        "shell_exec",
        &serde_json::json!({"command": "cargo build"}),
        None,
    );
    assert_eq!(timeout, Some(TOOL_TIMEOUT_SECS));
}

#[test]
fn tool_timeout_result_preserves_call_identity_and_message() {
    let call = ToolCall {
        id: "call-1".to_string(),
        name: "browser_use".to_string(),
        input: serde_json::json!({}),
    };

    let result = tool_timeout_result(&call, 12, false);

    assert_eq!(result.tool_use_id, "call-1");
    assert_eq!(result.content, "Tool 'browser_use' timed out after 12s.");
    assert!(result.is_error);
}

#[test]
fn streaming_tool_timeout_result_uses_same_user_visible_content() {
    let call = ToolCall {
        id: "call-2".to_string(),
        name: "shell_exec".to_string(),
        input: serde_json::json!({}),
    };

    let result = tool_timeout_result(&call, 120, true);

    assert_eq!(result.tool_use_id, "call-2");
    assert_eq!(result.content, "Tool 'shell_exec' timed out after 120s.");
    assert!(result.is_error);
}

#[tokio::test]
async fn run_tool_with_timeout_guard_returns_completed_result() {
    let call = ToolCall {
        id: "call-3".to_string(),
        name: "file_read".to_string(),
        input: serde_json::json!({}),
    };
    let result = ToolResult {
        tool_use_id: "call-3".to_string(),
        content: "ok".to_string(),
        is_error: false,
        transient_content: Vec::new(),
    };

    let actual =
        run_tool_with_timeout_guard(&call, Some(1), false, std::future::ready(result)).await;

    assert_eq!(actual.tool_use_id, "call-3");
    assert_eq!(actual.content, "ok");
    assert!(!actual.is_error);
}

#[tokio::test]
async fn run_tool_with_timeout_guard_returns_timeout_result() {
    let call = ToolCall {
        id: "call-4".to_string(),
        name: "shell_exec".to_string(),
        input: serde_json::json!({}),
    };

    let result =
        run_tool_with_timeout_guard(&call, Some(0), true, std::future::pending::<ToolResult>())
            .await;

    assert_eq!(result.tool_use_id, "call-4");
    assert_eq!(result.content, "Tool 'shell_exec' timed out after 0s.");
    assert!(result.is_error);
}

#[tokio::test]
async fn run_tool_with_timeout_guard_skips_outer_wall_when_disabled() {
    let call = ToolCall {
        id: "call-5".to_string(),
        name: "shell_exec".to_string(),
        input: serde_json::json!({}),
    };
    let result = ToolResult {
        tool_use_id: "call-5".to_string(),
        content: "own timeout handled".to_string(),
        is_error: false,
        transient_content: Vec::new(),
    };

    let actual = run_tool_with_timeout_guard(&call, None, false, std::future::ready(result)).await;

    assert_eq!(actual.content, "own timeout handled");
    assert!(!actual.is_error);
}

use super::*;

#[test]
fn daemon_stream_parser_ignores_non_data_lines() {
    let mut state = DaemonStreamState::default();

    assert!(daemon_stream_events_from_sse_line("", &mut state).is_empty());
    assert!(daemon_stream_events_from_sse_line("event: message", &mut state).is_empty());
    assert!(daemon_stream_events_from_sse_line("retry: 1000", &mut state).is_empty());
    assert!(daemon_stream_events_from_sse_line("data: not-json", &mut state).is_empty());
}

#[test]
fn daemon_stream_parser_maps_typed_tool_events() {
    let mut state = DaemonStreamState::default();

    let start = daemon_stream_events_from_sse_line(
        r#"data: {"type":"tool_start","tool":"shell_exec","id":"tool-id","tool_use_id":"tool-1"}"#,
        &mut state,
    );
    match &start[0] {
        StreamEvent::ToolUseStart { id, name } => {
            assert_eq!(id, "tool-id");
            assert_eq!(name, "shell_exec");
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let end = daemon_stream_events_from_sse_line(
        r#"data: {"type":"tool_end","tool":"shell_exec","id":"tool-1","input":{"cmd":"date"}}"#,
        &mut state,
    );
    match &end[0] {
        StreamEvent::ToolUseEnd { id, name, input } => {
            assert_eq!(id, "tool-1");
            assert_eq!(name, "shell_exec");
            assert_eq!(input["cmd"], "date");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn daemon_stream_parser_maps_tool_result_and_output_delta() {
    let mut state = DaemonStreamState::default();

    let result = daemon_stream_events_from_sse_line(
        r#"data: {"type":"tool_result","tool":"shell_exec","tool_use_id":"tool-1","result":"ok","is_error":true}"#,
        &mut state,
    );
    match &result[0] {
        StreamEvent::ToolExecutionResult {
            tool_use_id,
            name,
            result_preview,
            is_error,
        } => {
            assert_eq!(tool_use_id, "tool-1");
            assert_eq!(name, "shell_exec");
            assert_eq!(result_preview, "ok");
            assert!(*is_error);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    let delta = daemon_stream_events_from_sse_line(
        r#"data: {"type":"tool_output_delta","tool_use_id":"tool-1","stream":"progress","chunk":"50%"}"#,
        &mut state,
    );
    match &delta[0] {
        StreamEvent::ToolOutputDelta {
            tool_use_id,
            stream,
            chunk,
        } => {
            assert_eq!(tool_use_id, "tool-1");
            assert_eq!(*stream, "progress");
            assert_eq!(chunk, "50%");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn daemon_stream_parser_keeps_legacy_content_and_tool_shape() {
    let mut state = DaemonStreamState::default();

    let events = daemon_stream_events_from_sse_line(
        r#"data: {"content":"hello","tool":"file_read","input":{"path":"README.md"}}"#,
        &mut state,
    );

    match &events[0] {
        StreamEvent::TextDelta { text } => assert_eq!(text, "hello"),
        other => panic!("unexpected event: {other:?}"),
    }
    match &events[1] {
        StreamEvent::ToolUseEnd { id, name, input } => {
            assert!(id.is_empty());
            assert_eq!(name, "file_read");
            assert_eq!(input["path"], "README.md");
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn daemon_stream_parser_accumulates_usage_without_terminating_stream() {
    let mut state = DaemonStreamState::default();

    let first = daemon_stream_events_from_sse_line(
        r#"data: {"done":true,"usage":{"input_tokens":3,"output_tokens":5}}"#,
        &mut state,
    );
    let second = daemon_stream_events_from_sse_line(
        r#"data: {"done":true,"usage":{"input_tokens":7,"output_tokens":11}}"#,
        &mut state,
    );

    match &first[0] {
        StreamEvent::ContentComplete { usage, .. } => {
            assert_eq!(usage.input_tokens, 3);
            assert_eq!(usage.output_tokens, 5);
        }
        other => panic!("unexpected event: {other:?}"),
    }
    match &second[0] {
        StreamEvent::ContentComplete { usage, .. } => {
            assert_eq!(usage.input_tokens, 10);
            assert_eq!(usage.output_tokens, 16);
        }
        other => panic!("unexpected event: {other:?}"),
    }
    let total = state.total_usage();
    assert_eq!(total.input_tokens, 10);
    assert_eq!(total.output_tokens, 16);
}

#[test]
fn daemon_stream_parser_maps_ask_user_with_options() {
    // Regression: without this arm, ask_user fell through to the generic
    // "content"/"tool"/"done" checks, matched none of them, and was
    // silently dropped — the daemon-mode TUI never learned a question was
    // pending and the tool call hung until the 300s timeout.
    let mut state = DaemonStreamState::default();

    let events = daemon_stream_events_from_sse_line(
        r#"data: {"type":"ask_user","question":"Couleur ?","options":["bleu","rouge"]}"#,
        &mut state,
    );

    match &events[0] {
        StreamEvent::AskUser { question, options } => {
            assert_eq!(question, "Couleur ?");
            assert_eq!(
                options.as_deref(),
                Some(["bleu".to_string(), "rouge".to_string()].as_slice())
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn daemon_stream_parser_maps_ask_user_without_options() {
    let mut state = DaemonStreamState::default();

    let events = daemon_stream_events_from_sse_line(
        r#"data: {"type":"ask_user","question":"Continue ?","options":null}"#,
        &mut state,
    );

    match &events[0] {
        StreamEvent::AskUser { question, options } => {
            assert_eq!(question, "Continue ?");
            assert!(options.is_none());
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

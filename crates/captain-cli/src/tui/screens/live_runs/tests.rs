use super::render::visible_tail_lines;
use super::*;
use crate::i18n::Lang;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn run(id: &str, status: ToolRunStatus, cancellable: bool) -> OperatorToolRun {
    OperatorToolRun {
        run_id: id.to_string(),
        tool_name: "shell_exec".to_string(),
        status,
        detached: true,
        cancellable,
        started_at_unix_ms: 1,
        finished_at_unix_ms: None,
        elapsed_ms: 61_000,
        caller_agent_id: Some("captain".to_string()),
        origin_tool_use_id: Some("tool-use".to_string()),
        input_sha256: Some("a".repeat(64)),
        retry_of_run_id: None,
        retry_attempt: 0,
        is_error: None,
        result_available: false,
        result_truncated: false,
        output_available: true,
        output_stored_bytes: Some(12),
        output_total_bytes: Some(12),
        output_sha256: Some("b".repeat(64)),
        output_capped: false,
        output_redacted: true,
    }
}

fn tail(id: &str, content: &str) -> OperatorToolRunTail {
    OperatorToolRunTail {
        run_id: id.to_string(),
        status: ToolRunStatus::Running,
        start_line: 1,
        end_line: 2,
        total_lines: 2,
        content: content.to_string(),
        content_bytes: content.len(),
        content_truncated: false,
        content_withheld: false,
        sanitized: true,
    }
}

#[test]
fn refresh_preserves_selected_run_across_reordering() {
    let mut state = LiveRunsState::new();
    state.apply_runs(Ok(vec![
        run("toolrun-a", ToolRunStatus::Completed, false),
        run("toolrun-b", ToolRunStatus::Running, true),
    ]));
    state.list_state.select(Some(1));

    let selected = state.apply_runs(Ok(vec![
        run("toolrun-b", ToolRunStatus::Running, true),
        run("toolrun-a", ToolRunStatus::Completed, false),
    ]));

    assert_eq!(selected.as_deref(), Some("toolrun-b"));
    assert_eq!(state.selected_run().unwrap().run_id, "toolrun-b");
}

#[test]
fn stale_tail_cannot_replace_current_selection() {
    let mut state = LiveRunsState::new();
    state.apply_runs(Ok(vec![
        run("toolrun-a", ToolRunStatus::Running, true),
        run("toolrun-b", ToolRunStatus::Running, true),
    ]));
    assert!(state.begin_tail_load("toolrun-a"));
    assert_eq!(
        state.handle_key(KeyEvent::from(KeyCode::Down)),
        LiveRunsAction::LoadTail("toolrun-b".to_string())
    );
    assert!(state.begin_tail_load("toolrun-b"));

    state.apply_tail("toolrun-a", Ok(tail("toolrun-a", "stale")));

    assert!(state.tail.is_none());
    assert_eq!(state.loading_tail_for.as_deref(), Some("toolrun-b"));
}

#[test]
fn failed_tail_refresh_clears_previously_rendered_output() {
    let mut state = LiveRunsState::new();
    state.apply_runs(Ok(vec![run("toolrun-a", ToolRunStatus::Running, true)]));
    assert!(state.begin_tail_load("toolrun-a"));
    state.apply_tail("toolrun-a", Ok(tail("toolrun-a", "old output")));
    assert!(state.tail.is_some());

    assert!(state.begin_tail_load("toolrun-a"));
    state.apply_tail(
        "toolrun-a",
        Err("integrity verification failed".to_string()),
    );

    assert!(state.tail.is_none());
    assert!(state.error.contains("integrity"));
}

#[test]
fn filters_are_local_and_keep_only_matching_statuses() {
    let mut state = LiveRunsState::new();
    state.apply_runs(Ok(vec![
        run("toolrun-running", ToolRunStatus::Running, true),
        run("toolrun-failed", ToolRunStatus::Failed, false),
    ]));

    state.handle_key(KeyEvent::from(KeyCode::Right));
    assert_eq!(state.filter, RunFilter::Running);
    assert_eq!(state.selected_run().unwrap().run_id, "toolrun-running");
    state.handle_key(KeyEvent::from(KeyCode::Right));
    assert_eq!(state.filter, RunFilter::Failed);
    assert_eq!(state.selected_run().unwrap().run_id, "toolrun-failed");
}

#[test]
fn cancellation_requires_arm_then_confirmation_and_rejects_non_cancellable_runs() {
    let mut state = LiveRunsState::new();
    state.apply_runs(Ok(vec![run("toolrun-live", ToolRunStatus::Running, true)]));

    assert_eq!(
        state.handle_key(KeyEvent::from(KeyCode::Char('x'))),
        LiveRunsAction::Continue
    );
    assert!(state.confirm_cancel_for.is_some());
    assert_eq!(
        state.handle_key(KeyEvent::from(KeyCode::Char('y'))),
        LiveRunsAction::Cancel("toolrun-live".to_string())
    );
    assert!(state.cancelling_for.is_some());

    let mut blocked = LiveRunsState::new();
    blocked.apply_runs(Ok(vec![run(
        "toolrun-foreground",
        ToolRunStatus::Running,
        false,
    )]));
    blocked.handle_key(KeyEvent::from(KeyCode::Char('x')));
    assert!(blocked.confirm_cancel_for.is_none());
    assert!(blocked.error.contains("active run"));
}

#[test]
fn cancellation_response_survives_a_terminal_poll_race() {
    let mut state = LiveRunsState::new();
    state.apply_runs(Ok(vec![run("toolrun-live", ToolRunStatus::Running, true)]));
    state.handle_key(KeyEvent::from(KeyCode::Char('x')));
    assert_eq!(
        state.handle_key(KeyEvent::from(KeyCode::Char('y'))),
        LiveRunsAction::Cancel("toolrun-live".to_string())
    );

    state.apply_runs(Ok(vec![run(
        "toolrun-live",
        ToolRunStatus::Cancelled,
        false,
    )]));
    let applied = state.apply_cancel(
        "toolrun-live",
        Ok(run("toolrun-live", ToolRunStatus::Cancelled, false)),
    );

    assert_eq!(applied.as_deref(), Some("toolrun-live"));
    assert!(state.cancelling_for.is_none());
}

#[test]
fn maximum_tail_scroll_still_keeps_the_first_line_visible() {
    let lines = vec![
        "first".to_string(),
        "second".to_string(),
        "third".to_string(),
    ];
    assert_eq!(visible_tail_lines(&lines, 2, usize::MAX), vec!["first"]);
}

#[test]
fn desktop_and_compact_layouts_render_without_control_characters() {
    for (width, height) in [(118, 36), (60, 20)] {
        let mut state = LiveRunsState::new();
        state.apply_runs(Ok(vec![run("toolrun-live", ToolRunStatus::Running, true)]));
        state.items[0].tool_name = "shell\u{1b}_exec".to_string();
        assert!(state.begin_tail_load("toolrun-live"));
        state.apply_tail(
            "toolrun-live",
            Ok(tail("toolrun-live", "line one\nsecret-free\u{1b}tail")),
        );
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw(frame, frame.area(), &mut state))
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("shell"));
        assert!(rendered.contains("Live Runs"));
        assert!(!rendered.contains('\u{1b}'));
    }
}

#[test]
fn standalone_summary_is_bounded_to_metadata_only() {
    let items = (0..15)
        .map(|index| run(&format!("toolrun-{index}"), ToolRunStatus::Completed, false))
        .collect::<Vec<_>>();
    let message = chat_runs_message(&items, Lang::En);
    assert!(message.contains("... 3 more"));
    assert!(message.contains("Control Web"));
    assert!(!message.contains("input_sha256"));
    assert_eq!(
        message
            .lines()
            .filter(|line| line.starts_with("- `"))
            .count(),
        12
    );
}

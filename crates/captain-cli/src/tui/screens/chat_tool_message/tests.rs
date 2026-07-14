use super::*;

fn tool_info(name: &str) -> ToolInfo {
    ToolInfo {
        id: "tool-1".to_string(),
        name: name.to_string(),
        input: String::new(),
        result: String::new(),
        stdout: String::new(),
        stderr: String::new(),
        is_error: false,
        status: ToolStatus::Success,
        started_at: None,
        completed_at: None,
        duration_ms: Some(2_250),
        expanded: false,
    }
}

fn render_text(info: &ToolInfo, show_copy_button: bool) -> Vec<String> {
    let mut lines = Vec::new();
    render_tool_message(&mut lines, info, 90, 0, show_copy_button);
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn collapsed_tool_renders_summary_duration_and_output_counts() {
    let mut info = tool_info("shell_exec");
    info.input = r#"{"command":"cargo test -p captain-cli"}"#.to_string();
    info.stdout = "ok\nstill ok".to_string();

    let lines = render_text(&info, false);

    assert_eq!(lines.first().map(String::as_str), Some(""));
    assert!(lines.iter().any(|line| line.contains("shell_exec")));
    assert!(lines
        .iter()
        .any(|line| line.contains("cargo test -p captain-cli")));
    assert!(lines.iter().any(|line| line.contains("2.2s")));
    assert!(lines
        .iter()
        .any(|line| line.contains("done · stdout 2 lines")));
}

#[test]
fn collapsed_tool_renders_copy_badge_when_allowed() {
    let mut info = tool_info("shell_exec");
    info.input = r#"{"command":"cargo test"}"#.to_string();

    let lines = render_text(&info, true);

    assert!(lines.iter().any(|line| line.contains("[copy]")));
}

#[test]
fn collapsed_helpers_account_for_copy_badge_and_missing_duration() {
    let mut info = tool_info("shell_exec");
    info.duration_ms = None;

    assert!(collapsed_summary_width(90, true) < collapsed_summary_width(90, false));
    assert_eq!(collapsed_tool_duration(&info), "done");
}

#[test]
fn collapsed_output_line_is_absent_without_output() {
    let info = tool_info("shell_exec");
    let (_, status_label, status_style) = tool_status_parts(&info, None);

    assert!(collapsed_tool_output_line(&info, 90, status_label, status_style).is_none());
}

#[test]
fn running_tool_uses_expanded_renderer_even_when_not_explicitly_expanded() {
    let mut info = tool_info("browser-use");
    info.status = ToolStatus::Running;
    info.started_at = Some(std::time::Instant::now());
    info.stdout = "opened".to_string();

    let lines = render_text(&info, false);

    assert!(lines.iter().any(|line| line.contains("running")));
    assert!(lines.iter().any(|line| line.contains("activity · 1 lines")));
}

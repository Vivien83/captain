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
        duration_ms: Some(1_250),
        expanded: true,
    }
}

fn render_text(info: &ToolInfo, show_copy_button: bool) -> Vec<String> {
    let mut lines = Vec::new();
    render_tool_expanded(&mut lines, info, 80, 0, show_copy_button);
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
fn browser_stdout_is_labeled_activity() {
    let mut info = tool_info("browser-use");
    info.stdout = "opened page\nclicked button".to_string();

    let lines = render_text(&info, false);

    assert!(lines.iter().any(|line| line.contains("activity · 2 lines")));
    assert!(!lines.iter().any(|line| line.contains("stdout ·")));
}

#[test]
fn shell_command_gets_copy_badge_and_command_detail() {
    let mut info = tool_info("shell_exec");
    info.input = r#"{"command":"cargo test -p captain-cli"}"#.to_string();

    let lines = render_text(&info, true);

    assert!(lines.first().is_some_and(|line| line.contains("[copy]")));
    assert!(lines
        .iter()
        .any(|line| line.contains("$ cargo test -p captain-cli")));
}

#[test]
fn result_body_is_bounded_when_no_streams_exist() {
    let mut info = tool_info("tool");
    info.result = "one\ntwo\nthree\nfour\nfive".to_string();

    let lines = render_text(&info, false);

    assert!(lines.iter().any(|line| line.contains("result ")));
    assert!(lines.iter().any(|line| line.contains("  …")));
    assert!(!lines.iter().any(|line| line.contains("five")));
}

#[test]
fn edit_file_input_renders_diff_detail() {
    let mut info = tool_info("edit_file");
    info.input = serde_json::json!({
        "path": "src/lib.rs",
        "old_string": "old",
        "new_string": "new"
    })
    .to_string();

    let lines = render_text(&info, false);

    assert!(lines.iter().any(|line| line.contains("src/lib.rs")));
    assert!(lines.iter().any(|line| line.contains("old")));
    assert!(lines.iter().any(|line| line.contains("new")));
}

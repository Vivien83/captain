use super::*;
use crate::tui::screens::chat::{ToolInfo, ToolStatus};

fn line_texts(lines: &[Line<'static>]) -> Vec<String> {
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
        duration_ms: Some(120),
        expanded: false,
    }
}

#[test]
fn user_messages_wrap_with_prompt_prefix_once() {
    let messages = vec![ChatMessage {
        role: Role::User,
        text: "alpha beta gamma delta".to_string(),
        tool: None,
    }];
    let mut lines = Vec::new();
    let mut zones = Vec::new();

    push_message_history_lines(&mut lines, &mut zones, &messages, 15, 0, false);
    let text = line_texts(&lines);

    assert_eq!(text[0], "");
    assert!(text[1].starts_with("  \u{258c}❯ "));
    assert!(text
        .iter()
        .skip(2)
        .all(|line| line.starts_with("  \u{258c}  ")));
    assert!(zones.is_empty());
}

#[test]
fn user_messages_render_markdown_like_agent_messages() {
    // A pasted structured prompt (headers, bold) used to show up as a flat
    // plain-text block with literal `#`/`**` characters. It must now render
    // through the same markdown path as agent replies.
    let messages = vec![ChatMessage {
        role: Role::User,
        text: "**bold** plain".to_string(),
        tool: None,
    }];
    let mut lines = Vec::new();
    let mut zones = Vec::new();

    push_message_history_lines(&mut lines, &mut zones, &messages, 80, 0, false);
    let text = line_texts(&lines);

    assert!(!text.iter().any(|line| line.contains("**")));
    assert!(text.iter().any(|line| line.contains("bold plain")));

    let bold_line = lines
        .iter()
        .find(|line| line.spans.iter().any(|s| s.content.contains("bold")))
        .expect("line with 'bold' span");
    let bold_span = bold_line
        .spans
        .iter()
        .find(|s| s.content.contains("bold"))
        .unwrap();
    use ratatui::style::Modifier;
    assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
}

#[test]
fn system_messages_keep_each_line_dimmed() {
    let messages = vec![ChatMessage {
        role: Role::System,
        text: "first\nsecond".to_string(),
        tool: None,
    }];
    let mut lines = Vec::new();
    let mut zones = Vec::new();

    push_message_history_lines(&mut lines, &mut zones, &messages, 80, 0, false);
    let text = line_texts(&lines);

    assert_eq!(text, vec!["  first".to_string(), "  second".to_string()]);
    assert!(zones.is_empty());
}

#[test]
fn tool_messages_register_click_zone_metadata() {
    let mut info = tool_info("shell_exec");
    info.input = r#"{"command":"cargo test"}"#.to_string();
    let messages = vec![ChatMessage {
        role: Role::Tool,
        text: String::new(),
        tool: Some(info),
    }];
    let mut lines = vec![Line::from("before")];
    let mut zones = Vec::new();

    push_message_history_lines(&mut lines, &mut zones, &messages, 80, 0, true);

    assert_eq!(zones.len(), 1);
    assert_eq!(zones[0].line_idx, 2);
    assert_eq!(zones[0].message_idx, 0);
    assert!(zones[0].can_toggle);
    assert!(zones[0].can_copy);
    assert!(!zones[0].expanded);
}

#[test]
fn tool_without_metadata_uses_legacy_fallback_line() {
    let messages = vec![ChatMessage {
        role: Role::Tool,
        text: "legacy output".to_string(),
        tool: None,
    }];
    let mut lines = Vec::new();
    let mut zones = Vec::new();

    push_message_history_lines(&mut lines, &mut zones, &messages, 80, 0, true);
    let text = line_texts(&lines);

    assert_eq!(text, vec!["  ✔ legacy output".to_string()]);
    assert!(zones.is_empty());
}

/// User turns must be visually distinct from agent output: accent bar on
/// every line and a card background on the text — they used to blend in,
/// making sessions hard to re-read.
#[test]
fn user_messages_render_as_distinct_block() {
    let messages = vec![ChatMessage {
        role: Role::User,
        text: "premiere ligne\nseconde ligne".to_string(),
        tool: None,
    }];
    let mut lines = Vec::new();
    let mut zones = Vec::new();

    push_message_history_lines(&mut lines, &mut zones, &messages, 80, 0, false);

    let content_lines: Vec<&Line<'static>> = lines
        .iter()
        .filter(|l| l.spans.iter().any(|s| s.content.contains("ligne")))
        .collect();
    assert_eq!(content_lines.len(), 2, "pasted lines stay separate");
    for line in &content_lines {
        assert!(
            line.spans
                .first()
                .is_some_and(|s| s.content.contains('\u{258c}')),
            "every user line carries the accent bar"
        );
        assert!(
            line.spans
                .iter()
                .filter(|s| s.content.contains("ligne"))
                .all(|s| s.style.bg == Some(crate::tui::theme::BG_CARD)),
            "user text carries the card background"
        );
    }
}

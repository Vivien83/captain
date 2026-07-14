use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn summary() -> SessionSummary {
    SessionSummary {
        agent_key: "default-agent".into(),
        session_id: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into()),
        label: "Captain audit".into(),
        agent_name: "Captain".into(),
        model_label: "codex/gpt-5".into(),
        path: PathBuf::from("session.json"),
        updated_at: 1_000,
        message_count: 7,
        session_input_tokens: 1200,
        session_output_tokens: 345,
    }
}

fn line_text(line: &Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn format_age_uses_compact_french_labels() {
    assert_eq!(format_age(0), "à l'instant");
    assert_eq!(format_age(59), "à l'instant");
    assert_eq!(format_age(60), "il y a 1 min");
    assert_eq!(format_age(3600), "il y a 1 h");
    assert_eq!(format_age(86_400), "il y a 1 j");
}

#[test]
fn session_row_uses_display_name_token_total_and_age() {
    let line = session_picker_row(&summary(), true, 1_120);
    let text = line_text(&line);

    assert!(text.starts_with("▶ Captain audit"));
    assert!(text.contains("   7 msg"));
    assert!(text.contains(" 1545 tok"));
    assert!(text.contains("il y a 2 min"));
}

#[test]
fn session_row_falls_back_to_agent_key() {
    let mut session = summary();
    session.label.clear();
    session.agent_name.clear();

    let text = line_text(&session_picker_row(&session, false, 1_000));

    assert!(text.starts_with("  default-agent"));
    assert!(text.contains("à l'instant"));
}

#[test]
fn session_picker_key_action_maps_navigation_and_selection() {
    let cases = [
        (KeyCode::Esc, SessionPickerKeyAction::Close),
        (KeyCode::Up, SessionPickerKeyAction::Up),
        (KeyCode::Down, SessionPickerKeyAction::Down),
        (KeyCode::Enter, SessionPickerKeyAction::Select),
        (KeyCode::Char('x'), SessionPickerKeyAction::Continue),
    ];

    for (code, expected) in cases {
        assert_eq!(session_picker_key_action_for_key(key(code)), expected);
    }
}

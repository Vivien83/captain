use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn slash_filtered_returns_prefix_matches_in_command_order() {
    assert_eq!(slash_filtered("/co"), vec!["/cost", "/copy", "/config"]);
}

#[test]
fn slash_filtered_default_promotes_core_commands_only() {
    let matches = slash_filtered("/");

    assert!(matches.contains(&"/projects"));
    assert!(matches.contains(&"/automation"));
    assert!(matches.contains(&"/learning"));
    assert!(matches.contains(&"/capabilities"));
    assert!(matches.contains(&"/dashboard"));
    assert!(!matches.contains(&"/agents"));
    assert!(!matches.contains(&"/sessions"));
    assert!(!matches.contains(&"/settings"));
    assert!(!matches.contains(&"/logs"));
}

#[test]
fn slash_filtered_keeps_expert_commands_accessible_by_prefix() {
    assert_eq!(slash_filtered("/ag"), vec!["/agents"]);
    assert_eq!(slash_filtered("/ch"), vec!["/channels"]);
    assert_eq!(slash_filtered("/set"), vec!["/settings"]);
    assert_eq!(slash_filtered("/log"), vec!["/logs"]);
}

#[test]
fn slash_filtered_returns_empty_for_unknown_prefix() {
    assert!(slash_filtered("/does-not-exist").is_empty());
}

#[test]
fn longest_common_prefix_extends_ambiguous_matches() {
    let matches = slash_filtered("/s");
    assert_eq!(longest_common_prefix(&matches), "/s");

    let matches = slash_filtered("/sh");
    assert_eq!(longest_common_prefix(&matches), "/shutdown");
}

#[test]
fn longest_common_prefix_handles_empty_input() {
    assert_eq!(longest_common_prefix(&[]), "");
}

#[test]
fn slash_command_hint_covers_core_commands() {
    assert_eq!(slash_command_hint("/status"), "état du daemon");
    assert_eq!(slash_command_hint("/unknown"), "");
}

#[test]
fn slash_picker_popup_rejects_empty_tiny_or_occluded_views() {
    let input_area = Rect::new(4, 12, 80, 3);

    assert_eq!(slash_picker_popup(input_area, 0), None);
    assert_eq!(slash_picker_popup(Rect::new(4, 12, 10, 3), 3), None);
    assert_eq!(slash_picker_popup(Rect::new(4, 2, 80, 3), 8), None);
    assert_eq!(
        slash_picker_popup(input_area, 12),
        Some(Rect::new(4, 4, 48, 8))
    );
}

#[test]
fn slash_picker_visible_range_keeps_selected_row_visible() {
    assert_eq!(slash_picker_visible_range(20, 1, 8), 0..8);
    assert_eq!(slash_picker_visible_range(20, 10, 8), 3..11);
    assert_eq!(slash_picker_visible_range(5, 20, 8), 0..5);
}

#[test]
fn slash_picker_row_marks_selection_and_pads_width() {
    let line = slash_picker_row("/status", true, 32);
    let text = line
        .spans
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect::<String>();

    assert!(text.starts_with("▶ /status"));
    assert_eq!(text.chars().count(), 32);
}

#[test]
fn slash_picker_key_action_maps_live_picker_controls() {
    let cases = [
        (KeyCode::Up, SlashPickerKeyAction::Up),
        (KeyCode::Down, SlashPickerKeyAction::Down),
        (KeyCode::Esc, SlashPickerKeyAction::Cancel),
        (KeyCode::Enter, SlashPickerKeyAction::Select),
        (KeyCode::Char('x'), SlashPickerKeyAction::Continue),
    ];

    for (code, expected) in cases {
        assert_eq!(slash_picker_key_action_for_key(key(code)), expected);
    }
}

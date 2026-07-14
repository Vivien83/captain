use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn model(id: &str, display_name: &str, tier: &str) -> ModelEntry {
    ModelEntry {
        id: id.into(),
        display_name: display_name.into(),
        provider: "test".into(),
        tier: tier.into(),
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
fn model_row_prefers_display_name_and_lowercases_tier() {
    let line = model_picker_row(&model("openai/gpt-5", "GPT 5", "Frontier"), true, 20);

    assert_eq!(line_text(&line), "▶ GPT 5 frontier");
}

#[test]
fn model_row_falls_back_to_model_id() {
    let line = model_picker_row(&model("anthropic/claude", "", "smart"), false, 30);

    assert_eq!(line_text(&line), "  anthropic/claude smart");
}

#[test]
fn model_row_truncates_long_name_to_available_width() {
    let line = model_picker_row(
        &model("id", "very-long-model-display-name", "balanced"),
        false,
        10,
    );

    assert_eq!(line_text(&line), "  very-long… balanced");
}

#[test]
fn scroll_start_keeps_selected_row_visible() {
    assert_eq!(model_picker_scroll_start(0, 5), 0);
    assert_eq!(model_picker_scroll_start(4, 5), 0);
    assert_eq!(model_picker_scroll_start(5, 5), 1);
    assert_eq!(model_picker_scroll_start(9, 5), 5);
}

#[test]
fn popup_area_rejects_tiny_view_and_centers_picker() {
    assert!(model_picker_popup_area(Rect::new(0, 0, 19, 10), 3).is_none());
    assert!(model_picker_popup_area(Rect::new(0, 0, 40, 5), 3).is_none());

    let popup = model_picker_popup_area(Rect::new(10, 20, 80, 30), 4).expect("popup");

    assert_eq!(popup.width, 54);
    assert_eq!(popup.height, 8);
    assert_eq!(popup.x, 23);
    assert_eq!(popup.y, 31);
}

#[test]
fn model_picker_rows_keep_selected_entry_visible() {
    let entries = vec![
        model("one", "One", "fast"),
        model("two", "Two", "smart"),
        model("three", "Three", "balanced"),
    ];
    let refs = entries.iter().collect::<Vec<_>>();

    let rows = model_picker_rows(&refs, 2, 2, 30);

    assert_eq!(rows.len(), 2);
    assert_eq!(line_text(&rows[0]), "  Two smart");
    assert_eq!(line_text(&rows[1]), "▶ Three balanced");
}

#[test]
fn model_picker_key_action_maps_picker_controls() {
    let cases = [
        (KeyCode::Esc, ModelPickerKeyAction::Close),
        (KeyCode::Up, ModelPickerKeyAction::Up),
        (KeyCode::Down, ModelPickerKeyAction::Down),
        (KeyCode::Enter, ModelPickerKeyAction::Select),
        (KeyCode::Backspace, ModelPickerKeyAction::Backspace),
        (KeyCode::Char('g'), ModelPickerKeyAction::Insert('g')),
        (KeyCode::Tab, ModelPickerKeyAction::Continue),
    ];

    for (code, expected) in cases {
        assert_eq!(model_picker_key_action_for_key(key(code)), expected);
    }
}

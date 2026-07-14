use super::*;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn global_key_action_maps_exit_and_model_picker_close() {
    assert_eq!(
        global_key_action_for_key(ctrl_key(KeyCode::Char('c')), false, false),
        GlobalKeyAction::Back
    );
    assert_eq!(
        global_key_action_for_key(ctrl_key(KeyCode::Char('c')), true, false),
        GlobalKeyAction::CloseModelPicker
    );
    assert_eq!(
        global_key_action_for_key(ctrl_key(KeyCode::Char('d')), false, false),
        GlobalKeyAction::Back
    );
}

#[test]
fn global_key_action_maps_readline_and_completion_controls() {
    let cases = [
        (KeyCode::Char('l'), GlobalKeyAction::ResetChat),
        (KeyCode::Char('u'), GlobalKeyAction::ClearInput),
        (KeyCode::Char('w'), GlobalKeyAction::DeleteWordBeforeCursor),
        (KeyCode::Char('t'), GlobalKeyAction::ToggleThinking),
        (KeyCode::Char('e'), GlobalKeyAction::ToggleLatestTool),
        (KeyCode::Char('o'), GlobalKeyAction::ToggleSessionPicker),
    ];

    for (code, expected) in cases {
        assert_eq!(
            global_key_action_for_key(ctrl_key(code), false, false),
            expected
        );
    }
    assert_eq!(
        global_key_action_for_key(key(KeyCode::Tab), false, false),
        GlobalKeyAction::CompleteSlash
    );
    assert_eq!(
        global_key_action_for_key(key(KeyCode::Tab), true, false),
        GlobalKeyAction::Continue
    );
}

#[test]
fn global_key_action_maps_model_picker_toggle_states() {
    assert_eq!(
        global_key_action_for_key(ctrl_key(KeyCode::Char('m')), false, false),
        GlobalKeyAction::OpenModelPicker
    );
    assert_eq!(
        global_key_action_for_key(ctrl_key(KeyCode::Char('m')), true, false),
        GlobalKeyAction::CloseModelPicker
    );
    assert_eq!(
        global_key_action_for_key(ctrl_key(KeyCode::Char('m')), false, true),
        GlobalKeyAction::Noop
    );
    assert_eq!(
        global_key_action_for_key(ctrl_key(KeyCode::Char('m')), true, true),
        GlobalKeyAction::Noop
    );
}

#[test]
fn global_key_action_ignores_plain_input_keys() {
    assert_eq!(
        global_key_action_for_key(key(KeyCode::Char('x')), false, false),
        GlobalKeyAction::Continue
    );
    assert_eq!(
        global_key_action_for_key(key(KeyCode::Enter), false, false),
        GlobalKeyAction::Continue
    );
}

#[test]
fn streaming_key_action_maps_exit_and_staging() {
    assert_eq!(
        streaming_key_action_for_key(key(KeyCode::Esc)),
        StreamingKeyAction::Back
    );
    assert_eq!(
        streaming_key_action_for_key(key(KeyCode::Enter)),
        StreamingKeyAction::StageInput
    );
}

#[test]
fn streaming_key_action_maps_scroll_keys() {
    assert_eq!(
        streaming_key_action_for_key(ctrl_key(KeyCode::Char('b'))),
        StreamingKeyAction::ScrollPageUp
    );
    assert_eq!(
        streaming_key_action_for_key(ctrl_key(KeyCode::Char('f'))),
        StreamingKeyAction::ScrollPageDown
    );
    assert_eq!(
        streaming_key_action_for_key(key(KeyCode::PageUp)),
        StreamingKeyAction::PageUp
    );
    assert_eq!(
        streaming_key_action_for_key(key(KeyCode::PageDown)),
        StreamingKeyAction::PageDown
    );
}

#[test]
fn streaming_key_action_maps_input_editing() {
    let cases = [
        (KeyCode::Char('x'), StreamingKeyAction::Insert('x')),
        (KeyCode::Backspace, StreamingKeyAction::Backspace),
        (KeyCode::Delete, StreamingKeyAction::Delete),
        (KeyCode::Left, StreamingKeyAction::Left),
        (KeyCode::Right, StreamingKeyAction::Right),
        (KeyCode::Home, StreamingKeyAction::Home),
        (KeyCode::End, StreamingKeyAction::End),
        (KeyCode::Up, StreamingKeyAction::Up),
        (KeyCode::Down, StreamingKeyAction::Down),
        (KeyCode::Tab, StreamingKeyAction::Continue),
    ];

    for (code, expected) in cases {
        assert_eq!(streaming_key_action_for_key(key(code)), expected);
    }
}

#[test]
fn input_key_action_maps_submit_and_newline_variants() {
    assert_eq!(
        input_key_action_for_key(key(KeyCode::Enter)),
        InputKeyAction::Submit
    );
    assert_eq!(
        input_key_action_for_key(modified_key(KeyCode::Enter, KeyModifiers::SHIFT)),
        InputKeyAction::InsertNewline
    );
    assert_eq!(
        input_key_action_for_key(modified_key(KeyCode::Enter, KeyModifiers::ALT)),
        InputKeyAction::InsertNewline
    );
}

#[test]
fn input_key_action_maps_exit_and_scroll_keys() {
    assert_eq!(
        input_key_action_for_key(key(KeyCode::Esc)),
        InputKeyAction::Back
    );
    assert_eq!(
        input_key_action_for_key(ctrl_key(KeyCode::Char('b'))),
        InputKeyAction::ScrollPageUp
    );
    assert_eq!(
        input_key_action_for_key(ctrl_key(KeyCode::Char('f'))),
        InputKeyAction::ScrollPageDown
    );
    assert_eq!(
        input_key_action_for_key(key(KeyCode::PageUp)),
        InputKeyAction::PageUp
    );
    assert_eq!(
        input_key_action_for_key(key(KeyCode::PageDown)),
        InputKeyAction::PageDown
    );
}

#[test]
fn input_key_action_maps_editing_and_navigation() {
    let cases = [
        (KeyCode::Char('x'), InputKeyAction::Insert('x')),
        (KeyCode::Backspace, InputKeyAction::Backspace),
        (KeyCode::Delete, InputKeyAction::Delete),
        (KeyCode::Left, InputKeyAction::Left),
        (KeyCode::Right, InputKeyAction::Right),
        (KeyCode::Home, InputKeyAction::Home),
        (KeyCode::End, InputKeyAction::End),
        (KeyCode::Up, InputKeyAction::Up),
        (KeyCode::Down, InputKeyAction::Down),
        (KeyCode::Tab, InputKeyAction::Continue),
    ];

    for (code, expected) in cases {
        assert_eq!(input_key_action_for_key(key(code)), expected);
    }
}

/// Live bug: a terminal that doesn't deliver a Cmd+V paste as one
/// bracketed Paste event instead reports each embedded newline byte as a
/// raw key event — and since LF (0x0A) and CR (0x0D) are the C0 control
/// codes for Ctrl+J and Ctrl+M, that's what crossterm decodes them as.
/// Every newline in the pasted prompt turned into a literal 'j'.
#[test]
fn ctrl_j_and_ctrl_m_insert_newline_not_the_bare_letter() {
    assert_eq!(
        input_key_action_for_key(ctrl_key(KeyCode::Char('j'))),
        InputKeyAction::InsertNewline
    );
    assert_eq!(
        input_key_action_for_key(ctrl_key(KeyCode::Char('m'))),
        InputKeyAction::InsertNewline
    );
}

/// Any other unhandled Ctrl+<letter> must not silently drop the modifier
/// and insert the bare letter — same failure class, just less visible.
#[test]
fn unhandled_ctrl_chars_are_not_inserted_as_bare_letters() {
    for c in ['a', 'z', 'j'.to_ascii_uppercase(), 'q'] {
        let action = input_key_action_for_key(ctrl_key(KeyCode::Char(c)));
        assert_ne!(
            action,
            InputKeyAction::Insert(c),
            "Ctrl+{c} must not insert the bare character"
        );
    }
}

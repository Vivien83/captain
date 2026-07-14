//! Keyboard decision helpers for chat modes.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GlobalKeyAction {
    Back,
    CloseModelPicker,
    ResetChat,
    ClearInput,
    DeleteWordBeforeCursor,
    CompleteSlash,
    ToggleThinking,
    ToggleLatestTool,
    OpenModelPicker,
    ToggleSessionPicker,
    Noop,
    Continue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StreamingKeyAction {
    Back,
    StageInput,
    ScrollPageUp,
    ScrollPageDown,
    Insert(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    PageUp,
    PageDown,
    Continue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputKeyAction {
    Back,
    Submit,
    InsertNewline,
    ScrollPageUp,
    ScrollPageDown,
    Insert(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Up,
    Down,
    PageUp,
    PageDown,
    Continue,
}

pub(super) fn global_key_action_for_key(
    key: KeyEvent,
    show_model_picker: bool,
    is_streaming: bool,
) -> GlobalKeyAction {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if show_model_picker {
                GlobalKeyAction::CloseModelPicker
            } else {
                GlobalKeyAction::Back
            }
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            GlobalKeyAction::Back
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            GlobalKeyAction::ResetChat
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            GlobalKeyAction::ClearInput
        }
        KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            GlobalKeyAction::DeleteWordBeforeCursor
        }
        KeyCode::Tab if !show_model_picker => GlobalKeyAction::CompleteSlash,
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            GlobalKeyAction::ToggleThinking
        }
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            GlobalKeyAction::ToggleLatestTool
        }
        KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if is_streaming {
                GlobalKeyAction::Noop
            } else if show_model_picker {
                GlobalKeyAction::CloseModelPicker
            } else {
                GlobalKeyAction::OpenModelPicker
            }
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            GlobalKeyAction::ToggleSessionPicker
        }
        _ => GlobalKeyAction::Continue,
    }
}

pub(super) fn streaming_key_action_for_key(key: KeyEvent) -> StreamingKeyAction {
    match key.code {
        KeyCode::Esc => StreamingKeyAction::Back,
        KeyCode::Enter => StreamingKeyAction::StageInput,
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            StreamingKeyAction::ScrollPageUp
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            StreamingKeyAction::ScrollPageDown
        }
        KeyCode::Char(c) => StreamingKeyAction::Insert(c),
        KeyCode::Backspace => StreamingKeyAction::Backspace,
        KeyCode::Delete => StreamingKeyAction::Delete,
        KeyCode::Left => StreamingKeyAction::Left,
        KeyCode::Right => StreamingKeyAction::Right,
        KeyCode::Home => StreamingKeyAction::Home,
        KeyCode::End => StreamingKeyAction::End,
        KeyCode::Up => StreamingKeyAction::Up,
        KeyCode::Down => StreamingKeyAction::Down,
        KeyCode::PageUp => StreamingKeyAction::PageUp,
        KeyCode::PageDown => StreamingKeyAction::PageDown,
        _ => StreamingKeyAction::Continue,
    }
}

pub(super) fn input_key_action_for_key(key: KeyEvent) -> InputKeyAction {
    match key.code {
        KeyCode::Esc => InputKeyAction::Back,
        KeyCode::Enter
            if key.modifiers.contains(KeyModifiers::SHIFT)
                || key.modifiers.contains(KeyModifiers::ALT) =>
        {
            InputKeyAction::InsertNewline
        }
        KeyCode::Enter => InputKeyAction::Submit,
        KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            InputKeyAction::ScrollPageUp
        }
        KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            InputKeyAction::ScrollPageDown
        }
        // Ctrl+J and Ctrl+M are the raw C0 control codes for LF (0x0A) and
        // CR (0x0D) — the same bytes a literal newline uses. A terminal
        // that doesn't deliver a pasted multi-line blob as one bracketed
        // Paste event (falling back to per-character key events) reports
        // each embedded newline byte this way, since crossterm can't tell
        // "Ctrl+J was pressed" from "this raw LF byte arrived" without the
        // Kitty keyboard protocol. Observed live: a Cmd+V paste of a
        // multi-line prompt turned every newline into a literal 'j'
        // (single \n) or 'jj' (blank line) in the captured message.
        KeyCode::Char('j' | 'm') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            InputKeyAction::InsertNewline
        }
        // Any other unhandled Ctrl-modified character must not silently
        // drop the modifier and insert the bare letter — same failure
        // class as above, just less visible for combos that aren't
        // control-code lookalikes.
        KeyCode::Char(_) if key.modifiers.contains(KeyModifiers::CONTROL) => {
            InputKeyAction::Continue
        }
        KeyCode::Char(c) => InputKeyAction::Insert(c),
        KeyCode::Backspace => InputKeyAction::Backspace,
        KeyCode::Delete => InputKeyAction::Delete,
        KeyCode::Left => InputKeyAction::Left,
        KeyCode::Right => InputKeyAction::Right,
        KeyCode::Home => InputKeyAction::Home,
        KeyCode::End => InputKeyAction::End,
        KeyCode::Up => InputKeyAction::Up,
        KeyCode::Down => InputKeyAction::Down,
        KeyCode::PageUp => InputKeyAction::PageUp,
        KeyCode::PageDown => InputKeyAction::PageDown,
        _ => InputKeyAction::Continue,
    }
}

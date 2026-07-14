use ratatui::crossterm::event::KeyCode;

use super::{index, line, next, prev, shortcut_action, ShortcutAction};
use crate::tui::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    One,
    Two,
    Three,
}

const ITEMS: &[Item] = &[Item::One, Item::Two, Item::Three];

#[test]
fn index_falls_back_to_first_entry_for_unknown_value() {
    assert_eq!(index(ITEMS, Item::Two), 1);
    assert_eq!(index(&[Item::One, Item::Two], Item::Three), 0);
}

#[test]
fn next_and_prev_wrap_around() {
    assert_eq!(next(ITEMS, Item::Three), Item::One);
    assert_eq!(prev(ITEMS, Item::One), Item::Three);
}

#[test]
fn shortcut_action_maps_arrow_keys_to_direction() {
    assert_eq!(
        shortcut_action(KeyCode::Left, 3),
        Some(ShortcutAction::Prev)
    );
    assert_eq!(
        shortcut_action(KeyCode::Right, 3),
        Some(ShortcutAction::Next)
    );
}

#[test]
fn shortcut_action_maps_number_keys_inside_visible_range() {
    assert_eq!(
        shortcut_action(KeyCode::Char('1'), 4),
        Some(ShortcutAction::Index(0))
    );
    assert_eq!(
        shortcut_action(KeyCode::Char('4'), 4),
        Some(ShortcutAction::Index(3))
    );
    assert_eq!(shortcut_action(KeyCode::Char('5'), 4), None);
}

#[test]
fn hub_line_includes_title_hint_and_numbered_labels() {
    let line = line("Learning", &["Review", "Memory"], 1);
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(text, " Learning  Alt+1..2 / Alt+←→  1 Review  2 Memory ");
}

#[test]
fn hub_line_marks_only_active_label() {
    let line = line("Automation", &["Workflows", "Cron"], 1);

    assert_eq!(line.spans[2].style, theme::tab_inactive());
    assert_eq!(line.spans[3].style, theme::tab_active());
}

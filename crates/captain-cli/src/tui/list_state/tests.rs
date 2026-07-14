use ratatui::widgets::ListState;

use super::*;

#[test]
fn select_first_if_non_empty_selects_zero() {
    let mut state = ListState::default().with_selected(Some(3));

    select_first_if_non_empty(&mut state, &[1, 2]);

    assert_eq!(state.selected(), Some(0));
}

#[test]
fn select_first_if_non_empty_leaves_empty_list_selection_unchanged() {
    let mut state = ListState::default().with_selected(Some(2));

    select_first_if_non_empty::<u8>(&mut state, &[]);

    assert_eq!(state.selected(), Some(2));
}

#[test]
fn select_first_if_unselected_keeps_existing_selection() {
    let mut state = ListState::default().with_selected(Some(4));

    select_first_if_unselected(&mut state, &[1, 2]);

    assert_eq!(state.selected(), Some(4));
}

#[test]
fn select_first_if_unselected_selects_zero_for_non_empty_list() {
    let mut state = ListState::default();

    select_first_if_unselected(&mut state, &[1, 2]);

    assert_eq!(state.selected(), Some(0));
}

#[test]
fn select_first_if_unselected_leaves_empty_list_unselected() {
    let mut state = ListState::default();

    select_first_if_unselected::<u8>(&mut state, &[]);

    assert_eq!(state.selected(), None);
}

use ratatui::widgets::ListState;

pub(crate) fn select_first_if_non_empty<T>(state: &mut ListState, items: &[T]) {
    if !items.is_empty() {
        state.select(Some(0));
    }
}

pub(crate) fn select_first_if_unselected<T>(state: &mut ListState, items: &[T]) {
    if state.selected().is_none() {
        select_first_if_non_empty(state, items);
    }
}

#[cfg(test)]
#[path = "list_state/tests.rs"]
mod tests;

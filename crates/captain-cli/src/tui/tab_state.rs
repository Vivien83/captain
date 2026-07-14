use super::navigation_state::{Tab, TABS};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TabSwitchState {
    pub(crate) active_tab: Tab,
    pub(crate) scroll_offset: usize,
}

pub(crate) fn next_primary_tab(current: Tab) -> Tab {
    let idx = current.index();
    let next = (idx + 1) % TABS.len();
    TABS[next]
}

pub(crate) fn previous_primary_tab(current: Tab) -> Tab {
    let idx = current.index();
    let previous = if idx == 0 { TABS.len() - 1 } else { idx - 1 };
    TABS[previous]
}

pub(crate) fn tab_scroll_offset_after_switch(current_offset: usize, target: Tab) -> usize {
    let target_index = target.index();
    if target_index < current_offset {
        target_index
    } else {
        current_offset
    }
}

pub(crate) fn tab_switch_state_after_switch(
    current_scroll_offset: usize,
    target: Tab,
) -> TabSwitchState {
    TabSwitchState {
        active_tab: target,
        scroll_offset: tab_scroll_offset_after_switch(current_scroll_offset, target),
    }
}

#[cfg(test)]
#[path = "tab_state/tests.rs"]
mod tests;

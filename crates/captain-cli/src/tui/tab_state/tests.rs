use super::*;

#[test]
fn next_primary_tab_wraps_forward_in_operational_order() {
    assert_eq!(next_primary_tab(Tab::Chat), Tab::Projects);
    assert_eq!(next_primary_tab(Tab::Skills), Tab::Dashboard);
    assert_eq!(next_primary_tab(Tab::Dashboard), Tab::Chat);
}

#[test]
fn previous_primary_tab_wraps_backward_in_operational_order() {
    assert_eq!(previous_primary_tab(Tab::Projects), Tab::Chat);
    assert_eq!(previous_primary_tab(Tab::Chat), Tab::Dashboard);
}

#[test]
fn non_primary_tab_uses_legacy_first_index_for_cycling() {
    assert_eq!(next_primary_tab(Tab::Logs), Tab::Projects);
    assert_eq!(previous_primary_tab(Tab::Logs), Tab::Dashboard);
}

#[test]
fn tab_scroll_offset_keeps_target_visible_when_before_window() {
    assert_eq!(tab_scroll_offset_after_switch(5, Tab::Projects), 1);
}

#[test]
fn tab_scroll_offset_keeps_current_window_when_target_is_visible() {
    assert_eq!(tab_scroll_offset_after_switch(2, Tab::Skills), 2);
    assert_eq!(tab_scroll_offset_after_switch(0, Tab::Chat), 0);
}

#[test]
fn tab_switch_state_sets_active_tab_and_rewinds_hidden_target() {
    assert_eq!(
        tab_switch_state_after_switch(5, Tab::Projects),
        TabSwitchState {
            active_tab: Tab::Projects,
            scroll_offset: 1,
        }
    );
}

#[test]
fn tab_switch_state_keeps_visible_window() {
    assert_eq!(
        tab_switch_state_after_switch(2, Tab::Skills),
        TabSwitchState {
            active_tab: Tab::Skills,
            scroll_offset: 2,
        }
    );
}

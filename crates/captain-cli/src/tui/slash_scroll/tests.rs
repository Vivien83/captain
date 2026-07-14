use super::*;

#[test]
fn scroll_commands_map_to_actions() {
    assert_eq!(scroll_for("/top"), Some(SlashScroll::Top));
    assert_eq!(scroll_for("/bottom"), Some(SlashScroll::Bottom));
}

#[test]
fn non_scroll_commands_stay_in_slash_handler() {
    assert_eq!(scroll_for("/Top"), None);
    assert_eq!(scroll_for("/help"), None);
    assert_eq!(scroll_for(""), None);
}

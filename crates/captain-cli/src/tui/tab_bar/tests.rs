use super::{render_state, CTRL_C_HINT, NORMAL_HINT};

fn contents(render: &super::TabBarRender) -> String {
    render
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

#[test]
fn wide_bar_renders_first_tabs_without_left_overflow() {
    let render = render_state(&["Chat", "Projects", "Dashboard"], 0, 0, 96, false);
    let text = contents(&render);

    assert_eq!(render.scroll_offset, 0);
    assert!(text.contains(" Chat "));
    assert!(text.contains(" Projects "));
    assert!(!text.contains('◀'));
    assert!(text.contains(NORMAL_HINT));
}

#[test]
fn narrow_bar_scrolls_until_active_tab_is_visible() {
    let labels = ["Chat", "Projects", "Dashboard", "Agents", "Sessions"];
    let render = render_state(&labels, 4, 0, 76, false);
    let text = contents(&render);

    assert!(render.scroll_offset > 0);
    assert!(text.contains('◀'));
    assert!(text.contains(" Sessions "));
}

#[test]
fn scroll_offset_moves_back_when_active_tab_is_before_window() {
    let labels = ["Chat", "Projects", "Dashboard", "Agents"];
    let render = render_state(&labels, 1, 3, 72, false);
    let text = contents(&render);

    assert_eq!(render.scroll_offset, 1);
    assert!(text.contains(" Projects "));
    assert!(text.contains('◀'));
}

#[test]
fn right_overflow_is_shown_when_tabs_remain_hidden() {
    let labels = ["Chat", "Projects", "Dashboard", "Agents", "Sessions"];
    let render = render_state(&labels, 0, 0, 42, false);
    let text = contents(&render);

    assert!(text.contains('▶'));
    assert!(!text.contains(" Sessions "));
}

#[test]
fn pending_ctrl_c_uses_warning_hint() {
    let render = render_state(&["Chat", "Projects"], 0, 0, 80, true);
    let text = contents(&render);

    assert!(text.contains(CTRL_C_HINT));
    assert!(!text.contains(NORMAL_HINT));
}

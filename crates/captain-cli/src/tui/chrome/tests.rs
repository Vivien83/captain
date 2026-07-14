use ratatui::{layout::Rect, style::Color};

use super::{
    centered_rect, overlay_area, overlay_title, toast_area, toast_line, too_small_lines,
    welcome_toasts,
};
use crate::tui::theme;

fn line_text(line: &ratatui::text::Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

#[test]
fn centered_rect_keeps_parent_origin_and_percent_size() {
    let area = Rect::new(10, 20, 100, 50);

    assert_eq!(centered_rect(area, 60, 40), Rect::new(30, 35, 60, 20));
}

#[test]
fn overlay_area_matches_draw_overlay_percent_size() {
    let base = Rect::new(5, 7, 100, 50);

    assert_eq!(overlay_area(base), Rect::new(14, 11, 82, 41));
}

#[test]
fn overlay_title_lowercases_label_and_keeps_escape_hint() {
    assert_eq!(overlay_title("Learning"), " /learning — Esc to close ");
}

#[test]
fn too_small_lines_include_current_and_minimum_size() {
    let lines = too_small_lines(Rect::new(0, 0, 72, 18), 80, 24);
    let text = lines.iter().map(line_text).collect::<Vec<_>>();

    assert_eq!(text[1], "Captain CLI");
    assert_eq!(text[3], "Terminal trop petit (72x18)");
    assert_eq!(text[4], "Redimensionne à au moins 80x24");
}

#[test]
fn toast_area_is_bottom_centered_and_width_clamped() {
    assert_eq!(
        toast_area(Rect::new(0, 0, 40, 12), "hello"),
        Rect::new(15, 10, 9, 1)
    );
    assert_eq!(
        toast_area(Rect::new(0, 0, 8, 1), "very long message"),
        Rect::new(0, 0, 8, 1)
    );
}

#[test]
fn toast_line_preserves_message_and_color() {
    let line = toast_line(" Booting", Color::Yellow);

    assert_eq!(line_text(&line), " Booting");
    assert_eq!(line.spans[0].style.fg, Some(Color::Yellow));
}

#[test]
fn welcome_toasts_preserve_boot_then_error_order() {
    let toasts = welcome_toasts(true, 0, Some("offline"));

    let boot = toasts[0].as_ref().expect("boot toast");
    assert_eq!(boot.message, " ⠋ Booting kernel…");
    assert_eq!(boot.color, theme::YELLOW);

    let error = toasts[1].as_ref().expect("error toast");
    assert_eq!(error.message, " ✘ offline");
    assert_eq!(error.color, theme::RED);
}

#[test]
fn welcome_toasts_cycle_spinner_by_tick() {
    let toasts = welcome_toasts(true, theme::SPINNER_FRAMES.len(), None);

    assert_eq!(
        toasts[0].as_ref().map(|toast| toast.message.as_str()),
        Some(" ⠋ Booting kernel…")
    );
    assert!(toasts[1].is_none());
}

#[test]
fn welcome_toasts_are_empty_without_boot_or_error() {
    assert_eq!(welcome_toasts(false, 3, None), [None, None]);
}

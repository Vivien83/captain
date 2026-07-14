//! Compact Captain identity for terminal surfaces.
//!
//! A terminal cannot reproduce the bitmap wordmark reliably: font fallback,
//! cell metrics, and Unicode coverage vary between native terminals and
//! xterm.js. Product surfaces therefore use a short ASCII wordmark and reserve
//! the full crown artwork for graphical web and desktop assets.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::theme;

pub const CAPTAIN_WORDMARK: &str = "CAPTAIN";
pub const CAPTAIN_WIDTH: u16 = CAPTAIN_WORDMARK.len() as u16;
pub const CAPTAIN_HEIGHT: u16 = 1;

fn captain_line() -> Line<'static> {
    Line::from(Span::styled(
        CAPTAIN_WORDMARK,
        Style::default()
            .fg(theme::GOLD)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Build the compact wordmark widget used by the welcome screen.
pub fn captain_paragraph() -> Paragraph<'static> {
    Paragraph::new(captain_line())
}

/// Return the compact wordmark as transcript rows for the chat empty state.
pub fn captain_lines() -> Vec<Line<'static>> {
    vec![captain_line()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wordmark_dimensions_match_constants() {
        assert_eq!(CAPTAIN_HEIGHT, 1);
        assert_eq!(CAPTAIN_WORDMARK.chars().count() as u16, CAPTAIN_WIDTH);
    }

    #[test]
    fn wordmark_is_portable_across_native_and_web_terminals() {
        assert!(CAPTAIN_WORDMARK.is_ascii());
        assert!(CAPTAIN_WORDMARK
            .chars()
            .all(|character| character.is_ascii_uppercase()));
    }

    #[test]
    fn widgets_build_without_terminal_art() {
        let _ = captain_paragraph();
        let lines = captain_lines();
        assert_eq!(lines.len(), CAPTAIN_HEIGHT as usize);
    }
}

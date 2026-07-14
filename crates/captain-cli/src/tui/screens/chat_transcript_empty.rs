//! Empty transcript logo and welcome rows.

use super::{chat::ChatState, chat_welcome_summary::welcome_summary_lines};
use crate::tui::{branding, theme};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

#[cfg(test)]
mod tests;

/// Build the compact Captain identity for the transcript buffer.
///
/// The graphical crown belongs to the surrounding web/desktop shell. Keeping
/// this terminal-native header short leaves room for operational context and
/// avoids font-dependent artwork in xterm.js.
pub(super) fn captain_logo_lines(width: usize) -> Vec<Line<'static>> {
    use branding::{captain_lines, CAPTAIN_WIDTH};

    if width < CAPTAIN_WIDTH as usize {
        return Vec::new();
    }

    let mut out: Vec<Line<'static>> = Vec::with_capacity(4);
    out.push(Line::from(""));

    let pad = (width.saturating_sub(CAPTAIN_WIDTH as usize)) / 2;
    let pad_str = " ".repeat(pad);
    for line in captain_lines() {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 1);
        spans.push(Span::raw(pad_str.clone()));
        spans.extend(line.spans);
        out.push(Line::from(spans));
    }

    let version = crate::cli_runtime::captain_version();
    let version_label = format!("v{version}");
    if width >= version_label.chars().count() {
        let version_pad = width.saturating_sub(version_label.chars().count()) / 2;
        out.push(Line::from(vec![
            Span::raw(" ".repeat(version_pad)),
            Span::styled(version_label, Style::default().fg(theme::TEXT_TERTIARY)),
        ]));
    }
    out.push(Line::from(""));
    out
}

pub(super) fn empty_transcript_lines(
    logo_lines: Vec<Line<'static>>,
    logo_len: usize,
    state: &ChatState,
    width: usize,
    visible_height: usize,
) -> Vec<Line<'static>> {
    let lang = crate::i18n::current();
    build_empty_transcript_lines(
        logo_lines,
        logo_len,
        welcome_summary_lines(state, width),
        format!("  {}", crate::i18n::t("chat.empty.primary", lang)),
        format!("  {}", crate::i18n::t("chat.empty.secondary", lang)),
        visible_height,
    )
}

fn build_empty_transcript_lines(
    mut lines: Vec<Line<'static>>,
    logo_len: usize,
    summary_lines: Vec<Line<'static>>,
    primary: String,
    secondary: String,
    visible_height: usize,
) -> Vec<Line<'static>> {
    lines.extend(summary_lines);
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(primary, theme::dim_style())]));
    lines.push(Line::from(vec![Span::styled(
        secondary,
        theme::dim_style(),
    )]));

    if lines.len() < visible_height {
        let pad = visible_height - lines.len();
        let mut padded: Vec<Line> = Vec::with_capacity(visible_height);
        let placeholder = lines.split_off(logo_len);
        padded.extend(lines);
        for _ in 0..pad {
            padded.push(Line::from(""));
        }
        padded.extend(placeholder);
        padded
    } else {
        lines
    }
}

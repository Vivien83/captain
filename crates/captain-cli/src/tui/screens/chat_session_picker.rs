//! Session picker overlay for the chat screen.

use super::chat::ChatState;
use crate::tui::{session_store::SessionSummary, theme};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};
use ratatui::Frame;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionPickerKeyAction {
    Close,
    Up,
    Down,
    Select,
    Continue,
}

pub(super) fn session_picker_key_action_for_key(key: KeyEvent) -> SessionPickerKeyAction {
    match key.code {
        KeyCode::Esc => SessionPickerKeyAction::Close,
        KeyCode::Up => SessionPickerKeyAction::Up,
        KeyCode::Down => SessionPickerKeyAction::Down,
        KeyCode::Enter => SessionPickerKeyAction::Select,
        _ => SessionPickerKeyAction::Continue,
    }
}

pub(super) fn draw_session_picker(f: &mut Frame, area: Rect, state: &ChatState) {
    if area.height < 8 || area.width < 40 {
        return;
    }
    let popup_w = area.width.clamp(50, 80);
    let max_rows = area.height.saturating_sub(4);
    let visible = (state.session_picker_items.len() as u16 + 4).min(max_rows.max(6));
    let popup_h = visible.clamp(6, area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(popup_w)) / 2;
    let y = area.y + (area.height.saturating_sub(popup_h)) / 2;
    let popup = Rect::new(x, y, popup_w, popup_h);

    f.render_widget(Clear, popup);
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Sessions ",
            theme::title_style(),
        )]))
        .title_bottom(Line::from(vec![Span::styled(
            " ↑↓ navigue · Enter charge · Esc ferme ",
            theme::hint_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let now = current_unix_secs();
    let row_count = inner.height as usize;
    let start = state
        .session_picker_idx
        .saturating_sub(row_count.saturating_sub(1));
    let lines: Vec<Line<'static>> = state
        .session_picker_items
        .iter()
        .enumerate()
        .skip(start)
        .take(row_count)
        .map(|(i, s)| session_picker_row(s, i == state.session_picker_idx, now))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn current_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn session_picker_row(s: &SessionSummary, selected: bool, now: u64) -> Line<'static> {
    let marker_style = if selected {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        theme::dim_style()
    };
    let marker = if selected { "▶ " } else { "  " };
    let age = format_age(now.saturating_sub(s.updated_at));
    let label = if !s.label.is_empty() {
        s.label.clone()
    } else if s.agent_name.is_empty() {
        s.agent_key.clone()
    } else {
        s.agent_name.clone()
    };
    let total = s.session_input_tokens + s.session_output_tokens;
    Line::from(vec![
        Span::styled(marker.to_string(), marker_style),
        Span::styled(
            format!("{label:<18}"),
            Style::default().fg(theme::TEXT_PRIMARY),
        ),
        Span::styled(format!("{:>4} msg  ", s.message_count), theme::dim_style()),
        Span::styled(format!("{total:>5} tok  "), theme::dim_style()),
        Span::styled(age, theme::dim_style()),
    ])
}

fn format_age(secs: u64) -> String {
    if secs < 60 {
        "à l'instant".into()
    } else if secs < 3600 {
        format!("il y a {} min", secs / 60)
    } else if secs < 86400 {
        format!("il y a {} h", secs / 3600)
    } else {
        format!("il y a {} j", secs / 86400)
    }
}

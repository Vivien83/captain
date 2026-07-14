//! Model picker overlay for the chat screen.

use super::chat::{ChatState, ModelEntry};
use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Padding, Paragraph};
use ratatui::Frame;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModelPickerKeyAction {
    Close,
    Up,
    Down,
    Select,
    Backspace,
    Insert(char),
    Continue,
}

pub(super) fn model_picker_key_action_for_key(key: KeyEvent) -> ModelPickerKeyAction {
    match key.code {
        KeyCode::Esc => ModelPickerKeyAction::Close,
        KeyCode::Up => ModelPickerKeyAction::Up,
        KeyCode::Down => ModelPickerKeyAction::Down,
        KeyCode::Enter => ModelPickerKeyAction::Select,
        KeyCode::Backspace => ModelPickerKeyAction::Backspace,
        KeyCode::Char(c) => ModelPickerKeyAction::Insert(c),
        _ => ModelPickerKeyAction::Continue,
    }
}

pub(super) fn draw_model_picker(f: &mut Frame, area: Rect, state: &ChatState) {
    let filtered = state.filtered_models();
    let Some(popup_area) = model_picker_popup_area(area, filtered.len()) else {
        return;
    };

    // Clear background
    f.render_widget(Clear, popup_area);

    let block = model_picker_block();
    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    render_model_picker_search(f, chunks[0], &state.model_picker_filter);
    render_model_picker_list(f, chunks[1], &filtered, state.model_picker_idx);
}

fn model_picker_popup_area(area: Rect, filtered_len: usize) -> Option<Rect> {
    if area.height < 6 || area.width < 20 {
        return None;
    }

    let popup_w = area.width.clamp(30, 54);
    let popup_h = (filtered_len as u16 + 4).clamp(5, area.height.saturating_sub(2));
    Some(Rect::new(
        area.x + (area.width.saturating_sub(popup_w)) / 2,
        area.y + (area.height.saturating_sub(popup_h)) / 2,
        popup_w,
        popup_h,
    ))
}

fn model_picker_block() -> Block<'static> {
    Block::default()
        .title(Line::from(vec![Span::styled(
            " Switch Model ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1))
}

fn render_model_picker_search(f: &mut Frame, area: Rect, filter: &str) {
    let search_line = Line::from(vec![
        Span::styled("/ ", theme::dim_style()),
        Span::raw(filter.to_string()),
        Span::styled(
            "\u{2588}",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]);
    f.render_widget(Paragraph::new(search_line), area);
}

fn render_model_picker_list(
    f: &mut Frame,
    area: Rect,
    filtered: &[&ModelEntry],
    selected_idx: usize,
) {
    let visible_h = area.height as usize;
    let total = filtered.len();

    if total == 0 {
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                " No models match",
                theme::dim_style(),
            )])),
            area,
        );
        return;
    }

    let lines = model_picker_rows(filtered, selected_idx, visible_h, area.width as usize);
    f.render_widget(Paragraph::new(lines), area);
}

fn model_picker_rows(
    filtered: &[&ModelEntry],
    selected_idx: usize,
    visible_h: usize,
    width: usize,
) -> Vec<Line<'static>> {
    let scroll_start = model_picker_scroll_start(selected_idx, visible_h);
    let max_name = width.saturating_sub(14);
    filtered
        .iter()
        .enumerate()
        .skip(scroll_start)
        .take(visible_h)
        .map(|(i, entry)| model_picker_row(entry, i == selected_idx, max_name))
        .collect()
}

fn model_picker_scroll_start(selected_idx: usize, visible_h: usize) -> usize {
    if selected_idx >= visible_h {
        selected_idx - visible_h + 1
    } else {
        0
    }
}

fn model_picker_row(entry: &ModelEntry, selected: bool, max_name: usize) -> Line<'static> {
    let indicator = if selected { "\u{25b6} " } else { "  " };

    let name = if entry.display_name.is_empty() {
        &entry.id
    } else {
        &entry.display_name
    };
    let name_display = if name.len() > max_name && max_name > 1 {
        let truncated = captain_types::truncate_str(name, max_name.saturating_sub(1));
        format!("{truncated}\u{2026}")
    } else {
        name.to_string()
    };

    let bg = if selected {
        Style::default()
            .fg(theme::TEXT_PRIMARY)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::TEXT_SECONDARY)
    };

    Line::from(vec![
        Span::styled(indicator, Style::default().fg(theme::ACCENT)),
        Span::styled(name_display, bg),
        Span::raw(" "),
        Span::styled(
            entry.tier.to_lowercase(),
            model_picker_tier_style(&entry.tier),
        ),
    ])
}

fn model_picker_tier_style(tier: &str) -> Style {
    match tier.to_lowercase().as_str() {
        "frontier" => Style::default().fg(theme::PURPLE),
        "smart" => Style::default().fg(theme::BLUE),
        "balanced" => Style::default().fg(theme::GREEN),
        "fast" => Style::default().fg(theme::YELLOW),
        _ => theme::dim_style(),
    }
}

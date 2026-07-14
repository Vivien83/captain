use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::theme;

const INDICATOR_WIDTH: usize = 2;
const NORMAL_HINT: &str = "Ctrl+C×2 quit  F1-F9/Tab switch  slash Tab completes";
const CTRL_C_HINT: &str = "Press Ctrl+C again to quit";

pub(crate) fn draw(
    frame: &mut ratatui::Frame,
    area: Rect,
    labels: &[&str],
    active_idx: usize,
    scroll_offset: &mut usize,
    ctrl_c_pending: bool,
) {
    let render = render_state(
        labels,
        active_idx,
        *scroll_offset,
        area.width,
        ctrl_c_pending,
    );
    *scroll_offset = render.scroll_offset;
    let bar = Paragraph::new(Line::from(render.spans)).style(Style::default().bg(theme::BG_CARD));
    frame.render_widget(bar, area);
}

struct TabBarRender {
    scroll_offset: usize,
    spans: Vec<Span<'static>>,
}

fn render_state(
    labels: &[&str],
    active_idx: usize,
    scroll_offset: usize,
    width: u16,
    ctrl_c_pending: bool,
) -> TabBarRender {
    let hint = hint_text(ctrl_c_pending);
    let available = available_width(width as usize, hint);
    let label_widths = labels
        .iter()
        .map(|label| tab_width(label))
        .collect::<Vec<_>>();
    let scroll_offset = adjusted_scroll_offset(&label_widths, active_idx, scroll_offset, available);
    let mut spans = tab_spans(labels, active_idx, scroll_offset, available);
    push_hint(&mut spans, width as usize, hint, ctrl_c_pending);

    TabBarRender {
        scroll_offset,
        spans,
    }
}

fn hint_text(ctrl_c_pending: bool) -> &'static str {
    if ctrl_c_pending {
        CTRL_C_HINT
    } else {
        NORMAL_HINT
    }
}

fn available_width(width: usize, hint: &str) -> usize {
    width.saturating_sub(hint.len() + 4)
}

fn tab_width(label: &str) -> usize {
    label.len() + 3
}

fn adjusted_scroll_offset(
    label_widths: &[usize],
    active_idx: usize,
    mut scroll_offset: usize,
    available: usize,
) -> usize {
    if label_widths.is_empty() {
        return 0;
    }
    let max_idx = label_widths.len().saturating_sub(1);
    let active_idx = active_idx.min(max_idx);
    scroll_offset = scroll_offset.min(max_idx);
    if active_idx < scroll_offset {
        return active_idx;
    }

    loop {
        let last_visible = last_visible_index(label_widths, scroll_offset, available);
        if active_idx <= last_visible || scroll_offset >= max_idx {
            break scroll_offset;
        }
        scroll_offset += 1;
    }
}

fn last_visible_index(label_widths: &[usize], scroll_offset: usize, available: usize) -> usize {
    let mut used = if scroll_offset > 0 {
        INDICATOR_WIDTH
    } else {
        1
    };
    let mut last_visible = scroll_offset;
    for (idx, width) in label_widths.iter().enumerate().skip(scroll_offset) {
        if used + width > available {
            break;
        }
        used += width;
        last_visible = idx;
    }
    last_visible
}

fn tab_spans(
    labels: &[&str],
    active_idx: usize,
    scroll_offset: usize,
    available: usize,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    push_left_indicator(&mut spans, scroll_offset);

    let mut used = if scroll_offset > 0 {
        INDICATOR_WIDTH
    } else {
        1
    };
    let mut last_rendered = scroll_offset;
    for (idx, label) in labels.iter().enumerate().skip(scroll_offset) {
        let tab_width = tab_width(label);
        if used + tab_width > available {
            break;
        }
        push_tab(&mut spans, label, idx == active_idx);
        used += tab_width;
        last_rendered = idx;
    }

    if last_rendered < labels.len().saturating_sub(1) {
        spans.push(Span::styled(
            " ▶",
            Style::default().fg(theme::TEXT_TERTIARY),
        ));
    }
    spans
}

fn push_left_indicator(spans: &mut Vec<Span<'static>>, scroll_offset: usize) {
    if scroll_offset > 0 {
        spans.push(Span::styled(
            "◀ ",
            Style::default().fg(theme::TEXT_TERTIARY),
        ));
    } else {
        spans.push(Span::raw(" "));
    }
}

fn push_tab(spans: &mut Vec<Span<'static>>, label: &str, active: bool) {
    let style = if active {
        theme::tab_active()
    } else {
        theme::tab_inactive()
    };
    spans.push(Span::styled(format!(" {label} "), style));
    spans.push(Span::raw(" "));
}

fn push_hint(
    spans: &mut Vec<Span<'static>>,
    width: usize,
    hint: &'static str,
    ctrl_c_pending: bool,
) {
    let spans_width = spans.iter().map(|span| span.content.len()).sum::<usize>();
    let padding = width.saturating_sub(spans_width + hint.len());
    if padding == 0 {
        return;
    }
    let hint_style = if ctrl_c_pending {
        Style::default()
            .fg(theme::YELLOW)
            .add_modifier(Modifier::BOLD)
    } else {
        theme::hint_style()
    };
    spans.push(Span::raw(" ".repeat(padding)));
    spans.push(Span::styled(hint, hint_style));
}

#[cfg(test)]
#[path = "tab_bar/tests.rs"]
mod tests;

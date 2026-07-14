use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use super::theme;

/// Return a rectangle centered inside `area`, sized as `percent_x` x `percent_y`
/// percent of the parent. Used by modal overlays.
pub(crate) fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let w = (area.width as u32 * percent_x as u32 / 100) as u16;
    let h = (area.height as u32 * percent_y as u32 / 100) as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

pub(crate) fn overlay_area(base: Rect) -> Rect {
    centered_rect(base, 82, 82)
}

pub(crate) fn overlay_title(label: &str) -> String {
    format!(" /{} — Esc to close ", label.to_lowercase())
}

pub(crate) fn draw_overlay_shell(frame: &mut Frame, base: Rect, label: &str) -> Rect {
    let overlay_rect = overlay_area(base);
    frame.render_widget(Clear, overlay_rect);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .title(overlay_title(label));
    let inner = block.inner(overlay_rect);
    frame.render_widget(block, overlay_rect);
    inner
}

pub(crate) fn too_small_lines(area: Rect, min_w: u16, min_h: u16) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        Line::from(Span::styled(
            "Captain CLI",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(
            format!("Terminal trop petit ({}x{})", area.width, area.height),
            Style::default().fg(theme::RED),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            format!("Redimensionne à au moins {min_w}x{min_h}"),
            theme::dim_style(),
        ))
        .alignment(Alignment::Center),
    ]
}

pub(crate) fn draw_too_small(frame: &mut Frame, area: Rect, min_w: u16, min_h: u16) {
    frame.render_widget(Paragraph::new(too_small_lines(area, min_w, min_h)), area);
}

pub(crate) fn toast_area(area: Rect, msg: &str) -> Rect {
    let w = (msg.len() as u16 + 4).min(area.width);
    let x = area.width.saturating_sub(w) / 2;
    let y = area.height.saturating_sub(2);
    Rect::new(x, y, w, 1)
}

pub(crate) fn toast_line(msg: &str, color: Color) -> Line<'static> {
    Line::from(vec![Span::styled(
        msg.to_string(),
        Style::default().fg(color),
    )])
}

pub(crate) fn render_toast(frame: &mut Frame, area: Rect, msg: &str, color: Color) {
    frame.render_widget(
        Paragraph::new(toast_line(msg, color)),
        toast_area(area, msg),
    );
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToastSpec {
    pub(crate) message: String,
    pub(crate) color: Color,
}

pub(crate) fn welcome_toasts(
    kernel_booting: bool,
    welcome_tick: usize,
    kernel_boot_error: Option<&str>,
) -> [Option<ToastSpec>; 2] {
    [
        kernel_booting.then(|| {
            let spinner = theme::SPINNER_FRAMES[welcome_tick % theme::SPINNER_FRAMES.len()];
            ToastSpec {
                message: format!(" {spinner} Booting kernel\u{2026}"),
                color: theme::YELLOW,
            }
        }),
        kernel_boot_error.map(|error| ToastSpec {
            message: format!(" \u{2718} {error}"),
            color: theme::RED,
        }),
    ]
}

#[cfg(test)]
#[path = "chrome/tests.rs"]
mod tests;

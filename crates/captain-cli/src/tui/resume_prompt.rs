use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::{chrome, session_store, theme};

/// Render the boot-time "resume last session?" prompt.
pub(crate) fn draw(
    frame: &mut ratatui::Frame,
    area: Rect,
    summary: Option<&session_store::SessionSummary>,
) {
    let (header, hint) = summary_lines(summary);

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Captain",
            Style::default()
                .fg(theme::TEXT_PRIMARY)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(
            "Reprendre la dernière session ?",
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(header, theme::dim_style())).alignment(Alignment::Center),
        Line::from(Span::styled(hint, theme::dim_style())).alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(
            "[Y] / Enter   reprendre",
            Style::default().fg(theme::ACCENT),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            "[N] / Esc     nouvelle session",
            theme::dim_style(),
        ))
        .alignment(Alignment::Center),
    ];
    let card = chrome::centered_rect(area, 60, 14);
    frame.render_widget(Paragraph::new(lines), card);
}

fn summary_lines(summary: Option<&session_store::SessionSummary>) -> (String, String) {
    match summary {
        Some(s) => {
            let age = format_relative_age(s.updated_at);
            (
                format!(
                    "{} · {} message{} · {}",
                    s.agent_name,
                    s.message_count,
                    if s.message_count > 1 { "s" } else { "" },
                    age,
                ),
                format!(
                    "session: {}",
                    s.path.file_name().and_then(|f| f.to_str()).unwrap_or("?")
                ),
            )
        }
        None => ("(aucune session disponible)".into(), "".into()),
    }
}

/// "il y a 3h", "il y a 2j" from a unix epoch timestamp.
fn format_relative_age(updated_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let delta = now.saturating_sub(updated_at);
    if delta < 60 {
        "à l'instant".into()
    } else if delta < 3600 {
        format!("il y a {}m", delta / 60)
    } else if delta < 86_400 {
        format!("il y a {}h", delta / 3600)
    } else {
        format!("il y a {}j", delta / 86_400)
    }
}

#[cfg(test)]
#[path = "resume_prompt/tests.rs"]
mod tests;

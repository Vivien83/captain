use super::{
    model::{ChatMessage, MessageRole, SessionInfo},
    ConsoleApp, Focus,
};
use crate::ConsoleProfileSummary;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

const ACCENT: Color = Color::Rgb(212, 168, 83);
const ACCENT_DIM: Color = Color::Rgb(170, 134, 66);
const TEXT: Color = Color::Rgb(240, 239, 238);
const MUTED: Color = Color::Rgb(168, 162, 158);
const DIM: Color = Color::Rgb(120, 113, 108);
const BORDER: Color = Color::Rgb(63, 59, 56);
const GREEN: Color = Color::Rgb(34, 197, 94);
const RED: Color = Color::Rgb(239, 68, 68);
const PANEL: Color = Color::Rgb(24, 22, 21);

pub(super) fn draw_profile_picker(
    frame: &mut Frame<'_>,
    profiles: &[ConsoleProfileSummary],
    selected: usize,
) {
    let width = frame.area().width.saturating_sub(4).min(72);
    let height = (profiles.len() as u16)
        .saturating_add(6)
        .min(frame.area().height);
    let area = centered_rect(frame.area(), width, height);
    let rows = profiles
        .iter()
        .map(|profile| {
            let active = if profile.active { "active" } else { "paired" };
            ListItem::new(Line::from(vec![
                Span::styled(profile.label.clone(), Style::default().fg(TEXT)),
                Span::styled(format!("  {active}"), Style::default().fg(DIM)),
            ]))
        })
        .collect::<Vec<_>>();
    let list = List::new(rows)
        .block(
            Block::default()
                .title(" SELECT CAPTAIN ")
                .title_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER)),
        )
        .highlight_symbol("> ")
        .highlight_style(
            Style::default()
                .fg(ACCENT)
                .bg(PANEL)
                .add_modifier(Modifier::BOLD),
        );
    let mut state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
    let footer = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(area.height.saturating_sub(2)),
        area.width.saturating_sub(4),
        1,
    );
    frame.render_widget(
        Paragraph::new("Up/Down select  Enter connect  Esc cancel")
            .style(Style::default().fg(DIM))
            .alignment(Alignment::Center),
        footer,
    );
}

pub(super) fn draw_console(frame: &mut Frame<'_>, app: &ConsoleApp) {
    draw_console_state(frame, &ConsoleViewState::from(app));
}

struct ConsoleViewState<'a> {
    profile_label: &'a str,
    agent_name: &'a str,
    model: &'a str,
    sessions: &'a [SessionInfo],
    selected_session: usize,
    loaded_session_id: &'a str,
    messages: &'a [ChatMessage],
    stream_buffer: &'a str,
    input: &'a str,
    focus: Focus,
    scroll: u16,
    status: &'a str,
    busy: bool,
    streaming: bool,
    pending_question: bool,
}

impl<'a> From<&'a ConsoleApp> for ConsoleViewState<'a> {
    fn from(app: &'a ConsoleApp) -> Self {
        Self {
            profile_label: &app.profile.label,
            agent_name: &app.agent_name,
            model: &app.model,
            sessions: &app.sessions,
            selected_session: app.selected_session,
            loaded_session_id: &app.loaded_session_id,
            messages: &app.messages,
            stream_buffer: &app.stream_buffer,
            input: &app.input,
            focus: app.focus,
            scroll: app.scroll,
            status: &app.status,
            busy: app.busy,
            streaming: app.streaming,
            pending_question: app.pending_question,
        }
    }
}

impl ConsoleViewState<'_> {
    fn selected_session(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.selected_session)
    }
}

fn draw_console_state(frame: &mut Frame<'_>, app: &ConsoleViewState<'_>) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(2),
        ])
        .split(frame.area());
    draw_header(frame, root[0], app);

    let wide = root[1].width >= 96;
    let body = if wide {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(40)])
            .split(root[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1)])
            .split(root[1])
    };
    let transcript_area = if wide {
        draw_sessions(frame, body[0], app);
        body[1]
    } else {
        body[0]
    };
    draw_transcript(frame, transcript_area, app);
    draw_input(frame, root[2], app);
    draw_footer(frame, root[3], app, wide);
}

fn draw_header(frame: &mut Frame<'_>, area: Rect, app: &ConsoleViewState<'_>) {
    let session = app
        .selected_session()
        .map(|session| session.label.as_str())
        .unwrap_or("no session");
    let available = area.width.saturating_sub(5) as usize;
    let authority = truncate(
        &format!(
            "{}  |  {}  |  {}  |  {}",
            app.profile_label, app.agent_name, app.model, session
        ),
        available,
    );
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " CAPTAIN CONSOLE ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(authority, Style::default().fg(MUTED)),
    ]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(BORDER)),
    );
    frame.render_widget(header, area);
}

fn draw_sessions(frame: &mut Frame<'_>, area: Rect, app: &ConsoleViewState<'_>) {
    let rows = app
        .sessions
        .iter()
        .map(|session| {
            let marker = if session.id == app.loaded_session_id {
                "*"
            } else {
                " "
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(format!("{marker} "), Style::default().fg(GREEN)),
                    Span::styled(truncate(&session.label, 20), Style::default().fg(TEXT)),
                ]),
                Line::from(Span::styled(
                    format!("  {} messages", session.message_count),
                    Style::default().fg(DIM),
                )),
            ])
        })
        .collect::<Vec<_>>();
    let border = if app.focus == Focus::Sessions {
        ACCENT_DIM
    } else {
        BORDER
    };
    let list = List::new(rows)
        .block(
            Block::default()
                .title(" SESSIONS ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border)),
        )
        .highlight_symbol("> ")
        .highlight_style(Style::default().fg(ACCENT).bg(PANEL));
    let mut state = ListState::default().with_selected(Some(app.selected_session));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_transcript(frame: &mut Frame<'_>, area: Rect, app: &ConsoleViewState<'_>) {
    let inner_width = area.width.saturating_sub(2).max(1) as usize;
    let mut lines = Vec::new();
    for message in app.messages {
        append_message(&mut lines, message.role, &message.content);
    }
    if !app.stream_buffer.is_empty() {
        append_message(&mut lines, MessageRole::Assistant, app.stream_buffer);
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "Start a conversation with this Captain.",
            Style::default().fg(DIM),
        )));
    }
    let visual_lines = estimated_visual_lines(&lines, inner_width);
    let viewport = area.height.saturating_sub(2) as usize;
    let scroll = visual_lines
        .saturating_sub(viewport)
        .saturating_sub(app.scroll as usize)
        .min(u16::MAX as usize) as u16;
    let paragraph = Paragraph::new(Text::from(lines))
        .block(
            Block::default()
                .title(" CHAT ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BORDER)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn append_message<'a>(lines: &mut Vec<Line<'a>>, role: MessageRole, content: &'a str) {
    let (label, color) = match role {
        MessageRole::User => ("YOU", MUTED),
        MessageRole::Assistant => ("CAPTAIN", ACCENT),
        MessageRole::System => ("ACTIVITY", DIM),
    };
    lines.push(Line::from(Span::styled(
        label,
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )));
    for line in content.lines() {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(if role == MessageRole::System {
                MUTED
            } else {
                TEXT
            }),
        )));
    }
    lines.push(Line::default());
}

fn draw_input(frame: &mut Frame<'_>, area: Rect, app: &ConsoleViewState<'_>) {
    let title = if app.pending_question {
        " ANSWER CAPTAIN "
    } else {
        " MESSAGE "
    };
    let border = if app.focus == Focus::Input {
        ACCENT_DIM
    } else {
        BORDER
    };
    let input = Paragraph::new(app.input)
        .style(Style::default().fg(TEXT))
        .block(
            Block::default()
                .title(title)
                .title_style(Style::default().fg(if app.pending_question { ACCENT } else { MUTED }))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border)),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(input, area);
}

fn draw_footer(frame: &mut Frame<'_>, area: Rect, app: &ConsoleViewState<'_>, wide: bool) {
    let status_color = if app.status.contains("failed")
        || app.status.contains("unavailable")
        || app.status.contains("rejected")
        || app.status.contains("interrupted")
    {
        RED
    } else if app.streaming || app.busy || app.pending_question {
        ACCENT
    } else {
        GREEN
    };
    let controls = if wide {
        "Tab focus  Enter send/open  Shift+Enter newline  Ctrl+N new  Ctrl+R reload  Ctrl+C quit"
    } else {
        "Enter send  Tab sessions  Ctrl+N new  Ctrl+R reload  Ctrl+C quit"
    };
    let footer = Text::from(vec![
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(DIM)),
            Span::styled(
                truncate(app.status, (area.width as usize).saturating_sub(8)),
                Style::default().fg(status_color),
            ),
        ]),
        Line::from(Span::styled(controls, Style::default().fg(DIM))),
    ]);
    frame.render_widget(Paragraph::new(footer), area);
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width.min(area.width),
        height.min(area.height),
    )
}

fn truncate(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }
    let mut truncated = value.chars().take(max_chars - 3).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn estimated_visual_lines(lines: &[Line<'_>], width: usize) -> usize {
    lines
        .iter()
        .map(|line| {
            let chars = line
                .spans
                .iter()
                .map(|span| span.content.chars().count())
                .sum::<usize>();
            chars.max(1).div_ceil(width.max(1))
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn rendered(width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let sessions = vec![SessionInfo {
            id: "33333333-3333-4333-8333-333333333333".to_string(),
            label: "Release Alpha 15".to_string(),
            message_count: 2,
            active: true,
        }];
        let messages = vec![ChatMessage {
            role: MessageRole::Assistant,
            content: "Console ready.".to_string(),
        }];
        let app = ConsoleViewState {
            profile_label: "Production",
            agent_name: "captain",
            model: "gpt-5.6-sol",
            sessions: &sessions,
            selected_session: 0,
            loaded_session_id: "33333333-3333-4333-8333-333333333333",
            messages: &messages,
            stream_buffer: "",
            input: "",
            focus: Focus::Input,
            scroll: 0,
            status: "Ready",
            busy: false,
            streaming: false,
            pending_question: false,
        };
        terminal
            .draw(|frame| draw_console_state(frame, &app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn compact_text_never_exceeds_its_container() {
        assert_eq!(truncate("Captain Production", 10), "Captain...");
        assert_eq!(truncate("ok", 10), "ok");
        assert_eq!(truncate("long", 2), "lo");
    }

    #[test]
    fn wrapped_line_estimate_is_stable_for_empty_and_long_rows() {
        let lines = vec![Line::from(""), Line::from("123456789")];
        assert_eq!(estimated_visual_lines(&lines, 4), 4);
    }

    #[test]
    fn console_render_is_nonblank_and_operational_on_wide_and_compact_terminals() {
        let wide = rendered(100, 28);
        assert!(wide.contains("CAPTAIN CONSOLE"));
        assert!(wide.contains("SESSIONS"));
        assert!(wide.contains("Console ready."));
        assert!(wide.contains("MESSAGE"));

        let compact = rendered(52, 18);
        assert!(compact.contains("CAPTAIN CONSOLE"));
        assert!(compact.contains("Console ready."));
        assert!(compact.contains("Ctrl+N"));
    }
}

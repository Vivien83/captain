//! Cron jobs screen: list, enable/disable, run-now, delete scheduled jobs.

#![allow(dead_code)]

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

#[derive(Clone, Default)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub enabled: bool,
    pub last_status: String,
    pub agent_id: String,
    pub consecutive_errors: u64,
}

pub struct CronState {
    pub jobs: Vec<CronJob>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum CronAction {
    Continue,
    Refresh,
    Toggle(String),
    RunNow(String),
    Delete(String),
}

impl CronState {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn selected_id(&self) -> Option<String> {
        let i = self.list_state.selected()?;
        self.jobs.get(i).map(|j| j.id.clone())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> CronAction {
        match key.code {
            KeyCode::Char('r') => return CronAction::Refresh,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.jobs.is_empty() {
                    return CronAction::Continue;
                }
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 { self.jobs.len() - 1 } else { i - 1 };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.jobs.is_empty() {
                    return CronAction::Continue;
                }
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1) % self.jobs.len()));
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if let Some(id) = self.selected_id() {
                    return CronAction::Toggle(id);
                }
            }
            KeyCode::Char('R') => {
                if let Some(id) = self.selected_id() {
                    return CronAction::RunNow(id);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(id) = self.selected_id() {
                    return CronAction::Delete(id);
                }
            }
            _ => {}
        }
        CronAction::Continue
    }
}

pub fn draw(f: &mut Frame, area: Rect, state: &mut CronState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            format!(" Cron ({} jobs) ", state.jobs.len()),
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(inner);

    let header = format!(
        "  {:<18} {:<24} {:<10} {:<8} {}",
        "schedule", "name", "status", "errors", "id"
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(header, theme::table_header()))),
        chunks[0],
    );

    if state.jobs.is_empty() {
        let msg = if state.loading {
            "  Chargement…"
        } else {
            "  Aucun cron job configuré."
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, theme::dim_style()))),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .jobs
            .iter()
            .map(|j| {
                let enabled_style = if j.enabled {
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme::dim_style()
                };
                let status_style = match j.last_status.as_str() {
                    "success" => Style::default().fg(theme::GREEN),
                    "error" | "failed" => Style::default().fg(theme::RED),
                    _ => theme::dim_style(),
                };
                let short_id = j.id.chars().take(8).collect::<String>();
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{} ", if j.enabled { "\u{25cf}" } else { "\u{25cb}" }),
                        enabled_style,
                    ),
                    Span::styled(
                        format!("{:<18} ", j.schedule),
                        Style::default().fg(theme::ACCENT),
                    ),
                    Span::raw(format!("{:<24} ", j.name)),
                    Span::styled(format!("{:<10} ", j.last_status), status_style),
                    Span::styled(format!("{:<8} ", j.consecutive_errors), theme::dim_style()),
                    Span::styled(short_id, theme::dim_style()),
                ]))
            })
            .collect();
        let list = List::new(items).highlight_style(theme::selected_style());
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    let hints = "  [\u{2191}\u{2193}] nav  [Enter] toggle  [R] run now  [d] delete  [r] refresh";
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hints, theme::hint_style()))),
        chunks[2],
    );
}

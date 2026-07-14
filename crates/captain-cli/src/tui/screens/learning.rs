//! Learning screen: review and decide on committed patterns + pending proposals
//! from the v3.12 Learning Engine.

#![allow(dead_code)] // Several fields (id, created_at, source) are read by
                     // future detail-view / sort features. tick() hook kept
                     // symmetric with sibling screens.

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct CommittedRow {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub source: String,
    pub sync_status: String,
    pub created_at: i64,
}

#[derive(Clone, Default)]
pub struct ReviewItem {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
    pub source: String,
    pub created_at: i64,
}

#[derive(Clone, Default)]
pub struct LearningMetrics {
    pub synced: u64,
    pub pending: u64,
    pub error: u64,
    pub review_queue_pending: u64,
    pub mode: String,
    pub enabled: bool,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct LearningState {
    pub pending: Vec<ReviewItem>,
    pub committed: Vec<CommittedRow>,
    pub metrics: Option<LearningMetrics>,
    pub list_state: ListState,
    pub focus_pending: bool,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum LearningAction {
    Continue,
    Refresh,
    Approve(String),
    Deny(String),
}

impl LearningState {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            committed: Vec::new(),
            metrics: None,
            list_state: ListState::default(),
            focus_pending: true,
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> LearningAction {
        match key.code {
            KeyCode::Char('r') => return LearningAction::Refresh,
            KeyCode::Tab => {
                self.focus_pending = !self.focus_pending;
                self.list_state.select(Some(0));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.current_list_len();
                if len == 0 {
                    return LearningAction::Continue;
                }
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 { len - 1 } else { i - 1 };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.current_list_len();
                if len == 0 {
                    return LearningAction::Continue;
                }
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1) % len));
            }
            KeyCode::Char('a') => {
                if self.focus_pending {
                    if let Some(id) = self.selected_pending_id() {
                        return LearningAction::Approve(id);
                    }
                }
            }
            KeyCode::Char('d') if self.focus_pending => {
                if let Some(id) = self.selected_pending_id() {
                    return LearningAction::Deny(id);
                }
            }
            _ => {}
        }
        LearningAction::Continue
    }

    fn current_list_len(&self) -> usize {
        if self.focus_pending {
            self.pending.len()
        } else {
            self.committed.len()
        }
    }

    fn selected_pending_id(&self) -> Option<String> {
        let i = self.list_state.selected()?;
        self.pending.get(i).map(|p| p.id.clone())
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut LearningState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Learning ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(2), // metrics
        Constraint::Length(1), // focus tabs
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // ── Metrics ─────────────────────────────────────────────────────────────
    let metrics_line = match &state.metrics {
        Some(m) => {
            let enabled = if m.enabled { "on" } else { "off·cfg" };
            Line::from(vec![
                Span::styled(
                    format!("mode: {}·{}  ", enabled, m.mode),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "synced:{}  pending:{}  error:{}  review:{}",
                        m.synced, m.pending, m.error, m.review_queue_pending
                    ),
                    theme::dim_style(),
                ),
            ])
        }
        None if state.loading => {
            Line::from(Span::styled("loading metrics\u{2026}", theme::dim_style()))
        }
        None => Line::from(Span::styled("no metrics", theme::dim_style())),
    };
    f.render_widget(Paragraph::new(metrics_line), chunks[0]);

    // ── Focus tabs ──────────────────────────────────────────────────────────
    let pending_tab = if state.focus_pending {
        Span::styled(
            format!(" À valider ({}) ", state.pending.len()),
            theme::tab_active(),
        )
    } else {
        Span::styled(
            format!(" À valider ({}) ", state.pending.len()),
            theme::tab_inactive(),
        )
    };
    let committed_tab = if state.focus_pending {
        Span::styled(
            format!(" Committés ({}) ", state.committed.len()),
            theme::tab_inactive(),
        )
    } else {
        Span::styled(
            format!(" Committés ({}) ", state.committed.len()),
            theme::tab_active(),
        )
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            pending_tab,
            Span::raw("  "),
            committed_tab,
        ])),
        chunks[1],
    );

    // ── List ────────────────────────────────────────────────────────────────
    let items: Vec<ListItem> = if state.focus_pending {
        state
            .pending
            .iter()
            .map(|p| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:.2}  ", p.confidence),
                        Style::default().fg(theme::ACCENT),
                    ),
                    Span::styled(format!("{} ", p.subject), Style::default().fg(theme::TEXT)),
                    Span::styled(format!("{} ", p.predicate), theme::dim_style()),
                    Span::raw(p.object.clone()),
                ]))
            })
            .collect()
    } else {
        state
            .committed
            .iter()
            .map(|c| {
                let sync_style = match c.sync_status.as_str() {
                    "synced" => Style::default().fg(theme::GREEN),
                    "pending" => Style::default().fg(theme::ACCENT),
                    "error" => Style::default().fg(theme::RED),
                    _ => theme::dim_style(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<8}", c.sync_status), sync_style),
                    Span::styled(format!("{:<28} ", c.source), theme::dim_style()),
                    Span::styled(format!("{} ", c.subject), Style::default().fg(theme::TEXT)),
                    Span::styled(format!("{} ", c.predicate), theme::dim_style()),
                    Span::raw(c.object.clone()),
                ]))
            })
            .collect()
    };
    let list = List::new(items).highlight_style(theme::selected_style());
    f.render_stateful_widget(list, chunks[2], &mut state.list_state);

    // ── Hints ───────────────────────────────────────────────────────────────
    let hints = if state.focus_pending {
        "  [\u{2191}\u{2193}] nav  [a] approve  [d] deny  [Tab] switch  [r] refresh"
    } else {
        "  [\u{2191}\u{2193}] nav  [Tab] back to pending  [r] refresh"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hints, theme::hint_style()))),
        chunks[3],
    );
}

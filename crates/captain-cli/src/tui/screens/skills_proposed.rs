//! Skills Proposed screen: review and decide on auto-generated skill
//! proposals from the v3.13 Skill Synthesizer.

#![allow(dead_code)] // id/created_at/hash used by future detail view + sort.

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

#[derive(Clone, Default)]
pub struct Proposal {
    pub id: String,
    pub name: String,
    pub description: String,
    pub trigger_hint: String,
    pub tool_sequence: Vec<String>,
    pub confidence: f64,
    pub created_at: i64,
}

#[derive(Clone, Default)]
pub struct Pattern {
    pub hash: String,
    pub agent_id: String,
    pub tool_sequence: Vec<String>,
    pub count: u64,
    pub last_seen: i64,
}

#[derive(Clone, Default)]
pub struct SkillsMetrics {
    pub pending: u64,
    pub patterns_hot: u64,
    pub total_patterns: u64,
    pub approved: u64,
    pub denied: u64,
    pub mode: String,
    pub enabled: bool,
}

pub struct SkillsProposedState {
    pub proposals: Vec<Proposal>,
    pub patterns: Vec<Pattern>,
    pub metrics: Option<SkillsMetrics>,
    pub list_state: ListState,
    pub focus_proposals: bool,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

pub enum SkillsProposedAction {
    Continue,
    Refresh,
    Approve(String),
    Deny(String),
}

impl SkillsProposedState {
    pub fn new() -> Self {
        Self {
            proposals: Vec::new(),
            patterns: Vec::new(),
            metrics: None,
            list_state: ListState::default(),
            focus_proposals: true,
            loading: false,
            tick: 0,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SkillsProposedAction {
        match key.code {
            KeyCode::Char('r') => return SkillsProposedAction::Refresh,
            KeyCode::Tab => {
                self.focus_proposals = !self.focus_proposals;
                self.list_state.select(Some(0));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.current_len();
                if len == 0 {
                    return SkillsProposedAction::Continue;
                }
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 { len - 1 } else { i - 1 };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.current_len();
                if len == 0 {
                    return SkillsProposedAction::Continue;
                }
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1) % len));
            }
            KeyCode::Char('a') if self.focus_proposals => {
                if let Some(id) = self.selected_proposal_id() {
                    return SkillsProposedAction::Approve(id);
                }
            }
            KeyCode::Char('d') if self.focus_proposals => {
                if let Some(id) = self.selected_proposal_id() {
                    return SkillsProposedAction::Deny(id);
                }
            }
            _ => {}
        }
        SkillsProposedAction::Continue
    }

    fn current_len(&self) -> usize {
        if self.focus_proposals {
            self.proposals.len()
        } else {
            self.patterns.len()
        }
    }

    fn selected_proposal_id(&self) -> Option<String> {
        let i = self.list_state.selected()?;
        self.proposals.get(i).map(|p| p.id.clone())
    }
}

pub fn draw(f: &mut Frame, area: Rect, state: &mut SkillsProposedState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Skills proposés ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(inner);

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
                        "pending:{}  hot:{}  total:{}  ok:{}  deny:{}",
                        m.pending, m.patterns_hot, m.total_patterns, m.approved, m.denied
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

    let (props_tab, pats_tab) = if state.focus_proposals {
        (theme::tab_active(), theme::tab_inactive())
    } else {
        (theme::tab_inactive(), theme::tab_active())
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" À valider ({}) ", state.proposals.len()),
                props_tab,
            ),
            Span::raw("  "),
            Span::styled(format!(" Patterns ({}) ", state.patterns.len()), pats_tab),
        ])),
        chunks[1],
    );

    let items: Vec<ListItem> = if state.focus_proposals {
        state
            .proposals
            .iter()
            .map(|p| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:.2}  ", p.confidence),
                        Style::default().fg(theme::ACCENT),
                    ),
                    Span::styled(format!("{:<24} ", p.name), Style::default().fg(theme::TEXT)),
                    Span::styled(p.description.clone(), theme::dim_style()),
                ]))
            })
            .collect()
    } else {
        state
            .patterns
            .iter()
            .map(|pt| {
                let count_style = if pt.count >= 5 {
                    Style::default().fg(theme::ACCENT)
                } else {
                    theme::dim_style()
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<5} ", pt.count), count_style),
                    Span::styled(format!("{:<20} ", pt.agent_id), theme::dim_style()),
                    Span::raw(pt.tool_sequence.join(" → ")),
                ]))
            })
            .collect()
    };
    let list = List::new(items).highlight_style(theme::selected_style());
    f.render_stateful_widget(list, chunks[2], &mut state.list_state);

    let hints = if state.focus_proposals {
        "  [\u{2191}\u{2193}] nav  [a] approve  [d] deny  [Tab] patterns  [r] refresh"
    } else {
        "  [\u{2191}\u{2193}] nav  [Tab] back  [r] refresh"
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hints, theme::hint_style()))),
        chunks[3],
    );
}

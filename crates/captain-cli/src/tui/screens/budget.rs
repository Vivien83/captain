//! Budget screen: global limits + per-agent spend ranking (read-only).

#![allow(dead_code)]

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

#[derive(Clone, Default)]
pub struct BudgetGlobal {
    pub hourly_usd: f64,
    pub daily_usd: f64,
    pub monthly_usd: f64,
    pub hourly_limit: f64,
    pub daily_limit: f64,
    pub monthly_limit: f64,
    pub alert_threshold: f64,
}

#[derive(Clone, Default)]
pub struct AgentSpend {
    pub agent_id: String,
    pub agent_name: String,
    pub hourly_usd: f64,
    pub daily_usd: f64,
    pub monthly_usd: f64,
}

pub struct BudgetState {
    pub global: Option<BudgetGlobal>,
    pub agents: Vec<AgentSpend>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
}

pub enum BudgetAction {
    Continue,
    Refresh,
}

impl BudgetState {
    pub fn new() -> Self {
        Self {
            global: None,
            agents: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            tick: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> BudgetAction {
        match key.code {
            KeyCode::Char('r') => BudgetAction::Refresh,
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.agents.is_empty() {
                    let i = self.list_state.selected().unwrap_or(0);
                    let next = if i == 0 { self.agents.len() - 1 } else { i - 1 };
                    self.list_state.select(Some(next));
                }
                BudgetAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !self.agents.is_empty() {
                    let i = self.list_state.selected().unwrap_or(0);
                    self.list_state.select(Some((i + 1) % self.agents.len()));
                }
                BudgetAction::Continue
            }
            _ => BudgetAction::Continue,
        }
    }
}

fn pct(spent: f64, limit: f64) -> f64 {
    if limit <= 0.0 {
        0.0
    } else {
        (spent / limit * 100.0).min(999.0)
    }
}

fn bar_style(p: f64) -> Style {
    if p >= 95.0 {
        Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)
    } else if p >= 80.0 {
        Style::default().fg(theme::YELLOW)
    } else {
        Style::default().fg(theme::GREEN)
    }
}

pub fn draw(f: &mut Frame, area: Rect, state: &mut BudgetState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Budget ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(5), // global
        Constraint::Length(1), // header
        Constraint::Min(3),    // agents
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // ── Global ──────────────────────────────────────────────────────────────
    let global_lines = match &state.global {
        Some(g) => {
            let fmt = |label: &str, spent: f64, limit: f64| {
                let p = pct(spent, limit);
                let limit_str = if limit > 0.0 {
                    format!("${limit:.2}")
                } else {
                    "∞".to_string()
                };
                Line::from(vec![
                    Span::styled(format!("{label:<8} "), theme::dim_style()),
                    Span::styled(
                        format!("${spent:.4} / {limit_str}  "),
                        Style::default().fg(theme::TEXT),
                    ),
                    Span::styled(format!("({p:.1}%)"), bar_style(p)),
                ])
            };
            vec![
                Line::from(Span::styled(
                    "Global spend",
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                )),
                fmt("hourly", g.hourly_usd, g.hourly_limit),
                fmt("daily", g.daily_usd, g.daily_limit),
                fmt("monthly", g.monthly_usd, g.monthly_limit),
            ]
        }
        None if state.loading => vec![Line::from(Span::styled(
            "Chargement du budget…",
            theme::dim_style(),
        ))],
        None => vec![Line::from(Span::styled(
            "Pas de données budgétaires.",
            theme::dim_style(),
        ))],
    };
    f.render_widget(Paragraph::new(global_lines), chunks[0]);

    // ── Header agents ───────────────────────────────────────────────────────
    let header = format!(
        "  {:<24} {:>12} {:>12} {:>12}",
        "agent", "hourly", "daily", "monthly"
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(header, theme::table_header()))),
        chunks[1],
    );

    // ── Agents list ─────────────────────────────────────────────────────────
    if state.agents.is_empty() {
        let msg = if state.loading {
            "  Chargement…"
        } else {
            "  Aucun agent n'a de consommation récente."
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, theme::dim_style()))),
            chunks[2],
        );
    } else {
        let items: Vec<ListItem> = state
            .agents
            .iter()
            .map(|a| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:<24} ", a.agent_name),
                        Style::default().fg(theme::TEXT),
                    ),
                    Span::styled(format!("{:>11.4} ", a.hourly_usd), theme::dim_style()),
                    Span::styled(format!("{:>11.4} ", a.daily_usd), theme::dim_style()),
                    Span::styled(
                        format!("{:>11.4}", a.monthly_usd),
                        Style::default().fg(theme::ACCENT),
                    ),
                ]))
            })
            .collect();
        let list = List::new(items).highlight_style(theme::selected_style());
        f.render_stateful_widget(list, chunks[2], &mut state.list_state);
    }

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  [\u{2191}\u{2193}] nav  [r] refresh  (lecture seule — édition via config.toml)",
            theme::hint_style(),
        ))),
        chunks[3],
    );
}

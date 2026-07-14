//! Knowledge graph screen: stats header + entity list + recent facts.

#![allow(dead_code)]

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

#[derive(Clone, Default)]
pub struct GraphStats {
    pub entities: u64,
    pub facts: u64,
    pub episodes: u64,
}

#[derive(Clone, Default)]
pub struct GraphEntity {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub fact_count: u64,
}

#[derive(Clone, Default)]
pub struct GraphFact {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f64,
}

pub struct GraphState {
    pub stats: Option<GraphStats>,
    pub entities: Vec<GraphEntity>,
    pub facts: Vec<GraphFact>,
    pub list_state: ListState,
    pub focus_entities: bool,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
    /// Phase-i.4: text-search mode over the graph (BM25 + hybrid).
    pub search_mode: bool,
    pub search_buf: String,
}

pub enum GraphAction {
    Continue,
    Refresh,
    Search(String),
}

impl GraphState {
    pub fn new() -> Self {
        Self {
            stats: None,
            entities: Vec::new(),
            facts: Vec::new(),
            list_state: ListState::default(),
            focus_entities: true,
            loading: false,
            tick: 0,
            status_msg: String::new(),
            search_mode: false,
            search_buf: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> GraphAction {
        if self.search_mode {
            match key.code {
                KeyCode::Esc => {
                    self.search_mode = false;
                    self.search_buf.clear();
                }
                KeyCode::Enter => {
                    self.search_mode = false;
                    let q = self.search_buf.clone();
                    if !q.is_empty() {
                        return GraphAction::Search(q);
                    }
                }
                KeyCode::Backspace => {
                    self.search_buf.pop();
                }
                KeyCode::Char(c) => {
                    self.search_buf.push(c);
                }
                _ => {}
            }
            return GraphAction::Continue;
        }
        match key.code {
            KeyCode::Char('r') => return GraphAction::Refresh,
            KeyCode::Char('/') => {
                self.search_mode = true;
                self.search_buf.clear();
            }
            KeyCode::Tab => {
                self.focus_entities = !self.focus_entities;
                self.list_state.select(Some(0));
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let len = self.current_len();
                if len > 0 {
                    let i = self.list_state.selected().unwrap_or(0);
                    let next = if i == 0 { len - 1 } else { i - 1 };
                    self.list_state.select(Some(next));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.current_len();
                if len > 0 {
                    let i = self.list_state.selected().unwrap_or(0);
                    self.list_state.select(Some((i + 1) % len));
                }
            }
            _ => {}
        }
        GraphAction::Continue
    }

    fn current_len(&self) -> usize {
        if self.focus_entities {
            self.entities.len()
        } else {
            self.facts.len()
        }
    }
}

pub fn draw(f: &mut Frame, area: Rect, state: &mut GraphState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Graph ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1), // stats
        Constraint::Length(1), // focus tabs
        Constraint::Min(3),    // list
        Constraint::Length(1), // hints
    ])
    .split(inner);

    let stats_line = match &state.stats {
        Some(s) => Line::from(vec![
            Span::styled(
                format!("{} entités  ", s.entities),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{} faits  ", s.facts),
                Style::default().fg(theme::TEXT),
            ),
            Span::styled(format!("{} épisodes", s.episodes), theme::dim_style()),
        ]),
        None if state.loading => Line::from(Span::styled("Chargement…", theme::dim_style())),
        None => Line::from(Span::styled("pas de stats", theme::dim_style())),
    };
    f.render_widget(Paragraph::new(stats_line), chunks[0]);

    let (ent_tab, facts_tab) = if state.focus_entities {
        (theme::tab_active(), theme::tab_inactive())
    } else {
        (theme::tab_inactive(), theme::tab_active())
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" Entités ({}) ", state.entities.len()), ent_tab),
            Span::raw("  "),
            Span::styled(format!(" Faits ({}) ", state.facts.len()), facts_tab),
        ])),
        chunks[1],
    );

    let items: Vec<ListItem> = if state.focus_entities {
        state
            .entities
            .iter()
            .map(|e| {
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<20} ", e.name), Style::default().fg(theme::TEXT)),
                    Span::styled(format!("{:<14} ", e.kind), theme::dim_style()),
                    Span::styled(
                        format!("{} faits", e.fact_count),
                        Style::default().fg(theme::ACCENT),
                    ),
                ]))
            })
            .collect()
    } else {
        state
            .facts
            .iter()
            .map(|ft| {
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{:.2}  ", ft.confidence),
                        Style::default().fg(theme::ACCENT),
                    ),
                    Span::styled(format!("{} ", ft.subject), Style::default().fg(theme::TEXT)),
                    Span::styled(format!("{} ", ft.predicate), theme::dim_style()),
                    Span::raw(ft.object.clone()),
                ]))
            })
            .collect()
    };
    let list = List::new(items).highlight_style(theme::selected_style());
    f.render_stateful_widget(list, chunks[2], &mut state.list_state);

    let hints = if state.search_mode {
        format!(
            "  / {}\u{2588}  [Enter] chercher  [Esc] annuler",
            state.search_buf
        )
    } else {
        "  [\u{2191}\u{2193}] nav  [Tab] entités/faits  [/] chercher  [r] refresh".to_string()
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hints, theme::hint_style()))),
        chunks[3],
    );
}

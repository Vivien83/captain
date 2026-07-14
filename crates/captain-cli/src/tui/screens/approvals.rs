//! Approvals screen: list pending approval requests, approve / reject them.

#![allow(dead_code)]

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph};
use ratatui::Frame;

#[derive(Clone, Default)]
pub struct ApprovalRequest {
    pub id: String,
    pub agent_name: String,
    pub tool_name: String,
    pub description: String,
    pub action: String,
    pub risk_level: String,
    pub created_at: i64,
}

pub struct ApprovalsState {
    pub pending: Vec<ApprovalRequest>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
}

#[derive(Debug)]
pub enum ApprovalsAction {
    Continue,
    Refresh,
    /// Q.11 — approve this single occurrence (back-compat with `[a]`/`[y]`/`[o]`).
    Approve(String),
    /// Q.11 — approve all calls to the same `(agent, tool)` until daemon restart.
    ApproveSession(String),
    /// Q.11 — approve all calls to this tool forever (persisted to policy).
    ApproveAlways(String),
    Reject(String),
}

impl ApprovalsState {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
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
        self.pending.get(i).map(|a| a.id.clone())
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ApprovalsAction {
        match key.code {
            KeyCode::Char('r') => return ApprovalsAction::Refresh,
            KeyCode::Up | KeyCode::Char('k') => {
                if self.pending.is_empty() {
                    return ApprovalsAction::Continue;
                }
                let i = self.list_state.selected().unwrap_or(0);
                let next = if i == 0 {
                    self.pending.len() - 1
                } else {
                    i - 1
                };
                self.list_state.select(Some(next));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.pending.is_empty() {
                    return ApprovalsAction::Continue;
                }
                let i = self.list_state.selected().unwrap_or(0);
                self.list_state.select(Some((i + 1) % self.pending.len()));
            }
            KeyCode::Char('o') | KeyCode::Char('a') | KeyCode::Char('y') => {
                if let Some(id) = self.selected_id() {
                    return ApprovalsAction::Approve(id);
                }
            }
            KeyCode::Char('s') => {
                if let Some(id) = self.selected_id() {
                    return ApprovalsAction::ApproveSession(id);
                }
            }
            KeyCode::Char('A') => {
                if let Some(id) = self.selected_id() {
                    return ApprovalsAction::ApproveAlways(id);
                }
            }
            KeyCode::Char('R') | KeyCode::Char('d') | KeyCode::Char('n') => {
                if let Some(id) = self.selected_id() {
                    return ApprovalsAction::Reject(id);
                }
            }
            _ => {}
        }
        ApprovalsAction::Continue
    }
}

pub fn draw(f: &mut Frame, area: Rect, state: &mut ApprovalsState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            format!(" Approvals ({} en attente) ", state.pending.len()),
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(inner);

    let header = format!(
        "  {:<6} {:<18} {:<14} {}",
        "risk", "agent", "tool", "action"
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(header, theme::table_header()))),
        chunks[0],
    );

    if state.pending.is_empty() {
        let msg = if state.loading {
            "  Chargement…"
        } else {
            "  Aucune demande d'approbation en attente."
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, theme::dim_style()))),
            chunks[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .pending
            .iter()
            .map(|a| {
                let risk_style = match a.risk_level.as_str() {
                    "high" | "critical" => {
                        Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)
                    }
                    "medium" => Style::default().fg(theme::YELLOW),
                    _ => theme::dim_style(),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{:<6} ", a.risk_level), risk_style),
                    Span::styled(format!("{:<18} ", a.agent_name), theme::dim_style()),
                    Span::styled(
                        format!("{:<14} ", a.tool_name),
                        Style::default().fg(theme::ACCENT),
                    ),
                    Span::raw(a.action.clone()),
                ]))
            })
            .collect();
        let list = List::new(items).highlight_style(theme::selected_style());
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    let hints =
        "  [\u{2191}\u{2193}] nav  [o] once  [s] session  [A] always  [R] reject  [r] refresh";
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hints, theme::hint_style()))),
        chunks[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn state_with_one_pending() -> ApprovalsState {
        let mut s = ApprovalsState::new();
        s.pending.push(ApprovalRequest {
            id: "abc".into(),
            agent_name: "captain".into(),
            tool_name: "shell_exec".into(),
            description: "ls /tmp".into(),
            action: "ls".into(),
            risk_level: "high".into(),
            created_at: 0,
        });
        s.list_state.select(Some(0));
        s
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn shift_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    #[test]
    fn q11b_key_o_returns_approve_once() {
        let mut s = state_with_one_pending();
        match s.handle_key(key(KeyCode::Char('o'))) {
            ApprovalsAction::Approve(id) => assert_eq!(id, "abc"),
            other => panic!(
                "expected Approve, got: {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn q11b_key_s_returns_approve_session() {
        let mut s = state_with_one_pending();
        match s.handle_key(key(KeyCode::Char('s'))) {
            ApprovalsAction::ApproveSession(id) => assert_eq!(id, "abc"),
            other => panic!(
                "expected ApproveSession, got: {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn q11b_key_uppercase_a_returns_approve_always() {
        let mut s = state_with_one_pending();
        match s.handle_key(shift_key(KeyCode::Char('A'))) {
            ApprovalsAction::ApproveAlways(id) => assert_eq!(id, "abc"),
            other => panic!(
                "expected ApproveAlways, got: {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[test]
    fn q11b_key_uppercase_r_returns_reject() {
        let mut s = state_with_one_pending();
        match s.handle_key(shift_key(KeyCode::Char('R'))) {
            ApprovalsAction::Reject(id) => assert_eq!(id, "abc"),
            other => panic!("expected Reject, got: {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn q11b_legacy_keys_still_work() {
        let mut s = state_with_one_pending();
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('a'))),
            ApprovalsAction::Approve(_)
        ));
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('y'))),
            ApprovalsAction::Approve(_)
        ));
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('d'))),
            ApprovalsAction::Reject(_)
        ));
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('n'))),
            ApprovalsAction::Reject(_)
        ));
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('r'))),
            ApprovalsAction::Refresh
        ));
    }

    #[test]
    fn q11b_action_keys_noop_when_no_selection() {
        let mut s = ApprovalsState::new();
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('s'))),
            ApprovalsAction::Continue
        ));
        assert!(matches!(
            s.handle_key(shift_key(KeyCode::Char('A'))),
            ApprovalsAction::Continue
        ));
    }
}

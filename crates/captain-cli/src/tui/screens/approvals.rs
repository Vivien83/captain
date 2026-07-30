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

#[derive(Clone, Default)]
pub struct ApprovalRule {
    pub id: String,
    pub effect: String,
    pub agent_id: String,
    pub tool_name: String,
    pub action_digest: String,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalsFocus {
    Pending,
    Rules,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectScope {
    Once,
    Session,
    Always,
}

struct RejectDraft {
    id: String,
    scope: RejectScope,
    reason: String,
}

pub struct ApprovalsState {
    pub pending: Vec<ApprovalRequest>,
    pub list_state: ListState,
    pub rules: Vec<ApprovalRule>,
    pub rules_state: ListState,
    pub focus: ApprovalsFocus,
    pub loading: bool,
    pub tick: usize,
    pub status_msg: String,
    reject_draft: Option<RejectDraft>,
}

#[derive(Debug)]
pub enum ApprovalsAction {
    Continue,
    Refresh,
    /// Q.11 — approve this single occurrence (back-compat with `[a]`/`[y]`/`[o]`).
    Approve(String),
    /// Approve this exact agent/tool/action tuple until daemon restart.
    ApproveSession(String),
    /// Persist a revocable allow rule for this exact agent/tool/action tuple.
    ApproveAlways(String),
    Reject {
        id: String,
        scope: RejectScope,
        reason: String,
    },
    RevokeRule(String),
}

impl ApprovalsState {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            list_state: ListState::default(),
            rules: Vec::new(),
            rules_state: ListState::default(),
            focus: ApprovalsFocus::Pending,
            loading: false,
            tick: 0,
            status_msg: String::new(),
            reject_draft: None,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn selected_id(&self) -> Option<String> {
        let i = self.list_state.selected()?;
        self.pending.get(i).map(|a| a.id.clone())
    }

    fn selected_rule_id(&self) -> Option<String> {
        let i = self.rules_state.selected()?;
        self.rules.get(i).map(|rule| rule.id.clone())
    }

    fn start_reject(&mut self, scope: RejectScope) {
        if let Some(id) = self.selected_id() {
            self.reject_draft = Some(RejectDraft {
                id,
                scope,
                reason: String::new(),
            });
            self.status_msg = "Saisis un motif puis Entrée. Échap annule.".to_string();
        }
    }

    fn move_selection(&mut self, previous: bool) {
        let (len, state) = match self.focus {
            ApprovalsFocus::Pending => (self.pending.len(), &mut self.list_state),
            ApprovalsFocus::Rules => (self.rules.len(), &mut self.rules_state),
        };
        if len == 0 {
            return;
        }
        let current = state.selected().unwrap_or(0);
        let next = if previous {
            if current == 0 {
                len - 1
            } else {
                current - 1
            }
        } else {
            (current + 1) % len
        };
        state.select(Some(next));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ApprovalsAction {
        if let Some(draft) = self.reject_draft.as_mut() {
            match key.code {
                KeyCode::Esc => {
                    self.reject_draft = None;
                    self.status_msg = "Refus annulé.".to_string();
                }
                KeyCode::Enter => {
                    let reason = draft.reason.trim().to_string();
                    if draft.scope == RejectScope::Always && reason.is_empty() {
                        self.status_msg =
                            "Un motif est obligatoire pour une règle durable.".to_string();
                        return ApprovalsAction::Continue;
                    }
                    let action = ApprovalsAction::Reject {
                        id: draft.id.clone(),
                        scope: draft.scope,
                        reason,
                    };
                    self.reject_draft = None;
                    return action;
                }
                KeyCode::Backspace => {
                    draft.reason.pop();
                }
                KeyCode::Char(ch) if draft.reason.chars().count() < 280 => {
                    draft.reason.push(ch);
                }
                _ => {}
            }
            return ApprovalsAction::Continue;
        }

        match key.code {
            KeyCode::Char('r') => return ApprovalsAction::Refresh,
            KeyCode::Tab => {
                self.focus = match self.focus {
                    ApprovalsFocus::Pending => ApprovalsFocus::Rules,
                    ApprovalsFocus::Rules => ApprovalsFocus::Pending,
                };
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_selection(true);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_selection(false);
            }
            KeyCode::Char('o') | KeyCode::Char('a') | KeyCode::Char('y')
                if self.focus == ApprovalsFocus::Pending =>
            {
                if let Some(id) = self.selected_id() {
                    return ApprovalsAction::Approve(id);
                }
            }
            KeyCode::Char('s') if self.focus == ApprovalsFocus::Pending => {
                if let Some(id) = self.selected_id() {
                    return ApprovalsAction::ApproveSession(id);
                }
            }
            KeyCode::Char('A') if self.focus == ApprovalsFocus::Pending => {
                if let Some(id) = self.selected_id() {
                    return ApprovalsAction::ApproveAlways(id);
                }
            }
            KeyCode::Char('R') | KeyCode::Char('d') | KeyCode::Char('n')
                if self.focus == ApprovalsFocus::Pending =>
            {
                self.start_reject(RejectScope::Once)
            }
            KeyCode::Char('D') if self.focus == ApprovalsFocus::Pending => {
                self.start_reject(RejectScope::Session)
            }
            KeyCode::Char('X') if self.focus == ApprovalsFocus::Pending => {
                self.start_reject(RejectScope::Always)
            }
            KeyCode::Char('x') if self.focus == ApprovalsFocus::Rules => {
                if let Some(id) = self.selected_rule_id() {
                    return ApprovalsAction::RevokeRule(id);
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
            format!(
                " Approvals ({} en attente · {} règles) ",
                state.pending.len(),
                state.rules.len()
            ),
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER))
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Percentage(48),
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(2),
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
        let list = List::new(items).highlight_style(if state.focus == ApprovalsFocus::Pending {
            theme::selected_style()
        } else {
            theme::dim_style()
        });
        f.render_stateful_widget(list, chunks[1], &mut state.list_state);
    }

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  RÈGLES DURABLES — agent · outil · empreinte exacte (jamais la commande brute)",
            theme::table_header(),
        ))),
        chunks[2],
    );
    let rule_items: Vec<ListItem> = state
        .rules
        .iter()
        .map(|rule| {
            let effect_style = if rule.effect == "deny" {
                Style::default().fg(theme::RED)
            } else {
                Style::default().fg(theme::GREEN)
            };
            let digest: String = rule.action_digest.chars().take(10).collect();
            let mut spans = vec![
                Span::styled(format!("{:<8} ", rule.effect), effect_style),
                Span::styled(
                    format!("{:<14} ", rule.tool_name),
                    Style::default().fg(theme::ACCENT),
                ),
                Span::raw(format!("{} · {}", rule.agent_id, digest)),
            ];
            if !rule.reason.is_empty() {
                spans.push(Span::styled(
                    format!(" · {}", rule.reason),
                    theme::dim_style(),
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();
    if rule_items.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  Aucune règle durable.",
                theme::dim_style(),
            ))),
            chunks[3],
        );
    } else {
        let rules =
            List::new(rule_items).highlight_style(if state.focus == ApprovalsFocus::Rules {
                theme::selected_style()
            } else {
                theme::dim_style()
            });
        f.render_stateful_widget(rules, chunks[3], &mut state.rules_state);
    }

    let (status, hints) = if let Some(draft) = state.reject_draft.as_ref() {
        let scope = match draft.scope {
            RejectScope::Once => "une fois",
            RejectScope::Session => "session",
            RejectScope::Always => "durable",
        };
        (
            format!("  Motif ({scope}) : {}_", draft.reason),
            "  [Entrée] confirmer  [Échap] annuler".to_string(),
        )
    } else {
        (
            format!("  {}", state.status_msg),
            "  [Tab] zone  [↑↓] nav  [o/s/A] autoriser  [R/D/X] refuser  [x] révoquer  [r] refresh"
                .to_string(),
        )
    };
    f.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(status, theme::dim_style())),
            Line::from(Span::styled(hints, theme::hint_style())),
        ]),
        chunks[4],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};

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
        assert!(matches!(
            s.handle_key(shift_key(KeyCode::Char('R'))),
            ApprovalsAction::Continue
        ));
        for ch in "pas maintenant".chars() {
            s.handle_key(key(KeyCode::Char(ch)));
        }
        match s.handle_key(key(KeyCode::Enter)) {
            ApprovalsAction::Reject { id, scope, reason } => {
                assert_eq!(id, "abc");
                assert_eq!(scope, RejectScope::Once);
                assert_eq!(reason, "pas maintenant");
            }
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
            ApprovalsAction::Continue
        ));
        assert!(s.reject_draft.is_some());
        s.handle_key(key(KeyCode::Esc));
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('n'))),
            ApprovalsAction::Continue
        ));
        assert!(s.reject_draft.is_some());
        s.handle_key(key(KeyCode::Esc));
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

    #[test]
    fn durable_deny_requires_reason_and_rule_can_be_revoked() {
        let mut s = state_with_one_pending();
        s.handle_key(shift_key(KeyCode::Char('X')));
        assert!(matches!(
            s.handle_key(key(KeyCode::Enter)),
            ApprovalsAction::Continue
        ));
        assert!(s.status_msg.contains("obligatoire"));
        for ch in "commande interdite".chars() {
            s.handle_key(key(KeyCode::Char(ch)));
        }
        assert!(matches!(
            s.handle_key(key(KeyCode::Enter)),
            ApprovalsAction::Reject {
                scope: RejectScope::Always,
                ..
            }
        ));

        s.rules.push(ApprovalRule {
            id: "rule-1".into(),
            effect: "deny".into(),
            agent_id: "captain".into(),
            tool_name: "shell_exec".into(),
            action_digest: "a".repeat(64),
            reason: "commande interdite".into(),
        });
        s.rules_state.select(Some(0));
        s.focus = ApprovalsFocus::Rules;
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('x'))),
            ApprovalsAction::RevokeRule(id) if id == "rule-1"
        ));
    }

    #[test]
    fn approvals_render_without_overflow_on_desktop_and_compact_terminals() {
        for (width, height) in [(100, 28), (52, 18)] {
            let mut state = state_with_one_pending();
            state.rules.push(ApprovalRule {
                id: "rule-1".into(),
                effect: "deny".into(),
                agent_id: "captain".into(),
                tool_name: "shell_exec".into(),
                action_digest: "a".repeat(64),
                reason: "utilise le staging".into(),
            });
            state.rules_state.select(Some(0));
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| draw(frame, frame.area(), &mut state))
                .unwrap();

            let rendered = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("Approvals"));
            assert!(rendered.contains("shell_exec"));
        }
    }
}

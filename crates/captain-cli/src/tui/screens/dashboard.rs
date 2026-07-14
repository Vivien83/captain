//! Dashboard screen: system overview with stat cards and scrollable audit trail.

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::Frame;

pub use super::dashboard_status::StatusSnapshot;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct AuditRow {
    pub timestamp: String,
    pub agent: String,
    pub action: String,
    pub detail: String,
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct DashboardState {
    pub status: StatusSnapshot,
    pub agent_count: u64,
    pub uptime_secs: u64,
    pub version: String,
    pub provider: String,
    pub model: String,
    pub recent_audit: Vec<AuditRow>,
    pub loading: bool,
    pub tick: usize,
    pub audit_scroll: u16,
}

pub enum DashboardAction {
    Continue,
    Refresh,
}

impl DashboardState {
    pub fn new() -> Self {
        Self {
            status: StatusSnapshot::default(),
            agent_count: 0,
            uptime_secs: 0,
            version: String::new(),
            provider: String::new(),
            model: String::new(),
            recent_audit: Vec::new(),
            loading: false,
            tick: 0,
            audit_scroll: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DashboardAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return DashboardAction::Continue;
        }
        match key.code {
            KeyCode::Char('r') => DashboardAction::Refresh,
            KeyCode::Up | KeyCode::Char('k') => {
                self.audit_scroll = self.audit_scroll.saturating_add(1);
                DashboardAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.audit_scroll = self.audit_scroll.saturating_sub(1);
                DashboardAction::Continue
            }
            KeyCode::PageUp => {
                self.audit_scroll = self.audit_scroll.saturating_add(10);
                DashboardAction::Continue
            }
            KeyCode::PageDown => {
                self.audit_scroll = self.audit_scroll.saturating_sub(10);
                DashboardAction::Continue
            }
            _ => DashboardAction::Continue,
        }
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut DashboardState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Status ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Length(5), // operator cards
        Constraint::Length(7), // runtime signals
        Constraint::Length(1), // separator
        Constraint::Min(4),    // audit trail
        Constraint::Length(1), // hints
    ])
    .split(inner);

    super::dashboard_status_draw::draw_status_cockpit(f, chunks[0], chunks[1], &state.status);

    // ── Separator ───────────────────────────────────────────────────────────
    let sep = "\u{2500}".repeat(chunks[2].width as usize);
    f.render_widget(
        Paragraph::new(Span::styled(sep, theme::dim_style())),
        chunks[2],
    );

    // ── Audit trail ─────────────────────────────────────────────────────────
    draw_audit_trail(f, chunks[3], state);

    // ── Hints ───────────────────────────────────────────────────────────────
    let hints = Paragraph::new(Line::from(vec![Span::styled(
        "  [r] Refresh  [\u{2191}\u{2193}] Scroll audit",
        theme::hint_style(),
    )]));
    f.render_widget(hints, chunks[4]);
}

fn draw_audit_trail(f: &mut Frame, area: Rect, state: &DashboardState) {
    if state.loading {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
                Span::styled("Loading audit trail\u{2026}", theme::dim_style()),
            ])),
            area,
        );
        return;
    }

    if state.recent_audit.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled("  No audit entries yet.", theme::dim_style())),
            area,
        );
        return;
    }

    let mut lines: Vec<Line> = Vec::new();

    // Header
    lines.push(Line::from(vec![Span::styled(
        format!(
            "  {:<20} {:<14} {:<16} {}",
            "Timestamp", "Agent", "Action", "Detail"
        ),
        theme::table_header(),
    )]));

    for row in &state.recent_audit {
        lines.push(Line::from(vec![
            Span::styled(format!("  {:<20}", row.timestamp), theme::dim_style()),
            Span::styled(
                format!(" {:<14}", truncate(&row.agent, 13)),
                Style::default().fg(theme::CYAN),
            ),
            Span::styled(
                format!(" {:<16}", truncate(&row.action, 15)),
                Style::default().fg(theme::YELLOW),
            ),
            Span::styled(
                format!(" {}", truncate(&row.detail, 30)),
                theme::dim_style(),
            ),
        ]));
    }

    let total = lines.len() as u16;
    let visible = area.height;
    let max_scroll = total.saturating_sub(visible);
    let scroll = max_scroll
        .saturating_sub(state.audit_scroll)
        .min(max_scroll);

    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!(
            "{}\u{2026}",
            captain_types::truncate_str(s, max.saturating_sub(1))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_screen_renders_operational_cockpit_labels() {
        let mut state = DashboardState::new();
        state.status = StatusSnapshot {
            status: "running".to_string(),
            runtime_health_state: "ok".to_string(),
            agent_count: 3,
            tool_runs_completed: 7,
            agent_api_state: "ok".to_string(),
            consciousness_state: "steady".to_string(),
            channel_ready_count: 2,
            channel_total: 4,
            shutdown_status: "idle".to_string(),
            ..Default::default()
        };

        let backend = ratatui::backend::TestBackend::new(120, 28);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| draw(f, f.area(), &mut state))
            .expect("draw");

        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Status"));
        assert!(rendered.contains("Health"));
        assert!(rendered.contains("Tool Runs"));
        assert!(rendered.contains("Agent API"));
        assert!(rendered.contains("Awareness"));
    }
}

//! Budget screen: global limits + per-agent spend ranking (read-only).

#![allow(dead_code)]

use crate::tui::{
    provider_quota::{ProviderQuota, ProviderQuotaWindow},
    theme,
};
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
    pub provider_state: String,
    pub provider_quotas: Vec<ProviderQuota>,
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
            provider_state: "unavailable".to_string(),
            provider_quotas: Vec::new(),
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

    let provider_visible = state.provider_quotas.len().min(3);
    let provider_extra = usize::from(state.provider_quotas.len() > provider_visible);
    let provider_height = 1 + provider_visible.max(1) + provider_extra;
    let chunks = Layout::vertical([
        Constraint::Length(provider_height as u16), // provider subscription
        Constraint::Length(5),                      // global
        Constraint::Length(1),                      // header
        Constraint::Min(3),                         // agents
        Constraint::Length(1),                      // hints
    ])
    .split(inner);

    // Provider-owned subscription allowance, reported by the provider.
    let mut provider_lines = vec![Line::from(Span::styled(
        "Provider subscription (reported)",
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD),
    ))];
    if state.provider_quotas.is_empty() {
        provider_lines.push(Line::from(Span::styled(
            "Not observed yet; Captain does not infer provider allowance.",
            theme::dim_style(),
        )));
    } else {
        provider_lines.extend(
            state
                .provider_quotas
                .iter()
                .take(provider_visible)
                .map(provider_quota_line),
        );
        if provider_extra > 0 {
            provider_lines.push(Line::from(Span::styled(
                format!(
                    "+{} additional provider limit(s)",
                    state.provider_quotas.len() - provider_visible
                ),
                theme::dim_style(),
            )));
        }
    }
    f.render_widget(Paragraph::new(provider_lines), chunks[0]);

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
                    "Captain internal spend",
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
    f.render_widget(Paragraph::new(global_lines), chunks[1]);

    // ── Header agents ───────────────────────────────────────────────────────
    let header = format!(
        "  {:<24} {:>12} {:>12} {:>12}",
        "agent", "hourly", "daily", "monthly"
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(header, theme::table_header()))),
        chunks[2],
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
            chunks[3],
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
        f.render_stateful_widget(list, chunks[3], &mut state.list_state);
    }

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "  [\u{2191}\u{2193}] nav  [r] refresh  (lecture seule — édition via config.toml)",
            theme::hint_style(),
        ))),
        chunks[4],
    );
}

fn provider_quota_line(quota: &ProviderQuota) -> Line<'static> {
    let plan = quota
        .plan_type
        .as_deref()
        .map(|value| format!(" [{value}]"))
        .unwrap_or_default();
    let windows = quota
        .primary
        .iter()
        .chain(quota.secondary.iter())
        .map(provider_window_label)
        .collect::<Vec<_>>()
        .join(" | ");
    let stale = if quota.stale { " stale" } else { "" };
    let text = format!(
        "{}/{}{}  {}  [{}{}]",
        quota.provider, quota.limit_name, plan, windows, quota.alert_level, stale
    );
    Line::from(Span::styled(
        text,
        provider_alert_style(&quota.alert_level, quota.stale),
    ))
}

fn provider_window_label(window: &ProviderQuotaWindow) -> String {
    let duration = window
        .window_seconds
        .map(provider_duration_label)
        .unwrap_or_else(|| "window".to_string());
    let reset = window
        .resets_at
        .as_ref()
        .map(compact_reset_label)
        .map(|value| format!(" reset {value}"))
        .or_else(|| {
            window
                .reset_after_seconds
                .map(|seconds| format!(" reset ~{}", provider_duration_label(seconds)))
        })
        .unwrap_or_default();
    format!("{duration} {:.1}%{reset}", window.used_percent)
}

fn provider_duration_label(seconds: u64) -> String {
    if seconds % 604_800 == 0 {
        format!("{}w", seconds / 604_800)
    } else if seconds % 86_400 == 0 {
        format!("{}d", seconds / 86_400)
    } else if seconds % 3_600 == 0 {
        format!("{}h", seconds / 3_600)
    } else if seconds % 60 == 0 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn compact_reset_label(value: &chrono::DateTime<chrono::Utc>) -> String {
    value.format("%Y-%m-%d %H:%M").to_string()
}

fn provider_alert_style(alert: &str, stale: bool) -> Style {
    if alert == "exhausted" || alert == "critical" {
        Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)
    } else if alert == "warning" || stale {
        Style::default().fg(theme::YELLOW)
    } else {
        Style::default().fg(theme::GREEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_quota_parses_dynamic_windows_from_api() {
        let quota = ProviderQuota::from_json(&serde_json::json!({
            "provider": "codex",
            "limit_name": "Codex",
            "plan_type": "pro",
            "alert_level": "warning",
            "stale": false,
            "primary": {
                "used_percent": 72.5,
                "window_seconds": 18000,
                "resets_at": "2026-07-18T18:00:00Z"
            },
            "secondary": {"used_percent": 41.0, "window_seconds": 604800}
        }));

        assert_eq!(quota.primary.as_ref().unwrap().window_seconds, Some(18_000));
        assert_eq!(
            provider_window_label(quota.primary.as_ref().unwrap()),
            "5h 72.5% reset 2026-07-18 18:00"
        );
        assert_eq!(
            provider_window_label(quota.secondary.as_ref().unwrap()),
            "1w 41.0%"
        );
    }
}

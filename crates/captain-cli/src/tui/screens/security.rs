//! Security screen: security feature dashboard and chain verification.

use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph};
use ratatui::Frame;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SecurityFeature {
    pub name: String,
    pub active: bool,
    pub description: String,
    pub section: SecuritySection,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SecuritySection {
    Core,
    Configurable,
    Monitoring,
}

impl SecuritySection {
    fn label(self) -> &'static str {
        match self {
            Self::Core => "Core Security",
            Self::Configurable => "Configurable",
            Self::Monitoring => "Monitoring",
        }
    }
}

// ── Built-in feature definitions ────────────────────────────────────────────

pub(crate) fn features_for_execution_summary(
    policy_mode: &str,
    critical_mode: &str,
    isolation_level: &str,
    os_isolation: bool,
) -> Vec<SecurityFeature> {
    vec![
        // Core
        SecurityFeature {
            name: "Path Traversal Prevention".into(),
            active: true,
            description: "safe_resolve_path blocks ../../ attacks".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "SSRF Protection".into(),
            active: true,
            description: "Blocks private IPs and metadata endpoints in HTTP fetches".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Host Subprocess Boundary".into(),
            active: true,
            description: format!(
                "{isolation_level}; policy {policy_mode}/{critical_mode}; bounded lifecycle"
            ),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Host OS Isolation".into(),
            active: os_isolation,
            description: if os_isolation {
                "Operating-system isolation active for host subprocesses".into()
            } else {
                "None for shell_exec; use explicit Docker or WASM isolation".into()
            },
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "WASM Dual Metering".into(),
            active: true,
            description: "Fuel + epoch interruption with watchdog thread".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Capability Inheritance".into(),
            active: true,
            description: "validate_capability_inheritance prevents privilege escalation".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Secret Zeroization".into(),
            active: true,
            description: "Zeroizing<String> auto-wipes API keys from memory".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Ed25519 Manifest Signing".into(),
            active: true,
            description: "Signed agent manifests with Ed25519 verification".into(),
            section: SecuritySection::Core,
        },
        SecurityFeature {
            name: "Critical Command Guard".into(),
            active: true,
            description: "Normalized lexical heuristic; not OS containment".into(),
            section: SecuritySection::Core,
        },
        // Configurable (4)
        SecurityFeature {
            name: "OFP Wire Auth".into(),
            active: true,
            description: "HMAC-SHA256 mutual authentication with nonce".into(),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: "RBAC Multi-User".into(),
            active: true,
            description: "Role-based access control with user hierarchy".into(),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: "Rate Limiting".into(),
            active: true,
            description: "GCRA rate limiter with cost-aware tokens".into(),
            section: SecuritySection::Configurable,
        },
        SecurityFeature {
            name: "Security Headers".into(),
            active: true,
            description: "CSP, X-Frame-Options, HSTS middleware".into(),
            section: SecuritySection::Configurable,
        },
        // Monitoring (3)
        SecurityFeature {
            name: "Versioned Audit Hash Chain".into(),
            active: true,
            description: "Hash chain audit log with tamper detection".into(),
            section: SecuritySection::Monitoring,
        },
        SecurityFeature {
            name: "Heartbeat Monitor".into(),
            active: true,
            description: "Background health checks with restart limits".into(),
            section: SecuritySection::Monitoring,
        },
        SecurityFeature {
            name: "Advisory Skill Phrase Review".into(),
            active: true,
            description: "Bounded phrase matches; not a security proof".into(),
            section: SecuritySection::Monitoring,
        },
    ]
}

fn builtin_features() -> Vec<SecurityFeature> {
    let policy = captain_types::config::ExecPolicy::default();
    let posture = policy.host_execution_posture();
    features_for_execution_summary(
        posture.policy_mode.as_str(),
        posture.critical_mode.as_str(),
        posture.isolation_level,
        posture.os_isolation,
    )
}

// ── State ───────────────────────────────────────────────────────────────────

pub struct SecurityState {
    pub features: Vec<SecurityFeature>,
    pub chain_verified: Option<bool>,
    pub verify_result: String,
    pub scroll: u16,
    pub loading: bool,
    pub tick: usize,
}

pub enum SecurityAction {
    Continue,
    Refresh,
    VerifyChain,
}

impl SecurityState {
    pub fn new() -> Self {
        Self {
            features: builtin_features(),
            chain_verified: None,
            verify_result: String::new(),
            scroll: 0,
            loading: false,
            tick: 0,
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SecurityAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return SecurityAction::Continue;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_add(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_sub(1);
            }
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_add(10);
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_sub(10);
            }
            KeyCode::Char('v') => return SecurityAction::VerifyChain,
            KeyCode::Char('r') => return SecurityAction::Refresh,
            _ => {}
        }
        SecurityAction::Continue
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw(f: &mut Frame, area: Rect, state: &mut SecurityState) {
    let block = Block::default()
        .title(Line::from(vec![Span::styled(
            " Security ",
            theme::title_style(),
        )]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::vertical([
        Constraint::Min(4),    // features
        Constraint::Length(2), // verify result
        Constraint::Length(1), // hints
    ])
    .split(inner);

    // ── Features list ──
    let mut lines: Vec<Line> = Vec::new();
    let mut current_section: Option<SecuritySection> = None;

    for feat in &state.features {
        if current_section != Some(feat.section) {
            if current_section.is_some() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::from(vec![Span::styled(
                format!(
                    "  \u{2501}\u{2501} {} \u{2501}\u{2501}",
                    feat.section.label()
                ),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )]));
            current_section = Some(feat.section);
        }

        let (badge, badge_style) = if feat.active {
            ("\u{2714} Active", Style::default().fg(theme::GREEN))
        } else {
            ("\u{25cb} Inactive", Style::default().fg(theme::RED))
        };

        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<30}", feat.name),
                Style::default()
                    .fg(theme::CYAN)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {:<12}", badge), badge_style),
            Span::styled(format!(" {}", feat.description), theme::dim_style()),
        ]));
    }

    let total = lines.len() as u16;
    let visible = chunks[0].height;
    let max_scroll = total.saturating_sub(visible);
    let scroll = max_scroll.saturating_sub(state.scroll).min(max_scroll);

    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), chunks[0]);

    // ── Verify result ──
    match state.chain_verified {
        None => {
            if state.loading {
                let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(format!("  {spinner} "), Style::default().fg(theme::CYAN)),
                        Span::styled("Verifying audit chain\u{2026}", theme::dim_style()),
                    ])),
                    chunks[1],
                );
            } else {
                f.render_widget(
                    Paragraph::new(Line::from(vec![Span::styled(
                        "  Press [v] to verify audit chain integrity",
                        theme::dim_style(),
                    )])),
                    chunks[1],
                );
            }
        }
        Some(true) => {
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![Span::styled(
                        "  \u{2714} Audit chain verified",
                        Style::default().fg(theme::GREEN),
                    )]),
                    Line::from(vec![Span::styled(
                        format!("  {}", state.verify_result),
                        theme::dim_style(),
                    )]),
                ]),
                chunks[1],
            );
        }
        Some(false) => {
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(vec![Span::styled(
                        "  \u{2718} Audit chain verification failed",
                        Style::default().fg(theme::RED),
                    )]),
                    Line::from(vec![Span::styled(
                        format!("  {}", state.verify_result),
                        Style::default().fg(theme::RED),
                    )]),
                ]),
                chunks[1],
            );
        }
    }

    // ── Hints ──
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            "  [\u{2191}\u{2193}] Scroll  [v] Verify Chain  [r] Refresh",
            theme::hint_style(),
        )])),
        chunks[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_phrase_review_is_presented_as_advisory() {
        let features = features_for_execution_summary("full", "safe", "host_process", false);
        let review = features
            .iter()
            .find(|feature| feature.name == "Advisory Skill Phrase Review")
            .expect("advisory skill review must remain visible");

        assert!(review.active);
        assert!(review.description.contains("not a security proof"));
        assert!(features
            .iter()
            .all(|feature| feature.name != "Prompt Injection Scanner"));
    }
}

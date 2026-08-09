//! Read-only TUI representation of immutable user-facing artifacts.

use crate::i18n::Lang;
use crate::tui::theme;
use captain_types::artifact::{ArtifactInventory, ArtifactStoreStatus, ArtifactVersion};
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use uuid::Uuid;

const REFRESH_TICKS: u16 = 300;

pub struct ArtifactsState {
    pub items: Vec<ArtifactVersion>,
    pub versions: Vec<ArtifactVersion>,
    pub store_status: Option<ArtifactStoreStatus>,
    pub list_state: ListState,
    pub version_index: usize,
    pub loading_inventory: bool,
    pub error: String,
    loading_versions_for: Option<Uuid>,
    tick: usize,
    refresh_ticks: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactsAction {
    Continue,
    Close,
    Refresh,
    LoadVersions(Uuid),
}

impl ArtifactsState {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            versions: Vec::new(),
            store_status: None,
            list_state: ListState::default(),
            version_index: 0,
            loading_inventory: false,
            error: String::new(),
            loading_versions_for: None,
            tick: 0,
            refresh_ticks: 0,
        }
    }

    pub fn begin_inventory_load(&mut self) {
        self.loading_inventory = true;
        self.refresh_ticks = 0;
    }

    pub fn apply_inventory(&mut self, result: Result<ArtifactInventory, String>) -> Option<Uuid> {
        self.loading_inventory = false;
        match result {
            Ok(inventory) => {
                let selected_id = self.selected_artifact().map(|item| item.artifact_id);
                self.items = inventory.items;
                self.store_status = Some(inventory.status);
                self.error.clear();
                let selected = selected_id
                    .and_then(|id| self.items.iter().position(|item| item.artifact_id == id))
                    .unwrap_or(0);
                if self.items.is_empty() {
                    self.list_state.select(None);
                } else {
                    self.list_state.select(Some(selected));
                }
                let selected_id = self.selected_artifact().map(|item| item.artifact_id);
                if self.loading_versions_for != selected_id {
                    self.loading_versions_for = None;
                }
                self.versions.clear();
                self.version_index = 0;
                selected_id
            }
            Err(error) => {
                self.error = error;
                None
            }
        }
    }

    pub fn begin_versions_load(&mut self, artifact_id: Uuid) -> bool {
        if !self
            .selected_artifact()
            .is_some_and(|item| item.artifact_id == artifact_id)
        {
            return false;
        }
        if self.loading_versions_for == Some(artifact_id) {
            return false;
        }
        self.loading_versions_for = Some(artifact_id);
        self.versions.clear();
        self.version_index = 0;
        true
    }

    pub fn apply_versions(
        &mut self,
        artifact_id: Uuid,
        result: Result<Vec<ArtifactVersion>, String>,
    ) {
        if !self
            .selected_artifact()
            .is_some_and(|item| item.artifact_id == artifact_id)
            || self.loading_versions_for != Some(artifact_id)
        {
            return;
        }
        self.loading_versions_for = None;
        match result {
            Ok(versions) => {
                self.versions = versions;
                self.version_index = 0;
                self.error.clear();
            }
            Err(error) => {
                self.versions.clear();
                self.version_index = 0;
                self.error = error;
            }
        }
    }

    pub fn tick(&mut self) -> bool {
        self.tick = self.tick.wrapping_add(1);
        if self.loading_inventory {
            return false;
        }
        self.refresh_ticks = self.refresh_ticks.saturating_add(1);
        if self.refresh_ticks < REFRESH_TICKS {
            return false;
        }
        self.refresh_ticks = 0;
        true
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ArtifactsAction {
        match key.code {
            KeyCode::Esc => ArtifactsAction::Close,
            KeyCode::Char('r') => ArtifactsAction::Refresh,
            KeyCode::Up | KeyCode::Char('k') => self.move_artifact(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_artifact(1),
            KeyCode::Left | KeyCode::Char('h') => {
                if self.version_index + 1 < self.versions.len() {
                    self.version_index += 1;
                }
                ArtifactsAction::Continue
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.version_index = self.version_index.saturating_sub(1);
                ArtifactsAction::Continue
            }
            _ => ArtifactsAction::Continue,
        }
    }

    pub fn selected_artifact(&self) -> Option<&ArtifactVersion> {
        self.list_state
            .selected()
            .and_then(|index| self.items.get(index))
    }

    pub fn selected_version(&self) -> Option<&ArtifactVersion> {
        self.versions
            .get(self.version_index)
            .or_else(|| self.selected_artifact())
    }

    pub fn versions_loading_for(&self, artifact_id: Uuid) -> bool {
        self.loading_versions_for == Some(artifact_id)
    }

    fn move_artifact(&mut self, delta: isize) -> ArtifactsAction {
        if self.items.is_empty() {
            return ArtifactsAction::Continue;
        }
        let current = self.list_state.selected().unwrap_or(0);
        let next = if delta < 0 {
            current.checked_sub(1).unwrap_or(self.items.len() - 1)
        } else {
            (current + 1) % self.items.len()
        };
        self.list_state.select(Some(next));
        self.versions.clear();
        self.version_index = 0;
        ArtifactsAction::LoadVersions(self.items[next].artifact_id)
    }
}

pub fn draw(frame: &mut Frame, area: Rect, state: &mut ArtifactsState) {
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(area);

    frame.render_widget(status_line(state), rows[0]);

    let horizontal = area.width >= 78;
    let body = Layout::default()
        .direction(if horizontal {
            Direction::Horizontal
        } else {
            Direction::Vertical
        })
        .constraints(if horizontal {
            vec![Constraint::Percentage(43), Constraint::Percentage(57)]
        } else {
            vec![Constraint::Percentage(42), Constraint::Percentage(58)]
        })
        .split(rows[1]);
    draw_inventory(frame, body[0], state);
    draw_detail(frame, body[1], state);

    let footer = if state.error.is_empty() {
        if area.width >= 72 {
            " [up/down] File  [left/right] Version  [r] Refresh  [Esc] Close".to_string()
        } else {
            " [up/down] File  [left/right] Version  [r]  [Esc]".to_string()
        }
    } else {
        format!(" {}", state.error)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            footer,
            if state.error.is_empty() {
                theme::hint_style()
            } else {
                Style::default().fg(theme::RED)
            },
        ))),
        rows[2],
    );
}

fn status_line(state: &ArtifactsState) -> Paragraph<'static> {
    let (label, color, detail) = match state.store_status.as_ref() {
        Some(status) if status.healthy => (
            "Integrity verified",
            theme::GREEN,
            format!(
                "{} files | {} versions | {}",
                status.artifacts,
                status.versions,
                format_bytes(status.bytes)
            ),
        ),
        Some(status) => (
            "Integrity warning",
            theme::YELLOW,
            format!(
                "{} invalid | {} files | {}",
                status.invalid_entries,
                status.artifacts,
                format_bytes(status.bytes)
            ),
        ),
        None => (
            "Loading inventory",
            theme::CYAN,
            "immutable checksum-bound outputs".to_string(),
        ),
    };
    Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                format!(" {label}"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  {detail}"), theme::dim_style()),
        ]),
        Line::from(Span::styled(
            " Read-only metadata; active content is never rendered in the terminal.",
            theme::dim_style(),
        )),
    ])
}

fn draw_inventory(frame: &mut Frame, area: Rect, state: &mut ArtifactsState) {
    let block = Block::default()
        .title(" Files ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT));
    if state.loading_inventory && state.items.is_empty() {
        let spinner = theme::SPINNER_FRAMES[state.tick % theme::SPINNER_FRAMES.len()];
        frame.render_widget(
            Paragraph::new(format!(" {spinner} Loading verified files..."))
                .block(block)
                .style(theme::dim_style()),
            area,
        );
        return;
    }
    if state.items.is_empty() {
        frame.render_widget(
            Paragraph::new(" No files produced yet.")
                .block(block)
                .style(theme::dim_style()),
            area,
        );
        return;
    }
    let items = state
        .items
        .iter()
        .map(|item| {
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(truncate(&item.title, 30), theme::title_style()),
                    Span::styled(
                        format!("  v{}", item.version),
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(Span::styled(
                    format!(
                        "{} | {}",
                        truncate(&item.filename, 32),
                        format_bytes(item.size_bytes)
                    ),
                    theme::dim_style(),
                )),
            ])
        })
        .collect::<Vec<_>>();
    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_style())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut state.list_state);
}

fn draw_detail(frame: &mut Frame, area: Rect, state: &ArtifactsState) {
    let block = Block::default()
        .title(" Exact version ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::ACCENT));
    let Some(item) = state.selected_version() else {
        frame.render_widget(
            Paragraph::new(" Select a file to inspect its metadata.")
                .block(block)
                .style(theme::dim_style()),
            area,
        );
        return;
    };
    let version_position = if state.versions_loading_for(item.artifact_id) {
        format!("v{} (loading history)", item.version)
    } else if state.versions.is_empty() {
        format!("v{}", item.version)
    } else {
        format!(
            "v{} ({} of {})",
            item.version,
            state.version_index + 1,
            state.versions.len()
        )
    };
    let lines = vec![
        Line::from(Span::styled(item.title.clone(), theme::title_style())),
        Line::from(vec![
            Span::styled(
                format!("{version_position}  "),
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format_bytes(item.size_bytes), theme::dim_style()),
        ]),
        Line::from(Span::styled(
            "Preview and download: Control Web",
            theme::dim_style(),
        )),
        Line::from(""),
        detail_line("File", &item.filename),
        detail_line("Type", &item.mime_type),
        detail_line(
            "Created",
            &item.created_at.format("%Y-%m-%d %H:%M UTC").to_string(),
        ),
        detail_line("Agent", &item.agent_id),
        detail_line("Session", item.session_id.as_deref().unwrap_or("-")),
        detail_line("SHA-256", &item.sha256),
        Line::from(""),
        detail_line(
            "Summary",
            &single_line(item.summary.as_deref().unwrap_or("No summary")),
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn detail_line(label: &'static str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<9}"), theme::table_header()),
        Span::raw(value.to_string()),
    ])
}

pub fn chat_inventory_message(inventory: &ArtifactInventory, lang: Lang) -> String {
    if inventory.items.is_empty() {
        return match lang {
            Lang::Fr => "Aucun fichier produit n'est encore disponible.".to_string(),
            Lang::En => "No produced files are available yet.".to_string(),
        };
    }
    let heading = match lang {
        Lang::Fr => "Fichiers produits",
        Lang::En => "Produced files",
    };
    let totals = match lang {
        Lang::Fr => format!(
            "{} {}, {} {}, {}",
            inventory.status.artifacts,
            if inventory.status.artifacts == 1 {
                "fichier"
            } else {
                "fichiers"
            },
            inventory.status.versions,
            if inventory.status.versions == 1 {
                "version"
            } else {
                "versions"
            },
            format_bytes(inventory.status.bytes)
        ),
        Lang::En => format!(
            "{} {}, {} {}, {}",
            inventory.status.artifacts,
            if inventory.status.artifacts == 1 {
                "file"
            } else {
                "files"
            },
            inventory.status.versions,
            if inventory.status.versions == 1 {
                "version"
            } else {
                "versions"
            },
            format_bytes(inventory.status.bytes)
        ),
    };
    let mut lines = vec![format!("**{heading}** - {totals}")];
    for item in inventory.items.iter().take(12) {
        lines.push(format!(
            "- **{}** - `v{}` - `{}` - {} - SHA `{}`",
            item.title,
            item.version,
            item.filename,
            format_bytes(item.size_bytes),
            captain_types::truncate_str(&item.sha256, 12)
        ));
    }
    if inventory.items.len() > 12 {
        lines.push(match lang {
            Lang::Fr => format!("- ... {} autres", inventory.items.len() - 12),
            Lang::En => format!("- ... {} more", inventory.items.len() - 12),
        });
    }
    lines.push(match lang {
        Lang::Fr => {
            "Aperçu sandboxé, historique complet et téléchargement : Control Web.".to_string()
        }
        Lang::En => "Sandboxed preview, full history, and download: Control Web.".to_string(),
    });
    lines.join("\n")
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        format!(
            "{}...",
            captain_types::truncate_str(value, max_chars.saturating_sub(3))
        )
    }
}

fn single_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::artifact::ArtifactPreviewKind;
    use chrono::Utc;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn artifact(id: Uuid, version: u32, title: &str) -> ArtifactVersion {
        ArtifactVersion {
            artifact_id: id,
            version,
            agent_id: "captain".to_string(),
            session_id: Some(Uuid::nil().to_string()),
            title: title.to_string(),
            filename: format!("{}.md", title.to_lowercase()),
            mime_type: "text/markdown".to_string(),
            preview_kind: ArtifactPreviewKind::Markdown,
            size_bytes: 2048,
            sha256: "a".repeat(64),
            created_at: Utc::now(),
            summary: Some("Verified report".to_string()),
        }
    }

    fn inventory(items: Vec<ArtifactVersion>) -> ArtifactInventory {
        ArtifactInventory {
            status: ArtifactStoreStatus {
                healthy: true,
                artifacts: items.len(),
                versions: items.len() + 1,
                bytes: 4096,
                invalid_entries: 0,
                recovered_staging_entries: 0,
                max_artifact_bytes: 50 * 1024 * 1024,
                max_total_bytes: 512 * 1024 * 1024,
            },
            items,
        }
    }

    #[test]
    fn refresh_preserves_the_selected_artifact_by_id() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut state = ArtifactsState::new();
        state.apply_inventory(Ok(inventory(vec![
            artifact(first, 1, "First"),
            artifact(second, 1, "Second"),
        ])));
        state.list_state.select(Some(1));

        let selected = state.apply_inventory(Ok(inventory(vec![
            artifact(second, 2, "Second"),
            artifact(first, 1, "First"),
        ])));

        assert_eq!(selected, Some(second));
        assert_eq!(state.selected_artifact().unwrap().artifact_id, second);
    }

    #[test]
    fn stale_version_result_cannot_replace_the_current_selection() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut state = ArtifactsState::new();
        state.apply_inventory(Ok(inventory(vec![
            artifact(first, 1, "First"),
            artifact(second, 1, "Second"),
        ])));
        assert!(state.begin_versions_load(first));
        assert_eq!(
            state.handle_key(KeyEvent::from(KeyCode::Down)),
            ArtifactsAction::LoadVersions(second)
        );
        assert!(state.begin_versions_load(second));

        state.apply_versions(first, Ok(vec![artifact(first, 2, "First")]));

        assert!(state.versions.is_empty());
        assert_eq!(state.selected_artifact().unwrap().artifact_id, second);
        assert!(state.versions_loading_for(second));
    }

    #[test]
    fn keyboard_navigation_requests_versions_and_moves_through_history() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut state = ArtifactsState::new();
        state.apply_inventory(Ok(inventory(vec![
            artifact(first, 2, "First"),
            artifact(second, 1, "Second"),
        ])));

        assert_eq!(
            state.handle_key(KeyEvent::from(KeyCode::Down)),
            ArtifactsAction::LoadVersions(second)
        );
        assert!(state.begin_versions_load(second));
        state.apply_versions(
            second,
            Ok(vec![
                artifact(second, 3, "Second"),
                artifact(second, 2, "Second"),
            ]),
        );
        state.handle_key(KeyEvent::from(KeyCode::Left));
        assert_eq!(state.selected_version().unwrap().version, 2);
        state.handle_key(KeyEvent::from(KeyCode::Right));
        assert_eq!(state.selected_version().unwrap().version, 3);
    }

    #[test]
    fn desktop_and_compact_layouts_render_without_overflow_or_panic() {
        let id = Uuid::new_v4();
        for (width, height) in [(110, 34), (60, 20)] {
            let mut state = ArtifactsState::new();
            state.apply_inventory(Ok(inventory(vec![artifact(id, 2, "Report")])));
            assert!(state.begin_versions_load(id));
            state.apply_versions(id, Ok(vec![artifact(id, 2, "Report")]));
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| draw(frame, frame.area(), &mut state))
                .unwrap();
            let rendered = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("Report"));
            assert!(rendered.contains("Control Web"));
        }
    }

    #[test]
    fn standalone_summary_is_bounded_and_never_contains_payloads() {
        let items = (0..14)
            .map(|index| artifact(Uuid::new_v4(), 1, &format!("Report {index}")))
            .collect();
        let message = chat_inventory_message(&inventory(items), Lang::En);
        assert!(message.contains("Produced files"));
        assert!(message.contains("2 more"));
        assert!(!message.contains("Verified report"));
    }

    #[test]
    fn standalone_summary_localizes_counts_and_guidance() {
        let message = chat_inventory_message(
            &inventory(vec![artifact(Uuid::new_v4(), 1, "Rapport")]),
            Lang::Fr,
        );
        assert!(message.contains("1 fichier, 2 versions"));
        assert!(message.contains("Aperçu sandboxé"));
        assert!(!message.contains("fichier(s)"));
    }

    #[test]
    fn multiline_metadata_is_normalized_for_terminal_details() {
        assert_eq!(single_line("first\nsecond\tthird"), "first second third");
    }

    #[test]
    fn duplicate_version_reads_for_the_same_artifact_are_coalesced() {
        let id = Uuid::new_v4();
        let mut state = ArtifactsState::new();
        state.apply_inventory(Ok(inventory(vec![artifact(id, 2, "Report")])));

        assert!(state.begin_versions_load(id));
        assert!(!state.begin_versions_load(id));
        assert!(state.versions_loading_for(id));
    }
}

//! Live slash-command picker for the chat screen.

use super::chat::ChatState;
use crate::tui::theme;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use std::ops::Range;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SlashPickerKeyAction {
    Up,
    Down,
    Cancel,
    Select,
    Continue,
}

pub(super) fn slash_picker_key_action_for_key(key: KeyEvent) -> SlashPickerKeyAction {
    match key.code {
        KeyCode::Up => SlashPickerKeyAction::Up,
        KeyCode::Down => SlashPickerKeyAction::Down,
        KeyCode::Esc => SlashPickerKeyAction::Cancel,
        KeyCode::Enter => SlashPickerKeyAction::Select,
        _ => SlashPickerKeyAction::Continue,
    }
}

/// Slash commands promoted in the default picker. Expert routes still complete
/// once the operator types a concrete prefix, but `/` stays focused on the
/// six operational hubs plus common chat actions.
const SUGGESTED_SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/status",
    "/dashboard",
    "/projects",
    "/automation",
    "/learning",
    "/skills",
    "/capabilities",
    "/budget",
    "/model",
    "/new",
    "/resume",
    "/history",
    "/export",
    "/tokens",
    "/cost",
    "/queue",
    "/copy",
    "/clear",
    "/top",
    "/bottom",
    "/retry",
    "/undo",
    "/image",
    "/file",
    "/mouse",
    "/exit",
    "/quit",
];

/// Supported but non-promoted commands. These keep expert access actionnable
/// without turning the picker into another navigation drawer.
const EXPERT_SLASH_COMMANDS: &[&str] = &[
    "/health",
    "/version",
    "/config",
    "/restart",
    "/shutdown",
    "/agents",
    "/home",
    "/channels",
    "/sessions",
    "/tasks",
    "/workflows",
    "/triggers",
    "/memory",
    "/skills-proposed",
    "/proposed",
    "/cron",
    "/scheduler",
    "/approvals",
    "/graph",
    "/logs",
    "/settings",
    "/kill",
    "/fortune",
    "/like",
    "/dislike",
    "/voice",
    "/reload",
    "/think",
];
const SLASH_PICKER_MAX_ROWS: u16 = 8;

pub(super) fn slash_filtered(prefix: &str) -> Vec<&'static str> {
    let mut matches = SUGGESTED_SLASH_COMMANDS
        .iter()
        .copied()
        .filter(|cmd| cmd.starts_with(prefix))
        .collect::<Vec<_>>();
    if prefix.len() > 1 {
        matches.extend(
            EXPERT_SLASH_COMMANDS
                .iter()
                .copied()
                .filter(|cmd| cmd.starts_with(prefix)),
        );
    }
    matches
}

/// Longest common byte prefix of a non-empty list of slash commands. Commands
/// are ASCII, so the byte prefix can be spliced back into the input safely.
pub(super) fn longest_common_prefix(items: &[&str]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let first = items[0];
    let mut end = first.len();
    for s in &items[1..] {
        let common = first
            .bytes()
            .zip(s.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        if common < end {
            end = common;
        }
    }
    first[..end].to_string()
}

/// Popup slash picker just above the input bar.
pub(super) fn draw_slash_picker_live(f: &mut Frame, input_area: Rect, state: &ChatState) {
    let filtered = state.slash_filtered();
    let Some(popup) = slash_picker_popup(input_area, filtered.len()) else {
        return;
    };

    f.render_widget(Clear, popup);

    let visible = slash_picker_visible_range(filtered.len(), state.slash_picker_idx, popup.height);
    let lines = slash_picker_lines(&filtered, state.slash_picker_idx, popup.width, visible);
    f.render_widget(Paragraph::new(lines), popup);
}

fn slash_picker_popup(input_area: Rect, filtered_len: usize) -> Option<Rect> {
    if filtered_len == 0 {
        return None;
    }
    let rows = (filtered_len as u16).min(SLASH_PICKER_MAX_ROWS);
    let popup_width = input_area.width.min(48);
    if input_area.y < rows + 1 || popup_width < 12 {
        return None;
    }
    Some(Rect::new(
        input_area.x,
        input_area.y.saturating_sub(rows),
        popup_width,
        rows,
    ))
}

fn slash_picker_visible_range(total: usize, selected_idx: usize, rows: u16) -> Range<usize> {
    if total == 0 || rows == 0 {
        return 0..0;
    }
    let rows = rows as usize;
    let selected_idx = selected_idx.min(total.saturating_sub(1));
    let view_start = selected_idx.saturating_sub(rows.saturating_sub(1));
    let view_end = (view_start + rows).min(total);
    view_start..view_end
}

fn slash_picker_lines(
    filtered: &[&'static str],
    selected_idx: usize,
    popup_width: u16,
    visible: Range<usize>,
) -> Vec<Line<'static>> {
    filtered[visible.clone()]
        .iter()
        .enumerate()
        .map(|(idx, cmd)| {
            let real_idx = visible.start + idx;
            slash_picker_row(cmd, real_idx == selected_idx, popup_width)
        })
        .collect()
}

fn slash_picker_row(cmd: &str, selected: bool, popup_width: u16) -> Line<'static> {
    let bg = slash_picker_row_bg(selected);
    let mut spans = slash_picker_row_spans(cmd, selected);
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if used < popup_width as usize {
        spans.push(Span::styled(" ".repeat(popup_width as usize - used), bg));
    }
    Line::from(spans).style(bg)
}

fn slash_picker_row_spans(cmd: &str, selected: bool) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            slash_picker_marker(selected),
            slash_picker_marker_style(selected),
        ),
        Span::styled(format!("{cmd:<10}"), slash_picker_command_style(selected)),
        Span::styled(
            format!("  {}", slash_command_hint(cmd)),
            slash_picker_hint_style(selected),
        ),
    ]
}

fn slash_picker_row_bg(selected: bool) -> Style {
    if selected {
        Style::default().bg(theme::BG_HOVER)
    } else {
        Style::default()
    }
}

fn slash_picker_marker(selected: bool) -> &'static str {
    if selected {
        "▶ "
    } else {
        "  "
    }
}

fn slash_picker_marker_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        theme::dim_style()
    }
}

fn slash_picker_command_style(selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::ACCENT)
    }
}

fn slash_picker_hint_style(selected: bool) -> Style {
    if selected {
        Style::default().fg(theme::TEXT_PRIMARY)
    } else {
        theme::dim_style()
    }
}

fn slash_command_hint(cmd: &str) -> &'static str {
    match cmd {
        "/help" => "liste des commandes",
        "/status" => "état du daemon",
        "/health" => "santé daemon",
        "/version" => "version & chemins",
        "/config" => "config.toml exact",
        "/restart" => "restart daemon",
        "/shutdown" => "stop daemon",
        "/agents" => "liste des agents",
        "/home" | "/dashboard" => "ouvre Status",
        "/projects" => "ouvre Projects",
        "/sessions" | "/tasks" => "sessions actives",
        "/automation" | "/workflows" => "ouvre Automation",
        "/triggers" => "ouvre Triggers",
        "/memory" => "ouvre Memory",
        "/learning" => "ouvre Learning",
        "/skills" | "/capabilities" => "ouvre Capabilities",
        "/proposed" => "skills proposées",
        "/cron" | "/scheduler" => "ouvre Cron",
        "/approvals" => "approvals en attente",
        "/budget" => "tokens & coûts",
        "/graph" => "graph mémoire",
        "/logs" => "logs daemon",
        "/settings" => "paramètres",
        "/model" => "switch modèle",
        "/clear" => "vider l'historique",
        "/top" => "début du scrollback",
        "/bottom" => "bas du chat",
        "/kill" => "tuer un agent",
        "/exit" | "/quit" => "quitter le chat",
        "/retry" => "rejouer dernier message",
        "/undo" => "annuler dernier message",
        "/queue" => "messages en attente",
        "/fortune" => "citation",
        "/image" => "joindre une image",
        "/file" => "joindre un fichier",
        "/like" => "👍 dernier tour",
        "/dislike" => "👎 dernier tour",
        "/voice" => "enregistrement vocal",
        "/export" => "exporter session en .md",
        "/history" => "ouvrir picker sessions",
        "/resume" => "restaurer par UUID ou titre",
        "/new" => "nouvelle session",
        "/copy" => "copier réponse/commande",
        "/mouse" => "mode souris on/off",
        "/reload" => "recharger session sauvée",
        "/tokens" => "détail tokens session",
        "/cost" => "coût session USD",
        "/think" => "toggle reasoning blocks",
        _ => "",
    }
}

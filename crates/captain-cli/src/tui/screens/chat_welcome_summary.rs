//! Factual welcome summary rows for the chat empty state.

use super::chat::ChatState;
use crate::tui::theme;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

const ACTIVE_CHANNEL_NAMES: &[&str] = &["telegram", "discord", "signal", "email"];
const WELCOME_SUMMARY_WIDTH: usize = 55;

/// Phase M.6: factual welcome summary rendered under the logo on first
/// open of a fresh chat. Shows agent name, provider/model (from the live
/// ChatState), configured channels (parsed from config.toml), graph size
/// (proxy for memory richness), and active project (if any). All values
/// come from real files/state — no invented data.
///
/// Skipped on narrow viewports where the factual rows would wrap into noise.
pub(super) fn welcome_summary_lines(state: &ChatState, width: usize) -> Vec<Line<'static>> {
    if width < WELCOME_SUMMARY_WIDTH + 4 {
        return Vec::new();
    }

    let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    let captain_dir = home.join(".captain");
    let snapshot = load_welcome_summary_snapshot(&captain_dir);

    let rows = welcome_summary_rows(
        state.agent_name.as_str(),
        state.model_label.as_str(),
        snapshot.channels,
        snapshot.orphan_channels,
        snapshot.graph_size,
        snapshot.active_project,
    );
    render_summary_rows(rows, width)
}

struct WelcomeSummarySnapshot {
    channels: Vec<String>,
    orphan_channels: Vec<String>,
    graph_size: u64,
    active_project: Option<String>,
}

fn load_welcome_summary_snapshot(captain_dir: &Path) -> WelcomeSummarySnapshot {
    let (config_table, real_home) = load_config_and_home(captain_dir);
    let channels = declared_channels(config_table.as_ref());
    WelcomeSummarySnapshot {
        orphan_channels: orphan_channels(&real_home, &channels),
        graph_size: graph_size(&real_home),
        active_project: active_project(&real_home),
        channels,
    }
}

fn load_config_and_home(captain_dir: &Path) -> (Option<toml::Value>, PathBuf) {
    let mut config_table = read_config_table(&captain_dir.join("config.toml"));
    let real_home = configured_home(config_table.as_ref(), captain_dir);
    if real_home != captain_dir {
        config_table = read_config_table(&real_home.join("config.toml")).or(config_table);
    }
    (config_table, real_home)
}

fn read_config_table(path: &Path) -> Option<toml::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse().ok())
}

fn configured_home(config_table: Option<&toml::Value>, captain_dir: &Path) -> PathBuf {
    config_table
        .and_then(|t| t.get("home_dir"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .unwrap_or_else(|| captain_dir.to_path_buf())
}

fn declared_channels(config_table: Option<&toml::Value>) -> Vec<String> {
    config_table
        .and_then(|t| t.get("channels"))
        .and_then(|v| v.as_table())
        .map(|tbl| {
            tbl.keys()
                .filter(|name| ACTIVE_CHANNEL_NAMES.contains(&name.as_str()))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn orphan_channels(real_home: &Path, declared_channels: &[String]) -> Vec<String> {
    let combined_env = combined_env_text(real_home);
    channel_tokens()
        .into_iter()
        .filter(|(name, var)| {
            token_present(&combined_env, var)
                && !declared_channels.iter().any(|c| c.as_str() == *name)
        })
        .map(|(name, _)| name.to_string())
        .collect()
}

fn combined_env_text(real_home: &Path) -> String {
    format!(
        "{}\n{}",
        std::fs::read_to_string(real_home.join(".env")).unwrap_or_default(),
        std::fs::read_to_string(real_home.join("secrets.env")).unwrap_or_default(),
    )
}

fn channel_tokens() -> [(&'static str, &'static str); 3] {
    [
        ("telegram", "TELEGRAM_BOT_TOKEN"),
        ("discord", "DISCORD_BOT_TOKEN"),
        ("email", "EMAIL_PASSWORD"),
    ]
}

fn token_present(env_text: &str, var: &str) -> bool {
    env_text.lines().any(|line| {
        line.trim_start().starts_with(&format!("{var}=")) && !line.trim().ends_with('=')
    })
}

fn graph_size(real_home: &Path) -> u64 {
    std::fs::metadata(real_home.join("graph.hora"))
        .ok()
        .map(|m| m.len())
        .unwrap_or(0)
}

fn active_project(real_home: &Path) -> Option<String> {
    std::fs::read_to_string(real_home.join("active_project.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            let entries = v.get("entries")?.as_object()?;
            entries.values().next()?.as_str().map(str::to_string)
        })
}

fn welcome_summary_rows(
    agent_name: &str,
    model_label: &str,
    channels: Vec<String>,
    orphan_channels: Vec<String>,
    graph_size: u64,
    active_project: Option<String>,
) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    if !agent_name.is_empty() {
        rows.push(("agent".into(), agent_name.to_string()));
    }
    if !model_label.is_empty() {
        rows.push(("provider".into(), model_label.to_string()));
    }
    let channels_str = match (channels.is_empty(), orphan_channels.is_empty()) {
        (true, true) => "aucun (utilise /channel pour configurer)".to_string(),
        (true, false) => format!(
            "token trouvé mais section absente: {} (config.toml à réparer)",
            orphan_channels.join(", ")
        ),
        (false, true) => channels.join(", "),
        (false, false) => format!(
            "{} (orphans: {})",
            channels.join(", "),
            orphan_channels.join(", ")
        ),
    };
    rows.push(("canaux".into(), channels_str));
    if graph_size > 0 {
        rows.push(("mémoire".into(), format_bytes(graph_size)));
    }
    if let Some(p) = active_project {
        rows.push(("projet".into(), p));
    }
    rows
}

fn render_summary_rows(rows: Vec<(String, String)>, width: usize) -> Vec<Line<'static>> {
    let pad = (width.saturating_sub(WELCOME_SUMMARY_WIDTH)) / 2;
    let pad_str = " ".repeat(pad);

    let label_style = Style::default().fg(theme::ACCENT_DIM);
    let value_style = Style::default().fg(theme::ACCENT);

    rows.into_iter()
        .map(|(label, value)| {
            Line::from(vec![
                Span::raw(pad_str.clone()),
                Span::styled(format!("{label:<10}"), label_style),
                Span::styled(value, value_style),
            ])
        })
        .collect()
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

use std::path::PathBuf;

use super::log_events::parse_log_since_ms;
use super::session_api::{
    agent_display_name, fetch_agents, fetch_session_detail, fetch_session_rows, require_agent_id,
    session_agent_id, session_id_of, session_last_active, session_sort_key,
};
use super::session_text::{
    contains_ci, format_session_markdown, match_snippet, session_detail_text,
};
use crate::{
    daemon_client, daemon_json, require_daemon, truncate_display, ui, SessionExportFormat,
    SessionsCommands,
};

pub(crate) fn cmd_sessions(
    command: Option<SessionsCommands>,
    legacy_agent: Option<String>,
    legacy_json: bool,
) {
    match command {
        None => cmd_sessions_list(legacy_agent.as_deref(), legacy_json),
        Some(SessionsCommands::List { agent, json }) => {
            let agent = agent.as_deref().or(legacy_agent.as_deref());
            cmd_sessions_list(agent, json || legacy_json);
        }
        Some(SessionsCommands::Current { agent, json }) => {
            let agent = agent.as_deref().or(legacy_agent.as_deref());
            cmd_sessions_current(agent, json || legacy_json);
        }
        Some(SessionsCommands::Resume {
            session_id,
            agent,
            json,
        }) => {
            let agent = agent.as_deref().or(legacy_agent.as_deref());
            cmd_sessions_resume(&session_id, agent, json || legacy_json);
        }
        Some(SessionsCommands::Continue { agent, json }) => {
            let agent = agent.as_deref().or(legacy_agent.as_deref());
            cmd_sessions_continue(agent, json || legacy_json);
        }
        Some(SessionsCommands::Search {
            query,
            agent,
            limit,
            json,
        }) => {
            let agent = agent.as_deref().or(legacy_agent.as_deref());
            cmd_sessions_search(&query, agent, limit, json || legacy_json);
        }
        Some(SessionsCommands::Export {
            session_id,
            out,
            format,
        }) => cmd_sessions_export(&session_id, out, format),
        Some(SessionsCommands::Prune {
            agent,
            keep,
            older_than,
            dry_run,
            yes,
        }) => {
            let agent = agent.as_deref().or(legacy_agent.as_deref());
            cmd_sessions_prune(agent, keep, older_than.as_deref(), dry_run, yes);
        }
    }
}

fn cmd_sessions_list(agent: Option<&str>, json: bool) {
    let base = require_daemon("sessions");
    let client = daemon_client();
    let sessions = fetch_session_rows(&base, &client, agent);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "sessions": sessions }))
                .unwrap_or_default()
        );
        return;
    }

    if sessions.is_empty() {
        println!("No sessions found.");
        return;
    }

    let agents = fetch_agents(&base, &client);
    println!(
        "{:<38} {:<16} {:>6} {:>9} {:<24} LABEL",
        "ID", "AGENT", "MSGS", "TOKENS", "LAST ACTIVE"
    );
    println!("{}", "-".repeat(112));
    for session in &sessions {
        let agent_id = session_agent_id(session).unwrap_or("?");
        println!(
            "{:<38} {:<16} {:>6} {:>9} {:<24} {}",
            session_id_of(session).unwrap_or("?"),
            truncate_display(&agent_display_name(agent_id, &agents), 16),
            session["message_count"].as_u64().unwrap_or(0),
            session["context_window_tokens"].as_u64().unwrap_or(0),
            session_last_active(session),
            truncate_display(session["label"].as_str().unwrap_or(""), 24),
        );
    }
}

fn cmd_sessions_current(agent: Option<&str>, json: bool) {
    let base = require_daemon("sessions current");
    let client = daemon_client();
    let agent_id = require_agent_id(&base, &client, agent);
    let body = daemon_json(
        client
            .get(format!("{base}/api/agents/{agent_id}/session"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    print_session_summary("Current session", &body);
}

fn cmd_sessions_resume(session_id: &str, agent: Option<&str>, json: bool) {
    let base = require_daemon("sessions resume");
    let client = daemon_client();
    let detail = fetch_session_detail(&base, &client, session_id);
    let agent_id = match agent {
        Some(agent) => require_agent_id(&base, &client, Some(agent)),
        None => detail["agent_id"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| require_agent_id(&base, &client, None)),
    };
    let body = daemon_json(
        client
            .post(format!(
                "{base}/api/agents/{agent_id}/sessions/{session_id}/switch"
            ))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "switch": body,
                "session": detail,
                "agent_id": agent_id,
            }))
            .unwrap_or_default()
        );
        return;
    }
    if let Some(error) = body["error"].as_str() {
        ui::error(&format!("Session resume failed: {error}"));
        std::process::exit(1);
    }
    ui::success(&format!("Resumed session {session_id} on agent {agent_id}"));
    print_session_summary("Session", &detail);
}

fn cmd_sessions_continue(agent: Option<&str>, json: bool) {
    let base = require_daemon("sessions continue");
    let client = daemon_client();
    let agent_id = require_agent_id(&base, &client, agent);
    let sessions = fetch_session_rows(&base, &client, Some(&agent_id));
    let Some(session_id) = sessions.first().and_then(session_id_of).map(str::to_string) else {
        ui::error("No persisted session found for this agent.");
        std::process::exit(1);
    };
    let body = daemon_json(
        client
            .post(format!(
                "{base}/api/agents/{agent_id}/sessions/{session_id}/switch"
            ))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "switch": body,
                "session": sessions.first(),
                "agent_id": agent_id,
            }))
            .unwrap_or_default()
        );
        return;
    }
    if let Some(error) = body["error"].as_str() {
        ui::error(&format!("Session continue failed: {error}"));
        std::process::exit(1);
    }
    ui::success(&format!(
        "Continued newest session {session_id} on agent {agent_id}"
    ));
}

fn cmd_sessions_search(query: &str, agent: Option<&str>, limit: usize, json: bool) {
    let query = query.trim();
    if query.is_empty() {
        ui::error("Search query cannot be empty.");
        std::process::exit(2);
    }
    let limit = limit.max(1);
    let base = require_daemon("sessions search");
    let client = daemon_client();
    let mut rows = fetch_session_rows(&base, &client, agent);
    let mut results = Vec::new();
    let mut scanned_details = 0usize;

    for row in rows.iter_mut() {
        if results.len() >= limit {
            break;
        }
        let metadata_text = serde_json::to_string(row).unwrap_or_default();
        if contains_ci(&metadata_text, query) {
            row["match"] = serde_json::Value::String("metadata".to_string());
            results.push(row.clone());
            continue;
        }
        if scanned_details >= 200 {
            continue;
        }
        let Some(session_id) = session_id_of(row) else {
            continue;
        };
        scanned_details += 1;
        let detail = fetch_session_detail(&base, &client, session_id);
        let haystack = session_detail_text(&detail);
        if contains_ci(&haystack, query) {
            let mut matched = row.clone();
            matched["match"] = serde_json::Value::String("messages".to_string());
            matched["snippet"] = serde_json::Value::String(match_snippet(&haystack, query, 180));
            results.push(matched);
        }
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "query": query,
                "searched_sessions": rows.len(),
                "scanned_message_details": scanned_details,
                "results": results,
            }))
            .unwrap_or_default()
        );
        return;
    }

    if results.is_empty() {
        println!("No sessions matched.");
        return;
    }
    let agents = fetch_agents(&base, &client);
    println!(
        "{:<38} {:<16} {:>6} {:<10} MATCH",
        "ID", "AGENT", "MSGS", "UPDATED"
    );
    println!("{}", "-".repeat(95));
    for session in &results {
        let agent_id = session_agent_id(session).unwrap_or("?");
        println!(
            "{:<38} {:<16} {:>6} {:<10} {}",
            session_id_of(session).unwrap_or("?"),
            truncate_display(&agent_display_name(agent_id, &agents), 16),
            session["message_count"].as_u64().unwrap_or(0),
            truncate_display(session_last_active(session), 10),
            session["match"].as_str().unwrap_or("?"),
        );
        if let Some(snippet) = session["snippet"].as_str().filter(|s| !s.is_empty()) {
            println!("    {}", snippet.replace('\n', " "));
        }
    }
}

fn cmd_sessions_export(session_id: &str, out: Option<PathBuf>, format: SessionExportFormat) {
    let base = require_daemon("sessions export");
    let client = daemon_client();
    let detail = fetch_session_detail(&base, &client, session_id);
    let rendered = match format {
        SessionExportFormat::Json => serde_json::to_string_pretty(&detail).unwrap_or_default(),
        SessionExportFormat::Markdown => format_session_markdown(&detail),
    };
    if let Some(path) = out {
        if let Err(e) = std::fs::write(&path, rendered) {
            ui::error(&format!("Failed to write {}: {e}", path.display()));
            std::process::exit(1);
        }
        ui::success(&format!("Exported session to {}", path.display()));
    } else {
        println!("{rendered}");
    }
}

fn cmd_sessions_prune(
    agent: Option<&str>,
    keep: Option<usize>,
    older_than: Option<&str>,
    dry_run: bool,
    yes: bool,
) {
    if keep.is_none() && older_than.is_none() {
        ui::error("Refusing to prune without --keep or --older-than.");
        ui::hint("Use `captain sessions prune --older-than 30d --dry-run` first.");
        std::process::exit(2);
    }
    let cutoff_ms = match older_than.map(parse_log_since_ms).transpose() {
        Ok(value) => value,
        Err(e) => {
            ui::error(&e);
            ui::hint("Use a duration like 30d, 12h, 10m or a UTC timestamp.");
            std::process::exit(2);
        }
    };
    let base = require_daemon("sessions prune");
    let client = daemon_client();
    let sessions = fetch_session_rows(&base, &client, agent);
    let mut candidates = Vec::new();
    for (idx, session) in sessions.iter().enumerate() {
        let over_keep = keep.map(|n| idx >= n).unwrap_or(true);
        let old_enough = cutoff_ms
            .map(|cutoff| session_sort_key(session) < cutoff)
            .unwrap_or(true);
        if over_keep && old_enough {
            candidates.push(session.clone());
        }
    }

    if candidates.is_empty() {
        println!("No sessions to prune.");
        return;
    }

    if dry_run || !yes {
        ui::section(if dry_run {
            "Sessions that would be pruned"
        } else {
            "Prune requires confirmation"
        });
        for session in &candidates {
            println!(
                "{}  msgs={}  updated={}  label={}",
                session_id_of(session).unwrap_or("?"),
                session["message_count"].as_u64().unwrap_or(0),
                session_last_active(session),
                session["label"].as_str().unwrap_or("")
            );
        }
        if !dry_run {
            ui::hint("Re-run with `--yes` to delete these sessions.");
            std::process::exit(2);
        }
        return;
    }

    let mut deleted = 0usize;
    for session in &candidates {
        let Some(session_id) = session_id_of(session) else {
            continue;
        };
        let body = daemon_json(
            client
                .delete(format!("{base}/api/sessions/{session_id}"))
                .send(),
        );
        if body["status"].as_str() == Some("deleted") {
            deleted += 1;
        } else if let Some(error) = body["error"].as_str() {
            ui::error(&format!("Failed to delete {session_id}: {error}"));
        }
    }
    ui::success(&format!("Deleted {deleted} session(s)."));
}

fn print_session_summary(title: &str, session: &serde_json::Value) {
    ui::section(title);
    ui::kv("Session", session["session_id"].as_str().unwrap_or("?"));
    ui::kv("Agent", session["agent_id"].as_str().unwrap_or("?"));
    if let Some(label) = session["label"].as_str().filter(|s| !s.is_empty()) {
        ui::kv("Label", label);
    }
    ui::kv(
        "Messages",
        &session["message_count"].as_u64().unwrap_or(0).to_string(),
    );
    ui::kv(
        "Context tokens",
        &session["context_window_tokens"]
            .as_u64()
            .unwrap_or(0)
            .to_string(),
    );
}

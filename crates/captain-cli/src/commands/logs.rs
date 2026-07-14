use std::path::Path;

use super::log_events::{
    format_unix_ms_utc, parse_log_line_timestamp_ms, parse_log_since_ms, read_session_events,
    summarize_log_payload, CliLogEvent,
};
use crate::{cli_captain_home, daemon_client, daemon_json, find_daemon, ui, LogTarget};

pub(crate) fn cmd_logs(
    target: LogTarget,
    lines: usize,
    follow: bool,
    since: Option<&str>,
    agent: Option<&str>,
    channel: Option<&str>,
    json: bool,
) {
    let since_ms = match since.map(parse_log_since_ms).transpose() {
        Ok(v) => v,
        Err(e) => {
            ui::error(&e);
            ui::hint(
                "Use a duration like 10m, 2h, 1d or a UTC timestamp like 2026-05-05T20:00:00Z",
            );
            std::process::exit(2);
        }
    };
    let lines = lines.max(1);

    match target {
        LogTarget::Daemon => {
            let path = cli_captain_home().join("captain.log");
            print_log_file(&path, target, lines, follow, since_ms, agent, channel);
        }
        LogTarget::Tui => {
            let path = cli_captain_home().join("tui.log");
            print_log_file(&path, target, lines, follow, since_ms, agent, channel);
        }
        LogTarget::Events | LogTarget::Tools | LogTarget::Agent => {
            print_structured_logs(target, lines, follow, since_ms, agent, channel, json);
        }
        LogTarget::Channel | LogTarget::Errors | LogTarget::All => {
            let path = cli_captain_home().join("captain.log");
            print_log_file(&path, target, lines, false, since_ms, agent, channel);
            print_structured_logs(target, lines, follow, since_ms, agent, channel, json);
        }
    }
}

pub(crate) fn print_log_file(
    path: &Path,
    target: LogTarget,
    lines: usize,
    follow: bool,
    since_ms: Option<i64>,
    agent: Option<&str>,
    channel: Option<&str>,
) {
    if !path.exists() {
        ui::error_with_fix(
            "Log file not found",
            &format!("Expected at: {}", path.display()),
        );
        return;
    }

    let mut last_len = match std::fs::read_to_string(path) {
        Ok(content) => {
            let filtered = filter_log_file_lines(&content, target, since_ms, agent, channel);
            let start = filtered.len().saturating_sub(lines);
            for line in &filtered[start..] {
                println!("{line}");
            }
            content.len()
        }
        Err(e) => {
            ui::error(&format!("Failed to read {}: {e}", path.display()));
            return;
        }
    };

    if follow {
        ui::hint(&format!("Following {} (Ctrl+C to stop)", path.display()));
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            if content.len() < last_len {
                last_len = 0;
            }
            if content.len() <= last_len {
                continue;
            }
            if let Some(chunk) = content.get(last_len..) {
                for line in filter_log_file_lines(chunk, target, since_ms, agent, channel) {
                    println!("{line}");
                }
            }
            last_len = content.len();
        }
    }
}

fn filter_log_file_lines(
    content: &str,
    target: LogTarget,
    since_ms: Option<i64>,
    agent: Option<&str>,
    channel: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut keep_current_entry = false;
    for line in content.lines() {
        if let Some(ts) = parse_log_line_timestamp_ms(line) {
            let since_ok = since_ms.map(|cutoff| ts >= cutoff).unwrap_or(true);
            keep_current_entry = since_ok && log_file_line_matches(line, target, agent, channel);
        }
        if keep_current_entry {
            out.push(line.to_string());
        }
    }
    out
}

fn log_file_line_matches(
    line: &str,
    target: LogTarget,
    agent: Option<&str>,
    channel: Option<&str>,
) -> bool {
    let lower = line.to_ascii_lowercase();
    match target {
        LogTarget::Errors => {
            lower.contains(" error ")
                || lower.contains(" warn ")
                || lower.contains("panic")
                || lower.contains("failed")
                || lower.contains("exception")
        }
        LogTarget::Channel => channel
            .map(|ch| lower.contains(&ch.to_ascii_lowercase()))
            .unwrap_or_else(|| lower.contains("captain_channels::")),
        LogTarget::Agent => agent
            .map(|name| lower.contains(&name.to_ascii_lowercase()))
            .unwrap_or(true),
        LogTarget::Tools => lower.contains("tool"),
        LogTarget::All | LogTarget::Daemon | LogTarget::Tui | LogTarget::Events => true,
    }
}

fn print_structured_logs(
    target: LogTarget,
    lines: usize,
    follow: bool,
    since_ms: Option<i64>,
    agent: Option<&str>,
    channel: Option<&str>,
    json: bool,
) {
    let agent_terms = resolve_log_agent_terms(agent);
    let channel_term = channel.map(|s| s.to_ascii_lowercase());
    let mut last_seen_id = 0_i64;

    match read_session_events(None, since_ms, lines.saturating_mul(20).clamp(100, 5_000)) {
        Ok(events) => {
            let filtered =
                filter_structured_events(events, target, &agent_terms, channel_term.as_deref());
            let start = filtered.len().saturating_sub(lines);
            for event in &filtered[start..] {
                print_structured_event(event, json);
                last_seen_id = last_seen_id.max(event.id);
            }
        }
        Err(e) => {
            ui::error(&format!("Failed to read structured logs: {e}"));
            return;
        }
    }

    if follow {
        ui::hint("Following structured events (Ctrl+C to stop)");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let Ok(events) = read_session_events(Some(last_seen_id), since_ms, 500) else {
                continue;
            };
            let filtered =
                filter_structured_events(events, target, &agent_terms, channel_term.as_deref());
            for event in &filtered {
                print_structured_event(event, json);
                last_seen_id = last_seen_id.max(event.id);
            }
        }
    }
}

fn filter_structured_events(
    events: Vec<CliLogEvent>,
    target: LogTarget,
    agent_terms: &[String],
    channel: Option<&str>,
) -> Vec<CliLogEvent> {
    events
        .into_iter()
        .filter(|ev| structured_event_matches(ev, target, agent_terms, channel))
        .collect()
}

fn structured_event_matches(
    event: &CliLogEvent,
    target: LogTarget,
    agent_terms: &[String],
    channel: Option<&str>,
) -> bool {
    let event_type = event.event_type.to_ascii_lowercase();
    let session_id = event.session_id.to_ascii_lowercase();
    let payload = event.payload.to_string().to_ascii_lowercase();
    let agent_ok = agent_terms.is_empty()
        || agent_terms
            .iter()
            .any(|term| session_id.contains(term) || payload.contains(term));
    let channel_ok = channel
        .map(|ch| payload.contains(ch) || event_type.contains(ch))
        .unwrap_or(true);

    if !agent_ok || !channel_ok {
        return false;
    }

    match target {
        LogTarget::Tools => {
            event_type.starts_with("tool_") || event_type == "tool_execution_result"
        }
        LogTarget::Agent => true,
        LogTarget::Channel => {
            channel.is_some()
                || payload.contains("telegram")
                || payload.contains("discord")
                || payload.contains("channel")
                || event_type.contains("channel")
        }
        LogTarget::Errors => {
            event
                .payload
                .get("is_error")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
                || event_type.contains("error")
                || payload.contains("\"is_error\":true")
                || payload.contains("tool_error")
                || payload.contains("exception")
                || payload.contains("failed")
        }
        LogTarget::Events | LogTarget::All => true,
        LogTarget::Daemon | LogTarget::Tui => false,
    }
}

fn resolve_log_agent_terms(agent: Option<&str>) -> Vec<String> {
    let Some(agent) = agent else {
        return Vec::new();
    };
    let mut terms = vec![agent.to_ascii_lowercase()];
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/status")).send());
        if let Some(agents) = body["agents"].as_array() {
            for entry in agents {
                let name = entry["name"].as_str().unwrap_or_default();
                let id = entry["id"].as_str().unwrap_or_default();
                if name.eq_ignore_ascii_case(agent)
                    || id.eq_ignore_ascii_case(agent)
                    || id.starts_with(agent)
                {
                    terms.push(name.to_ascii_lowercase());
                    terms.push(id.to_ascii_lowercase());
                }
            }
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn print_structured_event(event: &CliLogEvent, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "id": event.id,
                "session_id": event.session_id,
                "ts": event.ts,
                "timestamp": format_unix_ms_utc(event.ts),
                "event_type": event.event_type,
                "payload": event.payload,
            }))
            .unwrap_or_default()
        );
        return;
    }
    println!(
        "{} {:<22} {:<8} {}",
        format_unix_ms_utc(event.ts),
        event.event_type,
        short_log_session(&event.session_id),
        summarize_log_payload(event)
    );
}

fn short_log_session(session_id: &str) -> String {
    session_id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_filter_keeps_multiline_error_entry() {
        let content = "2026-05-05T20:40:45.000000Z  INFO ok\n\
continuation hidden\n\
2026-05-05T20:40:46.000000Z  WARN captain_channels::telegram: failed\n\
continued detail\n";
        let lines = filter_log_file_lines(content, LogTarget::Errors, None, None, None);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("WARN"));
        assert_eq!(lines[1], "continued detail");
    }

    #[test]
    fn structured_tool_error_filter() {
        let event = CliLogEvent {
            id: 1,
            session_id: "agent-1".to_string(),
            ts: 0,
            event_type: "tool_execution_result".to_string(),
            payload: serde_json::json!({"name":"web_fetch","is_error":true}),
        };

        assert!(structured_event_matches(
            &event,
            LogTarget::Tools,
            &[],
            None
        ));
        assert!(structured_event_matches(
            &event,
            LogTarget::Errors,
            &[],
            None
        ));
    }
}

use super::log_events::parse_utc_timestamp_ms;
use crate::{daemon_json, ui};

pub(super) fn fetch_session_rows(
    base: &str,
    client: &reqwest::blocking::Client,
    agent: Option<&str>,
) -> Vec<serde_json::Value> {
    let body = daemon_json(client.get(format!("{base}/api/sessions")).send());
    let mut rows = body
        .as_array()
        .or_else(|| body.get("sessions").and_then(|value| value.as_array()))
        .cloned()
        .unwrap_or_default();
    if let Some(agent) = agent.map(str::trim).filter(|s| !s.is_empty()) {
        let resolved_agent =
            resolve_agent_id(base, client, agent).unwrap_or_else(|| agent.to_string());
        rows.retain(|session| {
            session_agent_id(session)
                .map(|id| id == resolved_agent || id == agent)
                .unwrap_or(false)
        });
    }
    rows.sort_by_key(|session| std::cmp::Reverse(session_sort_key(session)));
    rows
}

pub(super) fn fetch_session_detail(
    base: &str,
    client: &reqwest::blocking::Client,
    session_id: &str,
) -> serde_json::Value {
    let body = daemon_json(
        client
            .get(format!("{base}/api/sessions/{session_id}"))
            .send(),
    );
    if let Some(error) = body["error"].as_str() {
        ui::error(&format!("Session {session_id}: {error}"));
        std::process::exit(1);
    }
    body
}

pub(super) fn fetch_agents(
    base: &str,
    client: &reqwest::blocking::Client,
) -> Vec<serde_json::Value> {
    daemon_json(client.get(format!("{base}/api/agents")).send())
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn resolve_agent_id(base: &str, client: &reqwest::blocking::Client, agent: &str) -> Option<String> {
    let agents = fetch_agents(base, client);
    agents.iter().find_map(|item| {
        let id = item["id"].as_str()?;
        let name = item["name"].as_str().unwrap_or("");
        if id == agent || name.eq_ignore_ascii_case(agent) {
            Some(id.to_string())
        } else {
            None
        }
    })
}

pub(crate) fn require_agent_id(
    base: &str,
    client: &reqwest::blocking::Client,
    agent: Option<&str>,
) -> String {
    let agents = fetch_agents(base, client);
    if let Some(agent) = agent.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(id) = agents.iter().find_map(|item| {
            let id = item["id"].as_str()?;
            let name = item["name"].as_str().unwrap_or("");
            if id == agent || name.eq_ignore_ascii_case(agent) {
                Some(id.to_string())
            } else {
                None
            }
        }) {
            return id;
        }
        ui::error(&format!("Agent not found: {agent}"));
        let names = agents
            .iter()
            .filter_map(|item| item["name"].as_str())
            .collect::<Vec<_>>();
        if !names.is_empty() {
            ui::hint(&format!("Known agents: {}", names.join(", ")));
        }
        std::process::exit(1);
    }

    if let Some(id) = agents.iter().find_map(|item| {
        (item["name"].as_str() == Some("captain"))
            .then(|| item["id"].as_str())
            .flatten()
            .map(str::to_string)
    }) {
        return id;
    }
    if let Some(id) = agents.first().and_then(|item| item["id"].as_str()) {
        return id.to_string();
    }
    ui::error("No running agent found.");
    std::process::exit(1);
}

pub(super) fn session_id_of(session: &serde_json::Value) -> Option<&str> {
    session["session_id"]
        .as_str()
        .or_else(|| session["id"].as_str())
}

pub(super) fn session_agent_id(session: &serde_json::Value) -> Option<&str> {
    session["agent_id"].as_str()
}

pub(super) fn session_last_active(session: &serde_json::Value) -> &str {
    session["last_active"]
        .as_str()
        .or_else(|| session["updated_at"].as_str())
        .or_else(|| session["created_at"].as_str())
        .or_else(|| session["created"].as_str())
        .unwrap_or("?")
}

pub(super) fn session_sort_key(session: &serde_json::Value) -> i64 {
    parse_utc_timestamp_ms(session_last_active(session)).unwrap_or(i64::MIN)
}

pub(super) fn agent_display_name(agent_id: &str, agents: &[serde_json::Value]) -> String {
    agents
        .iter()
        .find(|agent| agent["id"].as_str() == Some(agent_id))
        .and_then(|agent| agent["name"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| agent_id.to_string())
}

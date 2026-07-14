use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;

use crate::{daemon_client, find_daemon, tui, ui};

pub(crate) fn cmd_quick_chat(config: Option<PathBuf>, agent: Option<String>, plain: bool) {
    if should_use_plain_chat(plain) {
        if let Some(base) = find_daemon() {
            cmd_plain_chat(&base, agent.as_deref());
            return;
        }
    }
    tui::chat_runner::run_chat_tui(config, agent);
}

fn should_use_plain_chat(plain_arg: bool) -> bool {
    if let Ok(value) = std::env::var("CAPTAIN_CHAT_TUI") {
        return !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
    }
    if plain_arg {
        return true;
    }
    if let Ok(value) = std::env::var("CAPTAIN_CHAT_PLAIN") {
        return matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        );
    }
    false
}

fn cmd_plain_chat(base: &str, agent: Option<&str>) {
    let client = daemon_client();
    let mut agent_id = super::require_agent_id(base, &client, agent);
    let mut session_id: Option<String> = None;
    ui::section("Captain Chat");
    ui::hint("Mode scrollback: les messages restent dans l'historique natif du terminal.");
    ui::hint("Commandes: /history, /resume <id|titre>, /new, /exit. TUI classique: captain chat.");

    let stdin = io::stdin();
    loop {
        print!("> ");
        let _ = io::stdout().flush();

        let mut input = String::new();
        match stdin.read_line(&mut input) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                ui::error(&format!("Lecture stdin impossible: {e}"));
                break;
            }
        }
        let message = input.trim_end_matches(['\r', '\n']).trim();
        if message.is_empty() {
            continue;
        }
        if matches!(message, "/exit" | "/quit") {
            break;
        }
        if matches!(message, "/history" | "/sessions") {
            print_plain_session_history(base, &client);
            continue;
        }
        if message == "/new" {
            session_id = None;
            println!("Nouvelle session prête. La précédente reste dans /history.");
            continue;
        }
        if message == "/resume" {
            println!("Usage: /resume <session-id ou titre>");
            continue;
        }
        if let Some(selector) = message.strip_prefix("/resume ") {
            match resolve_plain_session(base, &client, selector) {
                Ok(resumed) => {
                    agent_id = resumed.agent_id;
                    session_id = Some(resumed.session_id.clone());
                    println!(
                        "Session restaurée: {} ({})",
                        resumed.label,
                        short_session_id(&resumed.session_id)
                    );
                }
                Err(error) => println!("Impossible de restaurer la session: {error}"),
            }
            continue;
        }

        if session_id.is_none() {
            match create_plain_session(base, &client, &agent_id) {
                Ok(created) => session_id = Some(created),
                Err(error) => {
                    println!("Impossible de créer la session: {error}");
                    continue;
                }
            }
        }

        print!("Captain: ");
        let _ = io::stdout().flush();

        let resp = client
            .post(format!("{base}/api/agents/{agent_id}/message/stream"))
            .json(&serde_json::json!({
                "message": message,
                "channel_type": "cli",
                "session_id": session_id.as_deref(),
            }))
            .send();

        let mut printed_any = false;
        match resp {
            Ok(resp) if resp.status().is_success() => {
                struct RespReader(reqwest::blocking::Response);
                impl Read for RespReader {
                    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                        self.0.read(buf)
                    }
                }
                let reader = BufReader::new(RespReader(resp));
                for line in reader.lines().map_while(Result::ok) {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                            if let Some(content) = json.get("content").and_then(|v| v.as_str()) {
                                printed_any = true;
                                print!("{content}");
                                let _ = io::stdout().flush();
                            } else if let Some(kind) = json.get("type").and_then(|v| v.as_str()) {
                                match kind {
                                    "tool_start" => {
                                        let tool = json
                                            .get("tool")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool");
                                        if printed_any {
                                            println!();
                                        }
                                        println!("  [tool] {tool}...");
                                        print!("Captain: ");
                                        let _ = io::stdout().flush();
                                        printed_any = false;
                                    }
                                    "tool_result" => {
                                        let tool = json
                                            .get("tool")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("tool");
                                        let preview = json
                                            .get("result")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("");
                                        println!(
                                            "  [tool result] {tool}: {}",
                                            preview.chars().take(180).collect::<String>()
                                        );
                                        print!("Captain: ");
                                        let _ = io::stdout().flush();
                                        printed_any = false;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                println!();
            }
            Ok(resp) => {
                println!("Erreur daemon: HTTP {}", resp.status());
            }
            Err(e) => {
                println!("Erreur réseau: {e}");
            }
        }
    }
}

struct PlainSessionTarget {
    session_id: String,
    agent_id: String,
    label: String,
}

fn create_plain_session(
    base: &str,
    client: &reqwest::blocking::Client,
    agent_id: &str,
) -> Result<String, String> {
    let response = client
        .post(format!("{base}/api/agents/{agent_id}/sessions"))
        .json(&serde_json::json!({"activate": false}))
        .send()
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let body = response
        .json::<serde_json::Value>()
        .map_err(|error| format!("réponse invalide: {error}"))?;
    if !status.is_success() {
        return Err(body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("création refusée par le daemon")
            .to_string());
    }
    body.get("session_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| uuid::Uuid::parse_str(value).is_ok())
        .map(str::to_string)
        .ok_or_else(|| "le daemon n'a pas renvoyé d'UUID de session".to_string())
}

fn resolve_plain_session(
    base: &str,
    client: &reqwest::blocking::Client,
    selector: &str,
) -> Result<PlainSessionTarget, String> {
    let sessions = super::session_api::fetch_session_rows(base, client, None);
    let session_id = crate::tui::session_runtime::resolve_session_selector(&sessions, selector)?;
    let session = sessions
        .iter()
        .find(|session| super::session_api::session_id_of(session) == Some(session_id.as_str()))
        .ok_or_else(|| format!("Session introuvable : {selector}"))?;
    let agent_id = super::session_api::session_agent_id(session)
        .filter(|value| uuid::Uuid::parse_str(value).is_ok())
        .ok_or_else(|| "la session n'a pas de propriétaire valide".to_string())?
        .to_string();
    let label = session
        .get("label")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Session persistée")
        .to_string();
    Ok(PlainSessionTarget {
        session_id,
        agent_id,
        label,
    })
}

fn print_plain_session_history(base: &str, client: &reqwest::blocking::Client) {
    let sessions = super::session_api::fetch_session_rows(base, client, None);
    if sessions.is_empty() {
        println!("Aucune session persistée.");
        return;
    }

    println!("Sessions persistées (les plus récentes d'abord):");
    for session in sessions.iter().take(30) {
        let id = super::session_api::session_id_of(session).unwrap_or("?");
        let label = session
            .get("label")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("Session persistée");
        let agent = super::session_api::session_agent_id(session).unwrap_or("?");
        let messages = session
            .get("message_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        println!(
            "  {:8}  {:<36}  agent {:8}  {messages} msg",
            short_session_id(id),
            label.chars().take(36).collect::<String>(),
            short_session_id(agent),
        );
    }
    if sessions.len() > 30 {
        println!("  … {} autres sessions", sessions.len() - 30);
    }
}

fn short_session_id(id: &str) -> &str {
    id.get(..8).unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_session_id_keeps_short_values_and_truncates_uuids() {
        assert_eq!(short_session_id("abc"), "abc");
        assert_eq!(
            short_session_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
            "aaaaaaaa"
        );
    }
}

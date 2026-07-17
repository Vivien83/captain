//! Phase L.3 — persistence des sessions de chat.
//!
//! Chaque conversation est sauvegardée dans
//! `~/.captain/sessions/{agent_key}/{ts}.json`. La dernière session d'un
//! agent peut être rechargée au démarrage du chat pour préserver
//! l'historique entre relances du TUI.
//!
//! Format JSON simple (pas de schéma versionné — si on évolue, on ignore
//! les champs inconnus). Limite douce : on tronque l'écriture à 500 messages
//! pour éviter d'écrire un fichier de plusieurs MB après une longue session.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_PERSISTED_MESSAGES: usize = 500;

#[derive(Serialize, Deserialize, Clone)]
pub struct PersistedTool {
    pub name: String,
    pub input: String,
    pub result: String,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PersistedMessage {
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<PersistedTool>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct PersistedSession {
    /// Authoritative SQLite session shared by Web, CLI, TUI and Desktop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Owning agent UUID. Legacy snapshots derive it from their directory key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub agent_name: String,
    pub model_label: String,
    pub mode_label: String,
    pub messages: Vec<PersistedMessage>,
    /// Approximate active prompt size reported by the latest provider call.
    #[serde(default)]
    pub current_context_tokens: u64,
    /// Effective model window from the live catalog when this snapshot was written.
    #[serde(default)]
    pub context_window_tokens: u64,
    /// Cumul tokens input sur la session (toutes itérations confondues).
    #[serde(default)]
    pub session_input_tokens: u64,
    #[serde(default)]
    pub session_output_tokens: u64,
    #[serde(default)]
    pub session_cached_input_tokens: u64,
    #[serde(default)]
    pub session_cache_creation_tokens: u64,
    #[serde(default)]
    pub session_cost_usd: f64,
    /// Unix epoch seconds — utile pour ordonner les sessions au pick.
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

/// `~/.captain/sessions` (créé si manquant).
pub fn sessions_root() -> Option<PathBuf> {
    Some(captain_kernel::config::captain_home().join("sessions"))
}

fn agent_dir(agent_key: &str) -> Option<PathBuf> {
    Some(sessions_root()?.join(sanitize(agent_key)))
}

/// Sanitize un identifiant d'agent pour usage filesystem.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Convert a persisted TUI session into the kernel's native `Message`
/// vec — used by the "resume last session" boot flow to plumb the
/// snapshot back into the agent's authoritative context. Tool turns are
/// dropped (they are TUI-rendering artifacts; replaying them on the
/// kernel side would require regenerating tool_use_id pairs the next
/// LLM call wouldn't recognize). User and assistant text turns travel
/// through verbatim.
pub fn to_kernel_messages(persisted: &PersistedSession) -> Vec<captain_types::message::Message> {
    use captain_types::message::Message;
    persisted
        .messages
        .iter()
        .filter(|m| m.tool.is_none())
        .filter_map(|m| match m.role.as_str() {
            "user" => Some(Message::user(m.text.clone())),
            "agent" | "assistant" => Some(Message::assistant(m.text.clone())),
            "system" => Some(Message::system(m.text.clone())),
            _ => None,
        })
        .collect()
}

/// Sauvegarde une session sur disque via remplacement atomique durable.
/// Retourne le chemin écrit en cas de succès.
pub fn save_session(
    agent_key: &str,
    session_path: Option<&Path>,
    session: &PersistedSession,
) -> Option<PathBuf> {
    let dir = agent_dir(agent_key)?;
    if captain_types::durable_fs::create_dir_all(&dir).is_err() {
        return None;
    }

    let path = match session_path {
        Some(p) => p.to_path_buf(),
        None => dir.join(format!("{}.json", session.created_at.max(now_secs()))),
    };

    let mut to_write = session.clone();
    to_write.updated_at = now_secs();
    if to_write.messages.len() > MAX_PERSISTED_MESSAGES {
        let cut = to_write.messages.len() - MAX_PERSISTED_MESSAGES;
        to_write.messages.drain(0..cut);
    }

    let json = serde_json::to_string_pretty(&to_write).ok()?;
    captain_types::durable_fs::atomic_write(&path, json.as_bytes()).ok()?;
    Some(path)
}

/// Métadonnées légères d'une session pour le picker — on évite de charger
/// le payload complet (potentiellement 500 messages) juste pour afficher
/// une liste.
#[derive(Clone)]
pub struct SessionSummary {
    pub agent_key: String,
    pub session_id: Option<String>,
    pub label: String,
    pub agent_name: String,
    /// Affiché à terme dans le picker (provider/modèle), gardé pour stabilité du payload.
    #[allow(dead_code)]
    pub model_label: String,
    pub path: PathBuf,
    pub updated_at: u64,
    pub message_count: usize,
    pub session_input_tokens: u64,
    pub session_output_tokens: u64,
}

/// Liste TOUTES les sessions sauvegardées, triées par mtime décroissant.
/// Lecture best-effort : un fichier corrompu est silencieusement skippé.
pub fn list_sessions() -> Vec<SessionSummary> {
    let Some(root) = sessions_root() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(agent_dirs) = std::fs::read_dir(&root) else {
        return out;
    };
    for ad in agent_dirs.flatten() {
        if !ad.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let agent_key = ad.file_name().to_string_lossy().into_owned();
        let Ok(files) = std::fs::read_dir(ad.path()) else {
            continue;
        };
        for f in files.flatten() {
            let p = f.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(mut session) = serde_json::from_str::<PersistedSession>(&raw) else {
                continue;
            };
            hydrate_authoritative_ids(&mut session, &p, &agent_key);
            let label = local_session_label(&session);
            out.push(SessionSummary {
                agent_key: agent_key.clone(),
                session_id: session.session_id,
                label,
                agent_name: session.agent_name,
                model_label: session.model_label,
                path: p,
                updated_at: session.updated_at.max(session.created_at),
                message_count: session.messages.len(),
                session_input_tokens: session.session_input_tokens,
                session_output_tokens: session.session_output_tokens,
            });
        }
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    out
}

fn local_session_label(session: &PersistedSession) -> String {
    session
        .messages
        .iter()
        .find(|message| message.role == "user" && !message.text.trim().is_empty())
        .map(|message| {
            let text = message
                .text
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if text.chars().count() <= 48 {
                text
            } else {
                text.chars().take(47).collect::<String>() + "…"
            }
        })
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| session.agent_name.clone())
}

/// Charge une session précise depuis son chemin sur disque.
pub fn load_session_at(path: &Path) -> Option<PersistedSession> {
    let raw = std::fs::read_to_string(path).ok()?;
    let mut session: PersistedSession = serde_json::from_str(&raw).ok()?;
    let agent_key = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    hydrate_authoritative_ids(&mut session, path, agent_key);
    Some(session)
}

/// Charge la session la plus récente d'un agent (s'il y en a une).
pub fn load_latest_session(agent_key: &str) -> Option<(PathBuf, PersistedSession)> {
    let dir = agent_dir(agent_key)?;
    let mut entries: Vec<(PathBuf, u64)> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                return None;
            }
            let mtime = e
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(SystemTime::UNIX_EPOCH)
                .ok()?
                .as_secs();
            Some((p, mtime))
        })
        .collect();
    entries.sort_by_key(|(_, m)| std::cmp::Reverse(*m));
    let (path, _) = entries.first()?.clone();
    let session = load_session_at(&path)?;
    Some((path, session))
}

fn hydrate_authoritative_ids(session: &mut PersistedSession, path: &Path, agent_key: &str) {
    if session.agent_id.is_none() {
        session.agent_id = embedded_uuid(agent_key).map(|id| id.to_string());
    }
    if session.session_id.is_none() {
        session.session_id = legacy_source_key(path).map(|source_key| {
            captain_memory::session::legacy_tui_session_id(&source_key)
                .0
                .to_string()
        });
    }
}

fn legacy_source_key(path: &Path) -> Option<String> {
    let root = sessions_root()?;
    Some(
        path.strip_prefix(root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/"),
    )
}

fn embedded_uuid(value: &str) -> Option<uuid::Uuid> {
    let bytes = value.as_bytes();
    if bytes.len() < 36 {
        return uuid::Uuid::parse_str(value).ok();
    }
    (0..=bytes.len() - 36).find_map(|start| {
        std::str::from_utf8(&bytes[start..start + 36])
            .ok()
            .and_then(|candidate| uuid::Uuid::parse_str(candidate).ok())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn unique_key(prefix: &str) -> String {
        format!(
            "test_{}_{}",
            prefix,
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }

    #[test]
    fn sanitize_strips_unsafe_chars() {
        assert_eq!(sanitize("captain"), "captain");
        assert_eq!(sanitize("foo/bar"), "foo_bar");
        assert_eq!(sanitize("a..b"), "a__b");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let key = unique_key("roundtrip");
        let session = PersistedSession {
            session_id: None,
            agent_id: None,
            agent_name: "captain".into(),
            model_label: "anthropic/claude-sonnet-4".into(),
            mode_label: "in-process".into(),
            messages: vec![PersistedMessage {
                role: "user".into(),
                text: "salut".into(),
                tool: None,
            }],
            current_context_tokens: 10,
            context_window_tokens: 200_000,
            session_input_tokens: 12,
            session_output_tokens: 34,
            session_cached_input_tokens: 8,
            session_cache_creation_tokens: 3,
            session_cost_usd: 0.0001,
            created_at: now_secs(),
            updated_at: 0,
        };
        let path = save_session(&key, None, &session).expect("save ok");
        assert!(path.exists());
        let (loaded_path, loaded) = load_latest_session(&key).expect("load ok");
        assert_eq!(loaded_path, path);
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].text, "salut");
        assert_eq!(loaded.current_context_tokens, 10);
        assert_eq!(loaded.context_window_tokens, 200_000);
        assert_eq!(loaded.session_input_tokens, 12);
        assert_eq!(loaded.session_cached_input_tokens, 8);
        let _ = std::fs::remove_dir_all(agent_dir(&key).unwrap());
    }

    #[test]
    fn save_truncates_long_history() {
        let key = unique_key("truncate");
        let mut session = PersistedSession {
            agent_name: "captain".into(),
            ..Default::default()
        };
        session.created_at = now_secs();
        for i in 0..(MAX_PERSISTED_MESSAGES + 50) {
            session.messages.push(PersistedMessage {
                role: "user".into(),
                text: format!("msg {i}"),
                tool: None,
            });
        }
        save_session(&key, None, &session).expect("save ok");
        let (_, loaded) = load_latest_session(&key).expect("load ok");
        assert_eq!(loaded.messages.len(), MAX_PERSISTED_MESSAGES);
        assert_eq!(loaded.messages[0].text, "msg 50");
        let _ = std::fs::remove_dir_all(agent_dir(&key).unwrap());
    }

    #[test]
    fn to_kernel_messages_preserves_user_and_assistant_turns() {
        use captain_types::message::Role;

        let session = PersistedSession {
            agent_name: "captain".into(),
            messages: vec![
                PersistedMessage {
                    role: "user".into(),
                    text: "hello".into(),
                    tool: None,
                },
                PersistedMessage {
                    role: "agent".into(),
                    text: "hi back".into(),
                    tool: None,
                },
                PersistedMessage {
                    role: "user".into(),
                    text: "follow up".into(),
                    tool: None,
                },
            ],
            ..Default::default()
        };
        let kernel_msgs = to_kernel_messages(&session);
        assert_eq!(kernel_msgs.len(), 3);
        assert_eq!(kernel_msgs[0].role, Role::User);
        assert_eq!(kernel_msgs[1].role, Role::Assistant);
        assert_eq!(kernel_msgs[2].role, Role::User);
    }

    #[test]
    fn to_kernel_messages_skips_tool_turns_and_unknown_roles() {
        let session = PersistedSession {
            agent_name: "captain".into(),
            messages: vec![
                PersistedMessage {
                    role: "user".into(),
                    text: "hi".into(),
                    tool: None,
                },
                PersistedMessage {
                    role: "agent".into(),
                    text: "calling shell".into(),
                    tool: Some(PersistedTool {
                        name: "shell_exec".into(),
                        input: "{}".into(),
                        result: "ok".into(),
                        is_error: false,
                    }),
                },
                PersistedMessage {
                    role: "tool".into(),
                    text: "ok".into(),
                    tool: None,
                },
                PersistedMessage {
                    role: "agent".into(),
                    text: "done".into(),
                    tool: None,
                },
            ],
            ..Default::default()
        };
        let kernel_msgs = to_kernel_messages(&session);
        assert_eq!(kernel_msgs.len(), 2, "only the 2 plain-text turns survive");
    }

    #[test]
    fn to_kernel_messages_handles_empty_session() {
        let session = PersistedSession::default();
        assert!(to_kernel_messages(&session).is_empty());
    }
}

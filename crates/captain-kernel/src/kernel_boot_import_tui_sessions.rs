use super::CaptainKernel;
use captain_memory::session::{legacy_tui_session_id, Session};
use captain_types::agent::{AgentId, SessionId};
use captain_types::message::Message;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const MAX_LEGACY_SESSION_FILES: usize = 10_000;
const MAX_LEGACY_SESSION_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Default)]
struct ImportReport {
    discovered: usize,
    imported: usize,
    already_present: usize,
    skipped: usize,
    failed: usize,
}

pub(super) fn import_legacy_tui_sessions(kernel: &CaptainKernel) {
    let root = kernel.config.home_dir.join("sessions");
    let report = import_from_root(kernel, &root);
    if report.discovered == 0 {
        return;
    }
    info!(
        discovered = report.discovered,
        imported = report.imported,
        already_present = report.already_present,
        skipped = report.skipped,
        failed = report.failed,
        "Legacy TUI session migration completed"
    );
}

fn import_from_root(kernel: &CaptainKernel, root: &Path) -> ImportReport {
    let mut report = ImportReport::default();
    for path in legacy_session_files(root) {
        report.discovered += 1;
        match import_file(kernel, root, &path) {
            Ok(Some(imported)) => match mark_legacy_file_processed(&path) {
                Ok(()) if imported => report.imported += 1,
                Ok(()) => report.already_present += 1,
                Err(error) => {
                    report.failed += 1;
                    warn!(path = %path.display(), error = %error, "Legacy TUI session import marker failed");
                }
            },
            Ok(None) => report.skipped += 1,
            Err(error) => {
                report.failed += 1;
                warn!(path = %path.display(), error = %error, "Legacy TUI session import failed");
            }
        }
    }
    report
}

fn legacy_session_files(root: &Path) -> Vec<PathBuf> {
    let Ok(agent_dirs) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for agent_dir in agent_dirs.flatten() {
        if files.len() >= MAX_LEGACY_SESSION_FILES
            || !agent_dir
                .file_type()
                .map(|kind| kind.is_dir())
                .unwrap_or(false)
        {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(agent_dir.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            if files.len() >= MAX_LEGACY_SESSION_FILES {
                break;
            }
            let path = entry.path();
            let is_json_file = entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
                && path.extension().and_then(|value| value.to_str()) == Some("json");
            let size_allowed = entry
                .metadata()
                .map(|metadata| metadata.len() <= MAX_LEGACY_SESSION_BYTES)
                .unwrap_or(false);
            if is_json_file && size_allowed && !processed_marker_path(&path).is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn processed_marker_path(path: &Path) -> PathBuf {
    path.with_extension("json.imported")
}

fn mark_legacy_file_processed(path: &Path) -> Result<(), std::io::Error> {
    captain_types::durable_fs::atomic_write(
        &processed_marker_path(path),
        b"Imported into Captain's canonical session store.\n",
    )
}

fn import_file(kernel: &CaptainKernel, root: &Path, path: &Path) -> Result<Option<bool>, String> {
    let source_key = path
        .strip_prefix(root)
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .replace('\\', "/");
    let raw = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    let agent_key = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let agent_name = value
        .get("agent_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(agent_id) = resolve_agent_id(kernel, agent_key, agent_name) else {
        return Ok(None);
    };
    let Some(imported) = legacy_session_from_value(&source_key, agent_id, &value) else {
        return Ok(None);
    };
    kernel
        .memory
        .import_session_if_absent(&imported.session, imported.created_at, imported.updated_at)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn resolve_agent_id(kernel: &CaptainKernel, agent_key: &str, agent_name: &str) -> Option<AgentId> {
    embedded_uuid(agent_key)
        .map(AgentId)
        .filter(|agent_id| kernel.registry.get(*agent_id).is_some())
        .or_else(|| {
            kernel
                .registry
                .find_by_name(agent_name)
                .map(|entry| entry.id)
        })
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

struct LegacyImport {
    session: Session,
    created_at: u64,
    updated_at: u64,
}

fn legacy_session_from_value(
    source_key: &str,
    agent_id: AgentId,
    value: &Value,
) -> Option<LegacyImport> {
    let messages = value
        .get("messages")?
        .as_array()?
        .iter()
        .filter(|message| message.get("tool").is_none_or(Value::is_null))
        .filter_map(|message| {
            let text = message.get("text")?.as_str()?.trim();
            if text.is_empty() {
                return None;
            }
            match message.get("role")?.as_str()? {
                "user" => Some(Message::user(text.to_string())),
                "agent" | "assistant" => Some(Message::assistant(text.to_string())),
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    if messages.is_empty() {
        return None;
    }

    let session_id = value
        .get("session_id")
        .and_then(Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(SessionId)
        .unwrap_or_else(|| legacy_tui_session_id(source_key));
    let created_at = value
        .get("created_at")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let updated_at = value
        .get("updated_at")
        .and_then(Value::as_u64)
        .unwrap_or(created_at);

    Some(LegacyImport {
        session: Session {
            id: session_id,
            agent_id,
            messages,
            context_window_tokens: 0,
            label: None,
        },
        created_at,
        updated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::KernelConfig;
    use captain_types::message::Role;

    #[test]
    fn extracts_uuid_from_daemon_and_inprocess_keys() {
        let id = uuid::Uuid::new_v4();
        assert_eq!(embedded_uuid(&format!("daemon-{id}")), Some(id));
        assert_eq!(embedded_uuid(&format!("inprocess-{id}-web")), Some(id));
        assert_eq!(embedded_uuid("captain"), None);
    }

    #[test]
    fn legacy_payload_keeps_only_conversation_turns_and_uses_stable_id() {
        let agent_id = AgentId::new();
        let payload = serde_json::json!({
            "messages": [
                {"role": "system", "text": "visual notice"},
                {"role": "user", "text": "  Diagnose Inventory  "},
                {"role": "agent", "text": "running tool", "tool": {"name": "ssh_exec"}},
                {"role": "assistant", "text": "Inventory is healthy"}
            ],
            "created_at": 10,
            "updated_at": 20
        });

        let imported = legacy_session_from_value("daemon-captain/10.json", agent_id, &payload)
            .expect("importable conversation");
        assert_eq!(imported.session.messages.len(), 2);
        assert_eq!(imported.session.messages[0].role, Role::User);
        assert_eq!(imported.session.messages[1].role, Role::Assistant);
        assert_eq!(imported.created_at, 10);
        assert_eq!(imported.updated_at, 20);
        assert_eq!(
            imported.session.id,
            legacy_tui_session_id("daemon-captain/10.json")
        );
    }

    #[test]
    fn embedded_authoritative_session_id_wins_over_legacy_derivation() {
        let expected = SessionId::new();
        let payload = serde_json::json!({
            "session_id": expected.0.to_string(),
            "messages": [{"role": "user", "text": "hello"}]
        });
        let imported = legacy_session_from_value("captain/old.json", AgentId::new(), &payload)
            .expect("importable conversation");
        assert_eq!(imported.session.id, expected);
    }

    #[test]
    fn boot_import_makes_legacy_history_authoritative_and_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("home");
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let captain = kernel
            .registry
            .find_by_name("captain")
            .expect("default Captain exists");
        let root = tmp.path().join("legacy");
        let agent_dir = root.join("captain");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(
            agent_dir.join("1710000000.json"),
            serde_json::json!({
                "agent_name": "captain",
                "created_at": 1_710_000_000u64,
                "updated_at": 1_710_000_010u64,
                "messages": [
                    {"role": "user", "text": "Reprendre le dossier du couple"},
                    {"role": "agent", "text": "Le dossier est prêt"}
                ]
            })
            .to_string(),
        )
        .unwrap();

        let first = import_from_root(&kernel, &root);
        assert_eq!(first.imported, 1);
        let session_id = legacy_tui_session_id("captain/1710000000.json");
        let imported = kernel
            .memory
            .get_session(session_id)
            .unwrap()
            .expect("authoritative session imported");
        assert_eq!(imported.agent_id, captain.id);
        assert_eq!(imported.messages.len(), 2);

        let second = import_from_root(&kernel, &root);
        assert_eq!(second.imported, 0);
        assert_eq!(second.discovered, 0);
        assert!(processed_marker_path(&agent_dir.join("1710000000.json")).is_file());
        kernel.shutdown();
    }
}

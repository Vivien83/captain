use std::io;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::session_text::format_session_markdown;
use crate::SessionExportFormat;

pub(super) const SESSION_EXPORT_SCHEMA: &str = "captain.session.export.v1";

#[derive(Debug, Clone)]
pub(super) struct SessionExportItem {
    pub(super) catalog: Option<Value>,
    pub(super) session: Value,
}

#[derive(Serialize)]
struct SessionExportRecord<'a> {
    schema: &'static str,
    exported_at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    catalog: Option<&'a Value>,
    session: &'a Value,
}

pub(super) fn render_session_export(
    items: &[SessionExportItem],
    format: SessionExportFormat,
    catalog_export: bool,
    exported_at: &str,
) -> Result<String, String> {
    if items.is_empty() {
        return Err("No persisted sessions matched the export selection.".to_string());
    }
    if catalog_export && format != SessionExportFormat::Jsonl {
        return Err(
            "A catalog export uses JSONL. Remove --format or pass --format jsonl.".to_string(),
        );
    }
    if !catalog_export && items.len() != 1 {
        return Err("A single-session export must contain exactly one session.".to_string());
    }

    match format {
        SessionExportFormat::Json => serde_json::to_string_pretty(&items[0].session)
            .map_err(|error| format!("Could not serialize the session export: {error}")),
        SessionExportFormat::Markdown => Ok(format_session_markdown(&items[0].session)),
        SessionExportFormat::Jsonl => {
            let mut rendered = String::new();
            for item in items {
                let record = SessionExportRecord {
                    schema: SESSION_EXPORT_SCHEMA,
                    exported_at,
                    catalog: item.catalog.as_ref(),
                    session: &item.session,
                };
                let line = serde_json::to_string(&record)
                    .map_err(|error| format!("Could not serialize the session export: {error}"))?;
                rendered.push_str(&line);
                rendered.push('\n');
            }
            Ok(rendered)
        }
    }
}

pub(super) fn write_private_session_export(path: &Path, rendered: &str) -> io::Result<()> {
    captain_types::durable_fs::atomic_write(path, rendered.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn item(id: &str) -> SessionExportItem {
        SessionExportItem {
            catalog: Some(json!({
                "session_id": id,
                "updated_at": "2026-07-22T10:00:00Z",
            })),
            session: json!({
                "session_id": id,
                "agent_id": "captain",
                "messages": [{"role": "user", "content": "hello"}],
            }),
        }
    }

    #[test]
    fn single_json_export_keeps_the_existing_payload_shape() {
        let rendered = render_session_export(
            &[item("session-1")],
            SessionExportFormat::Json,
            false,
            "2026-07-22T12:00:00Z",
        )
        .unwrap();
        let value: Value = serde_json::from_str(&rendered).unwrap();

        assert_eq!(value["session_id"], "session-1");
        assert!(value.get("schema").is_none());
    }

    #[test]
    fn catalog_jsonl_is_versioned_complete_and_ordered() {
        let rendered = render_session_export(
            &[item("newest"), item("older")],
            SessionExportFormat::Jsonl,
            true,
            "2026-07-22T12:00:00Z",
        )
        .unwrap();
        let records = rendered
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["schema"], SESSION_EXPORT_SCHEMA);
        assert_eq!(records[0]["exported_at"], "2026-07-22T12:00:00Z");
        assert_eq!(records[0]["catalog"]["session_id"], "newest");
        assert_eq!(records[0]["session"]["session_id"], "newest");
        assert_eq!(records[1]["session"]["session_id"], "older");
        assert!(rendered.ends_with('\n'));
    }

    #[test]
    fn catalog_export_refuses_ambiguous_formats_and_empty_results() {
        assert!(render_session_export(
            &[item("session-1")],
            SessionExportFormat::Json,
            true,
            "2026-07-22T12:00:00Z",
        )
        .unwrap_err()
        .contains("JSONL"));
        assert!(render_session_export(
            &[],
            SessionExportFormat::Jsonl,
            true,
            "2026-07-22T12:00:00Z",
        )
        .unwrap_err()
        .contains("No persisted sessions"));
    }

    #[cfg(unix)]
    #[test]
    fn file_export_is_atomic_and_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sessions.jsonl");
        write_private_session_export(&path, "first\n").unwrap();
        write_private_session_export(&path, "complete\n").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "complete\n");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

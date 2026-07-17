use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const RESTART_DEDUPE_FILE: &str = "system-control/restart-last-processed.json";
const RESTART_DEDUPE_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RestartDedupeMarker {
    channel: String,
    source_message_id: String,
    requested_at_unix_secs: u64,
}

pub(crate) fn is_restart_redelivery(
    home_dir: &Path,
    channel: &str,
    source_message_id: Option<&str>,
) -> bool {
    let Some(source_message_id) = source_message_id.filter(|id| !id.trim().is_empty()) else {
        return false;
    };
    let Ok(raw) = std::fs::read_to_string(restart_dedupe_path(home_dir)) else {
        return false;
    };
    let Ok(marker) = serde_json::from_str::<RestartDedupeMarker>(&raw) else {
        return false;
    };
    if marker.channel != channel || marker.source_message_id != source_message_id {
        return false;
    }
    now_unix_secs().saturating_sub(marker.requested_at_unix_secs) <= RESTART_DEDUPE_TTL_SECS
}

pub(crate) fn record_restart_processed(
    home_dir: &Path,
    channel: &str,
    source_message_id: Option<&str>,
) -> Result<(), String> {
    let Some(source_message_id) = source_message_id.filter(|id| !id.trim().is_empty()) else {
        return Ok(());
    };
    let path = restart_dedupe_path(home_dir);
    if let Some(parent) = path.parent() {
        captain_types::durable_fs::create_dir_all(parent)
            .map_err(|e| format!("create restart dedupe dir: {e}"))?;
    }
    let marker = RestartDedupeMarker {
        channel: channel.to_string(),
        source_message_id: source_message_id.to_string(),
        requested_at_unix_secs: now_unix_secs(),
    };
    let raw = serde_json::to_vec_pretty(&marker).map_err(|e| format!("serialize marker: {e}"))?;
    captain_types::durable_fs::atomic_write(&path, &raw)
        .map_err(|e| format!("persist restart dedupe marker: {e}"))?;
    Ok(())
}

fn restart_dedupe_path(home_dir: &Path) -> PathBuf {
    home_dir.join(RESTART_DEDUPE_FILE)
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_redelivery_matches_channel_and_message_only() {
        let dir =
            std::env::temp_dir().join(format!("captain-restart-dedupe-{}", uuid::Uuid::new_v4()));

        record_restart_processed(&dir, "telegram", Some("42")).unwrap();

        assert!(is_restart_redelivery(&dir, "telegram", Some("42")));
        assert!(!is_restart_redelivery(&dir, "discord", Some("42")));
        assert!(!is_restart_redelivery(&dir, "telegram", Some("43")));
        assert!(!is_restart_redelivery(&dir, "telegram", None));

        let _ = std::fs::remove_dir_all(dir);
    }
}

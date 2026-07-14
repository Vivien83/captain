//! Durable storage for inbound channel follow-ups.
//!
//! This stores pending turns and recovered in-flight pending turns. A recovered
//! message stays durable until the dispatch loop reports completion, giving the
//! bridge at-least-once recovery across crashes.

use crate::inbound_queue_types::{PendingInboundMessage, INBOUND_DEAD_LETTER_RETENTION_SECS};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub(crate) struct InboundQueueStore {
    path: PathBuf,
    max_entries: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingInboundRecord {
    pub key: String,
    pub channel: String,
    #[serde(default)]
    pub recovery_attempts: u32,
    #[serde(default)]
    pub pending: Option<PendingInboundMessage>,
    #[serde(default)]
    pub inflight: Option<PendingInboundMessage>,
    #[serde(default)]
    pub dead_letter: Option<DeadInboundRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DeadInboundRecord {
    pub message: PendingInboundMessage,
    pub recovery_attempts: u32,
    pub reason: String,
    #[serde(default = "Utc::now")]
    pub dead_lettered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedInboundQueue {
    version: u32,
    pending: Vec<PendingInboundRecord>,
}

impl InboundQueueStore {
    pub(crate) fn new(path: impl Into<PathBuf>, max_entries: usize) -> Self {
        Self {
            path: path.into(),
            max_entries,
        }
    }

    pub(crate) fn load_records(&self) -> Vec<PendingInboundRecord> {
        let Ok(raw) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let Ok(mut payload) = serde_json::from_str::<PersistedInboundQueue>(&raw) else {
            return Vec::new();
        };
        if payload.version != FORMAT_VERSION {
            return Vec::new();
        }
        let mut pruned = false;
        let original_len = payload.pending.len();
        payload.pending = payload
            .pending
            .into_iter()
            .filter_map(|mut record| {
                if record
                    .dead_letter
                    .as_ref()
                    .map(dead_letter_expired)
                    .unwrap_or(false)
                {
                    record.dead_letter = None;
                    pruned = true;
                }
                if record.pending.is_none()
                    && record.inflight.is_none()
                    && record.dead_letter.is_none()
                {
                    pruned = true;
                    None
                } else {
                    Some(record)
                }
            })
            .collect();
        payload.pending.truncate(self.max_entries);
        if pruned || original_len > payload.pending.len() {
            let _ = self.save_records(&payload.pending);
        }
        payload.pending
    }

    pub(crate) fn save_records(&self, records: &[PendingInboundRecord]) -> std::io::Result<()> {
        if records.is_empty() {
            match std::fs::remove_file(&self.path) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(err) => return Err(err),
            }
        }

        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let payload = PersistedInboundQueue {
            version: FORMAT_VERSION,
            pending: records.iter().take(self.max_entries).cloned().collect(),
        };
        let tmp = tmp_path(&self.path);
        let json = serde_json::to_string_pretty(&payload)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    path.with_extension(match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{ext}.tmp"),
        _ => "tmp".to_string(),
    })
}

fn dead_letter_expired(dead_letter: &DeadInboundRecord) -> bool {
    Utc::now()
        .signed_duration_since(dead_letter.dead_lettered_at)
        .num_seconds()
        >= INBOUND_DEAD_LETTER_RETENTION_SECS
}

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_PROCESS_REGISTRY_RECORDS: usize = 256;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProcessRegistryRecord {
    pub id: String,
    pub agent_id: String,
    pub command: String,
    pub pid: Option<u32>,
    pub started_at_unix_secs: u64,
    pub last_activity_unix_secs: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct RecoveredProcess {
    pub agent_id: String,
    pub command: String,
    pub pid: u32,
    pub started_at_unix_secs: u64,
    pub last_activity_unix_secs: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessRegistryStore {
    path: PathBuf,
}

impl ProcessRegistryStore {
    pub(crate) fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub(crate) fn load_records(&self) -> Vec<ProcessRegistryRecord> {
        let Ok(raw) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        serde_json::from_str::<Vec<ProcessRegistryRecord>>(&raw).unwrap_or_default()
    }

    pub(crate) fn save_records(&self, records: &[ProcessRegistryRecord]) -> Result<(), String> {
        let records = bounded_records(records);
        let data = serde_json::to_vec_pretty(&records)
            .map_err(|e| format!("failed to encode process registry: {e}"))?;
        captain_types::durable_fs::atomic_write(&self.path, &data)
            .map_err(|e| format!("failed to persist process registry: {e}"))?;
        Ok(())
    }
}

impl From<ProcessRegistryRecord> for RecoveredProcess {
    fn from(record: ProcessRegistryRecord) -> Self {
        Self {
            agent_id: record.agent_id,
            command: record.command,
            pid: record.pid.unwrap_or(0),
            started_at_unix_secs: record.started_at_unix_secs,
            last_activity_unix_secs: record.last_activity_unix_secs,
        }
    }
}

pub(crate) fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .stderr(std::process::Stdio::null())
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }

    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

fn bounded_records(records: &[ProcessRegistryRecord]) -> Vec<ProcessRegistryRecord> {
    let mut records = records.to_vec();
    records.sort_by_key(|r| std::cmp::Reverse(r.started_at_unix_secs));
    records.truncate(MAX_PROCESS_REGISTRY_RECORDS);
    records.sort_by(|a, b| a.id.cmp(&b.id));
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_store_round_trips_records_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("process_registry.json");
        let store = ProcessRegistryStore::new(&path);
        let records = vec![ProcessRegistryRecord {
            id: "proc_7".to_string(),
            agent_id: "agent".to_string(),
            command: "sleep 30".to_string(),
            pid: Some(123),
            started_at_unix_secs: 10,
            last_activity_unix_secs: 12,
        }];

        store.save_records(&records).unwrap();

        assert_eq!(store.load_records(), records);
        assert!(!tmp.path().join("process_registry.json.tmp").exists());
    }

    #[test]
    fn pid_zero_is_never_alive() {
        assert!(!pid_is_alive(0));
    }

    #[test]
    fn registry_store_bounds_records() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("process_registry.json");
        let store = ProcessRegistryStore::new(&path);
        let records: Vec<ProcessRegistryRecord> = (0..300)
            .map(|idx| ProcessRegistryRecord {
                id: format!("proc_{idx}"),
                agent_id: "agent".to_string(),
                command: "sleep 30".to_string(),
                pid: Some(idx),
                started_at_unix_secs: idx as u64,
                last_activity_unix_secs: idx as u64,
            })
            .collect();

        store.save_records(&records).unwrap();

        let loaded = store.load_records();
        assert_eq!(loaded.len(), MAX_PROCESS_REGISTRY_RECORDS);
        assert!(loaded
            .iter()
            .all(|record| record.started_at_unix_secs >= 44));
    }
}

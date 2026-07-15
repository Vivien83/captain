//! Process-wide registry for observable tool executions.
//!
//! The agent loop still records normal `ToolResult` blocks for model
//! continuity, but long operations also need an operator/agent-visible handle.
//! This registry keeps a bounded, public-safe snapshot of recent tool runs and
//! owns abort handles for detached executions.

use captain_memory::detached_tool_runs::{DetachedToolRunRecord, DetachedToolRunStore};
use captain_types::tool::ToolResult;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task::AbortHandle;
use tracing::warn;

/// Bound on both in-memory entries and how many persisted rows are
/// reloaded into the registry at boot (see `hydrate_from_persisted`).
pub const MAX_RUNS: usize = 200;
const MAX_RESULT_CHARS: usize = 100_000;
const MAX_PREVIEW_CHARS: usize = 4_000;

static GLOBAL_TOOL_RUNS: OnceLock<Arc<ToolRunRegistry>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolRunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    /// Reconciled at boot from a persisted row still `Running` when
    /// Captain last shut down — the process died mid-run, so no real
    /// result was ever recorded. See `hydrate_from_persisted`.
    Interrupted,
}

impl ToolRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }

    fn from_db_str(value: &str) -> Self {
        match value {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            _ => Self::Running,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolRunSnapshot {
    pub run_id: String,
    pub tool_name: String,
    pub status: ToolRunStatus,
    pub detached: bool,
    pub cancellable: bool,
    pub started_at_unix_ms: u128,
    pub finished_at_unix_ms: Option<u128>,
    pub elapsed_ms: u128,
    pub caller_agent_id: Option<String>,
    pub origin_tool_use_id: Option<String>,
    pub is_error: Option<bool>,
    pub result_preview: Option<String>,
    pub result_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolRunResultSnapshot {
    #[serde(flatten)]
    pub snapshot: ToolRunSnapshot,
    pub result: Option<String>,
}

#[derive(Debug, Clone)]
struct ToolRunEntry {
    run_id: String,
    tool_name: String,
    status: ToolRunStatus,
    detached: bool,
    started_at: SystemTime,
    finished_at: Option<SystemTime>,
    caller_agent_id: Option<String>,
    origin_tool_use_id: Option<String>,
    is_error: Option<bool>,
    result: Option<String>,
    result_truncated: bool,
    abort_handle: Option<AbortHandle>,
}

impl ToolRunEntry {
    fn snapshot(&self, now: SystemTime) -> ToolRunSnapshot {
        let result_preview = self.result.as_ref().map(|result| {
            let clipped = crate::str_utils::safe_truncate_str(result, MAX_PREVIEW_CHARS);
            if result.len() > clipped.len() {
                format!(
                    "{clipped}\n[truncated preview, {} chars total]",
                    result.len()
                )
            } else {
                clipped.to_string()
            }
        });
        ToolRunSnapshot {
            run_id: self.run_id.clone(),
            tool_name: self.tool_name.clone(),
            status: self.status,
            detached: self.detached,
            cancellable: self.abort_handle.is_some() && !self.status.is_terminal(),
            started_at_unix_ms: unix_ms(self.started_at),
            finished_at_unix_ms: self.finished_at.map(unix_ms),
            elapsed_ms: elapsed_ms(self.started_at, self.finished_at.unwrap_or(now)),
            caller_agent_id: self.caller_agent_id.clone(),
            origin_tool_use_id: self.origin_tool_use_id.clone(),
            is_error: self.is_error,
            result_preview,
            result_truncated: self.result_truncated,
        }
    }
}

#[derive(Default)]
struct ToolRunState {
    runs: HashMap<String, ToolRunEntry>,
    order: VecDeque<String>,
}

#[derive(Default)]
pub struct ToolRunRegistry {
    state: Mutex<ToolRunState>,
    persistence: OnceLock<DetachedToolRunStore>,
}

impl ToolRunRegistry {
    /// Wire a persistence backend, once, at kernel boot. Before this is
    /// called, `start`/`finish`/`cancel` behave exactly as before
    /// (in-memory only) — persistence is best-effort and purely additive.
    pub fn configure_persistence(&self, store: DetachedToolRunStore) {
        if self.persistence.set(store).is_err() {
            warn!("Tool run registry persistence already configured, ignoring");
        }
    }

    /// Reload persisted detached runs into the in-memory registry so
    /// `tool_run_status`/`tool_run_result`/`tool_run_list` stay consistent
    /// across a restart. Rows still `Running` in the DB are a crash
    /// signature and get reconciled to `Interrupted` first. Call once at
    /// boot, after `configure_persistence`.
    pub fn hydrate_from_persisted(&self, mut records: Vec<DetachedToolRunRecord>) {
        // The store returns newest-first for operator queries, while `order`
        // is oldest-first and `list()` reverses it. Normalize before hydrate
        // so restart preserves the same ordering and pruning semantics.
        records.sort_by(|left, right| {
            left.started_at_unix_ms
                .cmp(&right.started_at_unix_ms)
                .then_with(|| left.run_id.cmp(&right.run_id))
        });
        let mut state = self.state.lock().expect("tool run registry poisoned");
        for record in records {
            if state.runs.contains_key(&record.run_id) {
                continue;
            }
            state.order.push_back(record.run_id.clone());
            state
                .runs
                .insert(record.run_id.clone(), entry_from_record(record));
        }
        prune_old_runs(&mut state);
    }

    pub fn start(
        &self,
        tool_name: impl Into<String>,
        caller_agent_id: Option<String>,
        origin_tool_use_id: Option<String>,
        detached: bool,
    ) -> String {
        let run_id = format!("toolrun-{}", uuid::Uuid::new_v4());
        let entry = ToolRunEntry {
            run_id: run_id.clone(),
            tool_name: tool_name.into(),
            status: ToolRunStatus::Running,
            detached,
            started_at: SystemTime::now(),
            finished_at: None,
            caller_agent_id,
            origin_tool_use_id,
            is_error: None,
            result: None,
            result_truncated: false,
            abort_handle: None,
        };
        if detached {
            self.persist_start(&entry);
        }
        let mut state = self.state.lock().expect("tool run registry poisoned");
        state.order.push_back(run_id.clone());
        state.runs.insert(run_id.clone(), entry);
        prune_old_runs(&mut state);
        run_id
    }

    fn persist_start(&self, entry: &ToolRunEntry) {
        let Some(store) = self.persistence.get() else {
            return;
        };
        if let Err(e) = store.upsert_running(
            &entry.run_id,
            &entry.tool_name,
            entry.caller_agent_id.as_deref(),
            entry.origin_tool_use_id.as_deref(),
            unix_ms(entry.started_at) as i64,
        ) {
            warn!(run_id = %entry.run_id, "Failed to persist detached tool run start: {e}");
        }
    }

    fn persist_finish(&self, entry: &ToolRunEntry) {
        if !entry.detached {
            return;
        }
        let Some(store) = self.persistence.get() else {
            return;
        };
        let Some(finished_at) = entry.finished_at else {
            return;
        };
        if let Err(e) = store.mark_finished(
            &entry.run_id,
            entry.status.as_str(),
            entry.is_error,
            entry.result.as_deref(),
            entry.result_truncated,
            unix_ms(finished_at) as i64,
        ) {
            warn!(run_id = %entry.run_id, "Failed to persist detached tool run finish: {e}");
            return;
        }
        if let Err(e) = store.prune_terminal_history(MAX_RUNS) {
            warn!(run_id = %entry.run_id, "Failed to prune detached tool run history: {e}");
        }
    }

    pub fn attach_abort_handle(&self, run_id: &str, handle: AbortHandle) {
        if let Some(entry) = self
            .state
            .lock()
            .expect("tool run registry poisoned")
            .runs
            .get_mut(run_id)
        {
            if !entry.status.is_terminal() {
                entry.abort_handle = Some(handle);
            }
        }
    }

    pub fn finish(&self, run_id: &str, result: &ToolResult) {
        let status = if result.is_error {
            ToolRunStatus::Failed
        } else {
            ToolRunStatus::Completed
        };
        self.finish_with_content(run_id, status, result.is_error, result.content.clone());
    }

    pub fn append_chunk(&self, run_id: &str, stream: &str, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        if let Some(entry) = self
            .state
            .lock()
            .expect("tool run registry poisoned")
            .runs
            .get_mut(run_id)
        {
            if entry.status.is_terminal() {
                return;
            }
            let mut content = entry.result.take().unwrap_or_default();
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            if stream == "stdout" {
                content.push_str(chunk);
            } else {
                content.push_str(&format!("--- {stream} ---\n{chunk}"));
            }
            let (content, truncated) = bounded_result(content);
            entry.result = Some(content);
            entry.result_truncated |= truncated;
        }
    }

    pub fn finish_with_content(
        &self,
        run_id: &str,
        status: ToolRunStatus,
        is_error: bool,
        content: String,
    ) {
        let (result, result_truncated) = bounded_result(content);
        let finished_entry = {
            let mut state = self.state.lock().expect("tool run registry poisoned");
            let Some(entry) = state.runs.get_mut(run_id) else {
                return;
            };
            if entry.status.is_terminal() {
                return;
            }
            entry.status = status;
            entry.finished_at = Some(SystemTime::now());
            entry.is_error = Some(is_error);
            entry.result = Some(result);
            entry.result_truncated = result_truncated;
            entry.abort_handle = None;
            entry.clone()
        };
        // Persisted outside the in-memory lock: this is a blocking SQLite
        // write and must never hold up other callers touching the registry.
        self.persist_finish(&finished_entry);
    }

    pub fn cancel(&self, run_id: &str) -> Result<ToolRunSnapshot, String> {
        let (abort, cancelled_entry) = {
            let mut state = self.state.lock().expect("tool run registry poisoned");
            let entry = state
                .runs
                .get_mut(run_id)
                .ok_or_else(|| format!("Unknown tool run id: {run_id}"))?;
            if entry.status.is_terminal() {
                return Ok(entry.snapshot(SystemTime::now()));
            }
            entry.status = ToolRunStatus::Cancelled;
            entry.finished_at = Some(SystemTime::now());
            entry.is_error = Some(true);
            entry.result = Some("Tool run cancelled by request.".to_string());
            entry.result_truncated = false;
            let abort = entry.abort_handle.take();
            (abort, entry.clone())
        };
        if let Some(abort) = abort {
            abort.abort();
        }
        self.persist_finish(&cancelled_entry);
        self.snapshot(run_id)
            .ok_or_else(|| format!("Unknown tool run id: {run_id}"))
    }

    pub fn snapshot(&self, run_id: &str) -> Option<ToolRunSnapshot> {
        let now = SystemTime::now();
        self.state
            .lock()
            .expect("tool run registry poisoned")
            .runs
            .get(run_id)
            .map(|entry| entry.snapshot(now))
    }

    pub fn result(&self, run_id: &str) -> Option<ToolRunResultSnapshot> {
        let now = SystemTime::now();
        self.state
            .lock()
            .expect("tool run registry poisoned")
            .runs
            .get(run_id)
            .map(|entry| ToolRunResultSnapshot {
                snapshot: entry.snapshot(now),
                result: entry.result.clone(),
            })
    }

    pub fn list(&self, status: Option<ToolRunStatus>, limit: usize) -> Vec<ToolRunSnapshot> {
        let now = SystemTime::now();
        let state = self.state.lock().expect("tool run registry poisoned");
        state
            .order
            .iter()
            .rev()
            .filter_map(|run_id| state.runs.get(run_id))
            .filter(|entry| status.is_none_or(|wanted| entry.status == wanted))
            .take(limit.clamp(1, MAX_RUNS))
            .map(|entry| entry.snapshot(now))
            .collect()
    }

    pub fn status_summary(&self) -> serde_json::Value {
        let state = self.state.lock().expect("tool run registry poisoned");
        let mut running = 0usize;
        let mut completed = 0usize;
        let mut failed = 0usize;
        let mut cancelled = 0usize;
        let mut interrupted = 0usize;
        for entry in state.runs.values() {
            match entry.status {
                ToolRunStatus::Running => running += 1,
                ToolRunStatus::Completed => completed += 1,
                ToolRunStatus::Failed => failed += 1,
                ToolRunStatus::Cancelled => cancelled += 1,
                ToolRunStatus::Interrupted => interrupted += 1,
            }
        }
        let recent: Vec<_> = state
            .order
            .iter()
            .rev()
            .filter_map(|run_id| state.runs.get(run_id))
            .take(10)
            .map(|entry| status_summary_entry(entry, SystemTime::now()))
            .collect();
        serde_json::json!({
            "running": running,
            "completed": completed,
            "failed": failed,
            "cancelled": cancelled,
            "interrupted": interrupted,
            "recent": recent,
        })
    }
}

fn status_summary_entry(entry: &ToolRunEntry, now: SystemTime) -> serde_json::Value {
    serde_json::json!({
        "run_id": entry.run_id,
        "tool_name": entry.tool_name,
        "status": entry.status,
        "detached": entry.detached,
        "cancellable": entry.abort_handle.is_some() && !entry.status.is_terminal(),
        "started_at_unix_ms": unix_ms(entry.started_at),
        "finished_at_unix_ms": entry.finished_at.map(unix_ms),
        "elapsed_ms": elapsed_ms(entry.started_at, entry.finished_at.unwrap_or(now)),
        "caller_agent_id": entry.caller_agent_id,
        "origin_tool_use_id": entry.origin_tool_use_id,
        "is_error": entry.is_error,
        "result_available": entry.result.is_some(),
        "result_truncated": entry.result_truncated,
    })
}

pub fn global_registry() -> Arc<ToolRunRegistry> {
    GLOBAL_TOOL_RUNS
        .get_or_init(|| Arc::new(ToolRunRegistry::default()))
        .clone()
}

pub fn parse_status_filter(raw: Option<&str>) -> Result<Option<ToolRunStatus>, String> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match raw {
        "running" => Ok(Some(ToolRunStatus::Running)),
        "completed" => Ok(Some(ToolRunStatus::Completed)),
        "failed" => Ok(Some(ToolRunStatus::Failed)),
        "cancelled" | "canceled" => Ok(Some(ToolRunStatus::Cancelled)),
        "interrupted" => Ok(Some(ToolRunStatus::Interrupted)),
        other => Err(format!(
            "Invalid tool run status `{other}`. Use running, completed, failed, cancelled, or interrupted."
        )),
    }
}

fn prune_old_runs(state: &mut ToolRunState) {
    while state.order.len() > MAX_RUNS {
        let Some(oldest) = state.order.pop_front() else {
            break;
        };
        if state
            .runs
            .get(&oldest)
            .is_some_and(|entry| entry.status == ToolRunStatus::Running)
        {
            state.order.push_back(oldest);
            break;
        }
        state.runs.remove(&oldest);
    }
}

fn bounded_result(content: String) -> (String, bool) {
    let clipped = crate::str_utils::safe_truncate_str(&content, MAX_RESULT_CHARS);
    if clipped.len() == content.len() {
        (content, false)
    } else {
        (
            format!(
                "{clipped}\n[truncated tool run result, {} chars total]",
                content.len()
            ),
            true,
        )
    }
}

fn unix_ms(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

fn system_time_from_unix_ms(ms: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(ms.max(0) as u64)
}

fn entry_from_record(record: DetachedToolRunRecord) -> ToolRunEntry {
    ToolRunEntry {
        run_id: record.run_id,
        tool_name: record.tool_name,
        status: ToolRunStatus::from_db_str(&record.status),
        detached: true,
        started_at: system_time_from_unix_ms(record.started_at_unix_ms),
        finished_at: record.finished_at_unix_ms.map(system_time_from_unix_ms),
        caller_agent_id: record.caller_agent_id,
        origin_tool_use_id: record.origin_tool_use_id,
        is_error: record.is_error,
        result: record.result,
        result_truncated: record.result_truncated,
        abort_handle: None,
    }
}

fn elapsed_ms(started_at: SystemTime, ended_at: SystemTime) -> u128 {
    ended_at
        .duration_since(started_at)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(content: &str, is_error: bool) -> ToolResult {
        ToolResult {
            tool_use_id: "tool-use".into(),
            content: content.into(),
            is_error,
            transient_content: Vec::new(),
        }
    }

    #[test]
    fn registry_tracks_running_and_completed_runs() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("shell_exec", Some("agent".into()), Some("tc1".into()), true);
        let running = registry.snapshot(&run_id).unwrap();
        assert_eq!(running.status, ToolRunStatus::Running);
        assert!(running.detached);
        assert_eq!(running.caller_agent_id.as_deref(), Some("agent"));

        registry.finish(&run_id, &result("ok", false));
        let completed = registry.result(&run_id).unwrap();
        assert_eq!(completed.snapshot.status, ToolRunStatus::Completed);
        assert_eq!(completed.result.as_deref(), Some("ok"));
    }

    fn persistence_store() -> DetachedToolRunStore {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        captain_memory::migration::run_migrations(&conn).unwrap();
        DetachedToolRunStore::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn detached_start_and_finish_are_persisted() {
        let registry = ToolRunRegistry::default();
        let store = persistence_store();
        registry.configure_persistence(store.clone());

        let run_id = registry.start("cargo", Some("agent-1".into()), None, true);
        let persisted = store.list_recent(10).unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].run_id, run_id);
        assert_eq!(persisted[0].status, "running");

        registry.finish(&run_id, &result("build ok", false));
        let persisted = store.list_recent(10).unwrap();
        assert_eq!(persisted[0].status, "completed");
        assert_eq!(persisted[0].result.as_deref(), Some("build ok"));
    }

    #[test]
    fn foreground_runs_are_never_persisted() {
        let registry = ToolRunRegistry::default();
        let store = persistence_store();
        registry.configure_persistence(store.clone());

        let run_id = registry.start("file_read", None, None, false);
        registry.finish(&run_id, &result("contents", false));

        assert!(store.list_recent(10).unwrap().is_empty());
    }

    #[test]
    fn restart_reconciles_running_rows_and_hydrates_registry() {
        let store = persistence_store();
        // Simulate the previous process dying mid-run: a row left `running`
        // with no matching in-memory registry (the OnceLock static reset on
        // every process start).
        store
            .upsert_running("toolrun-crashed", "ssh_exec", Some("agent-1"), None, 1_000)
            .unwrap();

        // Boot sequence: reconcile, then hydrate a fresh registry.
        let interrupted = store.reconcile_running_as_interrupted().unwrap();
        assert_eq!(interrupted.len(), 1);

        let registry = ToolRunRegistry::default();
        registry.configure_persistence(store.clone());
        registry.hydrate_from_persisted(store.list_recent(200).unwrap());

        let snapshot = registry.snapshot("toolrun-crashed").unwrap();
        assert_eq!(snapshot.status, ToolRunStatus::Interrupted);
        assert_eq!(snapshot.is_error, Some(true));
        let result = registry.result("toolrun-crashed").unwrap();
        assert!(result
            .result
            .unwrap()
            .contains("interrupted by a Captain restart"));
    }

    #[test]
    fn hydration_preserves_newest_first_operator_order() {
        let store = persistence_store();
        store
            .upsert_running("toolrun-old", "cargo", None, None, 1_000)
            .unwrap();
        store
            .mark_finished(
                "toolrun-old",
                "completed",
                Some(false),
                Some("old"),
                false,
                1_100,
            )
            .unwrap();
        store
            .upsert_running("toolrun-new", "cargo", None, None, 2_000)
            .unwrap();
        store
            .mark_finished(
                "toolrun-new",
                "completed",
                Some(false),
                Some("new"),
                false,
                2_100,
            )
            .unwrap();

        let registry = ToolRunRegistry::default();
        registry.hydrate_from_persisted(store.list_recent(10).unwrap());

        let ids: Vec<_> = registry
            .list(None, 10)
            .into_iter()
            .map(|snapshot| snapshot.run_id)
            .collect();
        assert_eq!(ids, vec!["toolrun-new", "toolrun-old"]);
    }

    #[test]
    fn registry_marks_tool_errors_as_failed() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("ssh_exec", None, None, false);
        registry.finish(&run_id, &result("boom", true));
        let snapshot = registry.snapshot(&run_id).unwrap();
        assert_eq!(snapshot.status, ToolRunStatus::Failed);
        assert_eq!(snapshot.is_error, Some(true));
    }

    #[test]
    fn registry_exposes_partial_chunks_while_running() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("shell_exec", None, None, true);
        registry.append_chunk(&run_id, "stdout", "hello");
        registry.append_chunk(&run_id, "stderr", "warn");

        let snapshot = registry.snapshot(&run_id).unwrap();
        assert_eq!(snapshot.status, ToolRunStatus::Running);
        let preview = snapshot.result_preview.unwrap();
        assert!(preview.contains("hello"));
        assert!(preview.contains("--- stderr ---"));
        assert!(preview.contains("warn"));
    }

    #[test]
    fn status_summary_omits_result_preview() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("memory_recall", None, Some("tc-memory".into()), false);
        registry.finish(&run_id, &result("sensitive-ish memory output", false));

        let summary = registry.status_summary();
        assert_eq!(summary["completed"], 1);
        let recent = &summary["recent"][0];
        assert_eq!(recent["tool_name"], "memory_recall");
        assert_eq!(recent["result_available"], true);
        assert!(recent.get("result_preview").is_none());
        assert!(recent.get("result").is_none());
    }

    #[test]
    fn status_filter_accepts_expected_values() {
        assert_eq!(
            parse_status_filter(Some("running")).unwrap(),
            Some(ToolRunStatus::Running)
        );
        assert_eq!(
            parse_status_filter(Some("canceled")).unwrap(),
            Some(ToolRunStatus::Cancelled)
        );
        assert_eq!(
            parse_status_filter(Some("interrupted")).unwrap(),
            Some(ToolRunStatus::Interrupted)
        );
        assert!(parse_status_filter(Some("weird")).is_err());
    }
}

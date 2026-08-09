//! Process-wide registry for observable tool executions.
//!
//! The agent loop still records normal `ToolResult` blocks for model
//! continuity, but long operations also need an operator/agent-visible handle.
//! This registry keeps a bounded, public-safe snapshot of recent tool runs and
//! owns abort handles for detached executions.

use captain_memory::detached_tool_runs::{
    DetachedToolRunCompletion, DetachedToolRunRecord, DetachedToolRunStore,
};
use captain_types::tool::ToolResult;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task::AbortHandle;
use tracing::warn;

type ToolRunOutputSources = (
    Option<crate::tool_run_output::ToolRunOutputMetadata>,
    Option<Arc<Mutex<crate::tool_run_output::ToolRunOutputCapture>>>,
    Option<String>,
);

/// Bound on both in-memory entries and how many persisted rows are
/// reloaded into the registry at boot (see `hydrate_from_persisted`).
pub const MAX_RUNS: usize = 200;
pub const MAX_RETRY_ATTEMPTS: usize = 3;
const MAX_RESULT_CHARS: usize = 100_000;
const MAX_PREVIEW_CHARS: usize = 4_000;
const WITHHELD_RESULT: &str = "[tool run result withheld because it could not be safely redacted]";

static GLOBAL_TOOL_RUNS: OnceLock<Arc<ToolRunRegistry>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

/// Typed failure returned by the operator cancellation path.
///
/// The normal agent-facing `cancel` method intentionally keeps its historical
/// semantics. Operator APIs use this stricter contract so they can only stop a
/// live task that Captain can actually abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRunCancelError {
    NotFound,
    NotActive { status: ToolRunStatus },
    NotCancellable,
}

impl std::fmt::Display for ToolRunCancelError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("tool run not found"),
            Self::NotActive { status } => {
                write!(formatter, "tool run is not active ({})", status.as_str())
            }
            Self::NotCancellable => formatter.write_str("tool run has no active abort handle"),
        }
    }
}

impl std::error::Error for ToolRunCancelError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancellationPolicy {
    Legacy,
    ActiveCancellableOnly,
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
    pub input_sha256: Option<String>,
    pub retry_of_run_id: Option<String>,
    pub retry_attempt: u32,
    pub is_error: Option<bool>,
    pub result_preview: Option<String>,
    pub result_truncated: bool,
    pub output_available: bool,
    pub output_stored_bytes: Option<u64>,
    pub output_total_bytes: Option<u64>,
    pub output_sha256: Option<String>,
    pub output_capped: bool,
    pub output_redacted: bool,
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
    input_sha256: Option<String>,
    retry_of_run_id: Option<String>,
    retry_attempt: u32,
    is_error: Option<bool>,
    result: Option<String>,
    result_truncated: bool,
    output: Option<crate::tool_run_output::ToolRunOutputMetadata>,
    output_capture: Option<Arc<Mutex<crate::tool_run_output::ToolRunOutputCapture>>>,
    abort_handle: Option<AbortHandle>,
}

impl ToolRunEntry {
    fn is_cancellable(&self) -> bool {
        !self.status.is_terminal()
            && self
                .abort_handle
                .as_ref()
                .is_some_and(|handle| !handle.is_finished())
    }

    fn snapshot(&self, now: SystemTime) -> ToolRunSnapshot {
        let (result_preview, preview_withheld) =
            self.result.as_ref().map_or((None, false), |result| {
                let (sanitized, withheld) = sanitize_retained_result(result.clone());
                let clipped = crate::str_utils::safe_truncate_str(&sanitized, MAX_PREVIEW_CHARS);
                let preview = if sanitized.len() > clipped.len() {
                    format!(
                        "{clipped}\n[truncated preview, {} chars total]",
                        sanitized.len()
                    )
                } else {
                    clipped.to_string()
                };
                (Some(preview), withheld)
            });
        ToolRunSnapshot {
            run_id: self.run_id.clone(),
            tool_name: self.tool_name.clone(),
            status: self.status,
            detached: self.detached,
            cancellable: self.is_cancellable(),
            started_at_unix_ms: unix_ms(self.started_at),
            finished_at_unix_ms: self.finished_at.map(unix_ms),
            elapsed_ms: elapsed_ms(self.started_at, self.finished_at.unwrap_or(now)),
            caller_agent_id: self.caller_agent_id.clone(),
            origin_tool_use_id: self.origin_tool_use_id.clone(),
            input_sha256: self.input_sha256.clone(),
            retry_of_run_id: self.retry_of_run_id.clone(),
            retry_attempt: self.retry_attempt,
            is_error: self.is_error,
            result_preview,
            result_truncated: self.result_truncated || preview_withheld,
            output_available: self.output.is_some() || self.output_capture.is_some(),
            output_stored_bytes: self.output.as_ref().map(|output| output.stored_bytes),
            output_total_bytes: self.output.as_ref().map(|output| output.total_bytes),
            output_sha256: self.output.as_ref().map(|output| output.sha256.clone()),
            output_capped: self.output.as_ref().is_some_and(|output| output.capped),
            output_redacted: self.output.as_ref().is_some_and(|output| output.redacted),
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
    output_store: OnceLock<Arc<crate::tool_run_output::ToolRunOutputStore>>,
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

    pub fn configure_output_store(&self, store: crate::tool_run_output::ToolRunOutputStore) {
        if self.output_store.set(Arc::new(store)).is_err() {
            warn!("Tool run output store already configured, ignoring");
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
        let mut sanitized_entries = Vec::new();
        let mut state = self.state.lock().expect("tool run registry poisoned");
        for record in records {
            if state.runs.contains_key(&record.run_id) {
                continue;
            }
            let original_result = record.result.clone();
            state.order.push_back(record.run_id.clone());
            let mut entry = entry_from_record(record);
            if entry.output.as_ref().is_some_and(|metadata| {
                self.output_store
                    .get()
                    .is_none_or(|store| !store.metadata_exists(metadata))
            }) {
                entry.output = None;
            }
            if entry.status.is_terminal() && entry.result != original_result {
                sanitized_entries.push(entry.clone());
            }
            state.runs.insert(entry.run_id.clone(), entry);
        }
        prune_old_runs(&mut state);
        drop(state);
        for entry in sanitized_entries {
            self.persist_finish(&entry);
        }
    }

    pub fn start(
        &self,
        tool_name: impl Into<String>,
        caller_agent_id: Option<String>,
        origin_tool_use_id: Option<String>,
        detached: bool,
        input_sha256: Option<String>,
    ) -> String {
        self.start_with_retry(
            tool_name,
            caller_agent_id,
            origin_tool_use_id,
            detached,
            input_sha256,
            None,
        )
    }

    pub(crate) fn start_with_retry(
        &self,
        tool_name: impl Into<String>,
        caller_agent_id: Option<String>,
        origin_tool_use_id: Option<String>,
        detached: bool,
        input_sha256: Option<String>,
        retry_of_run_id: Option<String>,
    ) -> String {
        let run_id = format!("toolrun-{}", uuid::Uuid::new_v4());
        let retry_attempt = retry_of_run_id
            .as_deref()
            .map(|parent_id| {
                self.state
                    .lock()
                    .expect("tool run registry poisoned")
                    .runs
                    .get(parent_id)
                    .map_or(1, |parent| parent.retry_attempt.saturating_add(1))
            })
            .unwrap_or(0);
        let output_capture = self.output_store.get().and_then(|store| {
            store
                .begin_capture(&run_id)
                .map(|capture| Arc::new(Mutex::new(capture)))
                .map_err(|error| {
                    warn!(run_id = %run_id, "Failed to start tool run output capture: {error}");
                    error
                })
                .ok()
        });
        let entry = ToolRunEntry {
            run_id: run_id.clone(),
            tool_name: tool_name.into(),
            status: ToolRunStatus::Running,
            detached,
            started_at: SystemTime::now(),
            finished_at: None,
            caller_agent_id,
            origin_tool_use_id,
            input_sha256,
            retry_of_run_id,
            retry_attempt,
            is_error: None,
            result: None,
            result_truncated: false,
            output: None,
            output_capture,
            abort_handle: None,
        };
        self.persist_start(&entry);
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
        if let Err(e) = store.upsert_running_with_retry(
            &entry.run_id,
            &entry.tool_name,
            entry.caller_agent_id.as_deref(),
            entry.origin_tool_use_id.as_deref(),
            entry.detached,
            entry.input_sha256.as_deref(),
            entry.retry_of_run_id.as_deref(),
            entry.retry_attempt,
            unix_ms(entry.started_at) as i64,
        ) {
            warn!(run_id = %entry.run_id, "Failed to persist tool run start: {e}");
        }
    }

    fn persist_finish(&self, entry: &ToolRunEntry) {
        let Some(store) = self.persistence.get() else {
            return;
        };
        let Some(finished_at) = entry.finished_at else {
            return;
        };
        if let Err(e) = store.mark_finished(
            &entry.run_id,
            DetachedToolRunCompletion {
                status: entry.status.as_str(),
                is_error: entry.is_error,
                result: entry.result.as_deref(),
                result_truncated: entry.result_truncated,
                output_file_name: entry
                    .output
                    .as_ref()
                    .map(|output| output.file_name.as_str()),
                output_stored_bytes: entry.output.as_ref().map(|output| output.stored_bytes),
                output_total_bytes: entry.output.as_ref().map(|output| output.total_bytes),
                output_sha256: entry.output.as_ref().map(|output| output.sha256.as_str()),
                output_capped: entry.output.as_ref().is_some_and(|output| output.capped),
                output_redacted: entry.output.as_ref().is_some_and(|output| output.redacted),
                finished_at_unix_ms: unix_ms(finished_at) as i64,
            },
        ) {
            warn!(run_id = %entry.run_id, "Failed to persist tool run finish: {e}");
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
        let formatted = format_stream_chunk(stream, chunk);
        let capture = {
            let mut state = self.state.lock().expect("tool run registry poisoned");
            let Some(entry) = state.runs.get_mut(run_id) else {
                return;
            };
            if entry.status.is_terminal() {
                return;
            }
            let mut content = entry.result.take().unwrap_or_default();
            if !entry.result_truncated {
                if !content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push_str(&formatted);
                let (bounded, truncated) = bounded_result(content);
                entry.result = Some(bounded);
                entry.result_truncated = truncated;
            } else {
                entry.result = Some(content);
            }
            entry.output_capture.clone()
        };
        if let (Some(store), Some(capture)) = (self.output_store.get(), capture) {
            if let Err(error) = store.append_capture(&capture, &formatted) {
                warn!(run_id, "Failed to append tool run output capture: {error}");
            }
        }
    }

    pub fn finish_with_content(
        &self,
        run_id: &str,
        status: ToolRunStatus,
        is_error: bool,
        content: String,
    ) {
        let capture = self
            .state
            .lock()
            .expect("tool run registry poisoned")
            .runs
            .get(run_id)
            .and_then(|entry| entry.output_capture.clone());
        let output = self.finalize_output(run_id, capture.as_ref(), &content);
        let (retained_content, withheld) = sanitize_retained_result(content.clone());
        if withheld {
            warn!(
                run_id,
                "Tool run result was withheld during secret redaction"
            );
        }
        let (result, result_truncated) = bounded_result(retained_content);
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
            entry.result_truncated = result_truncated || withheld;
            entry.output = output;
            entry.output_capture = None;
            entry.abort_handle = None;
            entry.clone()
        };
        // Persisted outside the in-memory lock: this is a blocking SQLite
        // write and must never hold up other callers touching the registry.
        self.persist_finish(&finished_entry);
    }

    fn finalize_output(
        &self,
        run_id: &str,
        capture: Option<&Arc<Mutex<crate::tool_run_output::ToolRunOutputCapture>>>,
        final_content: &str,
    ) -> Option<crate::tool_run_output::ToolRunOutputMetadata> {
        let store = self.output_store.get()?;
        if let Some(capture) = capture {
            match store.capture_stats(capture) {
                Ok((_, total_bytes, _)) if total_bytes > 0 => {
                    match store.finalize_capture(run_id, capture) {
                        Ok(metadata) => return Some(metadata),
                        Err(error) => {
                            warn!(run_id, "Failed to finalize tool run output: {error}");
                            store.discard_capture(capture);
                        }
                    }
                }
                Ok(_) => store.discard_capture(capture),
                Err(error) => {
                    warn!(run_id, "Failed to inspect tool run output capture: {error}");
                    store.discard_capture(capture);
                }
            }
        }
        if final_content.is_empty() {
            return None;
        }
        match store.persist_content(run_id, final_content) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                warn!(run_id, "Failed to persist final tool run output: {error}");
                None
            }
        }
    }

    pub fn cancel(&self, run_id: &str) -> Result<ToolRunSnapshot, String> {
        self.cancel_with_policy(run_id, CancellationPolicy::Legacy)
            .map_err(|error| match error {
                ToolRunCancelError::NotFound => format!("Unknown tool run id: {run_id}"),
                other => format!("Unable to cancel tool run {run_id}: {other}"),
            })
    }

    /// Cancel only a running task backed by a live abort handle.
    ///
    /// Validation and the transition to `Cancelled` happen under the same
    /// registry lock. This prevents an operator request from reporting success
    /// for foreground work that Captain cannot stop or for a run that finished
    /// concurrently.
    pub fn cancel_cancellable(&self, run_id: &str) -> Result<ToolRunSnapshot, ToolRunCancelError> {
        self.cancel_with_policy(run_id, CancellationPolicy::ActiveCancellableOnly)
    }

    fn cancel_with_policy(
        &self,
        run_id: &str,
        policy: CancellationPolicy,
    ) -> Result<ToolRunSnapshot, ToolRunCancelError> {
        let (abort, capture, final_content) = {
            let mut state = self.state.lock().expect("tool run registry poisoned");
            let entry = state
                .runs
                .get_mut(run_id)
                .ok_or(ToolRunCancelError::NotFound)?;
            if entry.status.is_terminal() {
                return match policy {
                    CancellationPolicy::Legacy => Ok(entry.snapshot(SystemTime::now())),
                    CancellationPolicy::ActiveCancellableOnly => {
                        Err(ToolRunCancelError::NotActive {
                            status: entry.status,
                        })
                    }
                };
            }
            if policy == CancellationPolicy::ActiveCancellableOnly && !entry.is_cancellable() {
                return Err(ToolRunCancelError::NotCancellable);
            }
            entry.status = ToolRunStatus::Cancelled;
            entry.finished_at = Some(SystemTime::now());
            entry.is_error = Some(true);
            let mut content = entry.result.take().unwrap_or_default();
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str("[tool run cancelled by request]");
            let (retained_content, withheld) = sanitize_retained_result(content.clone());
            if withheld {
                warn!(
                    run_id,
                    "Cancelled tool run result was withheld during secret redaction"
                );
            }
            let (bounded, truncated) = bounded_result(retained_content);
            entry.result = Some(bounded);
            entry.result_truncated = truncated || withheld;
            let abort = entry.abort_handle.take();
            let capture = entry.output_capture.take();
            (abort, capture, content)
        };
        if let Some(abort) = abort {
            abort.abort();
        }
        let output = self.finalize_output(run_id, capture.as_ref(), &final_content);
        let cancelled_entry = {
            let mut state = self.state.lock().expect("tool run registry poisoned");
            let entry = state
                .runs
                .get_mut(run_id)
                .ok_or(ToolRunCancelError::NotFound)?;
            entry.output = output;
            entry.clone()
        };
        self.persist_finish(&cancelled_entry);
        self.snapshot(run_id).ok_or(ToolRunCancelError::NotFound)
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

    pub fn validate_retry_source(
        &self,
        run_id: &str,
        caller_agent_id: Option<&str>,
    ) -> Result<ToolRunSnapshot, String> {
        let state = self.state.lock().expect("tool run registry poisoned");
        let entry = state
            .runs
            .get(run_id)
            .ok_or_else(|| format!("Unknown tool run id: {run_id}"))?;
        if entry.status == ToolRunStatus::Running {
            return Err(format!(
                "Tool run {run_id} is still running; inspect or cancel it before retrying."
            ));
        }
        if entry.status == ToolRunStatus::Completed && entry.is_error != Some(true) {
            return Err(format!(
                "Tool run {run_id} completed successfully and cannot be retried. Start a new explicit run if repeating the effect is intentional."
            ));
        }
        if let Some(owner) = entry.caller_agent_id.as_deref() {
            if caller_agent_id != Some(owner) {
                return Err(format!(
                    "Tool run {run_id} belongs to another agent and cannot be retried from this context."
                ));
            }
        }

        if entry.retry_attempt as usize >= MAX_RETRY_ATTEMPTS {
            return Err(format!(
                "Tool run {run_id} reached the {MAX_RETRY_ATTEMPTS}-retry circuit breaker. Inspect the evidence and start a new deliberate run instead."
            ));
        }

        let mut depth = 0usize;
        let mut cursor = entry.retry_of_run_id.as_deref();
        let mut visited = std::collections::HashSet::new();
        visited.insert(entry.run_id.as_str());
        while let Some(parent_id) = cursor {
            if !visited.insert(parent_id) {
                return Err(format!(
                    "Tool run {run_id} has invalid cyclic retry lineage and cannot be retried."
                ));
            }
            depth += 1;
            if depth > entry.retry_attempt as usize {
                return Err(format!(
                    "Tool run {run_id} has inconsistent retry lineage and cannot be retried."
                ));
            }
            cursor = state
                .runs
                .get(parent_id)
                .and_then(|parent| parent.retry_of_run_id.as_deref());
        }
        Ok(entry.snapshot(SystemTime::now()))
    }

    pub fn result(&self, run_id: &str) -> Option<ToolRunResultSnapshot> {
        let now = SystemTime::now();
        self.state
            .lock()
            .expect("tool run registry poisoned")
            .runs
            .get(run_id)
            .map(|entry| {
                let (result, withheld) = entry.result.as_ref().map_or((None, false), |result| {
                    let (sanitized, withheld) = sanitize_retained_result(result.clone());
                    (Some(sanitized), withheld)
                });
                let mut snapshot = entry.snapshot(now);
                snapshot.result_truncated |= withheld;
                ToolRunResultSnapshot { snapshot, result }
            })
    }

    pub fn read_output(
        &self,
        run_id: &str,
        start_line: usize,
        max_lines: usize,
    ) -> Result<crate::tool_run_output::ToolRunOutputPage, String> {
        let (metadata, capture, result) = self.output_sources(run_id)?;
        let store = self.output_store.get();
        if let (Some(store), Some(metadata)) = (store, metadata.as_ref()) {
            return store
                .read_lines(metadata, start_line, max_lines)
                .map_err(|error| output_read_error(run_id, error));
        }
        if let (Some(store), Some(capture)) = (store, capture.as_ref()) {
            return store
                .read_capture_lines(capture, start_line, max_lines)
                .map_err(|error| output_read_error(run_id, error));
        }
        crate::tool_run_output::page_content(result.as_deref().unwrap_or(""), start_line, max_lines)
            .map_err(|error| output_read_error(run_id, error))
    }

    pub fn tail_output(
        &self,
        run_id: &str,
        max_lines: usize,
    ) -> Result<crate::tool_run_output::ToolRunOutputPage, String> {
        let (metadata, capture, result) = self.output_sources(run_id)?;
        let store = self.output_store.get();
        if let (Some(store), Some(metadata)) = (store, metadata.as_ref()) {
            return store
                .tail_lines(metadata, max_lines)
                .map_err(|error| output_read_error(run_id, error));
        }
        if let (Some(store), Some(capture)) = (store, capture.as_ref()) {
            return store
                .tail_capture_lines(capture, max_lines)
                .map_err(|error| output_read_error(run_id, error));
        }
        crate::tool_run_output::tail_content(result.as_deref().unwrap_or(""), max_lines)
            .map_err(|error| output_read_error(run_id, error))
    }

    pub fn search_output(
        &self,
        run_id: &str,
        query: &str,
        max_matches: usize,
        case_sensitive: bool,
    ) -> Result<Vec<crate::tool_run_output::ToolRunOutputMatch>, String> {
        let (metadata, capture, result) = self.output_sources(run_id)?;
        let store = self.output_store.get();
        if let (Some(store), Some(metadata)) = (store, metadata.as_ref()) {
            return store
                .search_lines(metadata, query, max_matches, case_sensitive)
                .map_err(|error| output_read_error(run_id, error));
        }
        if let (Some(store), Some(capture)) = (store, capture.as_ref()) {
            return store
                .search_capture_lines(capture, query, max_matches, case_sensitive)
                .map_err(|error| output_read_error(run_id, error));
        }
        crate::tool_run_output::search_content(
            result.as_deref().unwrap_or(""),
            query,
            max_matches,
            case_sensitive,
        )
        .map_err(|error| output_read_error(run_id, error))
    }

    fn output_sources(&self, run_id: &str) -> Result<ToolRunOutputSources, String> {
        let state = self.state.lock().expect("tool run registry poisoned");
        let entry = state
            .runs
            .get(run_id)
            .ok_or_else(|| format!("Unknown tool run id: {run_id}"))?;
        let result = entry
            .result
            .as_ref()
            .map(|result| sanitize_retained_result(result.clone()).0);
        Ok((entry.output.clone(), entry.output_capture.clone(), result))
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
        "cancellable": entry.is_cancellable(),
        "started_at_unix_ms": unix_ms(entry.started_at),
        "finished_at_unix_ms": entry.finished_at.map(unix_ms),
        "elapsed_ms": elapsed_ms(entry.started_at, entry.finished_at.unwrap_or(now)),
        "caller_agent_id": entry.caller_agent_id,
        "origin_tool_use_id": entry.origin_tool_use_id,
        "input_sha256": entry.input_sha256,
        "retry_of_run_id": entry.retry_of_run_id,
        "retry_attempt": entry.retry_attempt,
        "is_error": entry.is_error,
        "result_available": entry.result.is_some(),
        "result_truncated": entry.result_truncated,
        "output_available": entry.output.is_some() || entry.output_capture.is_some(),
        "output_stored_bytes": entry.output.as_ref().map(|output| output.stored_bytes),
        "output_total_bytes": entry.output.as_ref().map(|output| output.total_bytes),
        "output_sha256": entry.output.as_ref().map(|output| output.sha256.as_str()),
        "output_capped": entry.output.as_ref().is_some_and(|output| output.capped),
        "output_redacted": entry.output.as_ref().is_some_and(|output| output.redacted),
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
        let Some(position) = state.order.iter().position(|run_id| {
            state
                .runs
                .get(run_id)
                .is_none_or(|entry| entry.status.is_terminal())
        }) else {
            break;
        };
        let Some(run_id) = state.order.remove(position) else {
            break;
        };
        state.runs.remove(&run_id);
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

fn sanitize_retained_result(content: String) -> (String, bool) {
    match crate::tool_run_output::sanitize_for_retention(&content) {
        Ok((sanitized, _)) => (sanitized, false),
        Err(_) => (WITHHELD_RESULT.to_string(), true),
    }
}

fn format_stream_chunk(stream: &str, chunk: &str) -> String {
    if stream == "stdout" {
        chunk.to_string()
    } else {
        format!("--- {stream} ---\n{chunk}")
    }
}

pub fn input_digest(input: &serde_json::Value) -> String {
    let serialized = serde_json::to_vec(input).unwrap_or_default();
    format!("{:x}", Sha256::digest(serialized))
}

fn output_read_error(run_id: &str, error: std::io::Error) -> String {
    format!(
        "Unable to read output for tool run {run_id}: {error}. The retained evidence may have expired or failed integrity verification."
    )
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
    let output = match (
        record.output_file_name,
        record.output_stored_bytes,
        record.output_total_bytes,
        record.output_sha256,
    ) {
        (Some(file_name), Some(stored_bytes), Some(total_bytes), Some(sha256)) => {
            Some(crate::tool_run_output::ToolRunOutputMetadata {
                file_name,
                stored_bytes,
                total_bytes,
                sha256,
                capped: record.output_capped,
                redacted: record.output_redacted,
            })
        }
        _ => None,
    };
    let (result, result_was_withheld) = record
        .result
        .map(sanitize_retained_result)
        .map_or((None, false), |(result, withheld)| (Some(result), withheld));
    ToolRunEntry {
        run_id: record.run_id,
        tool_name: record.tool_name,
        status: ToolRunStatus::from_db_str(&record.status),
        detached: record.detached,
        started_at: system_time_from_unix_ms(record.started_at_unix_ms),
        finished_at: record.finished_at_unix_ms.map(system_time_from_unix_ms),
        caller_agent_id: record.caller_agent_id,
        origin_tool_use_id: record.origin_tool_use_id,
        input_sha256: record.input_sha256,
        retry_of_run_id: record.retry_of_run_id,
        retry_attempt: record.retry_attempt,
        is_error: record.is_error,
        result,
        result_truncated: record.result_truncated || result_was_withheld,
        output,
        output_capture: None,
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
        let run_id = registry.start(
            "shell_exec",
            Some("agent".into()),
            Some("tc1".into()),
            true,
            Some("input-digest".into()),
        );
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

        let run_id = registry.start("cargo", Some("agent-1".into()), None, true, None);
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
    fn foreground_runs_are_persisted_for_cross_surface_recovery() {
        let registry = ToolRunRegistry::default();
        let store = persistence_store();
        registry.configure_persistence(store.clone());

        let run_id = registry.start("file_read", None, None, false, Some("digest".into()));
        registry.finish(&run_id, &result("contents", false));

        let persisted = store.list_recent(10).unwrap();
        assert_eq!(persisted.len(), 1);
        assert!(!persisted[0].detached);
        assert_eq!(persisted[0].input_sha256.as_deref(), Some("digest"));
    }

    #[test]
    fn retry_lineage_is_persisted_and_hydrated() {
        let registry = ToolRunRegistry::default();
        let store = persistence_store();
        registry.configure_persistence(store.clone());

        let retry_id = registry.start_with_retry(
            "shell_exec",
            Some("captain".into()),
            None,
            true,
            Some("same-digest".into()),
            Some("toolrun-parent".into()),
        );
        registry.finish(&retry_id, &result("failed", true));

        let persisted = store.list_recent(1).unwrap();
        assert_eq!(
            persisted[0].retry_of_run_id.as_deref(),
            Some("toolrun-parent")
        );
        let restored = ToolRunRegistry::default();
        restored.hydrate_from_persisted(persisted);
        assert_eq!(
            restored
                .snapshot(&retry_id)
                .unwrap()
                .retry_of_run_id
                .as_deref(),
            Some("toolrun-parent")
        );
    }

    #[test]
    fn restart_reconciles_running_rows_and_hydrates_registry() {
        let store = persistence_store();
        // Simulate the previous process dying mid-run: a row left `running`
        // with no matching in-memory registry (the OnceLock static reset on
        // every process start).
        store
            .upsert_running(
                "toolrun-crashed",
                "ssh_exec",
                Some("agent-1"),
                None,
                true,
                None,
                1_000,
            )
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
            .upsert_running("toolrun-old", "cargo", None, None, true, None, 1_000)
            .unwrap();
        store
            .mark_finished(
                "toolrun-old",
                DetachedToolRunCompletion {
                    status: "completed",
                    is_error: Some(false),
                    result: Some("old"),
                    result_truncated: false,
                    output_file_name: None,
                    output_stored_bytes: None,
                    output_total_bytes: None,
                    output_sha256: None,
                    output_capped: false,
                    output_redacted: false,
                    finished_at_unix_ms: 1_100,
                },
            )
            .unwrap();
        store
            .upsert_running("toolrun-new", "cargo", None, None, false, None, 2_000)
            .unwrap();
        store
            .mark_finished(
                "toolrun-new",
                DetachedToolRunCompletion {
                    status: "completed",
                    is_error: Some(false),
                    result: Some("new"),
                    result_truncated: false,
                    output_file_name: None,
                    output_stored_bytes: None,
                    output_total_bytes: None,
                    output_sha256: None,
                    output_capped: false,
                    output_redacted: false,
                    finished_at_unix_ms: 2_100,
                },
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
        let run_id = registry.start("ssh_exec", None, None, false, None);
        registry.finish(&run_id, &result("boom", true));
        let snapshot = registry.snapshot(&run_id).unwrap();
        assert_eq!(snapshot.status, ToolRunStatus::Failed);
        assert_eq!(snapshot.is_error, Some(true));
    }

    #[test]
    fn retry_source_enforces_owner_state_and_chain_circuit_breaker() {
        let registry = ToolRunRegistry::default();
        let original = registry.start(
            "shell_exec",
            Some("captain".into()),
            None,
            true,
            Some("digest".into()),
        );
        assert!(registry
            .validate_retry_source(&original, Some("captain"))
            .is_err());
        registry.finish(&original, &result("failed", true));
        assert!(registry
            .validate_retry_source(&original, Some("other"))
            .is_err());
        assert!(registry
            .validate_retry_source(&original, Some("captain"))
            .is_ok());

        let mut parent = original;
        for _ in 0..MAX_RETRY_ATTEMPTS {
            let retry = registry.start_with_retry(
                "shell_exec",
                Some("captain".into()),
                None,
                true,
                Some("digest".into()),
                Some(parent.clone()),
            );
            registry.finish(&retry, &result("failed", true));
            parent = retry;
        }
        assert!(registry
            .validate_retry_source(&parent, Some("captain"))
            .unwrap_err()
            .contains("circuit breaker"));

        let completed = registry.start("shell_exec", None, None, true, Some("digest".into()));
        registry.finish(&completed, &result("ok", false));
        assert!(registry
            .validate_retry_source(&completed, None)
            .unwrap_err()
            .contains("completed successfully"));
    }

    #[test]
    fn registry_exposes_partial_chunks_while_running() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("shell_exec", None, None, true, None);
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
    fn durable_output_is_searchable_live_and_survives_registry_restart() {
        let dir = tempfile::tempdir().unwrap();
        let output_root = dir.path().join("tool-runs");
        let store = persistence_store();
        let registry = ToolRunRegistry::default();
        registry.configure_persistence(store.clone());
        registry.configure_output_store(
            crate::tool_run_output::ToolRunOutputStore::new(output_root.clone()).unwrap(),
        );

        let run_id = registry.start(
            "shell_exec",
            Some("captain".into()),
            Some("tool-use-live".into()),
            false,
            Some(input_digest(&serde_json::json!({"command": "large"}))),
        );
        for line in 0..7_000 {
            let suffix = if line == 6_500 {
                " password=never-expose-this-secret"
            } else {
                ""
            };
            registry.append_chunk(
                &run_id,
                "stdout",
                &format!("output-line-{line:05}{suffix}\n"),
            );
        }

        let live_tail = registry.tail_output(&run_id, 2).unwrap();
        assert!(live_tail.content.contains("output-line-06999"));
        let live_secret = registry
            .search_output(&run_id, "password", 5, false)
            .unwrap();
        assert_eq!(live_secret.len(), 1);
        assert!(live_secret[0].content.contains("[REDACTED]"));
        assert!(!live_secret[0].content.contains("never-expose"));

        registry.finish(&run_id, &result("bounded summary", false));
        let completed = registry.snapshot(&run_id).unwrap();
        assert!(completed.output_available);
        assert!(completed.output_redacted);
        assert!(completed.output_stored_bytes.unwrap() > MAX_RESULT_CHARS as u64);
        assert_eq!(completed.output_sha256.as_deref().unwrap().len(), 64);

        let persisted = store.list_recent(10).unwrap();
        assert_eq!(persisted[0].run_id, run_id);
        assert!(persisted[0].output_file_name.is_some());

        let restored = ToolRunRegistry::default();
        restored.configure_output_store(
            crate::tool_run_output::ToolRunOutputStore::new(output_root).unwrap(),
        );
        restored.hydrate_from_persisted(persisted);
        let matches = restored
            .search_output(&run_id, "output-line-06499", 5, true)
            .unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 6_500);
    }

    #[test]
    fn retained_results_are_redacted_in_memory_sqlite_and_verified_output() {
        let dir = tempfile::tempdir().unwrap();
        let store = persistence_store();
        let registry = ToolRunRegistry::default();
        registry.configure_persistence(store.clone());
        registry.configure_output_store(
            crate::tool_run_output::ToolRunOutputStore::new(dir.path().join("tool-runs")).unwrap(),
        );

        let run_id = registry.start("shell_exec", None, None, false, None);
        registry.append_chunk(
            &run_id,
            "stdout",
            "\u{1b}[31mpassword=live-secret-value\u{1b}[0m\n",
        );
        let running = registry.snapshot(&run_id).unwrap();
        let running_preview = running.result_preview.unwrap();
        assert!(running_preview.contains("password=[REDACTED]"));
        assert!(!running_preview.contains("live-secret-value"));

        registry.finish(&run_id, &result("api_key=final-secret-value", false));
        let retained = registry.result(&run_id).unwrap();
        assert_eq!(retained.result.as_deref(), Some("api_key=[REDACTED]"));
        assert!(retained.snapshot.output_available);
        assert!(retained.snapshot.output_redacted);
        assert_eq!(
            retained.snapshot.output_sha256.as_deref().unwrap().len(),
            64
        );

        let tail = registry.tail_output(&run_id, 10).unwrap();
        assert!(tail.content.contains("password=[REDACTED]"));
        assert!(!tail.content.contains("live-secret-value"));
        let persisted = store.list_recent(1).unwrap();
        assert_eq!(persisted[0].result.as_deref(), Some("api_key=[REDACTED]"));
        assert!(!persisted[0]
            .result
            .as_deref()
            .unwrap()
            .contains("final-secret-value"));
    }

    #[test]
    fn small_non_streamed_result_is_kept_as_verified_output() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ToolRunRegistry::default();
        registry.configure_output_store(
            crate::tool_run_output::ToolRunOutputStore::new(dir.path().join("tool-runs")).unwrap(),
        );
        let run_id = registry.start("file_read", None, None, false, None);
        registry.finish(&run_id, &result("short verified result", false));

        let snapshot = registry.snapshot(&run_id).unwrap();
        assert!(snapshot.output_available);
        assert_eq!(snapshot.output_sha256.as_deref().unwrap().len(), 64);
        assert_eq!(
            registry.tail_output(&run_id, 10).unwrap().content,
            "short verified result"
        );
    }

    #[test]
    fn hydration_redacts_and_rewrites_legacy_terminal_results() {
        let store = persistence_store();
        store
            .upsert_running(
                "toolrun-legacy-secret",
                "shell_exec",
                None,
                None,
                false,
                None,
                1_000,
            )
            .unwrap();
        store
            .mark_finished(
                "toolrun-legacy-secret",
                DetachedToolRunCompletion {
                    status: "completed",
                    is_error: Some(false),
                    result: Some("password=legacy-secret-value"),
                    result_truncated: false,
                    output_file_name: None,
                    output_stored_bytes: None,
                    output_total_bytes: None,
                    output_sha256: None,
                    output_capped: false,
                    output_redacted: false,
                    finished_at_unix_ms: 1_100,
                },
            )
            .unwrap();

        let registry = ToolRunRegistry::default();
        registry.configure_persistence(store.clone());
        registry.hydrate_from_persisted(store.list_recent(10).unwrap());

        assert_eq!(
            registry
                .result("toolrun-legacy-secret")
                .unwrap()
                .result
                .as_deref(),
            Some("password=[REDACTED]")
        );
        assert_eq!(
            store.list_recent(1).unwrap()[0].result.as_deref(),
            Some("password=[REDACTED]")
        );
    }

    #[test]
    fn cancellation_preserves_partial_output_instead_of_erasing_it() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("shell_exec", None, None, true, None);
        registry.append_chunk(&run_id, "stdout", "useful partial line\n");

        let cancelled = registry.cancel(&run_id).unwrap();

        assert_eq!(cancelled.status, ToolRunStatus::Cancelled);
        let result = registry.result(&run_id).unwrap().result.unwrap();
        assert!(result.contains("useful partial line"));
        assert!(result.contains("cancelled by request"));
    }

    #[test]
    fn strict_cancellation_rejects_foreground_run_without_changing_it() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("shell_exec", None, None, false, None);

        assert_eq!(
            registry.cancel_cancellable(&run_id).unwrap_err(),
            ToolRunCancelError::NotCancellable
        );
        assert_eq!(
            registry.snapshot(&run_id).unwrap().status,
            ToolRunStatus::Running
        );
    }

    #[test]
    fn strict_cancellation_rejects_terminal_run() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("shell_exec", None, None, true, None);
        registry.finish(&run_id, &result("done", false));

        assert_eq!(
            registry.cancel_cancellable(&run_id).unwrap_err(),
            ToolRunCancelError::NotActive {
                status: ToolRunStatus::Completed,
            }
        );
    }

    #[tokio::test]
    async fn strict_cancellation_aborts_live_task_and_retains_partial_output() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("shell_exec", None, None, true, None);
        registry.append_chunk(&run_id, "stdout", "useful partial evidence\n");
        let task = tokio::spawn(std::future::pending::<()>());
        registry.attach_abort_handle(&run_id, task.abort_handle());

        let cancelled = registry.cancel_cancellable(&run_id).unwrap();

        assert_eq!(cancelled.status, ToolRunStatus::Cancelled);
        assert!(task.await.unwrap_err().is_cancelled());
        let retained = registry.result(&run_id).unwrap().result.unwrap();
        assert!(retained.contains("useful partial evidence"));
        assert!(retained.contains("cancelled by request"));
    }

    #[tokio::test]
    async fn strict_cancellation_rejects_finished_abort_handle() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("shell_exec", None, None, true, None);
        let task = tokio::spawn(async {});
        registry.attach_abort_handle(&run_id, task.abort_handle());
        task.await.unwrap();

        assert_eq!(
            registry.cancel_cancellable(&run_id).unwrap_err(),
            ToolRunCancelError::NotCancellable
        );
        assert!(!registry.snapshot(&run_id).unwrap().cancellable);
    }

    #[test]
    fn completed_small_stream_remains_inspectable() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ToolRunRegistry::default();
        registry.configure_output_store(
            crate::tool_run_output::ToolRunOutputStore::new(dir.path().join("tool-runs")).unwrap(),
        );
        let run_id = registry.start("shell_exec", None, None, false, None);
        registry.append_chunk(&run_id, "stdout", "small streamed evidence\n");

        assert!(registry.snapshot(&run_id).unwrap().output_available);
        registry.finish(&run_id, &result("short summary", false));

        let snapshot = registry.snapshot(&run_id).unwrap();
        assert!(snapshot.output_available);
        assert_eq!(
            registry.read_output(&run_id, 1, 10).unwrap().content,
            "small streamed evidence"
        );
    }

    #[test]
    fn pruning_skips_old_active_run_and_still_bounds_terminal_history() {
        let registry = ToolRunRegistry::default();
        let active = registry.start("shell_exec", None, None, true, None);
        for index in 0..MAX_RUNS {
            let run_id = registry.start(format!("tool-{index}"), None, None, false, None);
            registry.finish(&run_id, &result("done", false));
        }

        let state = registry.state.lock().unwrap();
        assert!(state.runs.contains_key(&active));
        assert_eq!(state.runs.len(), MAX_RUNS);
        assert_eq!(state.order.len(), MAX_RUNS);
    }

    #[test]
    fn status_summary_omits_result_preview() {
        let registry = ToolRunRegistry::default();
        let run_id = registry.start("memory_recall", None, Some("tc-memory".into()), false, None);
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

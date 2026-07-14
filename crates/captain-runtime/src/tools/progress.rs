//! Task-local tool progress and origin-channel context.

use std::cell::RefCell;
use std::future::Future;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// One progress tick from inside a long-running tool.
#[derive(Debug, Clone)]
pub struct ToolProgressEvent {
    /// Tool-use id this progress belongs to.
    pub tool_use_id: String,
    /// Human-readable progress note for the active channel.
    pub message: String,
    /// Completed frame index for batched media tools.
    pub frame_index: Option<usize>,
    /// Total frame count for batched media tools.
    pub frames_total: Option<usize>,
}

tokio::task_local! {
    static PROGRESS_SINK: RefCell<Option<mpsc::Sender<ToolProgressEvent>>>;
    static ORIGIN_CHANNEL: Option<String>;
}

/// Run `fut` with a progress sink installed in task-local storage.
pub async fn with_progress_sink<F, T>(sender: mpsc::Sender<ToolProgressEvent>, fut: F) -> T
where
    F: Future<Output = T>,
{
    PROGRESS_SINK.scope(RefCell::new(Some(sender)), fut).await
}

/// Get the current task-local progress sender, if a dispatch installed one.
pub fn progress_sink() -> Option<mpsc::Sender<ToolProgressEvent>> {
    PROGRESS_SINK
        .try_with(|cell| cell.borrow().clone())
        .ok()
        .flatten()
}

/// Run a tool dispatch with the current user-facing channel installed.
pub async fn with_origin_channel<F, T>(channel: Option<String>, fut: F) -> T
where
    F: Future<Output = T>,
{
    ORIGIN_CHANNEL.scope(channel, fut).await
}

/// Current origin channel for this tool dispatch, if any.
pub fn current_origin_channel() -> Option<String> {
    ORIGIN_CHANNEL.try_with(Clone::clone).ok().flatten()
}

/// Best-effort progress emit. Never blocks the tool execution path.
pub(crate) fn emit_progress(ev: ToolProgressEvent) {
    if let Some(tx) = progress_sink() {
        let _ = tx.try_send(ev);
    }
}

/// Throttle helper for progress emission.
#[derive(Debug)]
pub struct ProgressThrottle {
    last_emit: Option<Instant>,
    min_interval: Duration,
}

impl ProgressThrottle {
    pub fn new(min_interval: Duration) -> Self {
        Self {
            last_emit: None,
            min_interval,
        }
    }

    /// Returns true when enough time has elapsed since the last emission.
    pub fn ready(&mut self, now: Instant) -> bool {
        match self.last_emit {
            None => {
                self.last_emit = Some(now);
                true
            }
            Some(prev) if now.duration_since(prev) >= self.min_interval => {
                self.last_emit = Some(now);
                true
            }
            _ => false,
        }
    }
}

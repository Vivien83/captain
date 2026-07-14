//! Task-local execution context for tool dispatch.

use std::cell::Cell;
use std::future::Future;

/// Maximum inter-agent call depth to prevent infinite recursion.
pub(crate) const MAX_AGENT_CALL_DEPTH: u32 = 5;

tokio::task_local! {
    /// Tracks the current inter-agent call depth within a task.
    pub(crate) static AGENT_CALL_DEPTH: Cell<u32>;
    /// Tracks parent/child lineage depth for sub-agent tool hardening.
    pub(crate) static AGENT_LINEAGE_DEPTH: u32;
    /// Canvas max HTML size in bytes, set from kernel config at loop start.
    pub static CANVAS_MAX_BYTES: usize;
}

/// Get the current inter-agent call depth from the task-local context.
pub fn current_agent_depth() -> u32 {
    AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0)
}

/// Run a tool dispatch with the agent lineage depth installed.
pub async fn with_agent_lineage_depth<F, T>(depth: u32, fut: F) -> T
where
    F: Future<Output = T>,
{
    AGENT_LINEAGE_DEPTH.scope(depth, fut).await
}

pub(crate) fn current_agent_lineage_depth() -> u32 {
    AGENT_LINEAGE_DEPTH.try_with(|d| *d).unwrap_or(0)
}

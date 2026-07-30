//! R.2.1 — Goal-driven autopilot execution loop.
//!
//! For every active [`captain-kernel::goals::Goal`] we spawn a tokio task
//! that ticks at the goal's `interval_secs`, runs the `check_command`,
//! attempts `recovery_command` on a single failure, and finally escalates
//! to the user via `channel_send` after `escalation_threshold` consecutive
//! failures.
//!
//! Decoupling: rather than depend on the full `KernelHandle` trait (40+
//! methods) we define [`GoalLoopOps`], the narrow set of operations the
//! loop actually needs. Production wires it through [`KernelOps`], an
//! adapter that delegates to a `KernelHandle`. Tests can supply a
//! lightweight stub.
//!
//! Safety:
//! * Every tick revalidates the live command through `guarded_exec`; editing
//!   or restoring a goal cannot bypass critical patterns or execution policy.
//! * Child processes receive a scrubbed environment and cannot inherit the
//!   daemon's provider keys or `secrets.env` values.
//! * Each tick re-reads the goal via `goal_status` so a `goal_pause`
//!   takes effect at the very next tick.
//! * If `goal_status` returns NotFound, the loop exits cleanly.
//! * A check can opt into convergence monitoring by printing
//!   `CAPTAIN_PROGRESS=<token>` or JSON `{"captain_progress":"<token>"}`.
//!   Repeating the same token is treated as non-progress and escalates
//!   through the same stopped `Escalated` path as failed checks.
//! * Shell exec is hard-wrapped in a 60s timeout to prevent hung loops
//!   from blocking the executor.

use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tracing::{debug, info, warn};

use crate::kernel_handle::KernelHandle;

/// Hard timeout for a single `check_command` / `recovery_command` exec.
/// Anything longer is treated as failure to avoid hung loops.
const SHELL_TIMEOUT: Duration = Duration::from_secs(60);

/// Narrow contract the goal loop relies on. Implemented by [`KernelOps`]
/// in production and by a stub in tests.
#[async_trait]
pub trait GoalLoopOps: Send + Sync {
    /// JSON array of all goals (status + interval_secs + …) — used at boot.
    fn goal_list(&self) -> Result<String, String>;
    /// JSON of one goal's current state — used at every tick to honor
    /// pause/delete/threshold updates.
    fn goal_status(&self, id: &str) -> Result<String, String>;
    /// Workspace path for a project-scoped goal. Global goals return `None`
    /// and run from the daemon working directory.
    fn goal_workspace_path(&self, project_slug: Option<&str>) -> Option<String> {
        let _ = project_slug;
        None
    }
    /// Record a check result; returns the live `consecutive_fails` counter.
    fn goal_record_check(
        &self,
        id: &str,
        ok: bool,
        output: &str,
        latency_ms: u64,
    ) -> Result<u32, String>;
    /// Atomically mark the goal as Escalated.
    fn goal_mark_escalated(&self, id: &str) -> Result<bool, String>;
    /// Send the escalation message via the named channel adapter.
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
    ) -> Result<String, String>;
}

/// Production adapter: wraps a `KernelHandle` so the goal loop can
/// stay decoupled from the full kernel surface.
pub struct KernelOps {
    pub kh: Arc<dyn KernelHandle>,
}

#[async_trait]
impl GoalLoopOps for KernelOps {
    fn goal_list(&self) -> Result<String, String> {
        self.kh.goal_list()
    }
    fn goal_status(&self, id: &str) -> Result<String, String> {
        self.kh.goal_status(id)
    }
    fn goal_workspace_path(&self, project_slug: Option<&str>) -> Option<String> {
        let slug = project_slug?;
        let project = self.kh.project_find_by_slug(slug).ok().flatten()?;
        project_workspace_path(&project)
    }
    fn goal_record_check(
        &self,
        id: &str,
        ok: bool,
        output: &str,
        latency_ms: u64,
    ) -> Result<u32, String> {
        self.kh.goal_record_check(id, ok, output, latency_ms)
    }
    fn goal_mark_escalated(&self, id: &str) -> Result<bool, String> {
        self.kh.goal_mark_escalated(id)
    }
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
    ) -> Result<String, String> {
        self.kh
            .send_channel_message(channel, recipient, message, None)
            .await
    }
}

/// Spawn the loop for a single freshly-created goal. Used by
/// `tool_goal_create` so the user doesn't have to restart the daemon
/// to see the goal go live. Caller must ensure no other loop is
/// already running for `id` (the boot path already spawned every
/// previously-active goal — new goals are by definition not in that
/// set).
pub fn spawn_goal_loop_for(id: String, ops: Arc<dyn GoalLoopOps>) {
    tokio::spawn(async move {
        run_goal_loop(id, ops).await;
    });
}

/// Boot-time entry point — read every Active goal from the store and
/// spawn its loop. Idempotent in the sense that calling it twice would
/// double the loops; callers (server.rs run_daemon) must call it once.
pub fn spawn_goal_loops(ops: Arc<dyn GoalLoopOps>) {
    let raw = match ops.goal_list() {
        Ok(j) => j,
        Err(e) => {
            warn!("goal_loop: cannot list goals at boot: {e}");
            return;
        }
    };
    let goals: Vec<serde_json::Value> = serde_json::from_str(&raw).unwrap_or_default();
    let mut spawned = 0usize;
    for g in goals {
        let status = g.get("status").and_then(|s| s.as_str()).unwrap_or("");
        if status != "active" {
            continue;
        }
        let id = g
            .get("id")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let ops = ops.clone();
        tokio::spawn(async move {
            run_goal_loop(id, ops).await;
        });
        spawned += 1;
    }
    if spawned > 0 {
        info!(spawned, "Goal loops spawned at boot");
    }
}

/// Per-goal loop. Owns no state — re-reads from the kernel at every
/// tick to honor `goal_pause` / `goal_delete`.
async fn run_goal_loop(id: String, ops: Arc<dyn GoalLoopOps>) {
    let initial = match ops.goal_status(&id) {
        Ok(j) => j,
        Err(e) => {
            warn!(goal = %id, "goal disappeared before loop start: {e}");
            return;
        }
    };
    let initial: serde_json::Value = serde_json::from_str(&initial).unwrap_or_default();
    let interval_secs = initial
        .get("interval_secs")
        .and_then(|x| x.as_u64())
        .unwrap_or(300)
        .max(10);
    info!(goal = %id, interval_secs, "Goal loop started");

    let mut tick = interval(Duration::from_secs(interval_secs));
    // Drop the immediate-fire tick so the first check happens after one
    // interval (gives the daemon a moment to settle).
    tick.tick().await;

    loop {
        tick.tick().await;

        let snapshot = match ops.goal_status(&id) {
            Ok(j) => j,
            Err(_) => {
                info!(goal = %id, "Goal removed, exiting loop");
                return;
            }
        };
        let g: serde_json::Value = match serde_json::from_str(&snapshot) {
            Ok(v) => v,
            Err(e) => {
                warn!(goal = %id, "snapshot parse error: {e}");
                continue;
            }
        };

        let status = g.get("status").and_then(|s| s.as_str()).unwrap_or("");
        if status == "paused" {
            debug!(goal = %id, "paused — skipping tick");
            continue;
        }
        if status == "escalated" {
            debug!(goal = %id, "already escalated — skipping tick");
            continue;
        }

        let check_command = match g.get("check_command").and_then(|s| s.as_str()) {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => {
                warn!(goal = %id, "check_command empty — skipping tick");
                continue;
            }
        };
        let recovery_command = g
            .get("recovery_command")
            .and_then(|s| s.as_str())
            .map(String::from);
        let threshold = g
            .get("escalation_threshold")
            .and_then(|x| x.as_u64())
            .unwrap_or(3) as u32;
        let escalation_channel = g.get("escalation_channel").cloned();
        let goal_name = g
            .get("name")
            .and_then(|s| s.as_str())
            .unwrap_or(id.as_str())
            .to_string();
        let project_slug = g
            .get("project_slug")
            .and_then(|s| s.as_str())
            .map(str::to_string);
        let workspace_path = ops.goal_workspace_path(project_slug.as_deref());
        let previous_progress_signature = latest_progress_signature(&g);

        tick_once(
            &id,
            &goal_name,
            &check_command,
            recovery_command.as_deref(),
            threshold,
            escalation_channel.as_ref(),
            workspace_path.as_deref(),
            previous_progress_signature.as_deref(),
            ops.clone(),
        )
        .await;
    }
}

/// Run one check + maybe recovery + maybe escalation. Extracted so it
/// can be unit-tested with a mock GoalLoopOps.
#[allow(clippy::too_many_arguments)]
pub async fn tick_once(
    id: &str,
    goal_name: &str,
    check_command: &str,
    recovery_command: Option<&str>,
    escalation_threshold: u32,
    escalation_channel: Option<&serde_json::Value>,
    working_dir: Option<&str>,
    previous_progress_signature: Option<&str>,
    ops: Arc<dyn GoalLoopOps>,
) {
    let execution = execute_goal_check(
        check_command,
        recovery_command,
        working_dir,
        previous_progress_signature,
    )
    .await;
    debug!(
        goal = %id,
        ok = execution.ok,
        latency_ms = execution.latency_ms,
        recovery_attempted = execution.recovery_attempted,
        "check executed"
    );

    let consecutive_fails =
        match ops.goal_record_check(id, execution.ok, &execution.output, execution.latency_ms) {
            Ok(n) => n,
            Err(e) => {
                warn!(goal = %id, "goal_record_check failed: {e}");
                return;
            }
        };

    if !execution.ok && consecutive_fails >= escalation_threshold {
        escalate(
            id,
            goal_name,
            consecutive_fails,
            escalation_channel,
            execution.escalation_reason.as_deref(),
            ops,
        )
        .await;
    }
}

/// One bounded execution of a goal's check contract. Project verification
/// reuses this primitive so periodic monitoring and completion evidence cannot
/// drift on timeout, workspace, recovery, or convergence semantics.
#[derive(Debug, Clone)]
pub struct GoalCheckExecution {
    pub ok: bool,
    pub output: String,
    pub latency_ms: u64,
    pub recovery_attempted: bool,
    pub escalation_reason: Option<String>,
}

pub async fn execute_goal_check(
    check_command: &str,
    recovery_command: Option<&str>,
    working_dir: Option<&str>,
    previous_progress_signature: Option<&str>,
) -> GoalCheckExecution {
    let (ok, output, latency_ms) = run_shell_with_timeout(
        check_command,
        working_dir,
        crate::guarded_exec::ExecSurface::GoalCheck,
    )
    .await;

    // If the first check fails and recovery is configured, try it once, then
    // record only the recheck as the final result.
    let (final_ok, final_output, final_latency, recovery_attempted) = match (ok, recovery_command) {
        (false, Some(rec_cmd)) => {
            let (rec_ok, rec_out, _rec_lat) = run_shell_with_timeout(
                rec_cmd,
                working_dir,
                crate::guarded_exec::ExecSurface::GoalRecovery,
            )
            .await;
            if !rec_ok {
                warn!("goal recovery command failed: {}", truncate(&rec_out, 200));
            }
            let (post_ok, post_out, post_lat) = run_shell_with_timeout(
                check_command,
                working_dir,
                crate::guarded_exec::ExecSurface::GoalCheck,
            )
            .await;
            let combined = format!(
                "[recovery {}] {} | [recheck {}] {}",
                if rec_ok { "ok" } else { "fail" },
                truncate(&rec_out, 1000),
                if post_ok { "ok" } else { "fail" },
                truncate(&post_out, 1000)
            );
            (post_ok, combined, post_lat, true)
        }
        _ => (ok, output, latency_ms, false),
    };

    let progress = classify_progress_result(final_ok, final_output, previous_progress_signature);
    GoalCheckExecution {
        ok: progress.ok,
        output: progress.output,
        latency_ms: final_latency,
        recovery_attempted,
        escalation_reason: progress.escalation_reason,
    }
}

async fn escalate(
    id: &str,
    goal_name: &str,
    consecutive_fails: u32,
    escalation_channel: Option<&serde_json::Value>,
    reason: Option<&str>,
    ops: Arc<dyn GoalLoopOps>,
) {
    let mut message = format!(
        "🚨 Goal '{goal_name}' (id={id}) escalated: {consecutive_fails} consecutive failures."
    );
    if let Some(reason) = reason.filter(|value| !value.trim().is_empty()) {
        message.push_str(" Reason: ");
        message.push_str(&truncate(reason, 240));
        message.push('.');
    }
    if let Some(ec) = escalation_channel {
        let channel = ec.get("channel").and_then(|s| s.as_str()).unwrap_or("");
        let recipient = ec.get("recipient").and_then(|s| s.as_str()).unwrap_or("");
        if !channel.is_empty() && !recipient.is_empty() {
            match ops.send_channel_message(channel, recipient, &message).await {
                Ok(_) => info!(goal = %id, channel, "escalation sent"),
                Err(e) => warn!(goal = %id, "escalation send failed: {e}"),
            }
        } else {
            warn!(goal = %id, "escalation_channel missing 'channel' or 'recipient'");
        }
    } else {
        warn!(
            goal = %id,
            "no escalation_channel configured — only flipping status to Escalated"
        );
    }

    // Flip status regardless of send outcome so the loop stops re-firing
    // escalations on every subsequent failed tick.
    if let Err(e) = ops.goal_mark_escalated(id) {
        warn!(goal = %id, "goal_mark_escalated failed: {e}");
    }
}

struct ProgressClassification {
    ok: bool,
    output: String,
    escalation_reason: Option<String>,
}

fn classify_progress_result(
    ok: bool,
    output: String,
    previous_progress_signature: Option<&str>,
) -> ProgressClassification {
    if !ok {
        return ProgressClassification {
            ok,
            output,
            escalation_reason: None,
        };
    }

    let Some(current) = goal_progress_signature(&output) else {
        return ProgressClassification {
            ok,
            output,
            escalation_reason: None,
        };
    };
    let Some(previous) = previous_progress_signature
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return ProgressClassification {
            ok,
            output,
            escalation_reason: None,
        };
    };

    if current != previous {
        return ProgressClassification {
            ok,
            output,
            escalation_reason: None,
        };
    }

    let reason = format!(
        "no progress detected; progress marker stayed at '{}'",
        truncate(&current, 120)
    );
    let mut output = output;
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str("[Captain non-progress] ");
    output.push_str(&reason);
    ProgressClassification {
        ok: false,
        output,
        escalation_reason: Some(reason),
    }
}

fn latest_progress_signature(goal_snapshot: &serde_json::Value) -> Option<String> {
    goal_snapshot
        .get("recent_checks")
        .and_then(|value| value.as_array())
        .and_then(|checks| {
            checks.iter().rev().find_map(|check| {
                check
                    .get("output")
                    .and_then(|value| value.as_str())
                    .and_then(goal_progress_signature)
            })
        })
}

pub fn goal_progress_signature(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        for prefix in ["CAPTAIN_PROGRESS=", "captain_progress="] {
            if let Some(value) = trimmed.strip_prefix(prefix) {
                let value = value.trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }

    let value: serde_json::Value = serde_json::from_str(output.trim()).ok()?;
    progress_value_to_string(value.get("captain_progress")?)
}

fn progress_value_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.trim().to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
    .filter(|value| !value.is_empty())
}

/// Run a shell command through the shared guarded boundary. Returns
/// `(ok, output, latency_ms)`.
async fn run_shell_with_timeout(
    cmd: &str,
    working_dir: Option<&str>,
    surface: crate::guarded_exec::ExecSurface,
) -> (bool, String, u64) {
    let started = std::time::Instant::now();
    let workspace = working_dir
        .filter(|dir| !dir.trim().is_empty())
        .map(std::path::Path::new);
    match crate::guarded_exec::run_unattended_shell(crate::guarded_exec::ShellExecRequest {
        surface,
        command: cmd,
        policy: None,
        workspace,
        allowed_env_vars: &[],
        explicit_env: &[],
        timeout_secs: SHELL_TIMEOUT.as_secs(),
        no_output_timeout_secs: Some(SHELL_TIMEOUT.as_secs()),
        bash_required: false,
    })
    .await
    {
        Ok(outcome) => {
            let success = outcome.success();
            let mut combined = outcome.stdout;
            if !outcome.stderr.is_empty() {
                combined.push_str("\n[stderr] ");
                combined.push_str(&outcome.stderr);
            }
            (success, combined, outcome.elapsed_ms)
        }
        Err(error) => (false, error, started.elapsed().as_millis() as u64),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

fn project_workspace_path(project: &serde_json::Value) -> Option<String> {
    [
        "/metadata/launch/workspace/path",
        "/metadata/workspace/path",
        "/workspace_path",
        "/metadata/launch/source/local_path",
        "/metadata/source/local_path",
        "/metadata/launch/repo_path",
    ]
    .iter()
    .find_map(|pointer| {
        project
            .pointer(pointer)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

#[cfg(test)]
#[path = "goal_loop_tests.rs"]
mod tests;

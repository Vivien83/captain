use crate::llm_driver::StreamEvent;
use crate::tool_runner;
use captain_types::tool::{ToolCall, ToolResult};
use std::future::Future;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::warn;

/// Timeout for individual tool executions (seconds).
/// Raised from 60s to 120s for browser automation and long-running builds.
const TOOL_TIMEOUT_SECS: u64 = 120;

/// Minimum delay between semantic mid-tool progress ticks forwarded to UX
/// streams. Raw stdout/stderr still streams separately through `TOOL_STREAM`.
const TOOL_PROGRESS_STREAM_INTERVAL_SECS: u64 = 5;

/// Whether a tool is exec-style (live terminal output in TUI/channel
/// streaming bubbles).
pub fn is_exec_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "shell_exec"
            | "shell_background"
            | "execute_code"
            | "docker_exec"
            | "docker_build"
            | "docker_run"
            | "process_start"
            | "process_write"
    )
}

pub(crate) fn spawn_tool_progress_forwarder(
    mut progress_rx: mpsc::Receiver<tool_runner::ToolProgressEvent>,
    stream_tx: mpsc::Sender<StreamEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut throttle = tool_runner::ProgressThrottle::new(Duration::from_secs(
            TOOL_PROGRESS_STREAM_INTERVAL_SECS,
        ));

        while let Some(progress) = progress_rx.recv().await {
            if !throttle.ready(std::time::Instant::now()) {
                continue;
            }

            let chunk = format_tool_progress_chunk(&progress);
            if chunk.trim().is_empty() {
                continue;
            }

            if stream_tx
                .send(StreamEvent::ToolOutputDelta {
                    tool_use_id: progress.tool_use_id,
                    stream: "progress",
                    chunk,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    })
}

fn format_tool_progress_chunk(progress: &tool_runner::ToolProgressEvent) -> String {
    let message = progress.message.trim();
    if !message.is_empty() {
        return format!("{message}\n");
    }

    match (progress.frame_index, progress.frames_total) {
        (Some(frame_index), Some(total)) if total > 0 => {
            format!("Progression: {}/{}\n", frame_index + 1, total)
        }
        _ => String::new(),
    }
}

fn explicit_tool_timeout_secs(
    tool_name: &str,
    input: &serde_json::Value,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Option<u64> {
    match tool_name {
        "shell_exec" => input["timeout_seconds"]
            .as_u64()
            .or_else(|| exec_policy.map(|p| p.timeout_secs)),
        "cargo" | "npm" | "pip" => input["timeout_seconds"].as_u64(),
        _ => input["timeout_secs"].as_u64(),
    }
    .filter(|secs| *secs > 0)
}

pub(crate) fn tool_timeout_guard_secs(
    tool_name: &str,
    input: &serde_json::Value,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Option<u64> {
    if explicit_tool_timeout_secs(tool_name, input, exec_policy).is_some() {
        None
    } else {
        Some(TOOL_TIMEOUT_SECS)
    }
}

pub(crate) fn tool_timeout_result(
    tool_call: &ToolCall,
    timeout_secs: u64,
    streaming: bool,
) -> ToolResult {
    if streaming {
        warn!(tool = %tool_call.name, "Tool execution timed out after {}s (streaming)", timeout_secs);
    } else {
        warn!(tool = %tool_call.name, "Tool execution timed out after {}s", timeout_secs);
    }

    ToolResult {
        tool_use_id: tool_call.id.clone(),
        content: format!(
            "Tool '{}' timed out after {}s.",
            tool_call.name, timeout_secs
        ),
        is_error: true,
    }
}

pub(crate) async fn run_tool_with_timeout_guard<F>(
    tool_call: &ToolCall,
    timeout_guard_secs: Option<u64>,
    streaming: bool,
    exec_fut: F,
) -> ToolResult
where
    F: Future<Output = ToolResult>,
{
    if let Some(timeout_secs) = timeout_guard_secs {
        match tokio::time::timeout(Duration::from_secs(timeout_secs), exec_fut).await {
            Ok(result) => result,
            Err(_) => tool_timeout_result(tool_call, timeout_secs, streaming),
        }
    } else {
        exec_fut.await
    }
}

#[cfg(test)]
#[path = "agent_loop_tool_runtime_tests.rs"]
mod tests;

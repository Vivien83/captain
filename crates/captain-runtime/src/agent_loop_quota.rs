use crate::agent_loop_result::AgentLoopResult;
use crate::agent_loop_stream_events::send_phase_change_event;
use crate::agent_loop_tool_record::ToolCallRecord;
use crate::kernel_handle::KernelHandle;
use crate::llm_driver::StreamEvent;
use captain_types::agent::AgentManifest;
use captain_types::message::TokenUsage;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;

pub(crate) fn check_mid_loop_quota(
    manifest: &AgentManifest,
    kernel: Option<&Arc<dyn KernelHandle>>,
    iteration: u32,
    total_usage: &TokenUsage,
    tool_calls_recorded: &[ToolCallRecord],
) -> Option<AgentLoopResult> {
    if iteration == 0 {
        return None;
    }
    let kh = kernel?;
    let Err(e) = kh.check_agent_quota(&manifest.name) else {
        return None;
    };

    warn!(agent = %manifest.name, iteration, "Quota exceeded mid-loop: {e}");
    let message = quota_exceeded_message(&manifest.name, iteration, &e);
    Some(AgentLoopResult {
        response: format!("[{message}]"),
        total_usage: *total_usage,
        iterations: iteration,
        cost_usd: None,
        silent: false,
        directives: Default::default(),
        tool_calls: tool_calls_recorded.to_vec(),
    })
}

pub(crate) async fn streaming_quota_should_break(
    manifest: &AgentManifest,
    kernel: Option<&Arc<dyn KernelHandle>>,
    iteration: u32,
    stream_tx: &mpsc::Sender<StreamEvent>,
) -> bool {
    if iteration == 0 {
        return false;
    }
    let Some(kh) = kernel else {
        return false;
    };
    let Err(e) = kh.check_agent_quota(&manifest.name) else {
        return false;
    };

    warn!(agent = %manifest.name, iteration, "Quota exceeded mid-loop (streaming): {e}");
    send_phase_change_event(
        stream_tx,
        "quota_exceeded",
        Some(quota_exceeded_message(&manifest.name, iteration, &e)),
        None,
    )
    .await;
    true
}

fn quota_exceeded_message(agent_name: &str, iteration: u32, error: &str) -> String {
    format!(
        "Quota exceeded after {iteration} iterations: {error}. Run `captain agent caps {agent_name}` to inspect live budget and capabilities before retrying."
    )
}

#[cfg(test)]
#[path = "agent_loop_quota_tests.rs"]
mod tests;

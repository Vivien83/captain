use crate::agent_loop_stream_events::send_tool_execution_result_event;
use crate::agent_loop_tool_flow::expand_visible_tools_after_discovery_result;
use crate::agent_loop_tool_record::{record_completed_tool_call, ToolCallRecord};
use crate::agent_loop_tool_results::{prepare_tool_result_content, push_tool_result_block};
use crate::context_budget::ContextBudget;
use crate::kernel_handle::KernelHandle;
use crate::llm_driver::StreamEvent;
use crate::loop_guard::LoopGuardVerdict;
use captain_types::agent::AgentManifest;
use captain_types::message::ContentBlock;
use captain_types::tool::{ToolCall, ToolDefinition, ToolResult};
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) struct FinishToolCallInput<'a> {
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) tool_call: &'a ToolCall,
    pub(crate) result: ToolResult,
    pub(crate) verdict: &'a LoopGuardVerdict,
    pub(crate) context_budget: &'a ContextBudget,
    pub(crate) available_tools: &'a [ToolDefinition],
    pub(crate) visible_tools: &'a mut Vec<ToolDefinition>,
    pub(crate) tool_calls_recorded: &'a mut Vec<ToolCallRecord>,
    pub(crate) tool_result_blocks: &'a mut Vec<ContentBlock>,
    pub(crate) kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) caller_id_str: &'a str,
    pub(crate) tool_elapsed_ms: u64,
    pub(crate) streaming: bool,
    pub(crate) stream_tx: Option<&'a mpsc::Sender<StreamEvent>>,
}

pub(crate) async fn finish_tool_call(input: FinishToolCallInput<'_>) {
    let FinishToolCallInput {
        manifest,
        tool_call,
        mut result,
        verdict,
        context_budget,
        available_tools,
        visible_tools,
        tool_calls_recorded,
        tool_result_blocks,
        kernel,
        hooks,
        caller_id_str,
        tool_elapsed_ms,
        streaming,
        stream_tx,
    } = input;

    let transient_content = std::mem::take(&mut result.transient_content);

    record_completed_tool_call(
        tool_calls_recorded,
        kernel,
        hooks,
        manifest,
        caller_id_str,
        tool_call,
        &result,
        tool_elapsed_ms,
    );

    let final_content = prepare_tool_result_content(
        &tool_call.name,
        &result,
        context_budget,
        match verdict {
            LoopGuardVerdict::Warn(warn_msg) => Some(warn_msg.as_str()),
            _ => None,
        },
    );

    expand_visible_tools_after_discovery_result(
        visible_tools,
        available_tools,
        &tool_call.name,
        &result.content,
        streaming,
    );

    if let Some(stream_tx) = stream_tx {
        send_tool_execution_result_event(
            stream_tx,
            &manifest.name,
            tool_call,
            &result,
            &final_content,
        )
        .await;
    }

    push_tool_result_block(tool_result_blocks, &tool_call.name, result, final_content);
    tool_result_blocks.extend(transient_content);
}

#[cfg(test)]
#[path = "agent_loop_tool_finish_tests.rs"]
mod tests;

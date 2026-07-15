//! Memory and session/workspace dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;
use crate::mcp;

use super::{
    tool_memory_context_batch, tool_memory_forget, tool_memory_recall,
    tool_memory_recall_mempalace, tool_memory_save, tool_memory_store, tool_memory_store_mempalace,
    tool_session_recall, tool_session_tool_call_summary, tool_workspace_add,
};

pub(crate) async fn dispatch_memory_tool(
    tool_name: &str,
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    match tool_name {
        "memory_store" => {
            let backend = memory_backend(kernel);
            match backend {
                captain_types::config::MemoryBackend::Mempalace => {
                    tool_memory_store_mempalace(input, mcp_connections, kernel).await
                }
                captain_types::config::MemoryBackend::Graph => tool_memory_store(input, kernel),
            }
        }
        "memory_recall" => {
            let backend = memory_backend(kernel);
            match backend {
                captain_types::config::MemoryBackend::Mempalace => {
                    tool_memory_recall_mempalace(input, mcp_connections, kernel).await
                }
                captain_types::config::MemoryBackend::Graph => tool_memory_recall(input, kernel),
            }
        }
        "memory_context_batch" => {
            tool_memory_context_batch(input, mcp_connections, kernel, memory_backend(kernel)).await
        }
        "memory_save" => tool_memory_save(input, mcp_connections, kernel).await,
        "workspace_add" => tool_workspace_add(input, kernel).await,
        "memory_forget" => tool_memory_forget(input, mcp_connections, kernel).await,
        "session_recall" => tool_session_recall(input).await,
        "session_tool_call_summary" => {
            tool_session_tool_call_summary(input, kernel, caller_agent_id).await
        }
        other => Err(format!("Unknown memory/session tool: {other}")),
    }
}

fn memory_backend(kernel: Option<&Arc<dyn KernelHandle>>) -> captain_types::config::MemoryBackend {
    kernel.map(|kh| kh.memory_backend()).unwrap_or_default()
}

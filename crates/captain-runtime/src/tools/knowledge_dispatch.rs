//! Knowledge graph dispatch.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

use super::{tool_knowledge_add_entity, tool_knowledge_add_relation, tool_knowledge_query};

pub(crate) async fn dispatch_knowledge_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    match tool_name {
        "knowledge_add_entity" => tool_knowledge_add_entity(input, kernel).await,
        "knowledge_add_relation" => tool_knowledge_add_relation(input, kernel).await,
        "knowledge_query" => tool_knowledge_query(input, kernel).await,
        other => Err(format!("Unknown knowledge tool: {other}")),
    }
}

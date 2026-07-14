//! Batch context capsule across memory, sessions, and knowledge graph.

use std::sync::Arc;

use captain_types::config::MemoryBackend;

use crate::kernel_handle::KernelHandle;
use crate::mcp;

use super::{
    collect_string_list, compact_memory_context_result, tool_knowledge_query, tool_session_recall,
    truncate_owned, DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
};

const MAX_MEMORY_CONTEXT_BATCH_ITEMS: usize = 30;

pub(crate) async fn tool_memory_context_batch(
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    backend: MemoryBackend,
) -> Result<String, String> {
    let queries = collect_string_list(input, "queries")
        .or_else(|| input["query"].as_str().map(|q| vec![q.to_string()]))
        .ok_or("Missing 'query' or 'queries' parameter")?;
    let queries: Vec<String> = queries
        .into_iter()
        .map(|q| q.trim().to_string())
        .filter(|q| !q.is_empty())
        .take(MAX_MEMORY_CONTEXT_BATCH_ITEMS)
        .collect();
    if queries.is_empty() {
        return Err("memory_context_batch requires at least one query".to_string());
    }

    let include_memory = input["include_memory"].as_bool().unwrap_or(true);
    let include_sessions = input["include_sessions"].as_bool().unwrap_or(true);
    let include_knowledge = input["include_knowledge"].as_bool().unwrap_or(false);
    let max_results = input["max_results"].as_u64().unwrap_or(5).clamp(1, 20);
    let memory_max_results = input["memory_max_results"]
        .as_u64()
        .unwrap_or(max_results)
        .clamp(1, 10) as usize;
    let memory_min_similarity = input["memory_min_similarity"]
        .as_f64()
        .unwrap_or(DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY)
        .clamp(0.0, 1.0);
    let strict_memory_filter = input["strict_memory_filter"].as_bool().unwrap_or(true);
    let preview_chars = input["preview_chars"]
        .as_u64()
        .unwrap_or(2500)
        .clamp(500, 10_000) as usize;
    let stop_on_error = input["stop_on_error"].as_bool().unwrap_or(false);

    let mut results = Vec::new();
    for query in &queries {
        let mut entry = serde_json::json!({ "query": query });
        if include_memory {
            entry["memory"] = compact_memory_context_result(
                query,
                mcp_connections,
                kernel,
                backend,
                memory_max_results,
                preview_chars,
                memory_min_similarity,
                strict_memory_filter,
            )
            .await;
        }
        if include_sessions {
            let session_input = serde_json::json!({
                "query": query,
                "max_results": max_results,
            });
            entry["sessions"] =
                batch_text_result(tool_session_recall(&session_input).await, preview_chars);
        }
        if include_knowledge {
            let knowledge_input = serde_json::json!({
                "source": query,
                "max_depth": 2,
            });
            entry["knowledge"] = batch_text_result(
                tool_knowledge_query(&knowledge_input, kernel).await,
                preview_chars,
            );
        }

        let has_error = ["memory", "sessions", "knowledge"].iter().any(|key| {
            entry
                .get(*key)
                .and_then(|v| v["success"].as_bool())
                .is_some_and(|success| !success)
        });
        results.push(entry);
        if stop_on_error && has_error {
            break;
        }
    }

    serde_json::to_string_pretty(&serde_json::json!({
        "success": true,
        "tool": "memory_context_batch",
        "results": results,
        "note": "Compact context capsule. Recall exact memory/session/knowledge details separately only when precision matters.",
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

fn batch_text_result(result: Result<String, String>, preview_chars: usize) -> serde_json::Value {
    match result {
        Ok(text) => serde_json::json!({
            "success": true,
            "preview": truncate_owned(&text, preview_chars),
        }),
        Err(error) => serde_json::json!({
            "success": false,
            "error": error,
        }),
    }
}

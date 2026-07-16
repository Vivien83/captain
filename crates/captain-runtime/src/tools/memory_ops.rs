//! Memory store/recall handlers and MemPalace bridge.

use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::tools::{ensure_no_secret_literal, require_kernel};
use std::sync::Arc;

pub(crate) fn tool_memory_store(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let value = input.get("value").ok_or("Missing 'value' parameter")?;
    ensure_no_secret_literal("memory_store", "value", &value.to_string())?;
    kh.memory_store(key, value.clone())?;
    Ok(format!("Stored value under key '{key}'."))
}

pub(crate) fn tool_memory_recall(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    match kh.memory_recall(key)? {
        Some(val) => Ok(serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string())),
        None => Ok(format!("No value found for key '{key}'.")),
    }
}

pub(crate) async fn call_mempalace_tool(
    tool_name: &str,
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
) -> Result<String, String> {
    let conns_mutex =
        mcp_connections.ok_or("MemPalace backend configured but no MCP connections available")?;
    let mut conns = conns_mutex.lock().await;
    let conn = conns
        .iter_mut()
        .find(|c| c.name() == "mempalace")
        .ok_or("MemPalace MCP server not connected")?;
    conn.call_tool(tool_name, input)
        .await
        .map_err(|e| format!("MemPalace call failed: {e}"))
}

pub(crate) async fn tool_memory_store_mempalace(
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let value = input.get("value").ok_or("Missing 'value' parameter")?;
    let value_str = match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    ensure_no_secret_literal("memory_store", "value", &value_str)?;

    let record = captain_memory::memory_writer::NewMemoryWrite {
        subject: key.to_string(),
        predicate: "has_value".to_string(),
        object: value_str,
        wing: None,
        room: None,
        source: "memory_store".to_string(),
    };

    let conn_opt = kernel.and_then(|kh| kh.memory_writes_conn());
    match conn_opt {
        Some(conn) => {
            let sender =
                mcp_connections.map(|m| crate::memory_writer::McpMemPalaceSender { mcp_conns: m });
            let sender_ref: Option<&dyn crate::memory_writer::MemPalaceSender> = sender
                .as_ref()
                .map(|s| s as &dyn crate::memory_writer::MemPalaceSender);
            crate::memory_writer::write_through(conn, sender_ref, record).await?;
            Ok(format!(
                "Stored value under key '{key}' (mempalace, write-through)."
            ))
        }
        None => {
            let mcp_input = serde_json::json!({
                "subject": record.subject,
                "predicate": record.predicate,
                "object": record.object,
            });
            call_mempalace_tool(
                "mcp_mempalace_mempalace_kg_add",
                &mcp_input,
                mcp_connections,
            )
            .await?;
            Ok(format!(
                "Stored value under key '{key}' (mempalace, direct)."
            ))
        }
    }
}

pub(crate) async fn tool_memory_recall_mempalace(
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let key = input["key"].as_str().ok_or("Missing 'key' parameter")?;
    let mut parts: Vec<String> = Vec::new();
    let retractions = kernel.map(|kh| kh.memory_retractions()).unwrap_or_default();
    if let Some(part) = local_memory_recall_part(key, kernel, &retractions)? {
        parts.push(part);
    }
    let query_guarded = crate::memory_retractions::text_matches_any(key, &retractions);
    let mut suppressed = query_guarded;

    if !query_guarded {
        let mcp_input = serde_json::json!({
            "query": key,
            "limit": 5,
        });
        match call_mempalace_tool(
            "mcp_mempalace_mempalace_search",
            &mcp_input,
            mcp_connections,
        )
        .await
        {
            Ok(result) if !result.is_empty() && result != "[]" && result != "null" => {
                if let Some(part) = memory_recall_part("MemPalace", &result, &retractions) {
                    parts.push(part);
                } else {
                    suppressed = true;
                }
            }
            _ => {}
        }

        if let Some(kh) = kernel {
            if let Ok(Some(val)) = kh.memory_recall(key) {
                let s = serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
                if let Some(part) = memory_recall_part("Graph", &s, &retractions) {
                    parts.push(part);
                } else {
                    suppressed = true;
                }
            }
        }
    }

    if parts.is_empty() {
        if suppressed {
            Ok(format!(
                "No active memory found for key '{key}' (archived matches were suppressed by memory retraction guards)."
            ))
        } else {
            Ok(format!("No value found for key '{key}'."))
        }
    } else {
        Ok(parts.join("\n\n"))
    }
}

fn local_memory_recall_part(
    key: &str,
    kernel: Option<&Arc<dyn KernelHandle>>,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> Result<Option<String>, String> {
    let rows =
        crate::tools::memory_context::recall_local_memory_write_rows(key, kernel, 5, retractions)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let facts = rows
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "write_id": row.id,
                "subject": row.subject,
                "predicate": row.predicate,
                "object": row.object,
                "source": row.source,
                "created_at": row.created_at,
            })
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_string_pretty(&facts)
        .map_err(|error| format!("local memory recall serialization failed: {error}"))?;
    Ok(Some(format!(
        "[Local journal — active authoritative facts]\n{payload}"
    )))
}

pub(crate) fn memory_recall_part(
    label: &str,
    text: &str,
    retractions: &[crate::memory_retractions::MemoryRetraction],
) -> Option<String> {
    if crate::memory_retractions::text_matches_any(text, retractions) {
        return None;
    }
    Some(format!("[{label}] {text}"))
}

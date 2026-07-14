use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::tools::{call_mempalace_tool, current_origin_channel, require_kernel};
use std::sync::Arc;

pub(crate) async fn tool_memory_forget(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let subject = input["subject"].as_str().filter(|s| !s.is_empty());
    let predicate = input["predicate"].as_str().filter(|s| !s.is_empty());
    let object = input["object"].as_str().filter(|s| !s.is_empty());
    if subject.is_none() && predicate.is_none() && object.is_none() {
        return Err(
            "memory_forget refuses to delete with no filter — provide at least subject, predicate or object".into(),
        );
    }
    let retraction =
        crate::memory_retractions::MemoryRetraction::from_filters(subject, predicate, object);
    let (tombstone_recorded, active_context_sanitized) = if let Some(retraction) = retraction {
        let mut retractions = kh.memory_retractions();
        retractions = crate::memory_retractions::append_retraction(retractions, retraction);
        kh.memory_kv_store(
            crate::memory_retractions::MEMORY_RETRACTIONS_KEY,
            crate::memory_retractions::retractions_to_value(&retractions),
        )
        .map_err(|e| format!("memory_forget could not persist retraction guard: {e}"))?;
        let sanitized = kh
            .memory_sanitize_active_context(&retractions)
            .unwrap_or_else(|e| serde_json::json!({"status": "error", "error": e}));
        (true, sanitized)
    } else {
        (
            false,
            serde_json::json!({
                "status": "skipped",
                "reason": "no specific retraction terms derived"
            }),
        )
    };
    let conn_arc = kh
        .memory_writes_conn()
        .ok_or("memory_writes connection not available on this kernel")?;
    let removed = {
        let conn = conn_arc
            .lock()
            .map_err(|e| format!("memory_writes lock poisoned: {e}"))?;
        captain_memory::memory_writer::delete_by_match(&conn, subject, predicate, object)
            .map_err(|e| format!("delete_by_match: {e}"))?
    };
    Ok(serde_json::json!({
        "status": "ok",
        "removed": removed,
        "filters": {
            "subject": subject,
            "predicate": predicate,
            "object": object,
        },
        "active_context_suppressed": tombstone_recorded,
        "active_context_sanitized": active_context_sanitized
    })
    .to_string())
}

pub(crate) async fn tool_memory_save(
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    tool_memory_save_impl(input, mcp_connections, kernel)
        .await
        .map_err(|e| {
            tracing::warn!(
                tool = "memory_save",
                error = %e,
                input_keys = ?input.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
                "memory_save validation/runtime failed"
            );
            e
        })
}

async fn tool_memory_save_impl(
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;

    let subject = input["subject"]
        .as_str()
        .ok_or("Missing 'subject' parameter")?
        .trim();
    let predicate = input["predicate"]
        .as_str()
        .ok_or("Missing 'predicate' parameter")?
        .trim();
    let object = input["object"]
        .as_str()
        .ok_or("Missing 'object' parameter")?
        .trim();
    let category = input["category"]
        .as_str()
        .ok_or("Missing 'category' parameter")?
        .trim();

    if subject.is_empty() || predicate.is_empty() || object.is_empty() {
        return Err("subject / predicate / object cannot be empty".into());
    }
    const ALLOWED_CATEGORIES: &[&str] = &["info", "skill", "error_success", "solution", "other"];
    if !ALLOWED_CATEGORIES.contains(&category) {
        return Err(format!(
            "category must be one of {ALLOWED_CATEGORIES:?}, got '{category}'"
        ));
    }
    if object.chars().count() > 1000 {
        return Err("object is too long (max 1000 chars)".into());
    }

    if let Some(pat) = crate::pii_filter::check_memory_triple(subject, predicate, object) {
        return Err(format!(
            "memory_save refused: object contains PII matching '{pat}' — store secrets via secret_write, never in MemPalace"
        ));
    }

    let wing = input["wing"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let room = input["room"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let channel = input["channel"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .or_else(current_origin_channel);

    let record = captain_memory::memory_writer::NewMemoryWrite {
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        object: object.to_string(),
        wing: wing.clone(),
        room: room.clone(),
        source: format!("memory_save:{category}"),
    };

    let conn_opt = kh.memory_writes_conn();
    match conn_opt {
        Some(conn) => {
            let sender =
                mcp_connections.map(|m| crate::memory_writer::McpMemPalaceSender { mcp_conns: m });
            let sender_ref: Option<&dyn crate::memory_writer::MemPalaceSender> = sender
                .as_ref()
                .map(|s| s as &dyn crate::memory_writer::MemPalaceSender);
            crate::memory_writer::write_through(conn, sender_ref, record).await?;
        }
        None => {
            let mcp_input = serde_json::json!({
                "subject": subject,
                "predicate": predicate,
                "object": object,
            });
            call_mempalace_tool(
                "mcp_mempalace_mempalace_kg_add",
                &mcp_input,
                mcp_connections,
            )
            .await?;
        }
    }

    kh.publish_memory_stored(
        subject,
        predicate,
        object,
        &format!("memory_save:{category}"),
        wing.as_deref(),
        room.as_deref(),
        channel.as_deref(),
        Some(category),
    );

    Ok(format!("🧠 mémorisé · {subject}/{predicate} ({category})"))
}

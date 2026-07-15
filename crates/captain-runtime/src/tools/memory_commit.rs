use crate::kernel_handle::KernelHandle;
use crate::mcp;
use crate::tools::{call_mempalace_tool, current_origin_channel, require_kernel};
use std::sync::Arc;

pub(crate) async fn tool_memory_forget(
    input: &serde_json::Value,
    mcp_connections: Option<&tokio::sync::Mutex<Vec<mcp::McpConnection>>>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let subject = input["subject"].as_str().filter(|s| !s.is_empty());
    let predicate = input["predicate"].as_str().filter(|s| !s.is_empty());
    let object = input["object"].as_str().filter(|s| !s.is_empty());
    if subject.is_none() && predicate.is_none() && object.is_none() {
        return Err(
            "memory_forget refuses to retract with no filter — provide at least subject, predicate or object".into(),
        );
    }
    let conn_arc = kh
        .memory_writes_conn()
        .ok_or("memory_writes connection not available on this kernel")?;
    let mut batch = {
        let conn = conn_arc
            .lock()
            .map_err(|e| format!("memory_writes lock poisoned: {e}"))?;
        captain_memory::memory_writer::retract_by_match(
            &conn,
            subject,
            predicate,
            object,
            "memory_forget",
        )
        .map_err(|e| format!("retract_by_match: {e}"))?
    };

    // A fully exact triple may predate Captain's local journal. Queue an
    // idempotent invalidation anyway so the MemPalace index converges.
    let mut invalidations_created = batch.invalidations.len();
    if batch.retracted.is_empty()
        && [subject, predicate, object]
            .into_iter()
            .all(|value| value.is_some_and(|value| !value.contains('%')))
    {
        let conn = conn_arc
            .lock()
            .map_err(|e| format!("memory_writes lock poisoned: {e}"))?;
        let (invalidation, created) = captain_memory::memory_writer::ensure_exact_invalidation(
            &conn,
            captain_memory::memory_writer::NewMemoryWrite {
                subject: subject.unwrap_or_default().to_string(),
                predicate: predicate.unwrap_or_default().to_string(),
                object: object.unwrap_or_default().to_string(),
                wing: None,
                room: None,
                source: "memory_forget:legacy".into(),
            },
        )
        .map_err(|e| format!("ensure_exact_invalidation: {e}"))?;
        invalidations_created += usize::from(created);
        batch.invalidations.push(invalidation);
    }

    // Build precise archive guards from the exact facts that were retracted.
    // A broad subject/predicate guard would also suppress a future correction.
    let mut retractions = kh.memory_retractions();
    let mut tombstone_recorded = false;
    for row in &batch.retracted {
        if let Some(retraction) = crate::memory_retractions::MemoryRetraction::from_filters(
            Some(&row.subject),
            Some(&row.predicate),
            Some(&row.object),
        ) {
            retractions = crate::memory_retractions::append_retraction(retractions, retraction);
            tombstone_recorded = true;
        }
    }
    if !tombstone_recorded {
        if let Some(retraction) =
            crate::memory_retractions::MemoryRetraction::from_filters(subject, predicate, object)
        {
            retractions = crate::memory_retractions::append_retraction(retractions, retraction);
            tombstone_recorded = true;
        }
    }
    let active_context_sanitized = if tombstone_recorded {
        kh.memory_kv_store(
            crate::memory_retractions::MEMORY_RETRACTIONS_KEY,
            crate::memory_retractions::retractions_to_value(&retractions),
        )
        .map_err(|e| format!("memory_forget could not persist retraction guard: {e}"))?;
        kh.memory_sanitize_active_context(&retractions)
            .unwrap_or_else(|e| serde_json::json!({"status": "error", "error": e}))
    } else {
        serde_json::json!({
            "status": "skipped",
            "reason": "no specific retraction terms derived"
        })
    };

    let sender =
        mcp_connections.map(|mcp_conns| crate::memory_writer::McpMemPalaceSender { mcp_conns });
    let sender_ref = sender
        .as_ref()
        .map(|sender| sender as &dyn crate::memory_writer::MemPalaceSender);
    let mut remote_synced = 0usize;
    let mut remote_failed = 0usize;
    for invalidation in &batch.invalidations {
        if invalidation.sync_status == captain_memory::memory_writer::SyncStatus::Synced {
            remote_synced += 1;
            continue;
        }
        let outcome = crate::memory_writer::sync_existing_write(
            Arc::clone(&conn_arc),
            sender_ref,
            invalidation,
        )
        .await?;
        if outcome.status == captain_memory::memory_writer::SyncStatus::Synced {
            remote_synced += 1;
        }
        if outcome.error.is_some() {
            remote_failed += 1;
            break;
        }
        if !outcome.attempted {
            break;
        }
    }
    let remote_pending = batch.invalidations.len().saturating_sub(remote_synced);
    Ok(serde_json::json!({
        "status": "ok",
        "removed": batch.retracted.len(),
        "retracted": batch.retracted.len(),
        "invalidations_queued": invalidations_created,
        "remote_synced": remote_synced,
        "remote_pending": remote_pending,
        "remote_failed": remote_failed,
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
    let sync_status = match conn_opt {
        Some(conn) => {
            let sender =
                mcp_connections.map(|m| crate::memory_writer::McpMemPalaceSender { mcp_conns: m });
            let sender_ref: Option<&dyn crate::memory_writer::MemPalaceSender> = sender
                .as_ref()
                .map(|s| s as &dyn crate::memory_writer::MemPalaceSender);
            let write_id =
                crate::memory_writer::write_through(Arc::clone(&conn), sender_ref, record).await?;
            {
                let guard = conn
                    .lock()
                    .map_err(|e| format!("memory_writes lock poisoned: {e}"))?;
                captain_memory::memory_writer::get(&guard, &write_id)
                    .map_err(|e| format!("memory_writes receipt: {e}"))?
                    .map(|row| row.sync_status)
            }
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
            Some(captain_memory::memory_writer::SyncStatus::Synced)
        }
    };

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

    Ok(format_memory_save_receipt(
        subject,
        predicate,
        category,
        sync_status,
    ))
}

fn format_memory_save_receipt(
    subject: &str,
    predicate: &str,
    category: &str,
    sync_status: Option<captain_memory::memory_writer::SyncStatus>,
) -> String {
    let receipt = match sync_status {
        Some(captain_memory::memory_writer::SyncStatus::Synced) => "index=sync",
        Some(captain_memory::memory_writer::SyncStatus::Error) => {
            "local=durable · index=degraded/retry-auto"
        }
        _ => "local=durable · index=pending/retry-auto",
    };
    format!("🧠 mémorisé · {subject}/{predicate} ({category}) · {receipt}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::memory_writer::SyncStatus;

    #[test]
    fn memory_save_receipt_distinguishes_remote_sync_from_local_durability() {
        let synced =
            format_memory_save_receipt("user", "prefers", "info", Some(SyncStatus::Synced));
        let pending =
            format_memory_save_receipt("user", "prefers", "info", Some(SyncStatus::Pending));
        let degraded =
            format_memory_save_receipt("user", "prefers", "info", Some(SyncStatus::Error));
        assert!(synced.contains("index=sync"));
        assert!(pending.contains("local=durable · index=pending/retry-auto"));
        assert!(degraded.contains("local=durable · index=degraded/retry-auto"));
    }
}

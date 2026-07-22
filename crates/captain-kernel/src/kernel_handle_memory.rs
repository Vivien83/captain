use std::sync::{Arc, Mutex};

use captain_runtime::memory_retractions::MemoryRetraction;
use captain_types::memory::{MemoryFilter, MemorySource};
use rusqlite::Connection;
use serde_json::Value;

use super::kernel_workspace_security::shared_memory_agent_id;
use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn handle_memory_backend(&self) -> captain_types::config::MemoryBackend {
        self.config.memory.backend
    }

    pub(super) fn handle_memory_store(&self, key: &str, value: Value) -> Result<(), String> {
        let agent_id = shared_memory_agent_id();
        self.memory
            .structured_set(agent_id, key, value.clone())
            .map_err(|e| format!("Memory store (KV) failed: {e}"))?;

        let content = format!("[memory_store] {key}: {value}");
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("memory_key".to_string(), Value::String(key.to_string()));
        self.memory
            .semantic
            .remember(
                agent_id,
                &content,
                MemorySource::Observation,
                "explicit",
                metadata,
            )
            .map_err(|e| format!("Memory store (semantic) failed: {e}"))?;
        Ok(())
    }

    pub(super) fn handle_memory_kv_store(&self, key: &str, value: Value) -> Result<(), String> {
        self.memory
            .structured_set(shared_memory_agent_id(), key, value)
            .map_err(|e| format!("Memory KV store failed: {e}"))
    }

    pub(super) fn handle_memory_kv_recall(&self, key: &str) -> Result<Option<Value>, String> {
        self.memory
            .structured_get(shared_memory_agent_id(), key)
            .map_err(|e| format!("Memory KV recall failed: {e}"))
    }

    pub(super) fn handle_memory_retractions(&self) -> Vec<MemoryRetraction> {
        let persisted = captain_runtime::memory_retractions::load_retractions(
            self.handle_memory_kv_recall(
                captain_runtime::memory_retractions::MEMORY_RETRACTIONS_KEY,
            )
            .ok()
            .flatten(),
        );
        let conn = self.memory.usage_conn();
        let Ok(guard) = conn.lock() else {
            return persisted;
        };
        let Ok(rows) = captain_memory::memory_writer::list_recent_retracted(
            &guard,
            captain_runtime::memory_retractions::MAX_RETRACTIONS,
        ) else {
            return persisted;
        };
        captain_runtime::memory_retractions::merge_journal_retractions(persisted, &rows)
    }

    pub(super) fn handle_memory_sanitize_active_context(
        &self,
        retractions: &[MemoryRetraction],
    ) -> Result<Value, String> {
        if retractions.is_empty() {
            return Ok(memory_sanitize_result(0, 0));
        }

        let conn = self.memory.usage_conn();
        let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
        let summaries = canonical_summaries(&guard)?;

        let mut updated = 0usize;
        let mut cleared = 0usize;
        let now = chrono::Utc::now().to_rfc3339();
        for (agent_id, summary) in summaries {
            match captain_runtime::memory_retractions::filter_retracted_lines(&summary, retractions)
            {
                Some(filtered) if filtered != summary => {
                    guard
                        .execute(
                            "UPDATE canonical_sessions \
                             SET compacted_summary = ?2, updated_at = ?3 \
                             WHERE agent_id = ?1",
                            rusqlite::params![agent_id, filtered, now],
                        )
                        .map_err(|e| format!("canonical summary update: {e}"))?;
                    updated += 1;
                }
                None => {
                    guard
                        .execute(
                            "UPDATE canonical_sessions \
                             SET compacted_summary = NULL, updated_at = ?2 \
                             WHERE agent_id = ?1",
                            rusqlite::params![agent_id, now],
                        )
                        .map_err(|e| format!("canonical summary clear: {e}"))?;
                    cleared += 1;
                }
                _ => {}
            }
        }

        Ok(memory_sanitize_result(updated, cleared))
    }

    pub(super) fn handle_memory_writes_conn(&self) -> Option<Arc<Mutex<Connection>>> {
        Some(self.memory.usage_conn())
    }

    pub(super) fn handle_learning_review_list(&self, limit: usize) -> Result<Value, String> {
        let conn = self.memory.usage_conn();
        let guard = conn.lock().map_err(|e| format!("sqlite poisoned: {e}"))?;
        let items = captain_memory::learning_review::list_pending(&guard, limit)
            .map_err(|e| format!("list_pending: {e}"))?;
        serde_json::to_value(items).map_err(|e| format!("serialize: {e}"))
    }

    pub(super) async fn handle_learning_review_decide(
        &self,
        review_id: &str,
        approve: bool,
        decided_by: Option<&str>,
    ) -> Result<Value, String> {
        let committer = captain_runtime::memory_committer::MemoryCommitter::with_mode(
            self.memory.usage_conn(),
            self.config.learning.mode,
        );
        let sender = captain_runtime::memory_writer::McpMemPalaceSender {
            mcp_conns: &self.mcp_connections,
        };
        let sender_ref: Option<&dyn captain_runtime::memory_writer::MemPalaceSender> =
            Some(&sender);
        if approve {
            let committed = committer
                .approve_pending(review_id, decided_by, sender_ref)
                .await?;
            serde_json::to_value(committed).map_err(|e| format!("serialize: {e}"))
        } else {
            committer.deny_pending(review_id, decided_by)?;
            Ok(denied_response(review_id))
        }
    }

    pub(super) fn handle_memory_recall(&self, key: &str) -> Result<Option<Value>, String> {
        let agent_id = shared_memory_agent_id();
        if let Ok(Some(val)) = self.memory.structured_get(agent_id, key) {
            return Ok(Some(val));
        }

        let filter = MemoryFilter {
            agent_id: Some(agent_id),
            ..Default::default()
        };
        let results = self
            .memory
            .semantic
            .recall(key, 3, Some(filter))
            .map_err(|e| format!("Memory recall (semantic) failed: {e}"))?;
        Ok(results
            .first()
            .map(|best| Value::String(best.content.clone())))
    }
}

fn canonical_summaries(guard: &Connection) -> Result<Vec<(String, String)>, String> {
    let mut stmt = guard
        .prepare(
            "SELECT agent_id, compacted_summary \
             FROM canonical_sessions \
             WHERE compacted_summary IS NOT NULL AND compacted_summary != ''",
        )
        .map_err(|e| format!("canonical summary query prepare: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            let agent_id: String = row.get(0)?;
            let summary: String = row.get(1)?;
            Ok((agent_id, summary))
        })
        .map_err(|e| format!("canonical summary query: {e}"))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("canonical summary row: {e}"))
}

fn memory_sanitize_result(updated: usize, cleared: usize) -> Value {
    serde_json::json!({
        "status": "ok",
        "canonical_summaries_updated": updated,
        "canonical_summaries_cleared": cleared
    })
}

fn denied_response(id: &str) -> Value {
    serde_json::json!({ "status": "denied", "id": id })
}

#[cfg(test)]
mod tests {
    use super::{denied_response, memory_sanitize_result};

    #[test]
    fn memory_sanitize_empty_response_is_operator_safe() {
        let value = memory_sanitize_result(0, 0);

        assert_eq!(value["status"].as_str(), Some("ok"));
        assert_eq!(value["canonical_summaries_updated"].as_u64(), Some(0));
        assert_eq!(value["canonical_summaries_cleared"].as_u64(), Some(0));
    }

    #[test]
    fn denied_response_keeps_public_shape() {
        let value = denied_response("proposal-1");

        assert_eq!(value["status"].as_str(), Some("denied"));
        assert_eq!(value["id"].as_str(), Some("proposal-1"));
    }
}

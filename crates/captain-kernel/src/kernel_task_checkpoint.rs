//! Periodic durable task checkpoint.
//!
//! Long tasks must survive crashes and compactions without losing the
//! thread. Every LLM turn, once the estimated context usage crosses into a
//! new 20%-of-budget bucket, a deterministic checkpoint (last explicit user
//! request, tool activity since, mid-tool-activity status) is written to the
//! structured KV store — well before the compaction threshold, so the state
//! exists even if the process dies mid-task.

use super::CaptainKernel;
use captain_memory::session::Session;
use captain_runtime::task_checkpoint::extract_task_checkpoint;
use captain_types::agent::AgentId;
use chrono::Utc;
use tracing::{debug, warn};

/// KV key holding the latest task checkpoint for an agent.
pub const TASK_CHECKPOINT_KEY: &str = "__captain_task_checkpoint_v1";

/// Number of buckets the context budget is split into (5 → every 20%).
const CHECKPOINT_BUCKETS: usize = 5;

impl CaptainKernel {
    /// Write a task checkpoint when context usage enters a new bucket, or
    /// unconditionally when a compaction is about to run.
    ///
    /// Called on every pre-loop compaction plan (streaming and
    /// non-streaming), i.e. once per LLM turn. Cheap no-op when neither
    /// trigger fires. The bucket is also updated downward after compaction
    /// shrinks the session, re-arming the next crossing.
    ///
    /// `compaction_imminent` matters in practice: profiles like
    /// codex_economy compact on message count at 6-17% of the context
    /// window, so the 20%-bucket crossing alone would never fire — yet the
    /// moment right before compaction is exactly when the task state must
    /// be durable (observed live on 2026-07-02).
    pub(super) fn maybe_write_task_checkpoint(
        &self,
        agent_id: AgentId,
        session: &Session,
        estimated_tokens: usize,
        context_window: usize,
        compaction_imminent: bool,
    ) {
        let bucket = checkpoint_bucket(estimated_tokens, context_window);
        let previous = self
            .memory
            .structured_get(agent_id, TASK_CHECKPOINT_KEY)
            .ok()
            .flatten();
        let previous_bucket = previous
            .as_ref()
            .and_then(|v| v.get("bucket"))
            .and_then(|b| b.as_u64())
            .map(|b| b as usize);

        let bucket_crossed =
            previous_bucket != Some(bucket) && !(bucket == 0 && previous_bucket.is_none());
        if !bucket_crossed && !compaction_imminent {
            return;
        }

        let checkpoint = extract_task_checkpoint(&session.messages);
        let value = serde_json::json!({
            "session_id": session.id.to_string(),
            "bucket": bucket,
            "estimated_tokens": estimated_tokens,
            "context_window": context_window,
            "updated_at": Utc::now().to_rfc3339(),
            "checkpoint": checkpoint,
        });

        match self
            .memory
            .structured_set(agent_id, TASK_CHECKPOINT_KEY, value)
        {
            Ok(()) => debug!(
                agent_id = %agent_id,
                bucket,
                estimated_tokens,
                "Task checkpoint written"
            ),
            Err(e) => warn!(agent_id = %agent_id, "Task checkpoint write failed: {e}"),
        }
    }
}

fn checkpoint_bucket(estimated_tokens: usize, context_window: usize) -> usize {
    estimated_tokens
        .saturating_mul(CHECKPOINT_BUCKETS)
        .checked_div(context_window)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::SessionId;
    use captain_types::config::KernelConfig;
    use captain_types::message::Message;
    use std::collections::HashMap;

    #[test]
    fn buckets_split_the_context_window_in_fifths() {
        assert_eq!(checkpoint_bucket(0, 200_000), 0);
        assert_eq!(checkpoint_bucket(39_999, 200_000), 0);
        assert_eq!(checkpoint_bucket(40_000, 200_000), 1);
        assert_eq!(checkpoint_bucket(85_000, 200_000), 2);
        assert_eq!(checkpoint_bucket(200_000, 200_000), 5);
        assert_eq!(checkpoint_bucket(10_000, 0), 0);
    }

    #[test]
    fn checkpoint_written_on_bucket_crossings_only() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("captain-kernel-task-checkpoint-test");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");

        let session = Session {
            id: SessionId::new(),
            agent_id,
            messages: vec![Message::user("finish the migration")],
            context_window_tokens: 0,
            label: None,
        };
        let read = |kernel: &CaptainKernel| {
            kernel
                .memory
                .structured_get(agent_id, TASK_CHECKPOINT_KEY)
                .unwrap()
        };

        // Bucket 0, no compaction pending, no prior checkpoint: nothing written.
        kernel.maybe_write_task_checkpoint(agent_id, &session, 10_000, 200_000, false);
        assert!(read(&kernel).is_none());

        // Compaction imminent writes even in bucket 0 — the live case:
        // count-triggered compactions fire far below the first bucket.
        kernel.maybe_write_task_checkpoint(agent_id, &session, 17_000, 200_000, true);
        let value = read(&kernel).expect("checkpoint written before compaction");
        assert_eq!(value["session_id"], session.id.to_string());
        assert_eq!(value["bucket"], 0);
        assert_eq!(
            value["checkpoint"]["last_user_request"],
            "finish the migration"
        );

        // Crossing into bucket 1 writes the checkpoint.
        kernel.maybe_write_task_checkpoint(agent_id, &session, 45_000, 200_000, false);
        assert_eq!(read(&kernel).unwrap()["bucket"], 1);

        // Same bucket, no compaction: value untouched (same tokens marker).
        kernel.maybe_write_task_checkpoint(agent_id, &session, 55_000, 200_000, false);
        assert_eq!(read(&kernel).unwrap()["estimated_tokens"], 45_000);

        // Post-compaction shrink re-arms by writing the lower bucket.
        kernel.maybe_write_task_checkpoint(agent_id, &session, 10_000, 200_000, false);
        assert_eq!(read(&kernel).unwrap()["bucket"], 0);

        kernel.shutdown();
    }
}

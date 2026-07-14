//! Reinject the durable task checkpoint after an apparent crash.
//!
//! `kernel_task_checkpoint.rs` writes a deterministic snapshot of the
//! in-progress task to the KV store every turn, well before compaction —
//! but nothing ever read it back. This module closes that loop: every time
//! a session is loaded for a turn (`prepare_llm_turn_basics`), check for a
//! leftover checkpoint and, if the session was genuinely abandoned
//! mid-tool-activity (the crash signature), surface it as context before
//! the turn proceeds.

use super::kernel_task_checkpoint::TASK_CHECKPOINT_KEY;
use super::CaptainKernel;
use captain_memory::session::Session;
use captain_runtime::compaction_handoff::ends_mid_tool_activity;
use captain_runtime::task_checkpoint::{checkpoint_note, TaskCheckpoint};
use captain_types::agent::AgentId;
use captain_types::message::Message;
use tracing::{debug, warn};

pub(super) fn maybe_reinject_task_checkpoint(
    kernel: &CaptainKernel,
    agent_id: AgentId,
    session: &mut Session,
) {
    let value = match kernel.memory.structured_get(agent_id, TASK_CHECKPOINT_KEY) {
        Ok(value) => value,
        Err(e) => {
            warn!(agent_id = %agent_id, "Failed to read task checkpoint during recovery: {e}");
            None
        }
    };

    if !ends_mid_tool_activity(&session.messages) {
        // Normal turn boundary: the loaded session ends on a completed
        // assistant reply, not a dangling tool call. A compaction handoff
        // (if one happened) always ends the same way, so this also
        // correctly skips the already-handled-by-compaction case without
        // needing to detect it explicitly.
        clear_task_checkpoint(kernel, agent_id, value.is_some());
        return;
    }

    let checkpoint = value
        .as_ref()
        .filter(|value| checkpoint_matches_session(value, session))
        .and_then(|value| value.get("checkpoint"))
        .and_then(|value| serde_json::from_value::<TaskCheckpoint>(value.clone()).ok());
    let recovery_detail = checkpoint.as_ref().map(checkpoint_note).unwrap_or_else(|| {
        "- Le dernier appel d'outil a ete interrompu par un crash ou redemarrage et son resultat est inconnu.\n\
         - Verifier l'etat reel avant de conclure ou de relancer une action mutante.\n\
         - Reprendre ensuite la derniere demande utilisateur depuis la session durable."
            .to_string()
    });

    debug!(
        agent_id = %agent_id,
        has_checkpoint = checkpoint.is_some(),
        "Reinjecting durable task checkpoint after an apparent crash mid-task"
    );
    session.messages.push(Message::user(format!(
        "[Reprise apres redemarrage — le dernier tour s'est arrete en plein appel d'outil, \
         probablement suite a un crash ou redemarrage de Captain]\n{}",
        recovery_detail
    )));
    clear_task_checkpoint(kernel, agent_id, value.is_some());
}

fn checkpoint_matches_session(value: &serde_json::Value, session: &Session) -> bool {
    value
        .get("session_id")
        .and_then(|id| id.as_str())
        .is_none_or(|id| id == session.id.to_string())
}

fn clear_task_checkpoint(kernel: &CaptainKernel, agent_id: AgentId, checkpoint_present: bool) {
    if !checkpoint_present {
        return;
    }
    if let Err(e) = kernel
        .memory
        .structured_delete(agent_id, TASK_CHECKPOINT_KEY)
    {
        warn!(agent_id = %agent_id, "Failed to clear task checkpoint after recovery: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::SessionId;
    use captain_types::message::{ContentBlock, MessageContent};

    fn tool_use_only_session(agent_id: AgentId) -> Session {
        Session {
            id: SessionId::new(),
            agent_id,
            messages: vec![
                Message::user("finish the migration"),
                Message {
                    role: captain_types::message::Role::Assistant,
                    content: MessageContent::Blocks(vec![ContentBlock::ToolUse {
                        id: "tc1".to_string(),
                        name: "shell_exec".to_string(),
                        input: serde_json::json!({}),
                        provider_metadata: None,
                    }]),
                },
            ],
            context_window_tokens: 0,
            label: None,
        }
    }

    fn completed_reply_session(agent_id: AgentId) -> Session {
        Session {
            id: SessionId::new(),
            agent_id,
            messages: vec![
                Message::user("finish the migration"),
                Message::assistant("Done, migration applied."),
            ],
            context_window_tokens: 0,
            label: None,
        }
    }

    #[test]
    fn reinjects_generic_recovery_when_checkpoint_is_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("checkpoint-recovery-test-none");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = captain_types::config::KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..captain_types::config::KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", std::collections::HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");

        let mut session = tool_use_only_session(agent_id);
        let before = session.messages.len();
        maybe_reinject_task_checkpoint(&kernel, agent_id, &mut session);
        assert_eq!(session.messages.len(), before + 1);
        let injected = session.messages.last().unwrap().content.text_content();
        assert!(injected.contains("resultat est inconnu"));
        assert!(injected.contains("Verifier l'etat reel"));

        kernel.shutdown();
    }

    #[test]
    fn reinjects_when_checkpoint_present_and_session_ends_mid_tool() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("checkpoint-recovery-test-mid-tool");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = captain_types::config::KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..captain_types::config::KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", std::collections::HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");
        let mut session = tool_use_only_session(agent_id);

        let checkpoint = captain_runtime::task_checkpoint::TaskCheckpoint {
            last_user_request: Some("finish the migration".to_string()),
            tool_calls_since: vec!["shell_exec x1".to_string()],
            mid_tool_activity: true,
        };
        kernel
            .memory
            .structured_set(
                agent_id,
                TASK_CHECKPOINT_KEY,
                serde_json::json!({
                    "session_id": session.id.to_string(),
                    "bucket": 1,
                    "estimated_tokens": 1000,
                    "context_window": 200_000,
                    "updated_at": "2026-01-01T00:00:00Z",
                    "checkpoint": checkpoint,
                }),
            )
            .unwrap();

        let before = session.messages.len();
        maybe_reinject_task_checkpoint(&kernel, agent_id, &mut session);
        assert_eq!(session.messages.len(), before + 1);
        let injected = session.messages.last().unwrap().content.text_content();
        assert!(injected.contains("Reprise apres redemarrage"));
        assert!(injected.contains("finish the migration"));

        // Consumed and idempotent: the injected recovery message is now the
        // session boundary, so a second preparation does not re-inject.
        let before_second = session.messages.len();
        maybe_reinject_task_checkpoint(&kernel, agent_id, &mut session);
        assert_eq!(session.messages.len(), before_second);

        kernel.shutdown();
    }

    #[test]
    fn does_not_reinject_when_session_ended_normally() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("checkpoint-recovery-test-normal");
        std::fs::create_dir_all(&home_dir).unwrap();
        let config = captain_types::config::KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..captain_types::config::KernelConfig::default()
        };
        let kernel = CaptainKernel::boot_with_config(config).expect("kernel boot");
        let instance = kernel
            .activate_hand("browser", std::collections::HashMap::new())
            .expect("browser hand activates");
        let agent_id = instance.agent_id.expect("agent id present");
        let mut session = completed_reply_session(agent_id);

        let checkpoint = captain_runtime::task_checkpoint::TaskCheckpoint {
            last_user_request: Some("finish the migration".to_string()),
            tool_calls_since: vec![],
            mid_tool_activity: false,
        };
        kernel
            .memory
            .structured_set(
                agent_id,
                TASK_CHECKPOINT_KEY,
                serde_json::json!({
                    "session_id": session.id.to_string(),
                    "bucket": 1,
                    "estimated_tokens": 1000,
                    "context_window": 200_000,
                    "updated_at": "2026-01-01T00:00:00Z",
                    "checkpoint": checkpoint,
                }),
            )
            .unwrap();

        let before = session.messages.len();
        maybe_reinject_task_checkpoint(&kernel, agent_id, &mut session);
        assert_eq!(session.messages.len(), before);

        // Still consumed even though unused, so it doesn't leak into a
        // later mid-tool session.
        assert!(kernel
            .memory
            .structured_get(agent_id, TASK_CHECKPOINT_KEY)
            .unwrap()
            .is_none());

        kernel.shutdown();
    }
}

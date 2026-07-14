use crate::agent_loop::{AgentLoopResult, LoopPhase, PhaseCallback, ToolCallRecord};
use crate::embedding::EmbeddingDriver;
use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentManifest;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::memory::{Memory, MemorySource};
use captain_types::message::{Message, ReplyDirectives, TokenUsage};
use std::collections::HashMap;
use tracing::{info, warn};

pub(crate) async fn finish_silent_turn(
    session: &mut Session,
    memory: &MemorySubstrate,
    total_usage: TokenUsage,
    completed_iterations: u32,
    directives: ReplyDirectives,
    tool_calls_recorded: &[ToolCallRecord],
) -> CaptainResult<AgentLoopResult> {
    session
        .messages
        .push(Message::assistant("[no reply needed]".to_string()));
    memory
        .save_session_async(session)
        .await
        .map_err(|e| CaptainError::Memory(e.to_string()))?;

    Ok(AgentLoopResult {
        response: String::new(),
        total_usage,
        iterations: completed_iterations,
        cost_usd: None,
        silent: true,
        directives,
        tool_calls: tool_calls_recorded.to_vec(),
    })
}

pub(crate) struct SuccessfulTurnInput<'a> {
    pub(crate) manifest: &'a AgentManifest,
    pub(crate) user_message: &'a str,
    pub(crate) final_response: String,
    pub(crate) assistant_message: Message,
    pub(crate) completed_iterations: u32,
    pub(crate) session: &'a mut Session,
    pub(crate) memory: &'a MemorySubstrate,
    pub(crate) embedding_driver: Option<&'a (dyn EmbeddingDriver + Send + Sync)>,
    pub(crate) on_phase: Option<&'a PhaseCallback>,
    pub(crate) hooks: Option<&'a crate::hooks::HookRegistry>,
    pub(crate) agent_id_str: &'a str,
    pub(crate) total_usage: TokenUsage,
    pub(crate) tool_calls_recorded: &'a [ToolCallRecord],
    pub(crate) streaming: bool,
}

pub(crate) async fn finish_successful_turn(
    input: SuccessfulTurnInput<'_>,
) -> CaptainResult<AgentLoopResult> {
    let SuccessfulTurnInput {
        manifest,
        user_message,
        final_response,
        assistant_message,
        completed_iterations,
        session,
        memory,
        embedding_driver,
        on_phase,
        hooks,
        agent_id_str,
        total_usage,
        tool_calls_recorded,
        streaming,
    } = input;

    session.messages.push(assistant_message);
    crate::session_repair::prune_heartbeat_turns(&mut session.messages, 10);
    memory
        .save_session_async(session)
        .await
        .map_err(|e| CaptainError::Memory(e.to_string()))?;

    remember_interaction(
        session.agent_id,
        user_message,
        &final_response,
        memory,
        embedding_driver,
        streaming,
    )
    .await;

    if let Some(cb) = on_phase {
        cb(LoopPhase::Done);
    }

    if streaming {
        info!(
            agent = %manifest.name,
            iterations = completed_iterations,
            tokens = total_usage.total(),
            "Streaming agent loop completed"
        );
    } else {
        info!(
            agent = %manifest.name,
            iterations = completed_iterations,
            tokens = total_usage.total(),
            "Agent loop completed"
        );
    }

    fire_agent_loop_end_hook(
        hooks,
        manifest,
        agent_id_str,
        completed_iterations,
        final_response.len(),
    );

    Ok(AgentLoopResult {
        response: final_response,
        total_usage,
        iterations: completed_iterations,
        cost_usd: None,
        silent: false,
        directives: Default::default(),
        tool_calls: tool_calls_recorded.to_vec(),
    })
}

async fn remember_interaction(
    agent_id: captain_types::agent::AgentId,
    user_message: &str,
    final_response: &str,
    memory: &MemorySubstrate,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    streaming: bool,
) {
    let interaction_text = format!("User asked: {user_message}\nI responded: {final_response}");
    if let Some(emb) = embedding_driver {
        match emb.embed_one(&interaction_text).await {
            Ok(vec) => {
                let _ = memory
                    .remember_with_embedding_async(
                        agent_id,
                        &interaction_text,
                        MemorySource::Conversation,
                        "episodic",
                        HashMap::new(),
                        Some(&vec),
                    )
                    .await;
            }
            Err(e) => {
                if streaming {
                    warn!("Embedding for remember failed (streaming): {e}");
                } else {
                    warn!("Embedding for remember failed: {e}");
                }
                let _ = memory
                    .remember(
                        agent_id,
                        &interaction_text,
                        MemorySource::Conversation,
                        "episodic",
                        HashMap::new(),
                    )
                    .await;
            }
        }
        return;
    }

    let _ = memory
        .remember(
            agent_id,
            &interaction_text,
            MemorySource::Conversation,
            "episodic",
            HashMap::new(),
        )
        .await;
}

fn fire_agent_loop_end_hook(
    hooks: Option<&crate::hooks::HookRegistry>,
    manifest: &AgentManifest,
    agent_id_str: &str,
    completed_iterations: u32,
    response_length: usize,
) {
    if let Some(hook_reg) = hooks {
        let ctx = crate::hooks::HookContext {
            agent_name: &manifest.name,
            agent_id: agent_id_str,
            event: captain_types::agent::HookEvent::AgentLoopEnd,
            data: serde_json::json!({
                "iterations": completed_iterations,
                "response_length": response_length,
            }),
        };
        let _ = hook_reg.fire(&ctx);
    }
}

#[cfg(test)]
#[path = "agent_loop_finish_tests.rs"]
mod tests;

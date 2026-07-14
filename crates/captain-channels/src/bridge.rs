//! Channel bridge — connects channel adapters to the Captain kernel.
//!
//! Defines `ChannelBridgeHandle` (implemented by captain-api on the kernel) and
//! `BridgeManager` which owns running adapters and dispatches messages.

mod agent_error;
mod agent_resolution;
mod channel_mapping;
mod channel_policy;
mod command_agent;
mod command_agent_select;
mod command_automation;
mod command_dispatch;
mod command_format;
mod command_home;
mod command_model;
mod command_response;
mod command_review;
mod inbound_ack;
mod inbound_agent_activity;
mod inbound_agent_dispatch;
mod inbound_agent_input;
mod inbound_agent_preflight;
mod inbound_agent_result;
mod inbound_agent_send;
mod inbound_agent_target;
mod inbound_agent_turn;
mod inbound_audio;
mod inbound_authorization;
mod inbound_auto_reply;
mod inbound_broadcast;
mod inbound_command;
mod inbound_control;
mod inbound_dead_letter_ops;
mod inbound_delivery;
mod inbound_dispatch_settings;
mod inbound_error_response;
mod inbound_failure_response;
mod inbound_image;
mod inbound_image_blocks;
mod inbound_lifecycle;
mod inbound_media;
mod inbound_media_path;
mod inbound_mention;
mod inbound_prompt;
mod inbound_reresolution_retry;
mod inbound_retry_result;
mod inbound_session_agent;
mod inbound_session_cleanup;
mod inbound_success_response;
mod inbound_video;
mod model_switch_callback;
mod model_switch_decision;
mod model_switch_format;
mod model_switch_pending;
mod progress;
mod rate_limit;
mod routing;
mod sender_identity;

use crate::inbound_queue::InboundSessionQueue;
use crate::inbound_queue_types::{
    InboundStart, INBOUND_DEAD_LETTER_RETENTION_SECS, MAX_RECOVERED_INBOUND_ATTEMPTS,
};
use crate::router::AgentRouter;
use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage};
use async_trait::async_trait;
use captain_types::agent::AgentId;
use captain_types::config::ChannelOverrides;
use captain_types::message::ContentBlock;
use channel_mapping::channel_type_str;
use channel_policy::channel_policy_ignore_reason;
#[cfg(test)]
use command_dispatch::{handle_command, CommandContext};
use command_response::send_response;
use dashmap::DashMap;
use futures::StreamExt;
use inbound_ack::{send_inbound_interjection_ack, send_inbound_queued_ack};
use inbound_agent_dispatch::{dispatch_inbound_agent_turn, InboundAgentDispatchContext};
use inbound_agent_input::prepare_inbound_agent_input;
use inbound_agent_preflight::{
    prepare_inbound_agent_preflight, InboundAgentPreflight, InboundAgentPreflightContext,
};
use inbound_agent_target::{resolve_inbound_agent_dispatch_target, InboundAgentTargetContext};
use inbound_broadcast::{try_handle_inbound_broadcast, InboundBroadcastContext};
use inbound_command::{
    handle_inbound_command, try_handle_inbound_text_command, InboundCommandExecutionContext,
};
use inbound_control::{
    active_session_bypass_message, inbound_interjection_text, is_active_session_bypass_message,
};
use inbound_dispatch_settings::{resolve_inbound_dispatch_settings, InboundDispatchSettings};
use inbound_session_cleanup::InboundSessionCleanup;
use model_switch_pending::PendingModelSwitchStore;
use rate_limit::ChannelRateLimiter;
use sender_identity::sender_user_id;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{watch, Semaphore};
use tracing::{debug, info, warn};

type ChannelMessageStream = Pin<Box<dyn futures::Stream<Item = ChannelMessage> + Send>>;

/// Kernel operations needed by channel adapters.
///
/// Defined here to avoid circular deps (captain-channels can't depend on captain-kernel).
/// Implemented in captain-api on the actual kernel.
#[async_trait]
pub trait ChannelBridgeHandle: Send + Sync {
    /// Send a message to an agent and get the text response.
    /// `channel_type` identifies the source channel (e.g. "telegram", "discord").
    async fn send_message(
        &self,
        agent_id: AgentId,
        message: &str,
        channel_type: Option<&str>,
    ) -> Result<String, String>;

    /// Send a message with structured content blocks (text + images) to an agent.
    ///
    /// Default implementation extracts text from blocks and falls back to `send_message()`.
    async fn send_message_with_blocks(
        &self,
        agent_id: AgentId,
        blocks: Vec<ContentBlock>,
    ) -> Result<String, String> {
        // Default: extract text from blocks and send as plain text
        let text: String = blocks
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.send_message(agent_id, &text, None).await
    }

    /// Transcribe a channel-received audio file before it reaches the LLM.
    ///
    /// Direct channel normalization is preferred for voice messages: the LLM
    /// should reason over the transcript, not guess which STT tool/provider to
    /// use. Handles that do not expose media understanding return `Ok(None)`
    /// and the bridge falls back to a path-based prompt.
    async fn transcribe_channel_audio(
        &self,
        _path: &str,
        _language: Option<&str>,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// Describe a channel-received image before it reaches the LLM.
    ///
    /// This makes image handling model-independent: vision-capable engines can
    /// describe the saved local image once, then any downstream model can reason
    /// over the description and the durable file path. Handles without media
    /// understanding return `Ok(None)` and the bridge falls back to a path-based
    /// prompt.
    async fn describe_channel_image(
        &self,
        _path: &str,
        _prompt: Option<&str>,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// HS.3b — live streaming for Telegram. The dispatcher
    /// calls this *before* the regular `send_message` path; if it returns
    /// `Some`, the handle has already drained the agent's stream into the
    /// adapter (text edits + tool bubbles inline) and the caller MUST NOT
    /// post a final reply itself.
    ///
    /// Return values:
    /// - `None`         — the handle declined (channel disabled, agent not
    ///                    streamable, …). Caller falls back to
    ///                    `send_message`.
    /// - `Some(Ok(_))`  — streamed successfully. The String returned is
    ///                    the full assistant response captured for audit
    ///                    / persistence; the dispatcher logs it but does
    ///                    NOT re-post it (it's already on screen).
    /// - `Some(Err(e))` — streaming was attempted and failed. Caller may
    ///                    surface the error to the user; falling back to
    ///                    a non-streamed retry would risk duplicating
    ///                    text that may have already been edited live.
    ///
    /// Default implementation returns `None` — handles that don't know
    /// about Telegram (tests, alternate kernels) keep the legacy path.
    ///
    /// `user_message_id` (HS.7) is the Telegram message id of the user
    /// prompt that started this turn. The handle uses it to make the
    /// FIRST bubble of the agent's reply quote that message. `None` skips
    /// the reply marker (e.g. callback queries
    /// that don't have a parent text message).
    #[allow(clippy::too_many_arguments)]
    async fn try_stream_telegram_response(
        &self,
        _telegram: Arc<crate::telegram::TelegramAdapter>,
        _chat_id: i64,
        _thread_id: Option<i64>,
        _user_message_id: Option<i64>,
        _agent_id: AgentId,
        _session_key: Option<&str>,
        _message: &str,
    ) -> Option<Result<String, String>> {
        None
    }

    /// Try to inject a same-session follow-up into a running streaming turn.
    ///
    /// `Ok(false)` means the bridge should queue the message as the next turn.
    async fn try_interject_active_agent(
        &self,
        _agent_id: AgentId,
        _channel: &str,
        _session_key: &str,
        _message: &str,
        _platform_message_id: Option<&str>,
    ) -> Result<bool, String> {
        Ok(false)
    }

    /// TG2 — resolve a Telegram `ask_user` inline-keyboard button click.
    ///
    /// `short_id`/`idx` come from a `ask_user:<short_id>:<idx>` callback
    /// (see `telegram_callbacks::parse_ask_user_callback`). The handle owns
    /// the registry mapping `short_id` to the pending question's full
    /// option list and the stream/session it belongs to — resolving here
    /// (rather than in captain-channels) keeps that registry next to the
    /// `active_streams`/interjection machinery it reuses to deliver the
    /// answer, and avoids captain-channels depending on captain-api types.
    ///
    /// `Ok(chosen_text)` on success, `Err(reason)` if `short_id` is
    /// unknown/already answered/expired — shown to the user as-is rather
    /// than silently starting a new, unrelated agent turn.
    async fn try_answer_ask_user(&self, _short_id: &str, _idx: usize) -> Result<String, String> {
        Err("ask_user answer routing is not supported by this runtime".to_string())
    }

    /// Find an agent by name, returning its ID.
    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String>;

    /// List running agents as (id, name) pairs.
    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String>;

    /// Spawn an agent by manifest name, returning its ID.
    async fn spawn_agent_by_name(&self, manifest_name: &str) -> Result<AgentId, String>;

    /// Return uptime info string (e.g., "2h 15m, 5 agents").
    async fn uptime_info(&self) -> String {
        let agents = self.list_agents().await.unwrap_or_default();
        format!("{} agent(s) running", agents.len())
    }

    /// Handle global daemon commands before the LLM sees them.
    ///
    /// These commands are process/config controls (`/restart`, `/config`, ...)
    /// and must be implemented by the API bridge, not by individual agents.
    #[allow(clippy::too_many_arguments)]
    async fn daemon_command_text(
        &self,
        command: &str,
        _args: &[String],
        _channel_type: &str,
        _sender_platform_id: &str,
        _sender_user_id: &str,
        _thread_id: Option<&str>,
        _source_message_id: Option<&str>,
    ) -> String {
        if matches!(command, "status" | "health") {
            return self.uptime_info().await;
        }
        "Daemon command handling is not available for this runtime.".to_string()
    }

    /// List available models as formatted text for channel display.
    async fn list_models_text(&self) -> String {
        "Model listing not available.".to_string()
    }

    /// List providers and their auth status as formatted text for channel display.
    async fn list_providers_text(&self) -> String {
        "Provider listing not available.".to_string()
    }

    /// Reset an agent's session (clear messages, fresh session ID).
    async fn reset_session(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Trigger LLM-based session compaction for an agent.
    async fn compact_session(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Set an agent's model.
    async fn set_model(&self, _agent_id: AgentId, _model: &str) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Preflight a safe model/provider switch.
    async fn model_switch_plan(
        &self,
        _agent_id: AgentId,
        _target_model: &str,
    ) -> Result<serde_json::Value, String> {
        Err("Model switch preflight not available".to_string())
    }

    /// Apply a safe model/provider switch.
    async fn model_switch_apply(
        &self,
        _agent_id: AgentId,
        _target_model: &str,
        _target_provider: Option<&str>,
        _session_strategy: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        Err("Model switch apply not available".to_string())
    }

    /// Stop an agent's current LLM run.
    async fn stop_run(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Get session token usage and estimated cost.
    async fn session_usage(&self, _agent_id: AgentId) -> Result<String, String> {
        Err("Not implemented".to_string())
    }

    /// Toggle extended thinking mode for an agent.
    async fn set_thinking(&self, _agent_id: AgentId, _on: bool) -> Result<String, String> {
        Ok("Extended thinking preference saved.".to_string())
    }

    /// List installed skills as formatted text for channel display.
    async fn list_skills_text(&self) -> String {
        "Skill listing not available.".to_string()
    }

    /// List hands (marketplace + active) as formatted text for channel display.
    async fn list_hands_text(&self) -> String {
        "Hand listing not available.".to_string()
    }

    /// Authorize a channel user for an action.
    ///
    /// Returns Ok(()) if the user is allowed, Err(reason) if denied.
    /// Default implementation: allow all (RBAC disabled).
    async fn authorize_channel_user(
        &self,
        _channel_type: &str,
        _platform_id: &str,
        _action: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    /// Set the home channel for a user+channel pair (v3.8h).
    /// Home channels receive async cron results and proactive notifications
    /// by default when a recipient is not explicitly supplied.
    async fn set_home_channel(
        &self,
        _channel: &str,
        _user_platform_id: &str,
        _chat_id: &str,
    ) -> Result<String, String> {
        Err("Home channel not persisted (not implemented)".to_string())
    }

    /// Read the current home chat_id for a user on a given channel (v3.8h).
    async fn get_home_channel(&self, _channel: &str, _user_platform_id: &str) -> Option<String> {
        None
    }

    /// Get per-channel overrides for a given channel type.
    ///
    /// Returns `None` if the channel is not configured or has no overrides.
    async fn channel_overrides(&self, _channel_type: &str) -> Option<ChannelOverrides> {
        None
    }

    /// Get the agent assigned to a Telegram forum topic.
    /// Returns None if no agent is assigned to this topic.
    async fn get_agent_for_topic(&self, _thread_id: &str) -> Option<AgentId> {
        None
    }

    /// Assign an agent to a Telegram forum topic. Persisted across restarts.
    async fn set_topic_agent(&self, _thread_id: &str, _agent_id: AgentId) {}

    /// List all topic→agent mappings.
    async fn list_topic_mappings(&self) -> Vec<(String, AgentId, String)> {
        vec![]
    }

    /// Record a delivery result for tracking (optional — default no-op).
    ///
    /// `thread_id` preserves Telegram forum-topic context so cron/workflow
    /// delivery can target the same topic later.
    async fn record_delivery(
        &self,
        _agent_id: AgentId,
        _channel: &str,
        _recipient: &str,
        _success: bool,
        _error: Option<&str>,
        _thread_id: Option<&str>,
    ) {
        // Default: no tracking
    }

    /// Check if auto-reply is enabled and the message should trigger one.
    /// Returns Some(reply_text) if auto-reply fires, None otherwise.
    async fn check_auto_reply(&self, _agent_id: AgentId, _message: &str) -> Option<String> {
        None
    }

    /// Return a compact, evidence-based diagnostic context for the current
    /// agent. Used only when the user asks why an error happened, so the
    /// model answers from recent tool/session facts instead of guessing.
    async fn recent_agent_diagnostics(
        &self,
        _agent_id: AgentId,
        _channel_type: &str,
    ) -> Option<String> {
        None
    }

    // ── Automation: workflows, triggers, schedules, approvals ──

    /// List all registered workflows as formatted text.
    async fn list_workflows_text(&self) -> String {
        "Workflows not available.".to_string()
    }

    /// Run a workflow by name with the given input text.
    async fn run_workflow_text(&self, _name: &str, _input: &str) -> String {
        "Workflows not available.".to_string()
    }

    /// List all registered triggers as formatted text.
    async fn list_triggers_text(&self) -> String {
        "Triggers not available.".to_string()
    }

    /// Create a trigger for an agent with the given pattern and prompt.
    async fn create_trigger_text(
        &self,
        _agent_name: &str,
        _pattern: &str,
        _prompt: &str,
    ) -> String {
        "Triggers not available.".to_string()
    }

    /// Delete a trigger by UUID prefix.
    async fn delete_trigger_text(&self, _id_prefix: &str) -> String {
        "Triggers not available.".to_string()
    }

    /// List all cron jobs as formatted text.
    async fn list_schedules_text(&self) -> String {
        "Schedules not available.".to_string()
    }

    /// Manage a cron job: add, del, or run.
    async fn manage_schedule_text(&self, _action: &str, _args: &[String]) -> String {
        "Schedules not available.".to_string()
    }

    /// List pending approval requests as formatted text.
    async fn list_approvals_text(&self) -> String {
        "No approvals pending.".to_string()
    }

    /// Approve or reject a pending approval by UUID prefix (boolean form).
    /// Back-compat wrapper around `resolve_approval_text_with`.
    async fn resolve_approval_text(&self, id_prefix: &str, approve: bool) -> String {
        let decision = if approve {
            captain_types::approval::ApprovalDecision::Approved
        } else {
            captain_types::approval::ApprovalDecision::Denied
        };
        self.resolve_approval_text_with(id_prefix, decision).await
    }

    /// Q.11.b.2 — Resolve a pending approval with one of the 4
    /// decisions (Approved / ApprovedSession / ApprovedAlways / Denied).
    /// Override this in your bridge impl; the default returns a stub.
    async fn resolve_approval_text_with(
        &self,
        _id_prefix: &str,
        _decision: captain_types::approval::ApprovalDecision,
    ) -> String {
        "Approvals not available.".to_string()
    }

    /// List pending learning review items.
    async fn list_learning_review_text(&self) -> String {
        "Learning review not available.".to_string()
    }

    /// Approve or reject a pending learning candidate by id prefix.
    async fn resolve_learning_review_text(&self, _id_prefix: &str, _approve: bool) -> String {
        "Learning review not available.".to_string()
    }

    /// List pending generated-skill proposals.
    async fn list_skill_proposals_text(&self) -> String {
        "Skill proposals not available.".to_string()
    }

    /// Approve or reject a pending generated-skill proposal by id prefix.
    async fn resolve_skill_proposal_text(
        &self,
        _id_prefix: &str,
        _approve: bool,
        _external_validation: bool,
    ) -> String {
        "Skill proposals not available.".to_string()
    }

    /// List pending existing-skill refinement proposals.
    async fn list_skill_refinements_text(&self) -> String {
        "Skill refinements not available.".to_string()
    }

    /// Approve or reject an existing-skill refinement by id prefix.
    async fn resolve_skill_refinement_text(&self, _id_prefix: &str, _approve: bool) -> String {
        "Skill refinements not available.".to_string()
    }

    /// Answer a project-scoped ask_user request and wake the blocked worker.
    async fn resolve_project_ask_text(&self, _ask_id_prefix: &str, _answer: &str) -> String {
        "Project ask-user replies are not available.".to_string()
    }

    // ── Budget, Network, A2A ──

    /// Show global budget status (limits, spend, % used).
    async fn budget_text(&self) -> String {
        "Budget information not available.".to_string()
    }

    /// Show OFP peer network status.
    async fn peers_text(&self) -> String {
        "Peer network not available.".to_string()
    }

    /// List discovered external A2A agents.
    async fn a2a_agents_text(&self) -> String {
        "A2A agents not available.".to_string()
    }
}

/// Per-adapter task + its private shutdown signal.
///
/// Each running adapter owns its own `watch::Sender<bool>` so we can stop a
/// single channel (e.g. for hot-reload) without disturbing the others.
/// The adapter itself is kept so shutdown can call `ChannelAdapter::stop()`:
/// the dispatch loop and the adapter's internal poller (e.g. the Telegram
/// getUpdates task) run on separate shutdown signals, and stopping only the
/// dispatch loop leaks the poller — which then fights its replacement for
/// updates (409 Conflict storm observed live on config hot-reload).
struct AdapterEntry {
    adapter: Arc<dyn ChannelAdapter>,
    task: tokio::task::JoinHandle<()>,
    shutdown_tx: watch::Sender<bool>,
    /// Same semaphore handed to every detached `ChannelDispatchTask` spawned
    /// for this adapter. Each in-flight dispatch holds one permit for its
    /// entire lifetime, so re-acquiring all `DISPATCH_SEMAPHORE_PERMITS`
    /// permits after the dispatch loop stops is equivalent to waiting for
    /// every in-flight dispatch to finish — without a separate JoinSet.
    dispatch_semaphore: Arc<Semaphore>,
}

/// Concurrent dispatch cap per adapter, and the drain target used by
/// `AdapterEntry::shutdown` to wait for in-flight dispatches.
const DISPATCH_SEMAPHORE_PERMITS: u32 = 32;

/// How long `AdapterEntry::shutdown` waits for in-flight `ChannelDispatchTask`s
/// before giving up, so a stuck LLM call can't block shutdown/reload forever.
const DISPATCH_DRAIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl AdapterEntry {
    /// Stop this adapter and wait for its detached dispatch tasks to finish.
    ///
    /// Stops the adapter's own poller, signals the dispatch loop to stop
    /// reading new messages, awaits it, then waits for every outstanding
    /// dispatch permit to be released — bounded by `DISPATCH_DRAIN_TIMEOUT`
    /// so a hung dispatch can't block the caller indefinitely. Without this
    /// last step, `ChannelDispatchTask`s spawned for messages already read
    /// off the stream keep running detached after shutdown returns, which
    /// can race a replacement adapter or outlive process shutdown entirely.
    async fn shutdown(self, name: &str) {
        if let Err(e) = self.adapter.stop().await {
            warn!(adapter = %name, "adapter stop failed during shutdown: {e}");
        }
        let _ = self.shutdown_tx.send(true);
        let _ = self.task.await;

        match tokio::time::timeout(
            DISPATCH_DRAIN_TIMEOUT,
            self.dispatch_semaphore
                .acquire_many(DISPATCH_SEMAPHORE_PERMITS),
        )
        .await
        {
            Ok(Ok(_permits)) => {
                debug!(adapter = %name, "All in-flight dispatch tasks drained");
            }
            Ok(Err(_)) => {
                warn!(adapter = %name, "Dispatch semaphore closed unexpectedly while draining");
            }
            Err(_) => {
                warn!(
                    adapter = %name,
                    timeout_secs = DISPATCH_DRAIN_TIMEOUT.as_secs(),
                    "Timed out waiting for in-flight dispatch tasks to finish"
                );
            }
        }
    }
}

/// Owns all running channel adapters and dispatches messages to agents.
pub struct BridgeManager {
    handle: Arc<dyn ChannelBridgeHandle>,
    router: Arc<AgentRouter>,
    rate_limiter: ChannelRateLimiter,
    inbound_sessions: InboundSessionQueue,
    /// Telegram safe model-switch plans waiting for inline-keyboard choices.
    ///
    /// The kernel API does not expose a reusable `plan_id`; it revalidates on
    /// apply instead. This bridge-side cache is therefore the callback source
    /// of truth. Each entry carries a `created_at` timestamp; after
    /// `PENDING_MODEL_SWITCH_TTL` the plan is treated as expired and the user
    /// must relaunch `/model <name>` (lazy expiration on callback +
    /// opportunistic GC when a new plan is registered for the same agent).
    pending_model_switches: PendingModelSwitchStore,
    /// Running adapters keyed by adapter name (e.g. "telegram", "discord").
    /// Per-adapter shutdown lets `reload_channel` swap one without stopping
    /// the others.
    adapters: HashMap<String, AdapterEntry>,
}

impl BridgeManager {
    pub fn new(handle: Arc<dyn ChannelBridgeHandle>, router: Arc<AgentRouter>) -> Self {
        Self {
            handle,
            router,
            rate_limiter: ChannelRateLimiter::default(),
            inbound_sessions: InboundSessionQueue::default(),
            pending_model_switches: Arc::new(DashMap::new()),
            adapters: HashMap::new(),
        }
    }

    pub fn with_inbound_queue_path(
        handle: Arc<dyn ChannelBridgeHandle>,
        router: Arc<AgentRouter>,
        path: PathBuf,
    ) -> Self {
        Self {
            handle,
            router,
            rate_limiter: ChannelRateLimiter::default(),
            inbound_sessions: InboundSessionQueue::with_persistence(path),
            pending_model_switches: Arc::new(DashMap::new()),
            adapters: HashMap::new(),
        }
    }

    /// Return a reference to the underlying agent router.
    pub fn router(&self) -> &Arc<AgentRouter> {
        &self.router
    }

    pub fn inbound_queue_status(&self) -> serde_json::Value {
        let snapshot = self.inbound_sessions.snapshot();
        let channels: Vec<serde_json::Value> = snapshot
            .channels
            .into_iter()
            .map(|channel| {
                serde_json::json!({
                    "channel": channel.channel,
                    "active_sessions": channel.active_sessions,
                    "pending_sessions": channel.pending_sessions,
                    "pending_messages": channel.pending_messages,
                    "inflight_sessions": channel.inflight_sessions,
                    "inflight_messages": channel.inflight_messages,
                    "dead_letter_sessions": channel.dead_letter_sessions,
                    "dead_letter_messages": channel.dead_letter_messages,
                    "dead_letter_oldest_age_secs": channel.dead_letter_oldest_age_secs,
                    "interjected_sessions": channel.interjected_sessions,
                    "interjected_messages": channel.interjected_messages,
                })
            })
            .collect();

        serde_json::json!({
            "active_sessions": snapshot.active_sessions,
            "pending_sessions": snapshot.pending_sessions,
            "pending_messages": snapshot.pending_messages,
            "inflight_sessions": snapshot.inflight_sessions,
            "inflight_messages": snapshot.inflight_messages,
            "dead_letter_sessions": snapshot.dead_letter_sessions,
            "dead_letter_messages": snapshot.dead_letter_messages,
            "dead_letter_oldest_age_secs": snapshot.dead_letter_oldest_age_secs,
            "interjected_sessions": snapshot.interjected_sessions,
            "interjected_messages": snapshot.interjected_messages,
            "channels": channels,
            "operator_actions": {
                "dead_letter_clear_supported": true,
                "active_interjection_supported": true,
            },
            "recovery": {
                "max_recovered_attempts": MAX_RECOVERED_INBOUND_ATTEMPTS,
                "dead_letter_retention_secs": INBOUND_DEAD_LETTER_RETENTION_SECS,
            },
        })
    }

    /// Start an adapter: subscribe to its message stream and spawn a dispatch task.
    ///
    /// Each incoming message is dispatched as a concurrent task so that slow LLM
    /// calls (10-30s) don't block subsequent messages. This prevents voice/media
    /// messages sent in quick succession from appearing "lost" — all messages
    /// begin processing immediately. Per-agent serialization (to prevent session
    /// corruption) is handled by the kernel's `agent_msg_locks`.
    ///
    /// A semaphore limits concurrent dispatch tasks to prevent unbounded memory
    /// growth under burst traffic.
    ///
    /// If an adapter with the same name is already running it is stopped and
    /// awaited first — this keeps the manager's task table free of orphans.
    pub async fn start_adapter(
        &mut self,
        adapter: Arc<dyn ChannelAdapter>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let name = adapter.name().to_string();
        self.stop_replaced_adapter(&name).await;

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let stream = adapter.start().await?;
        let dispatch_semaphore = Arc::new(Semaphore::new(DISPATCH_SEMAPHORE_PERMITS as usize));
        let task = self.spawn_adapter_dispatch_loop(
            adapter.clone(),
            stream,
            shutdown_rx,
            dispatch_semaphore.clone(),
        );

        self.adapters.insert(
            name,
            AdapterEntry {
                adapter,
                task,
                shutdown_tx,
                dispatch_semaphore,
            },
        );
        Ok(())
    }

    async fn stop_replaced_adapter(&mut self, name: &str) {
        if let Some(prev) = self.adapters.remove(name) {
            warn!(adapter = %name, "Replacing already-running adapter via start_adapter");
            prev.shutdown(name).await;
        }
    }

    fn spawn_adapter_dispatch_loop(
        &self,
        adapter: Arc<dyn ChannelAdapter>,
        stream: ChannelMessageStream,
        shutdown_rx: watch::Receiver<bool>,
        semaphore: Arc<Semaphore>,
    ) -> tokio::task::JoinHandle<()> {
        AdapterDispatchLoop {
            adapter,
            stream,
            shutdown_rx,
            handle: self.handle.clone(),
            router: self.router.clone(),
            rate_limiter: self.rate_limiter.clone(),
            pending_model_switches: self.pending_model_switches.clone(),
            inbound_sessions: self.inbound_sessions.clone(),
            semaphore,
        }
        .spawn()
    }

    /// Hot-reload a single channel: stop the existing adapter for `name` (if
    /// any), drain its in-flight stream by awaiting its task, then spawn the
    /// new adapter. Other adapters are left untouched.
    ///
    /// The new adapter's `name()` must match `name` — otherwise the old
    /// adapter would silently leak under a different key.
    pub async fn reload_channel(
        &mut self,
        name: &str,
        new_adapter: Arc<dyn ChannelAdapter>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if new_adapter.name() != name {
            return Err(format!(
                "reload_channel name mismatch: expected '{name}', got '{}'",
                new_adapter.name()
            )
            .into());
        }

        if let Some(prev) = self.adapters.remove(name) {
            // Stop the adapter's own poller, then signal shutdown on this
            // adapter's private channel only — other adapters keep running.
            // Awaiting the task drains the dispatch loop's mpsc receiver,
            // so no message survives the swap.
            prev.shutdown(name).await;
            info!(adapter = %name, "reload_channel stopped previous adapter");
        }

        self.start_adapter(new_adapter).await
    }

    /// Stop all adapters (pollers included) and wait for dispatch tasks to finish.
    pub async fn stop(&mut self) {
        for (name, entry) in self.adapters.drain() {
            entry.shutdown(&name).await;
        }
    }
}

struct AdapterDispatchLoop {
    adapter: Arc<dyn ChannelAdapter>,
    stream: ChannelMessageStream,
    shutdown_rx: watch::Receiver<bool>,
    handle: Arc<dyn ChannelBridgeHandle>,
    router: Arc<AgentRouter>,
    rate_limiter: ChannelRateLimiter,
    pending_model_switches: PendingModelSwitchStore,
    inbound_sessions: InboundSessionQueue,
    semaphore: Arc<Semaphore>,
}

impl AdapterDispatchLoop {
    fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    async fn run(mut self) {
        self.dispatch_recovered_pending();
        loop {
            tokio::select! {
                msg = self.stream.next() => {
                    if !self.handle_next_message(msg) {
                        break;
                    }
                }
                _ = self.shutdown_rx.changed() => {
                    if *self.shutdown_rx.borrow() {
                        info!("Shutting down channel adapter {}", self.adapter.name());
                        break;
                    }
                }
            }
        }
    }

    fn dispatch_recovered_pending(&self) {
        for (key, pending) in self
            .inbound_sessions
            .recover_pending_for_channel(self.adapter.name())
        {
            self.spawn_dispatch(pending.message, Some(key));
        }
    }

    fn handle_next_message(&self, message: Option<ChannelMessage>) -> bool {
        match message {
            Some(message) => {
                self.spawn_dispatch(message, None);
                true
            }
            None => {
                info!("Channel adapter {} stream ended", self.adapter.name());
                false
            }
        }
    }

    fn spawn_dispatch(&self, message: ChannelMessage, recovered_key: Option<String>) {
        spawn_channel_dispatch_task(
            message,
            recovered_key,
            self.handle.clone(),
            self.router.clone(),
            self.adapter.clone(),
            self.rate_limiter.clone(),
            self.pending_model_switches.clone(),
            self.inbound_sessions.clone(),
            self.semaphore.clone(),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_channel_dispatch_task(
    message: ChannelMessage,
    recovered_key: Option<String>,
    handle: Arc<dyn ChannelBridgeHandle>,
    router: Arc<AgentRouter>,
    adapter: Arc<dyn ChannelAdapter>,
    rate_limiter: ChannelRateLimiter,
    pending_model_switches: PendingModelSwitchStore,
    inbound_sessions: InboundSessionQueue,
    semaphore: Arc<Semaphore>,
) {
    ChannelDispatchTask {
        message,
        recovered_key,
        handle,
        router,
        adapter,
        rate_limiter,
        pending_model_switches,
        inbound_sessions,
        semaphore,
    }
    .spawn();
}

struct ChannelDispatchTask {
    message: ChannelMessage,
    recovered_key: Option<String>,
    handle: Arc<dyn ChannelBridgeHandle>,
    router: Arc<AgentRouter>,
    adapter: Arc<dyn ChannelAdapter>,
    rate_limiter: ChannelRateLimiter,
    pending_model_switches: PendingModelSwitchStore,
    inbound_sessions: InboundSessionQueue,
    semaphore: Arc<Semaphore>,
}

impl ChannelDispatchTask {
    fn spawn(self) {
        tokio::spawn(async move {
            self.run().await;
        });
    }

    async fn run(self) {
        let _permit = match self.semaphore.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => return,
        };

        if self.try_resolve_ask_user_answer().await {
            return;
        }

        if let Some(key) = self.recovered_key.clone() {
            self.dispatch_started_message(self.message.clone(), key)
                .await;
            return;
        }
        if self.try_dispatch_active_session_bypass().await {
            return;
        }

        let sender_user_id = sender_user_id(&self.message).to_string();
        let key = self
            .inbound_sessions
            .session_key(&self.message, &sender_user_id);
        if self.try_forward_active_interjection(&key).await {
            return;
        }
        self.start_or_queue_inbound_session(key).await;
    }

    /// TG2 — resolve an `ask_user` inline-keyboard click before any of the
    /// generic bypass/session-key/interjection/queueing machinery below
    /// runs. This message is synthesized by
    /// `ask_user_answer_callback_message` purely to carry structured data
    /// (`ask_user_short_id`/`ask_user_idx` metadata) through the channel
    /// pipeline — it is NOT a real user reply and must never be treated as
    /// one.
    ///
    /// Discovered live: routing it through the normal
    /// `try_forward_active_interjection` path (below) forwards its
    /// placeholder `Text` content verbatim as if the human had typed it,
    /// since that path has no notion of this metadata. Worse, if no
    /// interjection is currently accepted it would fall through to
    /// `start_or_queue_inbound_session`, which — because the very stream
    /// this answer is meant for is the one blocked waiting for it — would
    /// deadlock: the queued turn can't start until the active one
    /// finishes, and the active one can't finish without this answer.
    /// `try_answer_ask_user` sidesteps all of that: it already has its own
    /// `short_id -> (session_key, options, agent_id)` registry from when
    /// the question was posted, so it resolves and delivers the answer
    /// directly, independent of the generic session/queue state.
    async fn try_resolve_ask_user_answer(&self) -> bool {
        let Some(short_id) = self
            .message
            .metadata
            .get("ask_user_short_id")
            .and_then(|v| v.as_str())
        else {
            return false;
        };
        let idx = self
            .message
            .metadata
            .get("ask_user_idx")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        // On success there is nothing to post here: the agent-loop turn
        // this unblocks will produce its own reply once it resumes. On
        // failure (stale/already-answered/expired), tell the clicking user
        // directly — there is no live turn left to piggyback a reply on.
        if let Err(reason) = self.handle.try_answer_ask_user(short_id, idx).await {
            let _ = self
                .adapter
                .send(&self.message.sender, ChannelContent::Text(reason))
                .await;
        }
        true
    }

    async fn dispatch_started_message(&self, message: ChannelMessage, key: String) {
        dispatch_message(
            message,
            key,
            self.inbound_sessions.clone(),
            &self.handle,
            &self.router,
            &self.adapter,
            &self.rate_limiter,
            &self.pending_model_switches,
        )
        .await;
    }

    async fn try_dispatch_active_session_bypass(&self) -> bool {
        if !is_active_session_bypass_message(&self.message) {
            return false;
        }
        let Some(bypass_message) = active_session_bypass_message(&self.message) else {
            return true;
        };
        dispatch_message_once(
            &bypass_message,
            &self.handle,
            &self.router,
            self.adapter.as_ref(),
            &self.adapter,
            &self.rate_limiter,
            &self.pending_model_switches,
            None,
        )
        .await;
        true
    }

    async fn try_forward_active_interjection(&self, key: &str) -> bool {
        let (Some(agent_id), Some(interjection)) = (
            self.inbound_sessions.active_agent(key),
            inbound_interjection_text(&self.message),
        ) else {
            return false;
        };

        match self
            .handle
            .try_interject_active_agent(
                agent_id,
                self.adapter.name(),
                key,
                &interjection,
                Some(&self.message.platform_message_id),
            )
            .await
        {
            Ok(true) => {
                self.inbound_sessions.record_interjection(key);
                debug!(
                    channel = %self.adapter.name(),
                    sender = %self.message.sender.platform_id,
                    agent_id = %agent_id,
                    "Forwarded inbound follow-up as active interjection"
                );
                if self.inbound_sessions.should_ack_active_interjection(key) {
                    send_inbound_interjection_ack(
                        &self.message,
                        &self.handle,
                        self.adapter.as_ref(),
                    )
                    .await;
                }
                true
            }
            Ok(false) => false,
            Err(err) => {
                debug!(
                    channel = %self.adapter.name(),
                    sender = %self.message.sender.platform_id,
                    agent_id = %agent_id,
                    "Active interjection failed, keeping follow-up queued: {err}"
                );
                false
            }
        }
    }

    async fn start_or_queue_inbound_session(&self, key: String) {
        match self
            .inbound_sessions
            .start_or_queue(key.clone(), self.message.clone())
        {
            InboundStart::Started { key } => {
                self.dispatch_started_message(self.message.clone(), key)
                    .await;
            }
            InboundStart::Queued(summary) => {
                if summary.ack_recommended {
                    send_inbound_queued_ack(&self.message, &self.handle, self.adapter.as_ref())
                        .await;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_message(
    mut message: ChannelMessage,
    key: String,
    inbound_sessions: InboundSessionQueue,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    adapter_arc: &Arc<dyn ChannelAdapter>,
    rate_limiter: &ChannelRateLimiter,
    pending_model_switches: &PendingModelSwitchStore,
) {
    let mut cleanup = InboundSessionCleanup::new(inbound_sessions.clone(), key.clone());
    loop {
        dispatch_message_once(
            &message,
            handle,
            router,
            adapter_arc.as_ref(),
            adapter_arc,
            rate_limiter,
            pending_model_switches,
            Some((&inbound_sessions, &key)),
        )
        .await;

        match inbound_sessions.next_or_finish(&key) {
            Some(pending) => {
                message = pending.message;
            }
            None => {
                cleanup.disarm();
                break;
            }
        }
    }
}

struct InboundMessageDispatchContext<'a> {
    message: &'a ChannelMessage,
    handle: &'a Arc<dyn ChannelBridgeHandle>,
    router: &'a Arc<AgentRouter>,
    adapter: &'a dyn ChannelAdapter,
    adapter_arc: &'a Arc<dyn ChannelAdapter>,
    rate_limiter: &'a ChannelRateLimiter,
    pending_model_switches: &'a PendingModelSwitchStore,
    active_session: Option<(&'a InboundSessionQueue, &'a str)>,
    channel_type: &'a str,
    overrides: Option<ChannelOverrides>,
    settings: InboundDispatchSettings<'a>,
}

impl<'a> InboundMessageDispatchContext<'a> {
    #[allow(clippy::too_many_arguments)]
    async fn new(
        message: &'a ChannelMessage,
        handle: &'a Arc<dyn ChannelBridgeHandle>,
        router: &'a Arc<AgentRouter>,
        adapter: &'a dyn ChannelAdapter,
        adapter_arc: &'a Arc<dyn ChannelAdapter>,
        rate_limiter: &'a ChannelRateLimiter,
        pending_model_switches: &'a PendingModelSwitchStore,
        active_session: Option<(&'a InboundSessionQueue, &'a str)>,
    ) -> Self {
        let channel_type = channel_type_str(&message.channel);
        let overrides = handle.channel_overrides(channel_type).await;
        let settings = resolve_inbound_dispatch_settings(
            channel_type,
            message.thread_id.as_deref(),
            overrides.as_ref(),
        );

        Self {
            message,
            handle,
            router,
            adapter,
            adapter_arc,
            rate_limiter,
            pending_model_switches,
            active_session,
            channel_type,
            overrides,
            settings,
        }
    }

    fn command_context(&self) -> InboundCommandExecutionContext<'_> {
        InboundCommandExecutionContext {
            handle: self.handle,
            router: self.router,
            adapter: self.adapter,
            sender: &self.message.sender,
            sender_user_id: sender_user_id(self.message),
            channel: self.channel_type,
            thread_id: self.settings.thread_id,
            source_message_id: Some(&self.message.platform_message_id),
            output_format: self.settings.output_format,
            pending_model_switches: self.pending_model_switches,
        }
    }

    fn broadcast_context<'b>(&'b self, text: &'b str) -> InboundBroadcastContext<'b> {
        InboundBroadcastContext {
            handle: self.handle,
            router: self.router,
            adapter: self.adapter,
            adapter_arc: self.adapter_arc.clone(),
            message: self.message,
            sender_user_id: sender_user_id(self.message),
            text,
            channel_type: self.channel_type,
            thread_id: self.settings.thread_id,
            output_format: self.settings.output_format,
        }
    }

    fn target_context<'b>(&'b self, text: &'b str) -> InboundAgentTargetContext<'b> {
        InboundAgentTargetContext {
            handle: self.handle,
            router: self.router,
            adapter: self.adapter,
            message: self.message,
            text,
            thread_id: self.settings.thread_id,
            output_format: self.settings.output_format,
            preferred_fallback_name: "captain",
        }
    }

    fn preflight_context<'b>(
        &'b self,
        agent_id: AgentId,
        text: &'b str,
    ) -> InboundAgentPreflightContext<'b> {
        InboundAgentPreflightContext {
            handle: self.handle,
            adapter: self.adapter,
            message: self.message,
            agent_id,
            text,
            active_session: self.active_session,
            sender_user_id: sender_user_id(self.message),
            channel_type: self.channel_type,
            thread_id: self.settings.thread_id,
            output_format: self.settings.output_format,
        }
    }

    fn agent_dispatch_context<'b>(
        &'b self,
        agent_id: AgentId,
        image_blocks_for_agent: Option<&'b [ContentBlock]>,
        text: &'b str,
        text_for_agent: &'b str,
        active_session_key: Option<&'b str>,
    ) -> InboundAgentDispatchContext<'b> {
        InboundAgentDispatchContext {
            handle: self.handle,
            router: self.router,
            message: self.message,
            adapter: self.adapter,
            adapter_arc: self.adapter_arc,
            agent_id,
            image_blocks_for_agent,
            text,
            text_for_agent,
            active_session_key,
            channel_type: self.channel_type,
            thread_id: self.settings.thread_id,
            output_format: self.settings.output_format,
            lifecycle_reactions: self.settings.lifecycle_reactions,
        }
    }
}

/// Dispatch a single incoming message — handles bot commands or routes to an agent.
///
/// Applies per-channel policies (DM/group filtering, rate limiting, formatting, threading).
#[allow(clippy::too_many_arguments)]
async fn dispatch_message_once(
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    router: &Arc<AgentRouter>,
    adapter: &dyn ChannelAdapter,
    adapter_arc: &Arc<dyn ChannelAdapter>,
    rate_limiter: &ChannelRateLimiter,
    pending_model_switches: &PendingModelSwitchStore,
    active_session: Option<(&InboundSessionQueue, &str)>,
) {
    let ctx = InboundMessageDispatchContext::new(
        message,
        handle,
        router,
        adapter,
        adapter_arc,
        rate_limiter,
        pending_model_switches,
        active_session,
    )
    .await;

    if inbound_message_is_ignored(&ctx) {
        return;
    }
    if !inbound_rate_limit_allows(&ctx).await {
        return;
    }

    if try_handle_native_inbound_command(&ctx).await {
        return;
    }

    let agent_input = prepare_inbound_agent_input(handle, message, ctx.channel_type).await;

    if try_handle_inbound_text_command(&agent_input.text, ctx.command_context()).await {
        return;
    }
    // Other slash commands pass through to the agent.

    if try_handle_inbound_broadcast(ctx.broadcast_context(&agent_input.text)).await {
        return;
    }

    let Some(agent_target) =
        resolve_inbound_agent_dispatch_target(ctx.target_context(&agent_input.text)).await
    else {
        return;
    };
    let agent_id = agent_target.agent_id;

    let active_session_key =
        match prepare_inbound_agent_preflight(ctx.preflight_context(agent_id, &agent_input.text))
            .await
        {
            InboundAgentPreflight::Continue { active_session_key } => active_session_key,
            InboundAgentPreflight::Stop => return,
        };

    dispatch_inbound_agent_turn(ctx.agent_dispatch_context(
        agent_id,
        agent_input.image_blocks_for_agent.as_deref(),
        &agent_input.text,
        &agent_target.text_for_agent,
        active_session_key,
    ))
    .await;
}

fn inbound_message_is_ignored(ctx: &InboundMessageDispatchContext<'_>) -> bool {
    let Some(reason) = channel_policy_ignore_reason(ctx.message, ctx.overrides.as_ref()) else {
        return false;
    };
    debug!("{}", reason.debug_message(ctx.channel_type));
    true
}

async fn inbound_rate_limit_allows(ctx: &InboundMessageDispatchContext<'_>) -> bool {
    let Some(overrides) = ctx.overrides.as_ref() else {
        return true;
    };
    if overrides.rate_limit_per_user == 0 {
        return true;
    }
    match ctx.rate_limiter.check(
        ctx.channel_type,
        sender_user_id(ctx.message),
        overrides.rate_limit_per_user,
    ) {
        Ok(()) => true,
        Err(message) => {
            send_response(
                ctx.adapter,
                &ctx.message.sender,
                message,
                ctx.settings.thread_id,
                ctx.settings.output_format,
            )
            .await;
            false
        }
    }
}

async fn try_handle_native_inbound_command(ctx: &InboundMessageDispatchContext<'_>) -> bool {
    let ChannelContent::Command { name, args } = &ctx.message.content else {
        return false;
    };
    handle_inbound_command(name, args, ctx.command_context()).await;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelType, ChannelUser};
    use captain_types::config::OutputFormat;
    use std::sync::Mutex;

    /// Mock kernel handle for testing.
    struct MockHandle {
        agents: Mutex<Vec<(AgentId, String)>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(format!("Echo: {message}"))
        }
        async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
            let agents = self.agents.lock().unwrap();
            Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
        }
        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(self.agents.lock().unwrap().clone())
        }
        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("spawn not implemented in mock".to_string())
        }
    }

    fn test_pending_model_switches() -> PendingModelSwitchStore {
        Arc::new(DashMap::new())
    }

    fn test_message(content: ChannelContent) -> ChannelMessage {
        ChannelMessage {
            channel: crate::types::ChannelType::Telegram,
            platform_message_id: "m1".to_string(),
            sender: ChannelUser {
                platform_id: "chat-1".to_string(),
                display_name: "Ada".to_string(),
                captain_user: Some("captain-user".to_string()),
            },
            content,
            target_agent: None,
            timestamp: chrono::Utc::now(),
            is_group: false,
            thread_id: Some("topic-1".to_string()),
            metadata: HashMap::new(),
        }
    }

    /// TG2 — records `try_answer_ask_user` calls and returns a configured
    /// result, so tests can assert on the resolver's routing without a real
    /// registry/interjection channel.
    struct AskUserHandle {
        answer: Result<String, String>,
        calls: Mutex<Vec<(String, usize)>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for AskUserHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(format!("Echo: {message}"))
        }
        async fn try_answer_ask_user(&self, short_id: &str, idx: usize) -> Result<String, String> {
            self.calls.lock().unwrap().push((short_id.to_string(), idx));
            self.answer.clone()
        }
        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(None)
        }
        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(Vec::new())
        }
        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("spawn not implemented in mock".to_string())
        }
    }

    /// Records every `send` call so tests can assert whether (and what) a
    /// reply was posted directly, bypassing the normal turn/reply pipeline.
    struct RecordingAdapter {
        sent: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording-test-adapter"
        }
        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }
        async fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn futures::Stream<Item = ChannelMessage> + Send>>,
            Box<dyn std::error::Error>,
        > {
            Ok(Box::pin(futures::stream::empty()))
        }
        async fn send(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if let ChannelContent::Text(text) = content {
                self.sent.lock().unwrap().push(text);
            }
            Ok(())
        }
        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    fn ask_user_dispatch_task(
        handle: Arc<AskUserHandle>,
        adapter: Arc<RecordingAdapter>,
        short_id: &str,
        idx: u64,
    ) -> ChannelDispatchTask {
        let mut message = test_message(ChannelContent::Text("[ask_user answer]".to_string()));
        message
            .metadata
            .insert("ask_user_short_id".to_string(), serde_json::json!(short_id));
        message
            .metadata
            .insert("ask_user_idx".to_string(), serde_json::json!(idx));
        ChannelDispatchTask {
            message,
            recovered_key: None,
            handle,
            router: Arc::new(AgentRouter::new()),
            adapter,
            rate_limiter: ChannelRateLimiter::default(),
            pending_model_switches: test_pending_model_switches(),
            inbound_sessions: InboundSessionQueue::default(),
            semaphore: Arc::new(Semaphore::new(1)),
        }
    }

    #[tokio::test]
    async fn ask_user_answer_resolves_without_posting_a_reply() {
        let handle = Arc::new(AskUserHandle {
            answer: Ok("bleu".to_string()),
            calls: Mutex::new(Vec::new()),
        });
        let adapter = Arc::new(RecordingAdapter {
            sent: Mutex::new(Vec::new()),
        });
        let task = ask_user_dispatch_task(Arc::clone(&handle), Arc::clone(&adapter), "short-1", 2);

        assert!(task.try_resolve_ask_user_answer().await);

        assert_eq!(
            *handle.calls.lock().unwrap(),
            vec![("short-1".to_string(), 2)]
        );
        // Success needs no direct reply — the unblocked agent turn posts
        // its own response once it resumes.
        assert!(adapter.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn stale_ask_user_answer_is_reported_directly_without_a_new_turn() {
        let handle = Arc::new(AskUserHandle {
            answer: Err("Cette question n'est plus active.".to_string()),
            calls: Mutex::new(Vec::new()),
        });
        let adapter = Arc::new(RecordingAdapter {
            sent: Mutex::new(Vec::new()),
        });
        let task = ask_user_dispatch_task(handle, Arc::clone(&adapter), "stale-id", 0);

        assert!(task.try_resolve_ask_user_answer().await);

        assert_eq!(
            *adapter.sent.lock().unwrap(),
            vec!["Cette question n'est plus active.".to_string()]
        );
    }

    #[tokio::test]
    async fn non_ask_user_messages_are_not_intercepted() {
        let handle = Arc::new(AskUserHandle {
            answer: Ok("unused".to_string()),
            calls: Mutex::new(Vec::new()),
        });
        let adapter = Arc::new(RecordingAdapter {
            sent: Mutex::new(Vec::new()),
        });
        let task = ChannelDispatchTask {
            message: test_message(ChannelContent::Text("hello".to_string())),
            recovered_key: None,
            handle,
            router: Arc::new(AgentRouter::new()),
            adapter,
            rate_limiter: ChannelRateLimiter::default(),
            pending_model_switches: test_pending_model_switches(),
            inbound_sessions: InboundSessionQueue::default(),
            semaphore: Arc::new(Semaphore::new(1)),
        };

        assert!(!task.try_resolve_ask_user_answer().await);
    }

    #[test]
    fn bridge_manager_clears_dead_letters_without_exposing_content() {
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(Vec::new()),
        });
        let manager = BridgeManager::new(handle, Arc::new(AgentRouter::new()));
        let first = test_message(ChannelContent::Text("premier".to_string()));
        let key = manager
            .inbound_sessions
            .session_key(&first, sender_user_id(&first));
        assert!(matches!(
            manager.inbound_sessions.start_or_queue(key.clone(), first),
            InboundStart::Started { .. }
        ));
        let second = test_message(ChannelContent::Text("a récupérer".to_string()));
        assert!(matches!(
            manager.inbound_sessions.start_or_queue(key.clone(), second),
            InboundStart::Queued(_)
        ));

        for _ in 0..=MAX_RECOVERED_INBOUND_ATTEMPTS {
            let _ = manager
                .inbound_sessions
                .recover_pending_for_channel("telegram");
        }
        assert_eq!(
            manager.inbound_queue_status()["dead_letter_messages"],
            serde_json::json!(1)
        );

        let cleared = manager.clear_inbound_dead_letters(Some("telegram"));
        assert_eq!(
            cleared["cleared_dead_letter_sessions"],
            serde_json::json!(1)
        );
        assert_eq!(
            cleared["cleared_dead_letter_messages"],
            serde_json::json!(1)
        );
        assert_eq!(cleared["channel"], serde_json::json!("telegram"));
        assert_eq!(
            cleared["remaining_dead_letter_messages"],
            serde_json::json!(0)
        );
        assert!(cleared.get("message").is_none());
        assert!(cleared.get("content").is_none());
        assert_eq!(
            manager.inbound_queue_status()["dead_letter_messages"],
            serde_json::json!(0)
        );
        assert_eq!(
            manager.inbound_queue_status()["operator_actions"]["dead_letter_clear_supported"],
            serde_json::json!(true)
        );
    }

    #[tokio::test]
    async fn test_dispatch_routes_to_correct_agent() {
        let agent_id = AgentId::new();
        let mock = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "test-agent".to_string())]),
        });

        let handle: Arc<dyn ChannelBridgeHandle> = mock;

        // Verify find_agent_by_name works
        let found = handle.find_agent_by_name("test-agent").await.unwrap();
        assert_eq!(found, Some(agent_id));

        let not_found = handle.find_agent_by_name("nonexistent").await.unwrap();
        assert_eq!(not_found, None);

        // Verify send_message echoes
        let response = handle.send_message(agent_id, "hello", None).await.unwrap();
        assert_eq!(response, "Echo: hello");
    }

    #[tokio::test]
    async fn test_handle_command_agents() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let router = Arc::new(AgentRouter::new());
        let sender = ChannelUser {
            platform_id: "user1".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        };

        let pending_model_switches = test_pending_model_switches();
        let result = handle_command(
            "agents",
            &[],
            CommandContext {
                handle: &handle,
                router: &router,
                sender: &sender,
                sender_user_id: &sender.platform_id,
                channel: "telegram",
                thread_id: None,
                source_message_id: None,
                pending_model_switches: &pending_model_switches,
            },
        )
        .await;
        assert!(result.contains("coder"));

        let result = handle_command(
            "help",
            &[],
            CommandContext {
                handle: &handle,
                router: &router,
                sender: &sender,
                sender_user_id: &sender.platform_id,
                channel: "telegram",
                thread_id: None,
                source_message_id: None,
                pending_model_switches: &pending_model_switches,
            },
        )
        .await;
        assert!(result.contains("/agents"));
    }

    #[tokio::test]
    async fn test_handle_command_agent_select() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "coder".to_string())]),
        });
        let router = Arc::new(AgentRouter::new());
        let sender = ChannelUser {
            platform_id: "user1".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        };

        // Select existing agent
        let pending_model_switches = test_pending_model_switches();
        let result = handle_command(
            "agent",
            &["coder".to_string()],
            CommandContext {
                handle: &handle,
                router: &router,
                sender: &sender,
                sender_user_id: &sender.platform_id,
                channel: "telegram",
                thread_id: None,
                source_message_id: None,
                pending_model_switches: &pending_model_switches,
            },
        )
        .await;
        assert!(result.contains("Now talking to agent: coder"));

        // Verify router was updated
        let resolved = router.resolve(&ChannelType::Telegram, "user1", None);
        assert_eq!(resolved, Some(agent_id));
    }

    #[test]
    fn test_dm_policy_filtering() {
        use captain_types::config::{DmPolicy, GroupPolicy};

        // Test that DmPolicy::Ignore would be checked
        assert_eq!(DmPolicy::default(), DmPolicy::Respond);
        assert_eq!(GroupPolicy::default(), GroupPolicy::MentionOnly);
    }

    #[test]
    fn test_channel_type_str() {
        assert_eq!(channel_type_str(&ChannelType::Telegram), "telegram");
        assert_eq!(channel_type_str(&ChannelType::Matrix), "matrix");
        assert_eq!(channel_type_str(&ChannelType::Email), "email");
        assert_eq!(
            channel_type_str(&ChannelType::Custom("irc".to_string())),
            "irc"
        );
    }

    #[test]
    fn test_default_output_format_for_channel() {
        assert_eq!(
            channel_mapping::default_output_format_for_channel("telegram"),
            OutputFormat::TelegramHtml
        );
        assert_eq!(
            channel_mapping::default_output_format_for_channel("slack"),
            OutputFormat::SlackMrkdwn
        );
        assert_eq!(
            channel_mapping::default_output_format_for_channel("wecom"),
            OutputFormat::PlainText
        );
        assert_eq!(
            channel_mapping::default_output_format_for_channel("discord"),
            OutputFormat::Markdown
        );
    }

    #[tokio::test]
    async fn test_send_message_with_blocks_default_fallback() {
        // The default implementation of send_message_with_blocks extracts text
        // from blocks and calls send_message
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "vision-agent".to_string())]),
        });

        let blocks = vec![
            ContentBlock::Text {
                text: "What is in this photo?".to_string(),
                provider_metadata: None,
            },
            ContentBlock::Image {
                media_type: "image/jpeg".to_string(),
                data: "base64data".to_string(),
            },
        ];

        // Default impl should extract text and call send_message
        let result = handle
            .send_message_with_blocks(agent_id, blocks)
            .await
            .unwrap();
        assert_eq!(result, "Echo: What is in this photo?");
    }

    #[tokio::test]
    async fn test_send_message_with_blocks_image_only() {
        // When there's no text block, the default should still work
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockHandle {
            agents: Mutex::new(vec![(agent_id, "vision-agent".to_string())]),
        });

        let blocks = vec![ContentBlock::Image {
            media_type: "image/png".to_string(),
            data: "base64data".to_string(),
        }];

        // Default impl sends empty text when no text blocks
        let result = handle
            .send_message_with_blocks(agent_id, blocks)
            .await
            .unwrap();
        assert_eq!(result, "Echo: ");
    }

    // ── AdapterEntry::shutdown must drain detached ChannelDispatchTasks ──

    /// Mock handle whose `send_message` sleeps before replying, simulating a
    /// slow LLM call that's still running when shutdown is requested.
    struct SlowSendHandle {
        delay: std::time::Duration,
        agent_id: AgentId,
    }

    #[async_trait]
    impl ChannelBridgeHandle for SlowSendHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            tokio::time::sleep(self.delay).await;
            Ok(format!("Echo: {message}"))
        }
        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(Some(self.agent_id))
        }
        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(vec![(self.agent_id, "test-agent".to_string())])
        }
        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("spawn not implemented in mock".to_string())
        }
    }

    /// Adapter whose `send` flips an `AtomicBool` once called, so the test
    /// can observe whether the detached dispatch task actually completed
    /// before `BridgeManager::stop` returns.
    struct FlagAdapter {
        messages: Mutex<Option<Vec<ChannelMessage>>>,
        sent: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl ChannelAdapter for FlagAdapter {
        fn name(&self) -> &str {
            "flag-test-adapter"
        }
        fn channel_type(&self) -> ChannelType {
            ChannelType::Custom("flag-test".to_string())
        }
        async fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn futures::Stream<Item = ChannelMessage> + Send>>,
            Box<dyn std::error::Error>,
        > {
            let messages = self.messages.lock().unwrap().take().unwrap_or_default();
            Ok(Box::pin(futures::stream::iter(messages)))
        }
        async fn send(
            &self,
            _user: &ChannelUser,
            _content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            self.sent.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn stop_waits_for_in_flight_detached_dispatch_tasks() {
        let agent_id = AgentId::new();
        let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(SlowSendHandle {
            delay: std::time::Duration::from_millis(300),
            agent_id,
        });
        let router = Arc::new(AgentRouter::new());
        let mut manager = BridgeManager::new(handle, router);

        let message = test_message(ChannelContent::Text("hello".to_string()));

        let sent = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let adapter: Arc<dyn ChannelAdapter> = Arc::new(FlagAdapter {
            messages: Mutex::new(Some(vec![message])),
            sent: sent.clone(),
        });

        manager
            .start_adapter(adapter)
            .await
            .expect("start_adapter must succeed");

        // Give the stream-reading task a moment to pick up the message and
        // spawn the detached dispatch task, but not enough time for the
        // 300ms `send_message` delay to finish.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            !sent.load(std::sync::atomic::Ordering::SeqCst),
            "dispatch should still be in flight before stop()"
        );

        manager.stop().await;

        assert!(
            sent.load(std::sync::atomic::Ordering::SeqCst),
            "stop() must wait for detached dispatch tasks to finish before returning"
        );
    }
}

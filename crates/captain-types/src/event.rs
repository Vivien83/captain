//! Event types for the Captain internal event bus.
//!
//! All inter-agent and system communication flows through events.

use crate::agent::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

/// Serde helper for `Option<Duration>` as milliseconds.
mod duration_ms {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    /// Serialize `Duration` as `u64` milliseconds.
    pub fn serialize<S: Serializer>(dur: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match dur {
            Some(d) => d.as_millis().serialize(s),
            None => s.serialize_none(),
        }
    }

    /// Deserialize `u64` milliseconds into `Duration`.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let opt: Option<u64> = Option::deserialize(d)?;
        Ok(opt.map(Duration::from_millis))
    }
}

/// Unique identifier for an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub Uuid);

impl EventId {
    /// Create a new random EventId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TriggerId(pub Uuid);

impl TriggerId {
    /// Create a new random TriggerId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TriggerId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for TriggerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for TriggerId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(value)?))
    }
}

/// Where an event is directed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum EventTarget {
    /// Send to a specific agent.
    Agent(AgentId),
    /// Broadcast to all agents.
    Broadcast,
    /// Send to agents matching a pattern (e.g., tag-based).
    Pattern(String),
    /// Send to the kernel/system.
    System,
}

/// The payload of an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum EventPayload {
    /// Direct agent-to-agent message.
    Message(AgentMessage),
    /// Tool execution result.
    ToolResult(ToolOutput),
    /// Memory changed notification.
    MemoryUpdate(MemoryDelta),
    /// Agent lifecycle event.
    Lifecycle(LifecycleEvent),
    /// Network event (remote agent activity).
    Network(NetworkEvent),
    /// System event (health, resources).
    System(SystemEvent),
    /// User-defined payload.
    Custom(Vec<u8>),
    /// Chat stream event for real-time sync across WebSocket connections.
    ChatStream(ChatStreamEvent),
    /// A detached tool_run (execute_code/shell_exec/... started via
    /// tool_run_start) changed status. Lets the caller agent's turn and
    /// external observers (TUI, SSE) learn about background work without
    /// polling tool_run_status.
    ToolRun(ToolRunEvent),
}

/// Status change of a detached `tool_run` (see `captain-runtime::tool_runs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolRunEvent {
    pub run_id: String,
    pub tool_name: String,
    pub status: String,
    pub caller_agent_id: Option<String>,
}

/// Real-time chat events broadcast to all connected WebSocket clients.
///
/// These events mirror the internal `StreamEvent` but are serializable
/// and designed for fan-out via the EventBus to multiple browser tabs,
/// enabling multi-client synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "chat_event")]
pub enum ChatStreamEvent {
    /// User sent a message (from any channel).
    UserMessage {
        message_id: String,
        content: String,
        agent_id: AgentId,
        channel: String,
    },
    /// Agent typing state changed.
    Typing {
        agent_id: AgentId,
        state: TypingState,
    },
    /// Incremental text from agent response.
    TextDelta { agent_id: AgentId, delta: String },
    /// Tool execution started.
    ToolStart {
        agent_id: AgentId,
        tool_name: String,
        tool_use_id: String,
    },
    /// Tool execution completed.
    ToolEnd {
        agent_id: AgentId,
        tool_use_id: String,
        result_preview: String,
        is_error: bool,
    },
    /// Agent phase changed (thinking, executing, etc.).
    Phase {
        agent_id: AgentId,
        phase: String,
        detail: Option<String>,
    },
    /// Intermediate narration message.
    IntermediateMessage { agent_id: AgentId, content: String },
    /// Agent asks user for input.
    AskUser {
        agent_id: AgentId,
        question: String,
        options: Option<Vec<String>>,
    },
    /// A long-running project worker needs user direction before it can continue.
    /// This is distinct from the generic AskUser event because routing surfaces
    /// need the project title, phase, and stable request id.
    ProjectAskUser {
        agent_id: AgentId,
        ask_id: String,
        project_id: String,
        project_slug: String,
        project_name: String,
        phase: String,
        worker_role: String,
        question: String,
        options: Option<Vec<String>>,
    },
    /// Final response complete.
    Response {
        agent_id: AgentId,
        content: String,
        input_tokens: u64,
        output_tokens: u64,
    },
    /// Message arrived from another channel (Telegram, Discord, etc.).
    ChannelMessage {
        agent_id: AgentId,
        channel: String,
        sender: String,
        content: String,
        response: Option<String>,
    },
    /// Phase-i.6: an agent is awaiting human approval for a tool invocation.
    /// Emitted by ApprovalManager::request_approval before it blocks on the
    /// resolution oneshot. Clients (web, CLI, Telegram) listen for this and
    /// surface a modal so the user can approve/deny in real time without
    /// polling /api/approvals.
    ApprovalRequested {
        agent_id: AgentId,
        request_id: String,
        tool_name: String,
        description: String,
        risk_level: String,
        timeout_secs: u64,
    },
    /// Phase O.2: emitted right after the auto-memorize pipeline writes a
    /// fact to MemPalace. Clients render a discreet 🧠 line in the chat
    /// so the user sees what the reflection model (Kimi K2.6 by default)
    /// has captured. `source` is typically the model id; `wing`/`room`
    /// are the MemPalace routing tags. `channel` (Commit-A) carries the
    /// origin canal (telegram, cli, web, …) so each surface can route
    /// the notification back to the conversation it came from.
    MemoryStored {
        subject: String,
        predicate: String,
        object: String,
        source: String,
        wing: String,
        room: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
        /// Free-text category set by the saver: info / skill /
        /// error_success / solution / other. Optional for backward
        /// compatibility with legacy auto-memorize entries that didn't
        /// classify.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
    },
    /// Commit-D — a candidate has been queued for human review under
    /// `LearningMode::Approval`. Subscribers (Telegram, CLI, web) render
    /// an interactive approval prompt with inline keyboard / slash so the
    /// user can approve/reject without opening the dashboard.
    MemoryQueued {
        review_id: String,
        subject: String,
        predicate: String,
        object: String,
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
    /// A reusable tool workflow has been detected and drafted as a skill
    /// proposal. This is a critical self-improvement item: clients must show
    /// it in the current conversation and require explicit approval before a
    /// generated skill is written.
    SkillProposalQueued {
        proposal_id: String,
        name: String,
        description: String,
        trigger_hint: String,
        tool_sequence: Vec<String>,
        confidence: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        family: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        source_agent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
    /// An existing skill has a proposed refinement after real usage or a
    /// curator pass. Like generated skills, this is a critical durable
    /// self-improvement item: clients must surface it with explicit
    /// approve/reject controls before any skill file is mutated.
    SkillRefinementQueued {
        refinement_id: String,
        skill: String,
        finding: String,
        suggested_change: String,
        risk: String,
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
}

/// Typing indicator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypingState {
    Start,
    Stop,
    Tool,
}

/// A message between agents or from user to agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessage {
    /// The text content of the message.
    pub content: String,
    /// Optional structured metadata.
    pub metadata: HashMap<String, serde_json::Value>,
    /// The role of the message sender.
    pub role: MessageRole,
}

/// Role of a message sender.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// A human user.
    User,
    /// An AI agent.
    Agent,
    /// The system.
    System,
    /// A tool.
    Tool,
}

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Which tool produced this output.
    pub tool_id: String,
    /// The tool_use ID this result corresponds to.
    pub tool_use_id: String,
    /// The output content.
    pub content: String,
    /// Whether the tool execution succeeded.
    pub success: bool,
    /// How long the tool took to execute.
    pub execution_time_ms: u64,
}

/// A change in the memory substrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryDelta {
    /// What kind of memory operation.
    pub operation: MemoryOperation,
    /// The key that changed.
    pub key: String,
    /// Which agent's memory changed.
    pub agent_id: AgentId,
}

/// The type of memory operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOperation {
    /// A new value was created.
    Created,
    /// An existing value was updated.
    Updated,
    /// A value was deleted.
    Deleted,
}

/// Agent lifecycle event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum LifecycleEvent {
    /// An agent was spawned.
    Spawned {
        /// The new agent's ID.
        agent_id: AgentId,
        /// The new agent's name.
        name: String,
    },
    /// An agent started running.
    Started {
        /// The agent's ID.
        agent_id: AgentId,
    },
    /// An agent was suspended.
    Suspended {
        /// The agent's ID.
        agent_id: AgentId,
    },
    /// An agent was resumed.
    Resumed {
        /// The agent's ID.
        agent_id: AgentId,
    },
    /// An agent was terminated.
    Terminated {
        /// The agent's ID.
        agent_id: AgentId,
        /// The reason for termination.
        reason: String,
    },
    /// An agent crashed.
    Crashed {
        /// The agent's ID.
        agent_id: AgentId,
        /// The error that caused the crash.
        error: String,
    },
}

/// Network-related event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum NetworkEvent {
    /// A peer connected.
    PeerConnected {
        /// The peer's ID.
        peer_id: String,
    },
    /// A peer disconnected.
    PeerDisconnected {
        /// The peer's ID.
        peer_id: String,
    },
    /// A message was received from a remote agent.
    MessageReceived {
        /// The peer that sent the message.
        from_peer: String,
        /// The agent that sent the message.
        from_agent: String,
    },
    /// A discovery query returned results.
    DiscoveryResult {
        /// The service that was searched for.
        service: String,
        /// The peers that provide the service.
        providers: Vec<String>,
    },
}

/// File-system event kind used by file-change triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEventKind {
    /// A file or directory was created.
    Create,
    /// A file or directory was modified.
    Modify,
    /// A file or directory was removed.
    Remove,
    /// A path moved from one location to another.
    Rename,
    /// Match any supported file event kind.
    Any,
}

impl FileEventKind {
    /// Stable lower-case label for prompts and tool output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Modify => "modify",
            Self::Remove => "remove",
            Self::Rename => "rename",
            Self::Any => "any",
        }
    }
}

impl std::str::FromStr for FileEventKind {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "create" | "created" => Ok(Self::Create),
            "modify" | "modified" | "write" | "changed" => Ok(Self::Modify),
            "remove" | "removed" | "delete" | "deleted" => Ok(Self::Remove),
            "rename" | "renamed" | "move" | "moved" => Ok(Self::Rename),
            "any" | "*" => Ok(Self::Any),
            other => Err(format!("unsupported file event kind '{other}'")),
        }
    }
}

/// System-level event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum SystemEvent {
    /// The kernel has started.
    KernelStarted,
    /// The kernel is stopping.
    KernelStopping,
    /// An agent is approaching a resource quota.
    QuotaWarning {
        /// The agent's ID.
        agent_id: AgentId,
        /// Which resource is running low.
        resource: String,
        /// How much of the quota has been used (0-100).
        usage_percent: f32,
    },
    /// A health check was performed.
    HealthCheck {
        /// The health status.
        status: String,
    },
    /// A quota enforcement event.
    QuotaEnforced {
        /// The agent whose quota was enforced.
        agent_id: AgentId,
        /// Amount spent in the current window.
        spent: f64,
        /// The quota limit.
        limit: f64,
    },
    /// A model was auto-routed based on complexity.
    ModelRouted {
        /// The agent using the routed model.
        agent_id: AgentId,
        /// The detected complexity level.
        complexity: String,
        /// The model selected.
        model: String,
    },
    /// A user action was performed.
    UserAction {
        /// The user who performed the action.
        user_id: String,
        /// The action performed.
        action: String,
        /// The result of the action.
        result: String,
    },
    /// A heartbeat health check failed for an agent.
    HealthCheckFailed {
        /// The agent that failed the health check.
        agent_id: AgentId,
        /// How long the agent has been unresponsive.
        unresponsive_secs: u64,
    },
    /// R.3.2 — An integration (Telegram, ElevenLabs, Slack…) was just
    /// configured via `config_setup` / `captain integration setup`.
    /// Channel managers listen for this to hot-reload the affected
    /// adapter without a daemon restart.
    IntegrationConfigured {
        /// Canonical integration name, e.g. `"telegram"`, `"tts_elevenlabs"`.
        name: String,
    },
    /// A file-change trigger observed a matching filesystem change.
    FileChanged {
        /// The trigger that matched the change.
        trigger_id: TriggerId,
        /// Changed path.
        path: PathBuf,
        /// Classified change kind.
        kind: FileEventKind,
        /// Previous path when the backend emitted a remove/create pair that
        /// could be collapsed into a rename.
        previous_path: Option<PathBuf>,
    },
    /// A trigger was auto-paused after exceeding its fire-rate guardrail.
    TriggerThrottled {
        /// The trigger that was paused.
        trigger_id: TriggerId,
        /// Human-readable throttle reason.
        reason: String,
    },
}

/// A complete event in the Captain event system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Unique event ID.
    pub id: EventId,
    /// Which agent (or system) produced this event.
    pub source: AgentId,
    /// Where this event is directed.
    pub target: EventTarget,
    /// The event payload.
    pub payload: EventPayload,
    /// When the event was created.
    pub timestamp: DateTime<Utc>,
    /// For request-response patterns: links response to request.
    pub correlation_id: Option<EventId>,
    /// Time-to-live: event expires after this duration.
    #[serde(with = "duration_ms")]
    pub ttl: Option<Duration>,
}

impl Event {
    /// Create a new event with the given source, target, and payload.
    pub fn new(source: AgentId, target: EventTarget, payload: EventPayload) -> Self {
        Self {
            id: EventId::new(),
            source,
            target,
            payload,
            timestamp: Utc::now(),
            correlation_id: None,
            ttl: None,
        }
    }

    /// Set the correlation ID for request-response linking.
    pub fn with_correlation(mut self, correlation_id: EventId) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }

    /// Set the TTL for this event.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = Some(ttl);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_creation() {
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::KernelStarted),
        );
        assert_eq!(event.source, agent_id);
        assert!(event.correlation_id.is_none());
        assert!(event.ttl.is_none());
    }

    #[test]
    fn test_event_with_correlation() {
        let agent_id = AgentId::new();
        let corr_id = EventId::new();
        let event = Event::new(
            agent_id,
            EventTarget::System,
            EventPayload::System(SystemEvent::HealthCheck {
                status: "ok".to_string(),
            }),
        )
        .with_correlation(corr_id);
        assert_eq!(event.correlation_id, Some(corr_id));
    }

    #[test]
    fn test_event_serialization() {
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::Agent(AgentId::new()),
            EventPayload::Message(AgentMessage {
                content: "Hello".to_string(),
                metadata: HashMap::new(),
                role: MessageRole::User,
            }),
        );
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, event.id);
    }

    #[test]
    fn test_chat_stream_event_serialization() {
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::Agent(agent_id),
            EventPayload::ChatStream(ChatStreamEvent::UserMessage {
                message_id: "msg-1".to_string(),
                content: "Hello".to_string(),
                agent_id,
                channel: "web".to_string(),
            }),
        );
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, event.id);
        match deserialized.payload {
            EventPayload::ChatStream(ChatStreamEvent::UserMessage {
                content, channel, ..
            }) => {
                assert_eq!(content, "Hello");
                assert_eq!(channel, "web");
            }
            _ => panic!("Wrong payload type"),
        }
    }

    #[test]
    fn test_chat_stream_all_variants_serialize() {
        let agent_id = AgentId::new();
        let variants: Vec<ChatStreamEvent> = vec![
            ChatStreamEvent::Typing {
                agent_id,
                state: TypingState::Start,
            },
            ChatStreamEvent::TextDelta {
                agent_id,
                delta: "chunk".to_string(),
            },
            ChatStreamEvent::ToolStart {
                agent_id,
                tool_name: "web_search".to_string(),
                tool_use_id: "tu-1".to_string(),
            },
            ChatStreamEvent::ToolEnd {
                agent_id,
                tool_use_id: "tu-1".to_string(),
                result_preview: "ok".to_string(),
                is_error: false,
            },
            ChatStreamEvent::Phase {
                agent_id,
                phase: "thinking".to_string(),
                detail: None,
            },
            ChatStreamEvent::IntermediateMessage {
                agent_id,
                content: "narration".to_string(),
            },
            ChatStreamEvent::AskUser {
                agent_id,
                question: "Continue?".to_string(),
                options: Some(vec!["yes".to_string(), "no".to_string()]),
            },
            ChatStreamEvent::Response {
                agent_id,
                content: "done".to_string(),
                input_tokens: 100,
                output_tokens: 50,
            },
            ChatStreamEvent::ChannelMessage {
                agent_id,
                channel: "telegram".to_string(),
                sender: "user123".to_string(),
                content: "hi".to_string(),
                response: Some("hello".to_string()),
            },
            ChatStreamEvent::MemoryStored {
                subject: "user".to_string(),
                predicate: "timezone".to_string(),
                object: "Europe/Paris".to_string(),
                source: "kimi-k2.6".to_string(),
                wing: "learnings".to_string(),
                room: "user_preferences".to_string(),
                channel: Some("telegram".to_string()),
                category: Some("info".to_string()),
            },
            ChatStreamEvent::MemoryQueued {
                review_id: "rev-1".to_string(),
                subject: "user".to_string(),
                predicate: "prefers".to_string(),
                object: "short answers".to_string(),
                source: "learning.conversation_turn".to_string(),
                channel: Some("telegram".to_string()),
            },
            ChatStreamEvent::SkillProposalQueued {
                proposal_id: "prop-1".to_string(),
                name: "status-checker".to_string(),
                description: "Checks service status".to_string(),
                trigger_hint: "user asks for a status check".to_string(),
                tool_sequence: vec!["ssh_exec".to_string(), "shell_exec".to_string()],
                confidence: 0.9,
                family: Some("general-automation".to_string()),
                language: Some("fr".to_string()),
                source_agent_id: "captain".to_string(),
                channel: Some("telegram".to_string()),
            },
            ChatStreamEvent::SkillRefinementQueued {
                refinement_id: "ref-1".to_string(),
                skill: "status-checker".to_string(),
                finding: "Missing recovery step".to_string(),
                suggested_change: "Document the retry path".to_string(),
                risk: "low".to_string(),
                source: "skill_use".to_string(),
                channel: Some("telegram".to_string()),
            },
        ];
        for variant in variants {
            let payload = EventPayload::ChatStream(variant);
            let json = serde_json::to_string(&payload).unwrap();
            let back: EventPayload = serde_json::from_str(&json).unwrap();
            assert!(matches!(back, EventPayload::ChatStream(_)));
        }
    }

    #[test]
    fn test_typing_state_values() {
        assert_eq!(
            serde_json::to_string(&TypingState::Start).unwrap(),
            "\"start\""
        );
        assert_eq!(
            serde_json::to_string(&TypingState::Stop).unwrap(),
            "\"stop\""
        );
        assert_eq!(
            serde_json::to_string(&TypingState::Tool).unwrap(),
            "\"tool\""
        );
    }

    #[test]
    fn test_event_with_ttl_serialization() {
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::KernelStarted),
        )
        .with_ttl(Duration::from_secs(60));
        let json = serde_json::to_string(&event).unwrap();
        let deserialized: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.ttl, Some(Duration::from_millis(60_000)));
    }
}

//! Chat broadcast — tracks active streams and publishes chat events to the EventBus.
//!
//! Each agent can have at most one active stream at a time (enforced by
//! `agent_msg_locks`). The `ActiveStreamTracker` stores the accumulated state
//! so that new WebSocket connections can "catch up" on an in-progress response.

use captain_types::agent::AgentId;
use captain_types::event::{ChatStreamEvent, Event, EventPayload, EventTarget};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::event_bus::EventBus;

/// Snapshot of an in-progress agent response, used for catch-up on new WS connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveStream {
    pub agent_id: AgentId,
    pub user_message: String,
    pub user_message_id: String,
    pub accumulated_text: String,
    pub tools: Vec<ActiveTool>,
    pub started_at: DateTime<Utc>,
    pub is_streaming: bool,
    pub channel: String,
}

/// A tool that was used during the active stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTool {
    pub tool_use_id: String,
    pub tool_name: String,
    pub result_preview: Option<String>,
    pub is_error: bool,
    pub completed: bool,
}

/// Tracks active (in-progress) agent streams for catch-up on new connections.
pub struct ActiveStreamTracker {
    streams: DashMap<AgentId, ActiveStream>,
}

impl ActiveStreamTracker {
    pub fn new() -> Self {
        Self {
            streams: DashMap::new(),
        }
    }

    /// Start tracking a new stream for an agent.
    pub fn start(
        &self,
        agent_id: AgentId,
        user_message: String,
        user_message_id: String,
        channel: String,
    ) {
        self.streams.insert(
            agent_id,
            ActiveStream {
                agent_id,
                user_message,
                user_message_id,
                accumulated_text: String::new(),
                tools: Vec::new(),
                started_at: Utc::now(),
                is_streaming: true,
                channel,
            },
        );
    }

    /// Append text delta to the accumulated response.
    pub fn append_text(&self, agent_id: AgentId, delta: &str) {
        if let Some(mut stream) = self.streams.get_mut(&agent_id) {
            stream.accumulated_text.push_str(delta);
        }
    }

    /// Record a tool start.
    pub fn tool_start(&self, agent_id: AgentId, tool_use_id: String, tool_name: String) {
        if let Some(mut stream) = self.streams.get_mut(&agent_id) {
            stream.tools.push(ActiveTool {
                tool_use_id,
                tool_name,
                result_preview: None,
                is_error: false,
                completed: false,
            });
        }
    }

    /// Record a tool completion.
    pub fn tool_end(
        &self,
        agent_id: AgentId,
        tool_use_id: &str,
        result_preview: String,
        is_error: bool,
    ) {
        if let Some(mut stream) = self.streams.get_mut(&agent_id) {
            if let Some(tool) = stream
                .tools
                .iter_mut()
                .find(|t| t.tool_use_id == tool_use_id)
            {
                tool.result_preview = Some(result_preview);
                tool.is_error = is_error;
                tool.completed = true;
            }
        }
    }

    /// Mark a stream as complete and remove it.
    pub fn finish(&self, agent_id: AgentId) -> Option<ActiveStream> {
        self.streams.remove(&agent_id).map(|(_, mut s)| {
            s.is_streaming = false;
            s
        })
    }

    /// Get the current active stream for catch-up (if any).
    pub fn get(&self, agent_id: AgentId) -> Option<ActiveStream> {
        self.streams.get(&agent_id).map(|s| s.clone())
    }

    /// Remove stale streams older than the given duration.
    /// Prevents unbounded memory growth from crashed/abandoned streams.
    pub fn cleanup_stale(&self, max_age: chrono::Duration) {
        let cutoff = Utc::now() - max_age;
        self.streams.retain(|_, stream| stream.started_at > cutoff);
    }
}

impl Default for ActiveStreamTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Publish a ChatStreamEvent to the EventBus, targeting a specific agent's channel.
pub async fn publish_chat_event(
    event_bus: &EventBus,
    agent_id: AgentId,
    chat_event: ChatStreamEvent,
) {
    let event = Event::new(
        agent_id,
        EventTarget::Agent(agent_id),
        EventPayload::ChatStream(chat_event),
    );
    event_bus.publish(event).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::event::TypingState;

    #[test]
    fn test_active_stream_lifecycle() {
        let tracker = ActiveStreamTracker::new();
        let agent_id = AgentId::new();

        // Start
        tracker.start(
            agent_id,
            "Hello".to_string(),
            "msg-1".to_string(),
            "web".to_string(),
        );
        let stream = tracker.get(agent_id).unwrap();
        assert!(stream.is_streaming);
        assert_eq!(stream.user_message, "Hello");
        assert_eq!(stream.accumulated_text, "");

        // Append text
        tracker.append_text(agent_id, "Hi ");
        tracker.append_text(agent_id, "there!");
        let stream = tracker.get(agent_id).unwrap();
        assert_eq!(stream.accumulated_text, "Hi there!");

        // Tool start + end
        tracker.tool_start(agent_id, "tu-1".to_string(), "web_search".to_string());
        tracker.tool_end(agent_id, "tu-1", "results found".to_string(), false);
        let stream = tracker.get(agent_id).unwrap();
        assert_eq!(stream.tools.len(), 1);
        assert!(stream.tools[0].completed);
        assert_eq!(
            stream.tools[0].result_preview.as_deref(),
            Some("results found")
        );

        // Finish
        let finished = tracker.finish(agent_id).unwrap();
        assert!(!finished.is_streaming);
        assert!(tracker.get(agent_id).is_none());
    }

    #[test]
    fn test_no_stream_returns_none() {
        let tracker = ActiveStreamTracker::new();
        assert!(tracker.get(AgentId::new()).is_none());
    }

    #[test]
    fn test_multiple_agents_independent() {
        let tracker = ActiveStreamTracker::new();
        let a1 = AgentId::new();
        let a2 = AgentId::new();

        tracker.start(a1, "msg1".to_string(), "id1".to_string(), "web".to_string());
        tracker.start(
            a2,
            "msg2".to_string(),
            "id2".to_string(),
            "telegram".to_string(),
        );

        tracker.append_text(a1, "response1");
        tracker.append_text(a2, "response2");

        assert_eq!(tracker.get(a1).unwrap().accumulated_text, "response1");
        assert_eq!(tracker.get(a2).unwrap().accumulated_text, "response2");

        tracker.finish(a1);
        assert!(tracker.get(a1).is_none());
        assert!(tracker.get(a2).is_some());
    }

    #[tokio::test]
    async fn test_publish_chat_event_reaches_subscriber() {
        let bus = EventBus::new();
        let agent_id = AgentId::new();
        let mut rx = bus.subscribe_agent(agent_id);

        publish_chat_event(
            &bus,
            agent_id,
            ChatStreamEvent::UserMessage {
                message_id: "msg-1".to_string(),
                content: "Hello".to_string(),
                agent_id,
                channel: "web".to_string(),
            },
        )
        .await;

        let event = rx.recv().await.unwrap();
        match event.payload {
            EventPayload::ChatStream(ChatStreamEvent::UserMessage { content, .. }) => {
                assert_eq!(content, "Hello");
            }
            _ => panic!("Wrong payload"),
        }
    }

    #[tokio::test]
    async fn test_broadcast_fanout_multiple_subscribers() {
        let bus = EventBus::new();
        let agent_id = AgentId::new();
        let mut rx1 = bus.subscribe_agent(agent_id);
        let mut rx2 = bus.subscribe_agent(agent_id);

        publish_chat_event(
            &bus,
            agent_id,
            ChatStreamEvent::TextDelta {
                agent_id,
                delta: "chunk".to_string(),
            },
        )
        .await;

        let e1 = rx1.recv().await.unwrap();
        let e2 = rx2.recv().await.unwrap();

        // Both subscribers receive the same event
        assert_eq!(e1.id, e2.id);
        match e1.payload {
            EventPayload::ChatStream(ChatStreamEvent::TextDelta { delta, .. }) => {
                assert_eq!(delta, "chunk");
            }
            _ => panic!("Wrong payload"),
        }
    }

    #[tokio::test]
    async fn test_events_dont_leak_between_agents() {
        let bus = EventBus::new();
        let a1 = AgentId::new();
        let a2 = AgentId::new();
        let mut rx1 = bus.subscribe_agent(a1);
        let mut rx2 = bus.subscribe_agent(a2);

        publish_chat_event(
            &bus,
            a1,
            ChatStreamEvent::Typing {
                agent_id: a1,
                state: TypingState::Start,
            },
        )
        .await;

        // rx1 gets the event
        let e1 = rx1.recv().await.unwrap();
        assert!(matches!(
            e1.payload,
            EventPayload::ChatStream(ChatStreamEvent::Typing { .. })
        ));

        // rx2 should NOT receive it (different agent) — use try_recv
        let result = rx2.try_recv();
        assert!(result.is_err());
    }
}

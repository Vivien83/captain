//! Memory event streaming route handlers.

use crate::state::AppState;
use axum::{
    extract::State,
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
};
use captain_types::event::{ChatStreamEvent, EventPayload, LifecycleEvent, ToolRunEvent};
use futures::stream;
use std::{convert::Infallible, sync::Arc};

/// GET /api/memory/events - SSE stream for live memory, skill proposal,
/// agent lifecycle, and background tool_run events.
pub async fn memory_events_stream(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let rx = state.kernel.event_bus.subscribe_all();
    let sse_stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let Some(sse_event) = event_payload_to_sse(event.payload) else {
                        continue;
                    };
                    return Some((Ok::<Event, Infallible>(sse_event), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });

    Sse::new(sse_stream)
        .keep_alive(axum::response::sse::KeepAlive::default())
        .into_response()
}

fn event_payload_to_sse(payload: EventPayload) -> Option<Event> {
    match payload {
        EventPayload::ChatStream(stream_event) => memory_event_to_sse(stream_event),
        EventPayload::Lifecycle(lifecycle_event) => lifecycle_event_to_sse(lifecycle_event),
        EventPayload::ToolRun(tool_run_event) => Some(tool_run_event_to_sse(tool_run_event)),
        _ => None,
    }
}

/// Pure mapping from a `LifecycleEvent` to (SSE event kind, JSON payload).
/// Kept separate from `Event` construction so the mapping itself is
/// testable without depending on axum's SSE `Event` internals.
/// `None` for variants the background-activity badge doesn't need today
/// (Started/Suspended/Resumed) — skip rather than guess at an unused shape.
fn lifecycle_event_fields(event: LifecycleEvent) -> Option<(&'static str, serde_json::Value)> {
    let (kind, agent_id, name, detail) = match event {
        LifecycleEvent::Spawned { agent_id, name } => ("spawned", agent_id, Some(name), None),
        LifecycleEvent::Terminated { agent_id, reason } => {
            ("terminated", agent_id, None, Some(reason))
        }
        LifecycleEvent::Crashed { agent_id, error } => ("crashed", agent_id, None, Some(error)),
        LifecycleEvent::Started { .. }
        | LifecycleEvent::Suspended { .. }
        | LifecycleEvent::Resumed { .. } => return None,
    };
    Some((
        "agent_lifecycle",
        serde_json::json!({
            "kind": kind,
            "agent_id": agent_id.to_string(),
            "name": name,
            "detail": detail,
        }),
    ))
}

fn lifecycle_event_to_sse(event: LifecycleEvent) -> Option<Event> {
    let (sse_kind, data) = lifecycle_event_fields(event)?;
    Some(
        Event::default()
            .event(sse_kind)
            .json_data(data)
            .unwrap_or_else(|_| Event::default().data("error")),
    )
}

fn tool_run_event_fields(event: ToolRunEvent) -> serde_json::Value {
    serde_json::json!({
        "run_id": event.run_id,
        "tool_name": event.tool_name,
        "status": event.status,
        "caller_agent_id": event.caller_agent_id,
    })
}

fn tool_run_event_to_sse(event: ToolRunEvent) -> Event {
    Event::default()
        .event("tool_run_status")
        .json_data(tool_run_event_fields(event))
        .unwrap_or_else(|_| Event::default().data("error"))
}

fn memory_event_to_sse(event: ChatStreamEvent) -> Option<Event> {
    match event {
        ChatStreamEvent::MemoryStored {
            subject,
            predicate,
            object,
            source,
            wing,
            room,
            channel,
            category,
        } => Some(
            Event::default()
                .event("memory_stored")
                .json_data(serde_json::json!({
                    "subject": subject,
                    "predicate": predicate,
                    "object": object,
                    "source": source,
                    "wing": wing,
                    "room": room,
                    "channel": channel,
                    "category": category,
                }))
                .unwrap_or_else(|_| Event::default().data("error")),
        ),
        ChatStreamEvent::MemoryQueued {
            review_id,
            subject,
            predicate,
            object,
            source,
            channel,
        } => Some(
            Event::default()
                .event("memory_queued")
                .json_data(serde_json::json!({
                    "review_id": review_id,
                    "subject": subject,
                    "predicate": predicate,
                    "object": object,
                    "source": source,
                    "channel": channel,
                }))
                .unwrap_or_else(|_| Event::default().data("error")),
        ),
        ChatStreamEvent::SkillProposalQueued {
            proposal_id,
            name,
            description,
            trigger_hint,
            tool_sequence,
            confidence,
            family,
            language,
            source_agent_id,
            channel,
        } => Some(
            Event::default()
                .event("skill_proposal_queued")
                .json_data(serde_json::json!({
                    "proposal_id": proposal_id,
                    "name": name,
                    "description": description,
                    "trigger_hint": trigger_hint,
                    "tool_sequence": tool_sequence,
                    "confidence": confidence,
                    "family": family,
                    "language": language,
                    "source_agent_id": source_agent_id,
                    "channel": channel,
                }))
                .unwrap_or_else(|_| Event::default().data("error")),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::AgentId;

    #[test]
    fn lifecycle_spawned_maps_to_agent_lifecycle_kind() {
        let agent_id = AgentId::new();
        let (sse_kind, data) = lifecycle_event_fields(LifecycleEvent::Spawned {
            agent_id,
            name: "researcher-hand".to_string(),
        })
        .expect("Spawned must produce an SSE event");
        assert_eq!(sse_kind, "agent_lifecycle");
        assert_eq!(data["kind"], "spawned");
        assert_eq!(data["agent_id"], agent_id.to_string());
        assert_eq!(data["name"], "researcher-hand");
        assert!(data["detail"].is_null());
    }

    #[test]
    fn lifecycle_terminated_and_crashed_carry_their_reason() {
        let agent_id = AgentId::new();
        let (_, terminated) = lifecycle_event_fields(LifecycleEvent::Terminated {
            agent_id,
            reason: "killed".to_string(),
        })
        .unwrap();
        assert_eq!(terminated["kind"], "terminated");
        assert_eq!(terminated["detail"], "killed");
        assert!(terminated["name"].is_null());

        let (_, crashed) = lifecycle_event_fields(LifecycleEvent::Crashed {
            agent_id,
            error: "unresponsive for 90s".to_string(),
        })
        .unwrap();
        assert_eq!(crashed["kind"], "crashed");
        assert_eq!(crashed["detail"], "unresponsive for 90s");
    }

    #[test]
    fn lifecycle_started_suspended_resumed_are_not_surfaced() {
        let agent_id = AgentId::new();
        assert!(lifecycle_event_fields(LifecycleEvent::Started { agent_id }).is_none());
        assert!(lifecycle_event_fields(LifecycleEvent::Suspended { agent_id }).is_none());
        assert!(lifecycle_event_fields(LifecycleEvent::Resumed { agent_id }).is_none());
    }

    #[test]
    fn tool_run_event_fields_round_trip_all_fields() {
        let data = tool_run_event_fields(ToolRunEvent {
            run_id: "toolrun-abc".to_string(),
            tool_name: "shell_exec".to_string(),
            status: "completed".to_string(),
            caller_agent_id: Some("agent-123".to_string()),
        });
        assert_eq!(data["run_id"], "toolrun-abc");
        assert_eq!(data["tool_name"], "shell_exec");
        assert_eq!(data["status"], "completed");
        assert_eq!(data["caller_agent_id"], "agent-123");
    }

    #[test]
    fn event_payload_to_sse_ignores_unrelated_payloads() {
        assert!(event_payload_to_sse(EventPayload::System(
            captain_types::event::SystemEvent::KernelStarted
        ))
        .is_none());
    }
}

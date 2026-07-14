use captain_types::agent::AgentId;
use captain_types::event::{ChatStreamEvent, Event, EventPayload, EventTarget};
use captain_types::memory::{Entity, GraphMatch, GraphPattern, Memory, Relation};
use serde_json::Value;

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) async fn handle_task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, String> {
        self.memory
            .task_post(title, description, assigned_to, created_by)
            .await
            .map_err(|e| format!("Task post failed: {e}"))
    }

    pub(super) async fn handle_task_claim(&self, agent_id: &str) -> Result<Option<Value>, String> {
        self.memory
            .task_claim(agent_id)
            .await
            .map_err(|e| format!("Task claim failed: {e}"))
    }

    pub(super) async fn handle_task_complete(
        &self,
        task_id: &str,
        result: &str,
    ) -> Result<(), String> {
        self.memory
            .task_complete(task_id, result)
            .await
            .map_err(|e| format!("Task complete failed: {e}"))
    }

    pub(super) async fn handle_task_list(
        &self,
        status: Option<&str>,
    ) -> Result<Vec<Value>, String> {
        self.memory
            .task_list(status)
            .await
            .map_err(|e| format!("Task list failed: {e}"))
    }

    pub(super) async fn handle_publish_event(
        &self,
        event_type: &str,
        payload: Value,
    ) -> Result<(), String> {
        let payload_bytes =
            serde_json::to_vec(&serde_json::json!({"type": event_type, "data": payload}))
                .map_err(|e| format!("Serialize failed: {e}"))?;
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Custom(payload_bytes),
        );
        CaptainKernel::publish_event(self, event).await;
        Ok(())
    }

    pub(super) async fn handle_knowledge_add_entity(
        &self,
        entity: Entity,
    ) -> Result<String, String> {
        self.memory
            .add_entity(entity)
            .await
            .map_err(|e| format!("Knowledge add entity failed: {e}"))
    }

    pub(super) async fn handle_knowledge_add_relation(
        &self,
        relation: Relation,
    ) -> Result<String, String> {
        self.memory
            .add_relation(relation)
            .await
            .map_err(|e| format!("Knowledge add relation failed: {e}"))
    }

    pub(super) async fn handle_knowledge_query(
        &self,
        pattern: GraphPattern,
    ) -> Result<Vec<GraphMatch>, String> {
        self.memory
            .query_graph(pattern)
            .await
            .map_err(|e| format!("Knowledge query failed: {e}"))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_publish_memory_stored(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        source: &str,
        wing: Option<&str>,
        room: Option<&str>,
        channel: Option<&str>,
        category: Option<&str>,
    ) {
        self.publish_chat_stream_payload(memory_stored_payload(
            subject, predicate, object, source, wing, room, channel, category,
        ));
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_publish_skill_refinement_queued(
        &self,
        refinement_id: &str,
        skill: &str,
        finding: &str,
        suggested_change: &str,
        risk: &str,
        source: &str,
        channel: Option<&str>,
    ) {
        self.publish_chat_stream_payload(skill_refinement_queued_payload(
            refinement_id,
            skill,
            finding,
            suggested_change,
            risk,
            source,
            channel,
        ));
    }

    fn publish_chat_stream_payload(&self, payload: EventPayload) {
        let event = Event::new(AgentId::default(), EventTarget::Broadcast, payload);
        let bus = self.event_bus.clone();
        tokio::spawn(async move {
            bus.publish(event).await;
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn memory_stored_payload(
    subject: &str,
    predicate: &str,
    object: &str,
    source: &str,
    wing: Option<&str>,
    room: Option<&str>,
    channel: Option<&str>,
    category: Option<&str>,
) -> EventPayload {
    EventPayload::ChatStream(ChatStreamEvent::MemoryStored {
        subject: subject.to_string(),
        predicate: predicate.to_string(),
        object: object.to_string(),
        source: source.to_string(),
        wing: wing.unwrap_or("").to_string(),
        room: room.unwrap_or("").to_string(),
        channel: channel.map(String::from),
        category: category.map(String::from),
    })
}

#[allow(clippy::too_many_arguments)]
fn skill_refinement_queued_payload(
    refinement_id: &str,
    skill: &str,
    finding: &str,
    suggested_change: &str,
    risk: &str,
    source: &str,
    channel: Option<&str>,
) -> EventPayload {
    EventPayload::ChatStream(ChatStreamEvent::SkillRefinementQueued {
        refinement_id: refinement_id.to_string(),
        skill: skill.to_string(),
        finding: finding.to_string(),
        suggested_change: suggested_change.to_string(),
        risk: risk.to_string(),
        source: source.to_string(),
        channel: channel.map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use captain_types::event::{ChatStreamEvent, EventPayload};

    use super::{memory_stored_payload, skill_refinement_queued_payload};

    #[test]
    fn memory_stored_payload_defaults_missing_routing_tags() {
        let payload =
            memory_stored_payload("user", "likes", "rust", "test", None, None, None, None);

        match payload {
            EventPayload::ChatStream(ChatStreamEvent::MemoryStored {
                subject,
                wing,
                room,
                channel,
                category,
                ..
            }) => {
                assert_eq!(subject, "user");
                assert_eq!(wing, "");
                assert_eq!(room, "");
                assert_eq!(channel, None);
                assert_eq!(category, None);
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn skill_refinement_payload_preserves_channel() {
        let payload = skill_refinement_queued_payload(
            "ref-1",
            "skill.md",
            "finding",
            "change",
            "low",
            "runtime",
            Some("telegram"),
        );

        match payload {
            EventPayload::ChatStream(ChatStreamEvent::SkillRefinementQueued {
                refinement_id,
                channel,
                ..
            }) => {
                assert_eq!(refinement_id, "ref-1");
                assert_eq!(channel.as_deref(), Some("telegram"));
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }
}

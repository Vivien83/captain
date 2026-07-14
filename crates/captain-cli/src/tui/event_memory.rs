use super::event::AppEvent;

pub(crate) fn memory_event_from_json(
    event_kind: &str,
    value: &serde_json::Value,
) -> Option<AppEvent> {
    match event_kind {
        "memory_stored" => Some(AppEvent::MemoryStored {
            subject: string_field(value, "subject"),
            predicate: string_field(value, "predicate"),
            object: string_field(value, "object"),
            source: string_field(value, "source"),
        }),
        "memory_queued" => Some(AppEvent::MemoryQueued {
            review_id: string_field(value, "review_id"),
            subject: string_field(value, "subject"),
            predicate: string_field(value, "predicate"),
            object: string_field(value, "object"),
            source: string_field(value, "source"),
        }),
        "skill_proposal_queued" => Some(AppEvent::SkillProposalQueued {
            proposal_id: string_field(value, "proposal_id"),
            name: string_field(value, "name"),
            description: string_field(value, "description"),
            trigger_hint: string_field(value, "trigger_hint"),
            confidence: value["confidence"].as_f64().unwrap_or(0.0) as f32,
            family: value["family"].as_str().map(str::to_string),
        }),
        "agent_lifecycle" => Some(AppEvent::AgentLifecycle {
            kind: string_field(value, "kind"),
            agent_id: string_field(value, "agent_id"),
            name: value["name"].as_str().map(str::to_string),
            detail: value["detail"].as_str().map(str::to_string),
        }),
        "tool_run_status" => Some(AppEvent::ToolRunStatus {
            run_id: string_field(value, "run_id"),
            tool_name: string_field(value, "tool_name"),
            status: string_field(value, "status"),
            caller_agent_id: value["caller_agent_id"].as_str().map(str::to_string),
        }),
        _ => None,
    }
}

fn string_field(value: &serde_json::Value, field: &str) -> String {
    value[field].as_str().unwrap_or("").to_string()
}

#[cfg(test)]
#[path = "event_memory/tests.rs"]
mod tests;

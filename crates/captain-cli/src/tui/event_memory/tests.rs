use super::*;
use serde_json::json;

#[test]
fn parses_memory_stored_event() {
    let event = memory_event_from_json(
        "memory_stored",
        &json!({
            "subject": "project",
            "predicate": "status",
            "object": "ready",
            "source": "agent"
        }),
    )
    .expect("memory event");

    match event {
        AppEvent::MemoryStored {
            subject,
            predicate,
            object,
            source,
        } => {
            assert_eq!(subject, "project");
            assert_eq!(predicate, "status");
            assert_eq!(object, "ready");
            assert_eq!(source, "agent");
        }
        _ => panic!("unexpected event"),
    }
}

#[test]
fn parses_memory_queued_event_with_empty_missing_fields() {
    let event = memory_event_from_json("memory_queued", &json!({"review_id": "review-1"})).unwrap();

    match event {
        AppEvent::MemoryQueued {
            review_id,
            subject,
            predicate,
            object,
            source,
        } => {
            assert_eq!(review_id, "review-1");
            assert!(subject.is_empty());
            assert!(predicate.is_empty());
            assert!(object.is_empty());
            assert!(source.is_empty());
        }
        _ => panic!("unexpected event"),
    }
}

#[test]
fn parses_skill_proposal_event() {
    let event = memory_event_from_json(
        "skill_proposal_queued",
        &json!({
            "proposal_id": "proposal-1",
            "name": "deploy-helper",
            "description": "Deploy safely",
            "trigger_hint": "after deploys",
            "confidence": 0.87,
            "family": "platform-devops"
        }),
    )
    .unwrap();

    match event {
        AppEvent::SkillProposalQueued {
            proposal_id,
            name,
            description,
            trigger_hint,
            confidence,
            family,
        } => {
            assert_eq!(proposal_id, "proposal-1");
            assert_eq!(name, "deploy-helper");
            assert_eq!(description, "Deploy safely");
            assert_eq!(trigger_hint, "after deploys");
            assert!((confidence - 0.87).abs() < f32::EPSILON);
            assert_eq!(family.as_deref(), Some("platform-devops"));
        }
        _ => panic!("unexpected event"),
    }
}

#[test]
fn ignores_unknown_event_kind() {
    assert!(memory_event_from_json("other", &json!({})).is_none());
}

#[test]
fn parses_agent_lifecycle_event() {
    let event = memory_event_from_json(
        "agent_lifecycle",
        &json!({
            "kind": "terminated",
            "agent_id": "agent-42",
            "name": "researcher-hand",
            "detail": "killed"
        }),
    )
    .expect("agent lifecycle event");

    match event {
        AppEvent::AgentLifecycle {
            kind,
            agent_id,
            name,
            detail,
        } => {
            assert_eq!(kind, "terminated");
            assert_eq!(agent_id, "agent-42");
            assert_eq!(name.as_deref(), Some("researcher-hand"));
            assert_eq!(detail.as_deref(), Some("killed"));
        }
        _ => panic!("unexpected event"),
    }
}

#[test]
fn parses_tool_run_status_event() {
    let event = memory_event_from_json(
        "tool_run_status",
        &json!({
            "run_id": "toolrun-1",
            "tool_name": "shell_exec",
            "status": "completed",
            "caller_agent_id": "agent-1"
        }),
    )
    .expect("tool run status event");

    match event {
        AppEvent::ToolRunStatus {
            run_id,
            tool_name,
            status,
            caller_agent_id,
        } => {
            assert_eq!(run_id, "toolrun-1");
            assert_eq!(tool_name, "shell_exec");
            assert_eq!(status, "completed");
            assert_eq!(caller_agent_id.as_deref(), Some("agent-1"));
        }
        _ => panic!("unexpected event"),
    }
}

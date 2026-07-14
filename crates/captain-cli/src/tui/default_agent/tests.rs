use serde_json::json;

use super::*;

#[derive(Debug)]
struct Agent {
    id: &'static str,
    name: &'static str,
}

fn select(agents: &[Agent], preference: AgentPreference<'_>) -> Option<usize> {
    select_index(
        agents,
        preference,
        |agent, id| agent.id == id,
        |agent| Some(agent.name),
    )
}

#[test]
fn workspace_agent_id_wins_first() {
    let agents = [
        Agent {
            id: "1",
            name: "captain",
        },
        Agent {
            id: "2",
            name: "Builder",
        },
    ];

    let index = select(
        &agents,
        AgentPreference {
            id: Some("2"),
            name: Some("captain"),
        },
    );

    assert_eq!(index, Some(1));
}

#[test]
fn workspace_agent_name_wins_when_id_misses() {
    let agents = [
        Agent {
            id: "1",
            name: "captain",
        },
        Agent {
            id: "2",
            name: "Builder",
        },
    ];

    let index = select(
        &agents,
        AgentPreference {
            id: Some("missing"),
            name: Some("builder"),
        },
    );

    assert_eq!(index, Some(1));
}

#[test]
fn captain_name_wins_before_first_agent() {
    let agents = [
        Agent {
            id: "1",
            name: "Research",
        },
        Agent {
            id: "2",
            name: "CAPTAIN",
        },
    ];

    assert_eq!(select(&agents, AgentPreference::default()), Some(1));
}

#[test]
fn first_agent_is_last_fallback() {
    let agents = [Agent {
        id: "1",
        name: "Research",
    }];

    assert_eq!(select(&agents, AgentPreference::default()), Some(0));
}

#[test]
fn empty_agent_list_has_no_pick() {
    assert_eq!(select(&[], AgentPreference::default()), None);
}

#[test]
fn daemon_identity_defaults_missing_name() {
    let agent = json!({ "id": "agent-1" });

    assert_eq!(
        daemon_agent_identity(&agent),
        Some(("agent-1".to_string(), "agent".to_string()))
    );
}

#[test]
fn daemon_identity_rejects_empty_id() {
    let agent = json!({ "id": "", "name": "captain" });

    assert_eq!(daemon_agent_identity(&agent), None);
}

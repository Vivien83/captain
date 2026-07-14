//! Agent selection command helpers.

use captain_types::agent::AgentId;
use std::future::Future;

pub(crate) async fn run_agent_selection_command<Find, FindFuture, Spawn, SpawnFuture, Select>(
    args: &[String],
    find_agent_by_name: Find,
    spawn_agent_by_name: Spawn,
    mut select_agent: Select,
) -> String
where
    Find: FnOnce(String) -> FindFuture,
    FindFuture: Future<Output = Result<Option<AgentId>, String>>,
    Spawn: FnOnce(String) -> SpawnFuture,
    SpawnFuture: Future<Output = Result<AgentId, String>>,
    Select: FnMut(AgentId),
{
    let Some(agent_name) = args.first().cloned() else {
        return "Usage: /agent <name>".to_string();
    };

    match find_agent_by_name(agent_name.clone()).await {
        Ok(Some(agent_id)) => {
            select_agent(agent_id);
            format!("Now talking to agent: {agent_name}")
        }
        Ok(None) => match spawn_agent_by_name(agent_name.clone()).await {
            Ok(agent_id) => {
                select_agent(agent_id);
                format!("Spawned and connected to agent: {agent_name}")
            }
            Err(error) => format!("Agent '{agent_name}' not found and could not spawn: {error}"),
        },
        Err(error) => format!("Error finding agent: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(name: &str) -> Vec<String> {
        vec![name.to_string()]
    }

    #[tokio::test]
    async fn agent_selection_requires_name() {
        let mut selected = None;
        let text = run_agent_selection_command(
            &[],
            |_| async { Ok(Some(AgentId::new())) },
            |_| async { Ok(AgentId::new()) },
            |agent_id| selected = Some(agent_id),
        )
        .await;

        assert_eq!(text, "Usage: /agent <name>");
        assert!(selected.is_none());
    }

    #[tokio::test]
    async fn agent_selection_uses_existing_agent() {
        let agent_id = AgentId::new();
        let mut selected = None;
        let text = run_agent_selection_command(
            &args("captain"),
            |_| async move { Ok(Some(agent_id)) },
            |_| async { Err("should not spawn".to_string()) },
            |agent_id| selected = Some(agent_id),
        )
        .await;

        assert_eq!(text, "Now talking to agent: captain");
        assert_eq!(selected, Some(agent_id));
    }

    #[tokio::test]
    async fn agent_selection_spawns_missing_agent() {
        let agent_id = AgentId::new();
        let mut selected = None;
        let text = run_agent_selection_command(
            &args("researcher"),
            |_| async { Ok(None) },
            |_| async move { Ok(agent_id) },
            |agent_id| selected = Some(agent_id),
        )
        .await;

        assert_eq!(text, "Spawned and connected to agent: researcher");
        assert_eq!(selected, Some(agent_id));
    }

    #[tokio::test]
    async fn agent_selection_reports_spawn_error() {
        let mut selected = None;
        let text = run_agent_selection_command(
            &args("ghost"),
            |_| async { Ok(None) },
            |_| async { Err("manifest missing".to_string()) },
            |agent_id| selected = Some(agent_id),
        )
        .await;

        assert_eq!(
            text,
            "Agent 'ghost' not found and could not spawn: manifest missing"
        );
        assert!(selected.is_none());
    }

    #[tokio::test]
    async fn agent_selection_reports_find_error_without_spawn() {
        let mut selected = None;
        let text = run_agent_selection_command(
            &args("captain"),
            |_| async { Err("store unavailable".to_string()) },
            |_| async { Ok(AgentId::new()) },
            |agent_id| selected = Some(agent_id),
        )
        .await;

        assert_eq!(text, "Error finding agent: store unavailable");
        assert!(selected.is_none());
    }
}

//! Selected-agent helpers for channel commands.

use super::channel_mapping::channel_type_from_str;
use crate::router::AgentRouter;
use crate::types::ChannelUser;
use captain_types::agent::AgentId;
use std::future::Future;

const NO_AGENT_SELECTED_MESSAGE: &str = "No agent selected. Use /agent <name> first.";

pub(crate) async fn run_selected_agent_command<F, Fut>(
    router: &AgentRouter,
    channel: &str,
    sender: &ChannelUser,
    action: F,
) -> String
where
    F: FnOnce(AgentId) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let Some(agent_id) = resolve_selected_agent(router, channel, sender) else {
        return NO_AGENT_SELECTED_MESSAGE.to_string();
    };
    action(agent_id)
        .await
        .unwrap_or_else(|e| format!("Error: {e}"))
}

pub(crate) fn parse_thinking_enabled(args: &[String]) -> bool {
    args.first().map(|a| a == "on").unwrap_or(true)
}

fn resolve_selected_agent(
    router: &AgentRouter,
    channel: &str,
    sender: &ChannelUser,
) -> Option<AgentId> {
    router.resolve(
        &channel_type_from_str(channel),
        &sender.platform_id,
        sender.captain_user.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sender(platform_id: &str, captain_user: Option<&str>) -> ChannelUser {
        ChannelUser {
            platform_id: platform_id.to_string(),
            display_name: "Ada".to_string(),
            captain_user: captain_user.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn selected_agent_command_runs_action_for_resolved_agent() {
        let router = AgentRouter::new();
        let agent_id = AgentId::new();
        router.set_user_default("captain-user".to_string(), agent_id);
        let sender = sender("platform-user", Some("captain-user"));

        let text =
            run_selected_agent_command(&router, "telegram", &sender, |resolved| async move {
                Ok(format!("acted:{resolved}"))
            })
            .await;

        assert_eq!(text, format!("acted:{agent_id}"));
    }

    #[tokio::test]
    async fn selected_agent_command_reports_missing_agent() {
        let router = AgentRouter::new();
        let sender = sender("platform-user", None);

        let text = run_selected_agent_command(&router, "telegram", &sender, |_resolved| async {
            Ok("should not run".to_string())
        })
        .await;

        assert_eq!(text, NO_AGENT_SELECTED_MESSAGE);
    }

    #[tokio::test]
    async fn selected_agent_command_formats_action_errors() {
        let router = AgentRouter::new();
        let agent_id = AgentId::new();
        router.set_user_default("platform-user".to_string(), agent_id);
        let sender = sender("platform-user", None);

        let text = run_selected_agent_command(&router, "telegram", &sender, |_resolved| async {
            Err("boom".to_string())
        })
        .await;

        assert_eq!(text, "Error: boom");
    }

    #[test]
    fn thinking_defaults_to_on_and_only_on_enables() {
        assert!(parse_thinking_enabled(&[]));
        assert!(parse_thinking_enabled(&["on".to_string()]));
        assert!(!parse_thinking_enabled(&["off".to_string()]));
        assert!(!parse_thinking_enabled(&["maybe".to_string()]));
    }
}

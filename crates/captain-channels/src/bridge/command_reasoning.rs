//! Exact per-agent reasoning control for channel commands and Telegram buttons.

use super::command_agent::resolve_selected_agent;
use super::command_dispatch::CommandContext;
use super::command_response::CommandResponse;
use crate::telegram_callbacks::build_reasoning_keyboard;
use captain_types::agent::AgentId;
use captain_types::reasoning::{AgentReasoningStatus, ReasoningSelectionSource};

const CALLBACK_AGENT_PREFIX_LEN: usize = 12;

pub(super) async fn handle_reasoning_command(
    args: &[String],
    ctx: &CommandContext<'_>,
) -> CommandResponse {
    if args.len() > 2 || (args.len() == 2 && !args[0].starts_with("@agent:")) {
        return CommandResponse::text("Usage : /reasoning [auto|niveau]");
    }
    let (agent_prefix, effort) = parse_reasoning_args(args);
    let agents = ctx.handle.list_agents().await.unwrap_or_default();
    let agent_id = if let Some(prefix) = agent_prefix {
        match resolve_agent_prefix(&agents, prefix) {
            Ok(agent_id) => agent_id,
            Err(error) => return CommandResponse::text(error),
        }
    } else {
        let Some(agent_id) = resolve_selected_agent(ctx.router, ctx.channel, ctx.sender) else {
            return CommandResponse::text("Aucun agent sélectionné. Utilise /agent <nom> d'abord.");
        };
        agent_id
    };

    let status = if let Some(effort) = effort {
        let effort = (!effort.eq_ignore_ascii_case("auto")).then_some(effort);
        ctx.handle.set_reasoning_effort(agent_id, effort).await
    } else {
        ctx.handle.reasoning_status(agent_id).await
    };
    let status = match status {
        Ok(status) => status,
        Err(error) => return CommandResponse::text(format!("Raisonnement refusé : {error}")),
    };
    let name = agents
        .iter()
        .find(|(id, _)| *id == agent_id)
        .map(|(_, name)| name.as_str())
        .unwrap_or("agent");
    let text = format_reasoning_status(name, &status);
    if ctx.channel == "telegram" && status.supported {
        CommandResponse::with_reply_markup(
            text,
            build_reasoning_keyboard(&agent_id.to_string()[..CALLBACK_AGENT_PREFIX_LEN], &status),
        )
    } else {
        CommandResponse::text(text)
    }
}

fn parse_reasoning_args(args: &[String]) -> (Option<&str>, Option<&str>) {
    match args {
        [agent, effort] if agent.starts_with("@agent:") => {
            (agent.strip_prefix("@agent:"), Some(effort.as_str()))
        }
        [effort] => (None, Some(effort.as_str())),
        [] => (None, None),
        _ => (None, None),
    }
}

fn resolve_agent_prefix(agents: &[(AgentId, String)], prefix: &str) -> Result<AgentId, String> {
    if prefix.len() != CALLBACK_AGENT_PREFIX_LEN
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err("Bouton de raisonnement invalide ou expiré.".to_string());
    }
    let matches = agents
        .iter()
        .filter(|(id, _)| id.to_string().starts_with(prefix))
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [agent_id] => Ok(*agent_id),
        [] => Err("Cet agent n'existe plus ; ouvre une nouvelle carte /reasoning.".to_string()),
        _ => Err("Préfixe agent ambigu ; ouvre une nouvelle carte /reasoning.".to_string()),
    }
}

fn format_reasoning_status(name: &str, status: &AgentReasoningStatus) -> String {
    let model = format!("{}/{}", status.provider, status.model);
    if !status.supported {
        return format!(
            "## Raisonnement\n\n**Agent :** `{name}`\n**Modèle :** `{model}`\n**Contrôle :** non configurable ; le provider décide."
        );
    }
    let configured = status
        .configured_effort
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "auto".to_string());
    let effective = status
        .effective_effort
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_else(|| "provider".to_string());
    let source = match status.source {
        ReasoningSelectionSource::AgentOverride => "override durable de l'agent",
        ReasoningSelectionSource::ModelDefault => "défaut publié du modèle",
        ReasoningSelectionSource::ProviderDefault => "défaut du provider",
        ReasoningSelectionSource::Unsupported => "non configurable",
    };
    let provenance = if status.reported_by_provider {
        "catalogue Codex vivant"
    } else {
        "fallback runtime conservateur"
    };
    let ultra_note = if status
        .options
        .iter()
        .any(|option| option.effort.as_str() == "ultra")
    {
        "\n\n**Ultra :** effort modèle `max` + délégation proactive sur l'agent racine."
    } else {
        ""
    };
    format!(
        "## Raisonnement\n\n**Agent :** `{name}`\n**Modèle :** `{model}`\n**Sélection :** `{configured}` → **`{effective}`**\n**Source :** {source} · {provenance}\n\n`Auto` omet l'override et laisse le modèle choisir. `None`, s'il est proposé, est explicite.{ultra_note}\n\nChoisis un niveau ci-dessous."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_callback_prefix_never_falls_back_to_selected_agent() {
        let first = AgentId::new();
        let second = AgentId::new();
        let agents = vec![(first, "first".to_string()), (second, "second".to_string())];
        let prefix = &second.to_string()[..CALLBACK_AGENT_PREFIX_LEN];
        assert_eq!(resolve_agent_prefix(&agents, prefix).unwrap(), second);
        assert!(resolve_agent_prefix(&agents, "deadbeefdead").is_err());
    }

    #[test]
    fn typed_and_callback_arguments_stay_unambiguous() {
        assert_eq!(parse_reasoning_args(&[]), (None, None));
        assert_eq!(
            parse_reasoning_args(&["high".to_string()]),
            (None, Some("high"))
        );
        assert_eq!(
            parse_reasoning_args(&["@agent:01234567-abc".to_string(), "auto".to_string()]),
            (Some("01234567-abc"), Some("auto"))
        );
    }

    #[test]
    fn rich_reasoning_status_distinguishes_auto_none_and_ultra() {
        let status = AgentReasoningStatus {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
            supported: true,
            configured_effort: None,
            effective_effort: Some("low".parse().unwrap()),
            source: ReasoningSelectionSource::ModelDefault,
            override_valid: true,
            options: ["none", "low", "ultra"]
                .into_iter()
                .map(|effort| captain_types::reasoning::ReasoningEffortOption {
                    effort: effort.parse().unwrap(),
                    description: None,
                })
                .collect(),
            reported_by_provider: true,
        };

        let text = format_reasoning_status("captain", &status);
        assert!(text.contains("`Auto` omet l'override"));
        assert!(text.contains("`None`"));
        assert!(text.contains("effort modèle `max` + délégation proactive"));
    }
}

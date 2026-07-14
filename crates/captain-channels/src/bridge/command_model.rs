//! Model command handlers for channel commands.

use super::channel_mapping::channel_type_from_str;
use super::command_dispatch::CommandContext;
use super::command_response::CommandResponse;
use super::model_switch_callback::parse_model_switch_callback;
use super::model_switch_decision::{classify_model_switch_plan, ModelSwitchPlanDecision};
use super::model_switch_format::{
    format_model_switch_apply_result, format_model_switch_blocked, format_model_switch_prompt,
};
use super::model_switch_pending::{remember_pending_model_switch, PendingModelSwitch};
use std::time::Instant;

pub(super) async fn handle_model_command(
    args: &[String],
    ctx: CommandContext<'_>,
) -> CommandResponse {
    let Some(agent_id) = ctx.router.resolve(
        &channel_type_from_str(ctx.channel),
        &ctx.sender.platform_id,
        ctx.sender.captain_user.as_deref(),
    ) else {
        return CommandResponse::text("No agent selected. Use /agent <name> first.");
    };

    // RBAC: `/model` can mutate the agent configuration, so it is checked
    // before both preflight/apply and the read-only current-model display.
    if let Err(denied) = ctx
        .handle
        .authorize_channel_user(ctx.channel, ctx.sender_user_id, "chat")
        .await
    {
        return CommandResponse::text(format!("Access denied: {denied}"));
    }

    if args.is_empty() {
        return CommandResponse::text(
            ctx.handle
                .set_model(agent_id, "")
                .await
                .unwrap_or_else(|error| format!("Error: {error}")),
        );
    }

    let requested_model = &args[0];
    let plan = match ctx
        .handle
        .model_switch_plan(agent_id, requested_model)
        .await
    {
        Ok(plan) => plan,
        Err(error) => {
            return CommandResponse::text(format!("Model switch preflight failed: {error}"));
        }
    };

    match classify_model_switch_plan(&plan, requested_model) {
        ModelSwitchPlanDecision::Blocked => {
            CommandResponse::text(format_model_switch_blocked(&plan))
        }
        ModelSwitchPlanDecision::Unchanged {
            provider,
            target_model,
        } => CommandResponse::text(format!(
            "C'est déjà le modèle actif : {provider} / {target_model}."
        )),
        ModelSwitchPlanDecision::NeedsConfirmation {
            target_model,
            target_provider,
            recommended_session_strategy,
        } => {
            let plan_id = uuid::Uuid::new_v4().to_string();
            remember_pending_model_switch(
                ctx.pending_model_switches,
                plan_id.clone(),
                PendingModelSwitch {
                    agent_id,
                    target_model,
                    target_provider,
                    created_at: Instant::now(),
                },
            );
            let keyboard = crate::telegram::build_model_switch_keyboard_with_recommendation(
                &plan_id,
                recommended_session_strategy.as_deref(),
            );
            CommandResponse::with_reply_markup(format_model_switch_prompt(&plan), keyboard)
        }
        ModelSwitchPlanDecision::ApplyNow {
            target_model,
            target_provider,
        } => match ctx
            .handle
            .model_switch_apply(agent_id, &target_model, target_provider.as_deref(), None)
            .await
        {
            Ok(result) => CommandResponse::text(format_model_switch_apply_result(&result)),
            Err(error) => CommandResponse::text(format!(
                "Je ne peux pas appliquer ce switch pour l'instant : {error}"
            )),
        },
    }
}

pub(super) async fn handle_model_switch_callback(
    args: &[String],
    ctx: CommandContext<'_>,
) -> CommandResponse {
    let Some(selection) = parse_model_switch_callback(args) else {
        return CommandResponse::text("Choix de switch invalide. Relance /model <modèle>.");
    };

    // RBAC is repeated on callback clicks because buttons can outlive the
    // original message and Telegram forwards a fresh user identity with each
    // callback_query.
    if let Err(denied) = ctx
        .handle
        .authorize_channel_user(ctx.channel, ctx.sender_user_id, "chat")
        .await
    {
        return CommandResponse::text(format!("Access denied: {denied}"));
    }

    let Some((_, pending)) = ctx.pending_model_switches.remove(selection.plan_id) else {
        return CommandResponse::text(
            "Ce choix de switch a expiré ou a été remplacé. Relance /model <modèle>.",
        );
    };
    if pending.is_expired() {
        return CommandResponse::text(
            "Ce choix de switch a expiré (5 min). Relance /model <modèle>.",
        );
    }

    if selection.is_cancel() {
        return CommandResponse::text("Switch annulé. Je garde le modèle actuel.");
    }

    match ctx
        .handle
        .model_switch_apply(
            pending.agent_id,
            &pending.target_model,
            pending.target_provider.as_deref(),
            Some(selection.choice),
        )
        .await
    {
        Ok(result) => CommandResponse::text(format_model_switch_apply_result(&result)),
        Err(error) => CommandResponse::text(format!(
            "Je ne peux pas appliquer ce switch pour l'instant : {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::super::model_switch_pending::PendingModelSwitchStore;
    use super::super::ChannelBridgeHandle;
    use super::*;
    use crate::router::AgentRouter;
    use crate::types::ChannelUser;
    use async_trait::async_trait;
    use captain_types::agent::AgentId;
    use dashmap::DashMap;
    use std::sync::Arc;

    struct TestHandle {
        set_model_response: Result<String, String>,
        auth_result: Result<(), String>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for TestHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(format!("Echo: {message}"))
        }

        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(None)
        }

        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(Vec::new())
        }

        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("spawn unavailable".to_string())
        }

        async fn set_model(&self, _agent_id: AgentId, _model: &str) -> Result<String, String> {
            self.set_model_response.clone()
        }

        async fn authorize_channel_user(
            &self,
            _channel_type: &str,
            _platform_id: &str,
            _action: &str,
        ) -> Result<(), String> {
            self.auth_result.clone()
        }
    }

    fn test_handle() -> Arc<dyn ChannelBridgeHandle> {
        Arc::new(TestHandle {
            set_model_response: Ok("Current model: codex/gpt-5.5".to_string()),
            auth_result: Ok(()),
        })
    }

    fn sender() -> ChannelUser {
        ChannelUser {
            platform_id: "user1".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        }
    }

    fn pending_model_switches() -> PendingModelSwitchStore {
        Arc::new(DashMap::new())
    }

    fn context<'a>(
        handle: &'a Arc<dyn ChannelBridgeHandle>,
        router: &'a Arc<AgentRouter>,
        sender: &'a ChannelUser,
        pending_model_switches: &'a PendingModelSwitchStore,
    ) -> CommandContext<'a> {
        CommandContext {
            handle,
            router,
            sender,
            sender_user_id: &sender.platform_id,
            channel: "telegram",
            thread_id: None,
            source_message_id: None,
            pending_model_switches,
        }
    }

    #[tokio::test]
    async fn model_command_requires_selected_agent() {
        let handle = test_handle();
        let router = Arc::new(AgentRouter::new());
        let sender = sender();
        let pending_model_switches = pending_model_switches();

        let response = handle_model_command(
            &["gpt-5.5".to_string()],
            context(&handle, &router, &sender, &pending_model_switches),
        )
        .await;

        assert_eq!(&*response, "No agent selected. Use /agent <name> first.");
    }

    #[tokio::test]
    async fn model_command_empty_args_shows_current_model() {
        let agent_id = AgentId::new();
        let handle = test_handle();
        let router = Arc::new(AgentRouter::new());
        router.set_user_default("user1".to_string(), agent_id);
        let sender = sender();
        let pending_model_switches = pending_model_switches();

        let response = handle_model_command(
            &[],
            context(&handle, &router, &sender, &pending_model_switches),
        )
        .await;

        assert_eq!(&*response, "Current model: codex/gpt-5.5");
    }

    #[tokio::test]
    async fn model_switch_callback_rejects_invalid_payload() {
        let handle = test_handle();
        let router = Arc::new(AgentRouter::new());
        let sender = sender();
        let pending_model_switches = pending_model_switches();

        let response = handle_model_switch_callback(
            &["not-enough".to_string()],
            context(&handle, &router, &sender, &pending_model_switches),
        )
        .await;

        assert_eq!(
            &*response,
            "Choix de switch invalide. Relance /model <modèle>."
        );
    }
}

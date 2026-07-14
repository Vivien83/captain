//! Dispatcher for channel slash commands.

use super::command_agent::{parse_thinking_enabled, run_selected_agent_command};
use super::command_agent_select::run_agent_selection_command;
use super::command_automation::{run_schedule_command, run_trigger_command, run_workflow_command};
use super::command_format::{format_agents_message, format_help_message, format_start_message};
use super::command_home::{
    format_get_home_response, format_set_home_error, format_set_home_success, resolve_home_chat_id,
};
use super::command_model::{handle_model_command, handle_model_switch_callback};
use super::command_response::CommandResponse;
use super::command_review::{
    run_id_prefix_command, run_project_answer_command, run_skill_approval_command,
};
use super::model_switch_pending::PendingModelSwitchStore;
use super::ChannelBridgeHandle;
use crate::router::AgentRouter;
use crate::types::ChannelUser;
use std::sync::Arc;

pub(super) struct CommandContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) router: &'a Arc<AgentRouter>,
    pub(super) sender: &'a ChannelUser,
    pub(super) sender_user_id: &'a str,
    pub(super) channel: &'a str,
    pub(super) thread_id: Option<&'a str>,
    pub(super) source_message_id: Option<&'a str>,
    pub(super) pending_model_switches: &'a PendingModelSwitchStore,
}

/// Handle a bot command (returns the response text and optional rich metadata).
///
/// **RBAC contract**: this function does NOT check `is_authorized` itself.
/// Sender authorisation is the responsibility of the channel adapter's parse
/// path: an unauthorised user is filtered out before its message ever reaches
/// `dispatch_message` -> `handle_command`. As of 2026-05-04, four adapters
/// honour this contract (Telegram, Discord, Signal, WhatsApp); the remaining
/// adapters do not yet apply RBAC and any sensitive command (model, compact,
/// stop, usage, think, new/clear) routed through them is reachable by any
/// sender. See `docs/captain-tools/channel.md#rbac-coverage` for the
/// per-adapter matrix.
pub(super) async fn handle_command(
    name: &str,
    args: &[String],
    ctx: CommandContext<'_>,
) -> CommandResponse {
    if name == "model" {
        return handle_model_command(args, ctx).await;
    }
    if name == "model_switch" {
        return handle_model_switch_callback(args, ctx).await;
    }

    let text = dispatch_command_text(name, args, &ctx).await;
    command_response_for(name, text)
}

async fn dispatch_command_text(name: &str, args: &[String], ctx: &CommandContext<'_>) -> String {
    if let Some(text) = dispatch_intro_command(name, args, ctx).await {
        return text;
    }
    if let Some(text) = dispatch_daemon_command(name, args, ctx).await {
        return text;
    }
    if let Some(text) = dispatch_session_command(name, args, ctx).await {
        return text;
    }
    if let Some(text) = dispatch_capability_command(name, ctx).await {
        return text;
    }
    if let Some(text) = dispatch_automation_command(name, args, ctx).await {
        return text;
    }
    if let Some(text) = dispatch_review_command(name, args, ctx).await {
        return text;
    }
    if let Some(text) = dispatch_network_command(name, ctx).await {
        return text;
    }
    if let Some(text) = dispatch_home_command(name, args, ctx).await {
        return text;
    }
    format!("Unknown command: /{name}")
}

async fn dispatch_intro_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    let handle = ctx.handle;
    match name {
        "start" => {
            let agents = handle.list_agents().await.unwrap_or_default();
            Some(format_start_message(&agents))
        }
        "help" => Some(format_help_message()),
        "agents" => {
            let agents = handle.list_agents().await.unwrap_or_default();
            Some(format_agents_message(&agents))
        }
        "agent" => {
            let router = ctx.router;
            let platform_id = ctx.sender.platform_id.clone();
            Some(
                run_agent_selection_command(
                    args,
                    |agent_name| async move { handle.find_agent_by_name(&agent_name).await },
                    |agent_name| async move { handle.spawn_agent_by_name(&agent_name).await },
                    |agent_id| router.set_user_default(platform_id.clone(), agent_id),
                )
                .await,
            )
        }
        _ => None,
    }
}

async fn dispatch_daemon_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    match name {
        "status" | "health" | "version" | "config" | "reload" | "restart" | "shutdown" => Some(
            ctx.handle
                .daemon_command_text(
                    name,
                    args,
                    ctx.channel,
                    &ctx.sender.platform_id,
                    ctx.sender_user_id,
                    ctx.thread_id,
                    ctx.source_message_id,
                )
                .await,
        ),
        _ => None,
    }
}

async fn dispatch_session_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    let handle = ctx.handle;
    let router = ctx.router;
    let sender = ctx.sender;
    let channel = ctx.channel;
    match name {
        "new" | "clear" => Some(
            run_selected_agent_command(router, channel, sender, |aid| handle.reset_session(aid))
                .await,
        ),
        "compact" => Some(
            run_selected_agent_command(router, channel, sender, |aid| handle.compact_session(aid))
                .await,
        ),
        "stop" => Some(
            run_selected_agent_command(router, channel, sender, |aid| handle.stop_run(aid)).await,
        ),
        "usage" => Some(
            run_selected_agent_command(router, channel, sender, |aid| handle.session_usage(aid))
                .await,
        ),
        "think" => {
            let on = parse_thinking_enabled(args);
            Some(
                run_selected_agent_command(router, channel, sender, |aid| {
                    handle.set_thinking(aid, on)
                })
                .await,
            )
        }
        _ => None,
    }
}

async fn dispatch_capability_command(name: &str, ctx: &CommandContext<'_>) -> Option<String> {
    match name {
        "models" => Some(ctx.handle.list_models_text().await),
        "providers" => Some(ctx.handle.list_providers_text().await),
        "skills" => Some(ctx.handle.list_skills_text().await),
        "hands" => Some(ctx.handle.list_hands_text().await),
        _ => None,
    }
}

async fn dispatch_automation_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    let handle = ctx.handle;
    match name {
        "workflows" => Some(handle.list_workflows_text().await),
        "workflow" => Some(
            run_workflow_command(args, |wf_name, input| async move {
                handle.run_workflow_text(&wf_name, &input).await
            })
            .await,
        ),
        "triggers" => Some(handle.list_triggers_text().await),
        "trigger" => Some(
            run_trigger_command(
                args,
                |agent_name, pattern, prompt| async move {
                    handle
                        .create_trigger_text(&agent_name, &pattern, &prompt)
                        .await
                },
                |id_prefix| async move { handle.delete_trigger_text(&id_prefix).await },
            )
            .await,
        ),
        "schedules" => Some(handle.list_schedules_text().await),
        "schedule" => Some(
            run_schedule_command(args, |action, rest| async move {
                handle.manage_schedule_text(&action, &rest).await
            })
            .await,
        ),
        _ => None,
    }
}

async fn dispatch_review_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    if let Some(text) = dispatch_review_list_command(name, ctx).await {
        return Some(text);
    }
    if let Some(text) = dispatch_approval_resolution_command(name, args, ctx).await {
        return Some(text);
    }
    if let Some(text) = dispatch_learning_review_command(name, args, ctx).await {
        return Some(text);
    }
    if let Some(text) = dispatch_skill_review_command(name, args, ctx).await {
        return Some(text);
    }
    if let Some(text) = dispatch_project_review_command(name, args, ctx).await {
        return Some(text);
    }
    None
}

async fn dispatch_review_list_command(name: &str, ctx: &CommandContext<'_>) -> Option<String> {
    match name {
        "approvals" => Some(ctx.handle.list_approvals_text().await),
        "learnings" => Some(ctx.handle.list_learning_review_text().await),
        "skill_proposals" => Some(ctx.handle.list_skill_proposals_text().await),
        "skill_refinements" => Some(ctx.handle.list_skill_refinements_text().await),
        _ => None,
    }
}

async fn dispatch_approval_resolution_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    let handle = ctx.handle;
    match name {
        "approve" => Some(
            run_id_prefix_command(args, "approve", |id| async move {
                handle
                    .resolve_approval_text_with(
                        &id,
                        captain_types::approval::ApprovalDecision::Approved,
                    )
                    .await
            })
            .await,
        ),
        "approve_session" => Some(
            run_id_prefix_command(args, "approve_session", |id| async move {
                handle
                    .resolve_approval_text_with(
                        &id,
                        captain_types::approval::ApprovalDecision::ApprovedSession,
                    )
                    .await
            })
            .await,
        ),
        "approve_always" => Some(
            run_id_prefix_command(args, "approve_always", |id| async move {
                handle
                    .resolve_approval_text_with(
                        &id,
                        captain_types::approval::ApprovalDecision::ApprovedAlways,
                    )
                    .await
            })
            .await,
        ),
        "reject" => Some(
            run_id_prefix_command(args, "reject", |id| async move {
                handle
                    .resolve_approval_text_with(
                        &id,
                        captain_types::approval::ApprovalDecision::Denied,
                    )
                    .await
            })
            .await,
        ),
        _ => None,
    }
}

async fn dispatch_learning_review_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    let handle = ctx.handle;
    match name {
        "learn_approve" => Some(
            run_id_prefix_command(args, "learn_approve", |id| async move {
                handle.resolve_learning_review_text(&id, true).await
            })
            .await,
        ),
        "learn_reject" => Some(
            run_id_prefix_command(args, "learn_reject", |id| async move {
                handle.resolve_learning_review_text(&id, false).await
            })
            .await,
        ),
        _ => None,
    }
}

async fn dispatch_skill_review_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    let handle = ctx.handle;
    match name {
        "skill_approve" => Some(
            run_skill_approval_command(args, |id| async move {
                handle.resolve_skill_proposal_text(&id, true, true).await
            })
            .await,
        ),
        "skill_reject" => Some(
            run_id_prefix_command(args, "skill_reject", |id| async move {
                handle.resolve_skill_proposal_text(&id, false, false).await
            })
            .await,
        ),
        "skill_refine_approve" => Some(
            run_id_prefix_command(args, "skill_refine_approve", |id| async move {
                handle.resolve_skill_refinement_text(&id, true).await
            })
            .await,
        ),
        "skill_refine_reject" => Some(
            run_id_prefix_command(args, "skill_refine_reject", |id| async move {
                handle.resolve_skill_refinement_text(&id, false).await
            })
            .await,
        ),
        _ => None,
    }
}

async fn dispatch_project_review_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    let handle = ctx.handle;
    match name {
        "project_answer" => Some(
            run_project_answer_command(args, |id, answer| async move {
                handle.resolve_project_ask_text(&id, &answer).await
            })
            .await,
        ),
        _ => None,
    }
}

async fn dispatch_network_command(name: &str, ctx: &CommandContext<'_>) -> Option<String> {
    match name {
        "budget" => Some(ctx.handle.budget_text().await),
        "peers" => Some(ctx.handle.peers_text().await),
        "a2a" => Some(ctx.handle.a2a_agents_text().await),
        _ => None,
    }
}

async fn dispatch_home_command(
    name: &str,
    args: &[String],
    ctx: &CommandContext<'_>,
) -> Option<String> {
    let handle = ctx.handle;
    match name {
        "sethome" => {
            let chat_id = resolve_home_chat_id(args, &ctx.sender.platform_id);
            Some(
                match handle
                    .set_home_channel(ctx.channel, &ctx.sender.platform_id, &chat_id)
                    .await
                {
                    Ok(_) => format_set_home_success(ctx.channel, &chat_id),
                    Err(error) => format_set_home_error(&error),
                },
            )
        }
        "gethome" => {
            let chat_id = handle
                .get_home_channel(ctx.channel, &ctx.sender.platform_id)
                .await;
            Some(format_get_home_response(ctx.channel, chat_id.as_deref()))
        }
        _ => None,
    }
}

fn command_response_for(name: &str, text: String) -> CommandResponse {
    if name == "config" {
        CommandResponse::raw(text)
    } else {
        CommandResponse::text(text)
    }
}

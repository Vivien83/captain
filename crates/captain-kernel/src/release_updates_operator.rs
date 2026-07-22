//! Exact, authenticated Telegram decisions for one runtime release candidate.

use std::path::Path;

use captain_types::release_update::{
    RuntimeUpdateInstallMode, RuntimeUpdateNoticeKind, RuntimeUpdateOperatorContext,
    RuntimeUpdateOperatorResolution, RuntimeUpdateResolutionStatus, RuntimeUpdateTelegramAction,
};

use super::{
    allocate_decision_version, enqueue_notice, format_unix_ms, record_decision,
    runtime_update_log_path, suppress_actionable_outbox, CaptainKernel, RuntimeUpdateAttempt,
    RuntimeUpdateState, DEFER_INTERVAL_MS,
};

#[derive(Debug)]
pub(super) enum OperatorDecisionEffect {
    Resolved(RuntimeUpdateOperatorResolution),
    Launch {
        attempt: RuntimeUpdateAttempt,
        resolution: RuntimeUpdateOperatorResolution,
    },
}

pub(super) fn apply_operator_decision(
    state: &mut RuntimeUpdateState,
    callback: &captain_channels::telegram::RuntimeUpdateTelegramCallback,
    actor: &str,
    captain_home: &Path,
    now: i64,
) -> Result<OperatorDecisionEffect, String> {
    let candidate = state
        .pending
        .as_ref()
        .ok_or_else(|| "Cette proposition n'est plus active.".to_string())?
        .clone();
    if candidate.token != callback.token || candidate.decision_version != callback.decision_version
    {
        return Err(
            "Cette carte est périmée ; utilise la notification la plus récente.".to_string(),
        );
    }
    if state.active_attempt.is_some() {
        return Err("Une mise à jour Captain est déjà en cours.".to_string());
    }

    let base_resolution = RuntimeUpdateOperatorResolution {
        status: RuntimeUpdateResolutionStatus::Refused,
        current_version: candidate.current_version.clone(),
        available_version: candidate.available_version.clone(),
        retire_keyboard: true,
        next_prompt_at: None,
        log_path: None,
    };
    match callback.action {
        RuntimeUpdateTelegramAction::Refuse => {
            suppress_actionable_outbox(state);
            state.pending = None;
            record_decision(state, &candidate, "refused", actor, now);
            Ok(OperatorDecisionEffect::Resolved(base_resolution))
        }
        RuntimeUpdateTelegramAction::Defer => {
            let deferred_until = schedule_reminder(state, now);
            record_decision(state, &candidate, "deferred", actor, now);
            Ok(OperatorDecisionEffect::Resolved(
                RuntimeUpdateOperatorResolution {
                    status: RuntimeUpdateResolutionStatus::Deferred,
                    next_prompt_at: Some(format_unix_ms(deferred_until)),
                    ..base_resolution
                },
            ))
        }
        RuntimeUpdateTelegramAction::Install => match candidate.install_mode {
            RuntimeUpdateInstallMode::Container => {
                let deferred_until = schedule_reminder(state, now);
                record_decision(state, &candidate, "container_manual", actor, now);
                Ok(OperatorDecisionEffect::Resolved(
                    RuntimeUpdateOperatorResolution {
                        status: RuntimeUpdateResolutionStatus::ContainerManual,
                        next_prompt_at: Some(format_unix_ms(deferred_until)),
                        ..base_resolution
                    },
                ))
            }
            RuntimeUpdateInstallMode::Manual => {
                let deferred_until = schedule_reminder(state, now);
                record_decision(state, &candidate, "platform_manual", actor, now);
                Ok(OperatorDecisionEffect::Resolved(
                    RuntimeUpdateOperatorResolution {
                        status: RuntimeUpdateResolutionStatus::PlatformManual,
                        next_prompt_at: Some(format_unix_ms(deferred_until)),
                        ..base_resolution
                    },
                ))
            }
            RuntimeUpdateInstallMode::SelfUpdate => {
                suppress_actionable_outbox(state);
                let attempt_id = uuid::Uuid::new_v4().to_string();
                let log_path = runtime_update_log_path(captain_home, &attempt_id)
                    .display()
                    .to_string();
                let attempt = RuntimeUpdateAttempt {
                    attempt_id,
                    requested_version: candidate.release_tag.clone(),
                    started_at_unix_ms: now,
                    log_path: log_path.clone(),
                };
                state.active_attempt = Some(attempt.clone());
                record_decision(state, &candidate, "install_started", actor, now);
                Ok(OperatorDecisionEffect::Launch {
                    attempt,
                    resolution: RuntimeUpdateOperatorResolution {
                        status: RuntimeUpdateResolutionStatus::InstallStarted,
                        log_path: Some(log_path),
                        ..base_resolution
                    },
                })
            }
        },
    }
}

fn schedule_reminder(state: &mut RuntimeUpdateState, now: i64) -> i64 {
    suppress_actionable_outbox(state);
    let deferred_until = now.saturating_add(DEFER_INTERVAL_MS);
    let decision_version = allocate_decision_version(state);
    let pending = state.pending.as_mut().expect("candidate checked above");
    pending.decision_version = decision_version;
    pending.deferred_until_unix_ms = Some(deferred_until);
    let reminder = pending.clone();
    enqueue_notice(
        state,
        &reminder,
        RuntimeUpdateNoticeKind::Reminder,
        None,
        deferred_until,
    );
    deferred_until
}

pub(super) fn authorize_runtime_update_operator(
    kernel: &CaptainKernel,
    actor: &str,
    context: &RuntimeUpdateOperatorContext,
) -> Result<(), String> {
    let telegram = kernel
        .config
        .channels
        .telegram
        .as_ref()
        .ok_or_else(|| "Le canal Telegram n'est pas configuré.".to_string())?;
    authorize_runtime_update_identity(telegram, actor, context)
}

pub(super) fn authorize_runtime_update_identity(
    telegram: &captain_types::config::TelegramConfig,
    actor: &str,
    context: &RuntimeUpdateOperatorContext,
) -> Result<(), String> {
    let expected_chat = telegram
        .default_chat_id
        .as_deref()
        .map(str::trim)
        .filter(|chat| !chat.is_empty())
        .ok_or_else(|| "Aucun chat opérateur Telegram n'est configuré.".to_string())?;
    if context.chat_id.trim() != expected_chat {
        return Err("Cette carte n'appartient pas au chat opérateur configuré.".to_string());
    }
    let user_id = actor
        .strip_prefix("telegram:")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Identité opérateur Telegram invalide.".to_string())?;
    if !telegram
        .allowed_users
        .iter()
        .any(|allowed| allowed == user_id)
    {
        return Err(
            "La mise à jour exige un identifiant Telegram explicitement autorisé ; le joker `*` ne suffit pas."
                .to_string(),
        );
    }
    Ok(())
}

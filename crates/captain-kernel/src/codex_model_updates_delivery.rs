use std::collections::BTreeSet;
use std::sync::Arc;

use captain_runtime::kernel_handle::KernelHandle;
use tracing::warn;

use super::{CaptainKernel, CodexModelUpdate};

pub(super) async fn deliver_pending_telegram_notifications(kernel: &Arc<CaptainKernel>) {
    let Some(recipient) = kernel
        .config
        .channels
        .telegram
        .as_ref()
        .and_then(|config| config.default_chat_id.as_deref())
        .filter(|chat_id| !chat_id.trim().is_empty())
        .map(str::to_string)
    else {
        return;
    };
    if !kernel.channel_adapters.contains_key("telegram") {
        return;
    }

    let updates = match kernel.codex_model_update_snapshot() {
        Ok(snapshot) => snapshot
            .pending
            .into_iter()
            .filter(|update| update.telegram_notified_at.is_none())
            .collect::<Vec<_>>(),
        Err(error) => {
            warn!(error = %error, "failed to load Codex model notifications");
            return;
        }
    };
    if updates.is_empty() {
        return;
    }

    let message = format_telegram_notification(kernel, &updates);
    match <CaptainKernel as KernelHandle>::send_channel_message_from(
        kernel.as_ref(),
        "telegram",
        &recipient,
        &message,
        None,
        Some("captain"),
    )
    .await
    {
        Ok(_) => mark_telegram_notifications_sent(kernel, &updates),
        Err(error) => warn!(error = %error, "Codex model notification delivery failed"),
    }
}

fn mark_telegram_notifications_sent(kernel: &CaptainKernel, updates: &[CodexModelUpdate]) {
    let notified_at = chrono::Utc::now().to_rfc3339();
    let update_ids = updates
        .iter()
        .map(|update| update.model_id.clone())
        .collect::<BTreeSet<_>>();
    if let Err(error) = kernel.mutate_codex_model_update_state(|state| {
        for update in &mut state.pending {
            if update_ids.contains(&update.model_id) {
                update.telegram_notified_at = Some(notified_at.clone());
            }
        }
    }) {
        warn!(error = %error, "Codex model notification sent but marker persistence failed");
    }
}

fn format_telegram_notification(kernel: &CaptainKernel, updates: &[CodexModelUpdate]) -> String {
    let model_lines = updates
        .iter()
        .map(|update| format!("- {} (`{}`)", update.display_name, update.model_id))
        .collect::<Vec<_>>()
        .join("\n");
    let current = kernel
        .codex_model_update_agents()
        .first()
        .map(|agent| format!("{}: `{}`", agent.agent_name, agent.current_model))
        .unwrap_or_else(|| "Captain: inconnu".to_string());
    let first_model = &updates[0].model_id;

    if kernel
        .config
        .language
        .to_ascii_lowercase()
        .starts_with("fr")
    {
        format!(
            "Nouveau modèle Codex disponible\n{model_lines}\n\nModèle actuel — {current}\nAucun changement automatique. Réponds `Basculer vers {first_model}` pour préparer le switch sécurisé, ou `Garder le modèle actuel` pour conserver la configuration."
        )
    } else {
        format!(
            "New Codex model available\n{model_lines}\n\nCurrent model — {current}\nNothing changes automatically. Reply `Switch to {first_model}` to prepare the safe switch, or `Keep the current model` to retain the configuration."
        )
    }
}

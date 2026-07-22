//! Model-independent Telegram delivery for durable runtime-update cards.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use captain_channels::telegram::{build_runtime_update_keyboard, format_runtime_update_card};
use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};
use tracing::{info, warn};

use super::{now_unix_ms, CaptainKernel, RuntimeUpdateOutbox};

const IDLE_DELAY: Duration = Duration::from_secs(2);
const TARGET_DELAY: Duration = Duration::from_secs(15);
const ERROR_DELAY: Duration = Duration::from_secs(10);

pub(super) fn spawn_runtime_update_delivery_worker(kernel: Arc<CaptainKernel>) {
    tokio::spawn(run_runtime_update_delivery_worker(kernel));
}

async fn run_runtime_update_delivery_worker(kernel: Arc<CaptainKernel>) {
    let lease_owner = format!("captain:runtime-update-delivery:{}", std::process::id());
    let mut target_was_ready = false;
    let mut state_error_logged = false;
    loop {
        if kernel.supervisor.is_shutting_down() {
            break;
        }
        let Some((recipient, adapter)) = telegram_target(&kernel) else {
            if target_was_ready {
                info!("runtime update notifications paused until Telegram is ready");
            }
            target_was_ready = false;
            tokio::time::sleep(TARGET_DELAY).await;
            continue;
        };
        if !target_was_ready {
            info!("runtime update Telegram notification worker ready");
            target_was_ready = true;
        }

        let now = now_unix_ms();
        let claimed = match kernel.claim_runtime_update_outbox(&lease_owner, now) {
            Ok(claimed) => {
                state_error_logged = false;
                claimed
            }
            Err(error) => {
                if !state_error_logged {
                    warn!(error = %error, "runtime update notification claim failed");
                    state_error_logged = true;
                }
                tokio::time::sleep(ERROR_DELAY).await;
                continue;
            }
        };
        let Some(claimed) = claimed else {
            tokio::time::sleep(IDLE_DELAY).await;
            continue;
        };
        match send_runtime_update_notification(
            Arc::clone(&adapter),
            &recipient,
            &kernel.config.language,
            &claimed,
        )
        .await
        {
            Ok(external_message_id) => {
                if let Err(error) = kernel.complete_runtime_update_outbox(
                    &claimed,
                    external_message_id,
                    now_unix_ms(),
                ) {
                    warn!(
                        outbox_id = claimed.id,
                        error = %error,
                        "Telegram accepted runtime update card but receipt persistence failed"
                    );
                    tokio::time::sleep(ERROR_DELAY).await;
                }
            }
            Err(error) => {
                warn!(
                    outbox_id = claimed.id,
                    error, "runtime update notification failed"
                );
                if let Err(settle_error) =
                    kernel.retry_runtime_update_outbox(&claimed, &error, now_unix_ms())
                {
                    warn!(error = %settle_error, "runtime update notification retry could not be persisted");
                }
                tokio::time::sleep(ERROR_DELAY).await;
            }
        }
    }
}

fn telegram_target(kernel: &CaptainKernel) -> Option<(String, Arc<dyn ChannelAdapter>)> {
    let recipient = kernel
        .config
        .channels
        .telegram
        .as_ref()?
        .default_chat_id
        .as_deref()?
        .trim();
    if recipient.is_empty() {
        return None;
    }
    let adapter = kernel.channel_adapters.get("telegram")?;
    Some((recipient.to_string(), Arc::clone(adapter.value())))
}

pub(super) async fn send_runtime_update_notification(
    adapter: Arc<dyn ChannelAdapter>,
    recipient: &str,
    language: &str,
    delivery: &RuntimeUpdateOutbox,
) -> Result<Option<String>, String> {
    let user = ChannelUser {
        platform_id: recipient.to_string(),
        display_name: "Captain operator".to_string(),
        captain_user: None,
    };
    let content = ChannelContent::Text(format_runtime_update_card(&delivery.card, language));
    let mut metadata = HashMap::new();
    metadata.insert(
        "reply_markup".to_string(),
        build_runtime_update_keyboard(&delivery.card, language),
    );
    adapter
        .send_rich(&user, content, &metadata)
        .await
        .map_err(|error| error.to_string())
        .map(|message_id| message_id.map(|id| bounded_external_id(&id)))
}

fn bounded_external_id(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

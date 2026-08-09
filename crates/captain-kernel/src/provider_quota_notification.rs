//! Native Rich Telegram delivery for provider-confirmed quota resets.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_memory::provider_quota_reset::{
    ProviderQuotaResetNotification, ProviderQuotaResetWindowKind,
};
use chrono::{SecondsFormat, Utc};
use tracing::{info, warn};

use crate::CaptainKernel;

const IDLE_DELAY: Duration = Duration::from_secs(2);
const TARGET_DELAY: Duration = Duration::from_secs(15);
const ERROR_DELAY: Duration = Duration::from_secs(10);
const DELIVERY_LEASE_MS: i64 = 120_000;

pub(super) fn spawn_provider_quota_reset_delivery_worker(kernel: Arc<CaptainKernel>) {
    tokio::spawn(run_provider_quota_reset_delivery_worker(kernel));
}

async fn run_provider_quota_reset_delivery_worker(kernel: Arc<CaptainKernel>) {
    let lease_owner = format!("captain:quota-reset-delivery:{}", std::process::id());
    let mut target_was_ready = false;
    let mut state_error_logged = false;
    loop {
        if kernel.supervisor.is_shutting_down() {
            break;
        }
        let now = Utc::now().timestamp_millis().max(0);
        if kernel.config.channels.silent_mode {
            match kernel
                .memory
                .provider_quotas()
                .suppress_pending_reset_notifications("proactive notifications disabled", now)
            {
                Ok(count) => {
                    state_error_logged = false;
                    if count > 0 {
                        info!(
                            count,
                            "provider quota reset notifications suppressed by silent mode"
                        );
                    }
                }
                Err(error) if !state_error_logged => {
                    warn!(error = %error, "provider quota reset suppression could not be persisted");
                    state_error_logged = true;
                }
                Err(_) => {}
            }
            target_was_ready = false;
            tokio::time::sleep(TARGET_DELAY).await;
            continue;
        }
        let Some((recipient, adapter)) = telegram_target(&kernel) else {
            if target_was_ready {
                info!("provider quota reset notifications paused until Telegram is ready");
            }
            target_was_ready = false;
            tokio::time::sleep(TARGET_DELAY).await;
            continue;
        };
        if !target_was_ready {
            info!("provider quota reset Telegram notification worker ready");
            target_was_ready = true;
        }

        let claimed = match kernel.memory.provider_quotas().claim_reset_notification(
            &lease_owner,
            now,
            DELIVERY_LEASE_MS,
        ) {
            Ok(claimed) => {
                state_error_logged = false;
                claimed
            }
            Err(error) => {
                if !state_error_logged {
                    warn!(error = %error, "provider quota reset notification claim failed");
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

        match send_provider_quota_reset_notification(
            Arc::clone(&adapter),
            &recipient,
            &kernel.config.language,
            &claimed.notification,
        )
        .await
        {
            Ok(external_message_id) => {
                if let Err(error) = kernel.memory.provider_quotas().complete_reset_notification(
                    &claimed,
                    external_message_id.as_deref(),
                    Utc::now().timestamp_millis().max(0),
                ) {
                    warn!(
                        notification_id = claimed.notification.id,
                        error = %error,
                        "Telegram accepted quota reset card but receipt persistence failed"
                    );
                    tokio::time::sleep(ERROR_DELAY).await;
                } else {
                    info!(
                        provider = claimed.notification.provider,
                        limit_id = claimed.notification.limit_id,
                        windows = claimed.notification.windows.len(),
                        "provider quota reset notification delivered"
                    );
                }
            }
            Err(error) => {
                warn!(
                    notification_id = claimed.notification.id,
                    attempt = claimed.attempt_count,
                    error,
                    "provider quota reset notification delivery failed"
                );
                if let Err(settle_error) = kernel.memory.provider_quotas().retry_reset_notification(
                    &claimed,
                    &error,
                    Utc::now().timestamp_millis().max(0),
                ) {
                    warn!(error = %settle_error, "provider quota reset notification retry could not be persisted");
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

async fn send_provider_quota_reset_notification(
    adapter: Arc<dyn ChannelAdapter>,
    recipient: &str,
    language: &str,
    notification: &ProviderQuotaResetNotification,
) -> Result<Option<String>, String> {
    let user = ChannelUser {
        platform_id: recipient.to_string(),
        display_name: "Captain operator".to_string(),
        captain_user: None,
    };
    let content = ChannelContent::Text(format_provider_quota_reset_card(notification, language));
    adapter
        .send_rich(&user, content, &HashMap::new())
        .await
        .map_err(|error| error.to_string())
}

fn format_provider_quota_reset_card(
    notification: &ProviderQuotaResetNotification,
    language: &str,
) -> String {
    let french = language.to_ascii_lowercase().starts_with("fr");
    let limit = safe_markdown_text(
        notification
            .limit_name
            .as_deref()
            .unwrap_or(&notification.limit_id),
        120,
    );
    let provider = safe_markdown_text(&notification.provider, 48);
    let mut body = if french {
        format!(
            "## Quota {limit} réinitialisé\n\nLe provider a confirmé une nouvelle fenêtre d’abonnement.\n"
        )
    } else {
        format!("## {limit} quota reset\n\nThe provider confirmed a new subscription window.\n")
    };

    for window in &notification.windows {
        let label = window_label(window.kind, window.window_seconds, french);
        let previous = format_percent(window.previous_used_percent);
        let current = format_percent(window.current_used_percent);
        let remaining = format_percent((100.0 - window.current_used_percent).clamp(0.0, 100.0));
        let next_reset = window
            .current_resets_at
            .to_rfc3339_opts(SecondsFormat::Secs, true);
        if french {
            body.push_str(&format!(
                "\n### {label}\n**Utilisé** : `{previous} → {current}`\n**Disponible** : `{remaining}`\n**Prochaine réinitialisation** : `{next_reset}`\n"
            ));
        } else {
            body.push_str(&format!(
                "\n### {label}\n**Used**: `{previous} → {current}`\n**Available**: `{remaining}`\n**Next reset**: `{next_reset}`\n"
            ));
        }
    }

    let observed = notification
        .observed_at
        .to_rfc3339_opts(SecondsFormat::Secs, true);
    let event = notification.id.chars().take(8).collect::<String>();
    if french {
        body.push_str(&format!(
            "\n_Source officielle : {provider} · observé {observed} · événement `{event}`_"
        ));
    } else {
        body.push_str(&format!(
            "\n_Official source: {provider} · observed {observed} · event `{event}`_"
        ));
    }
    body
}

fn window_label(
    kind: ProviderQuotaResetWindowKind,
    window_seconds: Option<u64>,
    french: bool,
) -> String {
    let kind = match (kind, french) {
        (ProviderQuotaResetWindowKind::Primary, true) => "Fenêtre principale",
        (ProviderQuotaResetWindowKind::Primary, false) => "Primary window",
        (ProviderQuotaResetWindowKind::Secondary, true) => "Fenêtre longue",
        (ProviderQuotaResetWindowKind::Secondary, false) => "Long window",
        (ProviderQuotaResetWindowKind::SpendControl, true) => "Contrôle de dépense",
        (ProviderQuotaResetWindowKind::SpendControl, false) => "Spend control",
    };
    match window_seconds.and_then(|seconds| duration_label(seconds, french)) {
        Some(duration) => format!("{kind} · {duration}"),
        None => kind.to_string(),
    }
}

// `is_multiple_of` is newer than Captain's declared Rust 1.75 MSRV.
#[allow(clippy::manual_is_multiple_of)]
fn duration_label(seconds: u64, french: bool) -> Option<String> {
    if seconds == 0 {
        return None;
    }
    if seconds % 86_400 == 0 {
        let days = seconds / 86_400;
        return Some(if french {
            format!("{days} j")
        } else {
            format!("{days} d")
        });
    }
    if seconds % 3_600 == 0 {
        return Some(format!("{} h", seconds / 3_600));
    }
    Some(format!("{} min", seconds / 60))
}

fn format_percent(value: f64) -> String {
    let value = if value.is_finite() {
        value.clamp(0.0, 999.0)
    } else {
        0.0
    };
    if value.fract().abs() < 0.05 {
        format!("{value:.0} %")
    } else {
        format!("{value:.1} %")
    }
}

fn safe_markdown_text(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for character in value
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
    {
        if matches!(
            character,
            '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '>' | '#' | '|'
        ) {
            output.push('\\');
        }
        output.push(character);
    }
    if output.is_empty() {
        "Provider".to_string()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use captain_channels::types::{ChannelMessage, ChannelType};
    use captain_memory::provider_quota_reset::ProviderQuotaResetWindow;
    use captain_types::quota::ProviderQuotaSource;
    use chrono::Duration;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct RecordingAdapter {
        legacy_sends: AtomicUsize,
        rich_sends: AtomicUsize,
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording-telegram"
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }

        async fn start(
            &self,
        ) -> Result<
            Pin<Box<dyn futures::Stream<Item = ChannelMessage> + Send>>,
            Box<dyn std::error::Error>,
        > {
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn send(
            &self,
            _user: &ChannelUser,
            _content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            self.legacy_sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_rich(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
            _metadata: &HashMap<String, serde_json::Value>,
        ) -> Result<Option<String>, Box<dyn std::error::Error>> {
            let ChannelContent::Text(text) = content else {
                panic!("quota reset notification must be text-backed rich content");
            };
            assert!(text.contains("quota reset"));
            self.rich_sends.fetch_add(1, Ordering::SeqCst);
            Ok(Some("telegram-message-42".to_string()))
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    fn notification() -> ProviderQuotaResetNotification {
        let now = Utc::now();
        ProviderQuotaResetNotification {
            id: "12345678-1234-1234-1234-123456789012".to_string(),
            provider: "codex".to_string(),
            limit_id: "codex".to_string(),
            limit_name: Some("**Codex**\n[unsafe]".to_string()),
            plan_type: Some("plus".to_string()),
            source: ProviderQuotaSource::AccountStatus,
            observed_at: now,
            windows: vec![ProviderQuotaResetWindow {
                kind: ProviderQuotaResetWindowKind::Primary,
                previous_used_percent: 96.0,
                current_used_percent: 2.5,
                previous_resets_at: now - Duration::minutes(1),
                current_resets_at: now + Duration::hours(5),
                window_seconds: Some(18_000),
            }],
        }
    }

    #[test]
    fn rich_card_is_mobile_readable_and_escapes_provider_text() {
        let card = format_provider_quota_reset_card(&notification(), "fr-FR");

        assert!(card.starts_with("## Quota \\*\\*Codex\\*\\*\\[unsafe\\] réinitialisé"));
        assert!(card.contains("### Fenêtre principale · 5 h"));
        assert!(card.contains("`96 % → 2.5 %`"));
        assert!(card.contains("**Disponible** : `97.5 %`"));
        assert!(card.contains("événement `12345678`"));
        assert!(card.contains('\n'));
        assert!(!card.contains('|'));
    }

    #[test]
    fn rich_card_has_an_english_contract() {
        let card = format_provider_quota_reset_card(&notification(), "en-US");

        assert!(card.contains("quota reset"));
        assert!(card.contains("**Available**: `97.5 %`"));
        assert!(card.contains("_Official source: codex"));
    }

    #[tokio::test]
    async fn delivery_uses_the_rich_channel_contract_and_returns_its_receipt() {
        let adapter = Arc::new(RecordingAdapter::default());

        let receipt = send_provider_quota_reset_notification(
            adapter.clone(),
            "123456789",
            "en-US",
            &notification(),
        )
        .await
        .unwrap();

        assert_eq!(receipt.as_deref(), Some("telegram-message-42"));
        assert_eq!(adapter.rich_sends.load(Ordering::SeqCst), 1);
        assert_eq!(adapter.legacy_sends.load(Ordering::SeqCst), 0);
    }
}

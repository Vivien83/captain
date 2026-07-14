use std::collections::HashMap;
use std::sync::Arc;

use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_types::config::{ChannelsConfig, OutputFormat};
use tracing::info;

use super::kernel_delivery_runtime::format_for_telegram;
use super::CaptainKernel;

impl CaptainKernel {
    /// Get the Telegram topic ID for an agent (from persisted store or config).
    pub fn get_telegram_topic(&self, agent_name: &str) -> Option<String> {
        let key = format!("telegram_topic:{}", agent_name);
        if let Ok(Some(serde_json::Value::String(tid))) = self
            .memory
            .structured_get(captain_types::agent::AgentId(uuid::Uuid::nil()), &key)
        {
            if !tid.is_empty() {
                return Some(tid);
            }
        }
        self.config
            .channels
            .telegram
            .as_ref()
            .and_then(|tg| tg.topics.get(agent_name).cloned())
    }

    /// Persist a Telegram topic ID association for an agent/hand.
    pub fn set_telegram_topic(&self, agent_name: &str, topic_id: &str) {
        let key = format!("telegram_topic:{}", agent_name);
        let _ = self.memory.structured_set(
            captain_types::agent::AgentId(uuid::Uuid::nil()),
            &key,
            serde_json::Value::String(topic_id.to_string()),
        );
        info!(agent = %agent_name, topic_id = %topic_id, "Telegram topic persisted");
    }

    pub(super) async fn handle_get_channel_default_recipient(
        &self,
        channel: &str,
    ) -> Option<String> {
        match channel {
            "telegram" => self
                .config
                .channels
                .telegram
                .as_ref()?
                .default_chat_id
                .clone(),
            "discord" => self
                .config
                .channels
                .discord
                .as_ref()?
                .default_channel_id
                .clone(),
            _ => None,
        }
    }

    pub(super) async fn handle_get_channels_context(&self) -> Option<String> {
        build_channels_context(&self.config.channels, |key| {
            self.channel_adapters.contains_key(key)
        })
    }

    pub(super) fn handle_get_telegram_topic(&self, agent_name: &str) -> Option<String> {
        CaptainKernel::get_telegram_topic(self, agent_name)
    }

    pub(super) fn handle_set_telegram_topic(&self, agent_name: &str, topic_id: &str) {
        CaptainKernel::set_telegram_topic(self, agent_name, topic_id);
    }

    pub(super) async fn handle_send_channel_message_from(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        caller_agent_name: Option<&str>,
    ) -> Result<String, String> {
        let adapter = self.require_channel_adapter(channel)?;
        let user = channel_user(recipient);
        let formatted = if channel == "wecom" {
            let output_format = self
                .config
                .channels
                .wecom
                .as_ref()
                .and_then(|c| c.overrides.output_format)
                .unwrap_or(OutputFormat::PlainText);
            captain_channels::formatter::format_for_wecom(message, output_format)
        } else {
            format_channel_text(channel, message)
        };
        let content = ChannelContent::Text(formatted);

        // Resolve thread/topic: explicit > agent-config > none
        let resolved_tid = thread_id.map(String::from).or_else(|| {
            if channel == "telegram" {
                caller_agent_name.and_then(|name| self.get_telegram_topic(name))
            } else {
                None
            }
        });

        send_content_with_retry(
            adapter,
            user,
            content,
            resolved_tid.clone(),
            crate::delivery_reliability::channel_target(channel, recipient),
            "Channel send failed",
        )
        .await?;

        let topic_info = topic_suffix(resolved_tid.as_deref());
        Ok(format!(
            "Message sent to {} via {}{}",
            recipient, channel, topic_info
        ))
    }

    pub(super) async fn handle_send_channel_rich(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        metadata: &HashMap<String, serde_json::Value>,
    ) -> Result<String, String> {
        let adapter = self
            .channel_adapters
            .get(channel)
            .ok_or_else(|| format!("Channel '{}' not found", channel))?
            .value()
            .clone();
        let user = channel_user(recipient);
        let content = ChannelContent::Text(format_channel_text(channel, message));
        let target = crate::delivery_reliability::channel_target(channel, recipient);
        let delivery = crate::channel_delivery_retry::retry_channel_delivery(&target, || {
            let adapter = adapter.clone();
            let user = user.clone();
            let content = content.clone();
            let metadata = metadata.clone();
            async move {
                adapter
                    .send_rich(&user, content, &metadata)
                    .await
                    .map_err(|e| format!("Channel rich send failed: {e}"))
            }
        })
        .await?;

        Ok(match delivery.value {
            Some(id) => format!(
                "Message sent to {} via {} (msg_id: {})",
                recipient, channel, id
            ),
            None => format!("Message sent to {} via {}", recipient, channel),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_send_channel_media(
        &self,
        channel: &str,
        recipient: &str,
        media_type: &str,
        media_url: &str,
        caption: Option<&str>,
        filename: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let adapter = self.require_channel_adapter(channel)?;
        let user = channel_user(recipient);
        let content = match media_type {
            "image" => ChannelContent::Image {
                url: media_url.to_string(),
                caption: caption.map(|s| s.to_string()),
            },
            "file" => ChannelContent::File {
                url: media_url.to_string(),
                filename: filename.unwrap_or("file").to_string(),
            },
            _ => {
                return Err(format!(
                    "Unsupported media type: '{media_type}'. Use 'image' or 'file'."
                ));
            }
        };

        send_content_with_retry(
            adapter,
            user,
            content,
            thread_id.map(str::to_string),
            crate::delivery_reliability::channel_target(channel, recipient),
            "Channel media send failed",
        )
        .await?;

        Ok(format!(
            "{} sent to {} via {}",
            media_type, recipient, channel
        ))
    }

    pub(super) async fn handle_send_channel_file_data(
        &self,
        channel: &str,
        recipient: &str,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let adapter = self.require_channel_adapter(channel)?;
        let content = ChannelContent::FileData {
            data,
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
        };

        send_content_with_retry(
            adapter,
            channel_user(recipient),
            content,
            thread_id.map(str::to_string),
            crate::delivery_reliability::channel_target(channel, recipient),
            "Channel file send failed",
        )
        .await?;

        Ok(format!(
            "File '{}' sent to {} via {}",
            filename, recipient, channel
        ))
    }

    pub(super) async fn handle_send_channel_image_data(
        &self,
        channel: &str,
        recipient: &str,
        data: Vec<u8>,
        mime_type: &str,
        caption: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let adapter = self.require_channel_adapter(channel)?;
        let content = ChannelContent::ImageData {
            data,
            mime_type: mime_type.to_string(),
            caption: caption.map(|s| s.to_string()),
        };

        send_content_with_retry(
            adapter,
            channel_user(recipient),
            content,
            thread_id.map(str::to_string),
            crate::delivery_reliability::channel_target(channel, recipient),
            "Channel image send failed",
        )
        .await?;

        Ok(format!("Image sent to {recipient} via {channel}"))
    }

    fn require_channel_adapter(&self, channel: &str) -> Result<Arc<dyn ChannelAdapter>, String> {
        self.channel_adapters
            .get(channel)
            .map(|adapter| adapter.value().clone())
            .ok_or_else(|| missing_channel_error(channel, self.available_channels()))
    }

    fn available_channels(&self) -> Vec<String> {
        self.channel_adapters
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
}

async fn send_content_with_retry(
    adapter: Arc<dyn ChannelAdapter>,
    user: ChannelUser,
    content: ChannelContent,
    thread_id: Option<String>,
    target: String,
    error_prefix: &'static str,
) -> Result<(), String> {
    let _delivery = crate::channel_delivery_retry::retry_channel_delivery(&target, || {
        let adapter = adapter.clone();
        let user = user.clone();
        let content = content.clone();
        let thread_id = thread_id.clone();
        async move {
            if let Some(tid) = thread_id.as_deref() {
                adapter
                    .send_in_thread(&user, content, tid)
                    .await
                    .map_err(|e| format!("{error_prefix}: {e}"))
            } else {
                adapter
                    .send(&user, content)
                    .await
                    .map_err(|e| format!("{error_prefix}: {e}"))
            }
        }
    })
    .await?;
    Ok(())
}

fn channel_user(recipient: &str) -> ChannelUser {
    ChannelUser {
        platform_id: recipient.to_string(),
        display_name: recipient.to_string(),
        captain_user: None,
    }
}

fn format_channel_text(channel: &str, message: &str) -> String {
    if channel == "telegram" {
        format_for_telegram(message)
    } else {
        message.to_string()
    }
}

fn topic_suffix(thread_id: Option<&str>) -> String {
    thread_id
        .map(|thread| format!(" (topic: {thread})"))
        .unwrap_or_default()
}

fn missing_channel_error(channel: &str, available: Vec<String>) -> String {
    format!(
        "Channel '{}' not found. Available channels: {:?}",
        channel, available
    )
}

fn build_channels_context<F>(channels: &ChannelsConfig, is_active: F) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    let mut lines = Vec::new();
    lines.push("## Channels".to_string());
    lines.push("Use `channel_send` to send messages. The `recipient` field is optional when a default is configured. ACTIVE = adapter live in the bridge; CONFIGURED = section present in config.toml but no live adapter (token missing or boot skipped) — call `secret_write` for the token then `channel_reconfigure({channel})` to bring it up.".to_string());

    if let Some(tg) = &channels.telegram {
        let mut info = format!("- **telegram**: {}", channel_status("telegram", &is_active));
        if let Some(ref cid) = tg.default_chat_id {
            info.push_str(&format!(
                " (default_chat_id: {cid} — recipient is optional)"
            ));
        }
        lines.push(info);
    }
    if let Some(dc) = &channels.discord {
        let mut info = format!("- **discord**: {}", channel_status("discord", &is_active));
        if let Some(ref cid) = dc.default_channel_id {
            info.push_str(&format!(" (default_channel_id: {cid})"));
        }
        lines.push(info);
    }

    for (key, configured) in plain_channel_configured(channels) {
        if configured {
            lines.push(format!("- **{key}**: {}", channel_status(key, &is_active)));
        }
    }

    if lines.len() <= 2 {
        return None;
    }
    Some(lines.join("\n"))
}

fn channel_status<F>(key: &str, is_active: &F) -> &'static str
where
    F: Fn(&str) -> bool,
{
    if is_active(key) {
        "ACTIVE"
    } else {
        "CONFIGURED"
    }
}

fn plain_channel_configured(channels: &ChannelsConfig) -> [(&'static str, bool); 40] {
    [
        ("slack", channels.slack.is_some()),
        ("whatsapp", channels.whatsapp.is_some()),
        ("signal", channels.signal.is_some()),
        ("matrix", channels.matrix.is_some()),
        ("email", channels.email.is_some()),
        ("teams", channels.teams.is_some()),
        ("mattermost", channels.mattermost.is_some()),
        ("irc", channels.irc.is_some()),
        ("google_chat", channels.google_chat.is_some()),
        ("twitch", channels.twitch.is_some()),
        ("rocketchat", channels.rocketchat.is_some()),
        ("zulip", channels.zulip.is_some()),
        ("xmpp", channels.xmpp.is_some()),
        ("line", channels.line.is_some()),
        ("viber", channels.viber.is_some()),
        ("messenger", channels.messenger.is_some()),
        ("reddit", channels.reddit.is_some()),
        ("mastodon", channels.mastodon.is_some()),
        ("bluesky", channels.bluesky.is_some()),
        ("feishu", channels.feishu.is_some()),
        ("revolt", channels.revolt.is_some()),
        ("nextcloud", channels.nextcloud.is_some()),
        ("guilded", channels.guilded.is_some()),
        ("keybase", channels.keybase.is_some()),
        ("threema", channels.threema.is_some()),
        ("nostr", channels.nostr.is_some()),
        ("webex", channels.webex.is_some()),
        ("pumble", channels.pumble.is_some()),
        ("flock", channels.flock.is_some()),
        ("twist", channels.twist.is_some()),
        ("mumble", channels.mumble.is_some()),
        ("dingtalk", channels.dingtalk.is_some()),
        ("dingtalk_stream", channels.dingtalk_stream.is_some()),
        ("discourse", channels.discourse.is_some()),
        ("gitter", channels.gitter.is_some()),
        ("ntfy", channels.ntfy.is_some()),
        ("gotify", channels.gotify.is_some()),
        ("webhook", channels.webhook.is_some()),
        ("linkedin", channels.linkedin.is_some()),
        ("wecom", channels.wecom.is_some()),
    ]
}

#[cfg(test)]
mod tests {
    use captain_types::config::{ChannelsConfig, DiscordConfig, TelegramConfig};

    use super::{build_channels_context, format_channel_text, missing_channel_error, topic_suffix};

    #[test]
    fn channel_context_includes_defaults_and_active_state() {
        let channels = ChannelsConfig {
            telegram: Some(TelegramConfig {
                default_chat_id: Some("42".to_string()),
                ..TelegramConfig::default()
            }),
            discord: Some(DiscordConfig {
                default_channel_id: Some("99".to_string()),
                ..DiscordConfig::default()
            }),
            ..ChannelsConfig::default()
        };

        let context = build_channels_context(&channels, |key| key == "telegram").expect("context");

        assert!(context.contains("- **telegram**: ACTIVE"));
        assert!(context.contains("default_chat_id: 42"));
        assert!(context.contains("- **discord**: CONFIGURED"));
        assert!(context.contains("default_channel_id: 99"));
    }

    #[test]
    fn channel_context_is_absent_without_configured_channels() {
        assert!(build_channels_context(&ChannelsConfig::default(), |_| false).is_none());
    }

    #[test]
    fn channel_text_formats_telegram_only() {
        assert_eq!(format_channel_text("discord", "**hi**"), "**hi**");
        assert_ne!(format_channel_text("telegram", "**hi**"), "**hi**");
    }

    #[test]
    fn channel_error_and_topic_suffix_match_public_contract() {
        assert_eq!(topic_suffix(Some("123")), " (topic: 123)");
        assert_eq!(topic_suffix(None), "");
        assert_eq!(
            missing_channel_error("signal", vec!["telegram".to_string()]),
            "Channel 'signal' not found. Available channels: [\"telegram\"]"
        );
    }
}

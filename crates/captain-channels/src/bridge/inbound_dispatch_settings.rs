//! Per-message dispatch settings derived from channel overrides.

use super::channel_mapping::default_output_format_for_channel;
use captain_types::config::{ChannelOverrides, OutputFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InboundDispatchSettings<'a> {
    pub(super) output_format: OutputFormat,
    pub(super) thread_id: Option<&'a str>,
    pub(super) lifecycle_reactions: bool,
}

pub(super) fn resolve_inbound_dispatch_settings<'a>(
    channel_type: &str,
    message_thread_id: Option<&'a str>,
    overrides: Option<&ChannelOverrides>,
) -> InboundDispatchSettings<'a> {
    let output_format = overrides
        .and_then(|o| o.output_format)
        .unwrap_or_else(|| default_output_format_for_channel(channel_type));
    let thread_id = match overrides {
        Some(overrides) if overrides.threading => message_thread_id,
        _ => None,
    };
    // HS.8: lifecycle reactions only opt in from explicit channel overrides.
    let lifecycle_reactions = overrides.map(|o| o.lifecycle_reactions).unwrap_or(false);

    InboundDispatchSettings {
        output_format,
        thread_id,
        lifecycle_reactions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_settings_use_channel_defaults_without_overrides() {
        assert_eq!(
            resolve_inbound_dispatch_settings("telegram", Some("topic-1"), None),
            InboundDispatchSettings {
                output_format: OutputFormat::TelegramHtml,
                thread_id: None,
                lifecycle_reactions: false,
            }
        );
    }

    #[test]
    fn dispatch_settings_apply_output_format_override() {
        let overrides = ChannelOverrides {
            output_format: Some(OutputFormat::PlainText),
            ..Default::default()
        };

        assert_eq!(
            resolve_inbound_dispatch_settings("telegram", None, Some(&overrides)).output_format,
            OutputFormat::PlainText
        );
    }

    #[test]
    fn dispatch_settings_enable_threading_and_reactions_only_when_configured() {
        let overrides = ChannelOverrides {
            threading: true,
            lifecycle_reactions: true,
            ..Default::default()
        };

        assert_eq!(
            resolve_inbound_dispatch_settings("discord", Some("thread-7"), Some(&overrides)),
            InboundDispatchSettings {
                output_format: OutputFormat::Markdown,
                thread_id: Some("thread-7"),
                lifecycle_reactions: true,
            }
        );
    }
}

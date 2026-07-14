//! Operator acknowledgements for queued and interjected inbound messages.

use super::channel_mapping::{channel_type_str, default_output_format_for_channel};
use super::command_response::send_response;
use super::ChannelBridgeHandle;
use crate::types::{ChannelAdapter, ChannelMessage};
use std::sync::Arc;

const QUEUED_ACK_TEXT: &str =
    "Message reçu. Je termine le tour en cours, puis je le traite juste après.";
const INTERJECTION_ACK_TEXT: &str = "Complément reçu. Je l'intègre au contexte du tour en cours.";

pub(super) async fn send_inbound_queued_ack(
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter: &dyn ChannelAdapter,
) {
    send_inbound_status_ack(message, handle, adapter, QUEUED_ACK_TEXT).await;
}

pub(super) async fn send_inbound_interjection_ack(
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter: &dyn ChannelAdapter,
) {
    send_inbound_status_ack(message, handle, adapter, INTERJECTION_ACK_TEXT).await;
}

async fn send_inbound_status_ack(
    message: &ChannelMessage,
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter: &dyn ChannelAdapter,
    text: &str,
) {
    let channel = channel_type_str(&message.channel);
    let overrides = handle.channel_overrides(channel).await;
    let output_format = overrides
        .as_ref()
        .and_then(|overrides| overrides.output_format)
        .unwrap_or_else(|| default_output_format_for_channel(channel));
    let threading_enabled = overrides
        .as_ref()
        .map(|overrides| overrides.threading)
        .unwrap_or(false);
    let thread_id = if threading_enabled {
        message.thread_id.as_deref()
    } else {
        None
    };

    send_response(
        adapter,
        &message.sender,
        text.to_string(),
        thread_id,
        output_format,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_ack_text_stays_short_and_actionable() {
        assert_eq!(
            QUEUED_ACK_TEXT,
            "Message reçu. Je termine le tour en cours, puis je le traite juste après."
        );
    }

    #[test]
    fn interjection_ack_text_mentions_active_context() {
        assert_eq!(
            INTERJECTION_ACK_TEXT,
            "Complément reçu. Je l'intègre au contexte du tour en cours."
        );
    }
}

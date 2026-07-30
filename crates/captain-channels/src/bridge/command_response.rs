//! Response formatting and delivery for channel commands.

use super::ChannelBridgeHandle;
use crate::formatter;
use crate::outbound_delivery::{
    OutboundDeliveryClaim, OutboundDeliveryIntent, OutboundDeliveryPreparation,
    OutboundDeliveryTransport,
};
use crate::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_types::agent::AgentId;
use captain_types::config::OutputFormat;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tracing::{error, warn};

const RECOVERED_DELIVERY_NOTICE: &str =
    "⚠️ Recovered reply: the previous delivery outcome was uncertain; this message may be a duplicate.";

pub(super) struct DurableResponseContext<'a> {
    pub(super) handle: &'a Arc<dyn ChannelBridgeHandle>,
    pub(super) agent_id: Option<AgentId>,
    pub(super) channel: &'a str,
    pub(super) source_message_id: &'a str,
    pub(super) purpose: &'a str,
}

/// Send a response, applying output formatting and optional threading.
pub(super) async fn send_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    text: String,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) -> Result<(), String> {
    let (content, transport) = prepare_response(adapter, text, thread_id, output_format);
    execute_transport(adapter, user, content, &transport)
        .await
        .map(|_| ())
}

/// Persist a response before transport and commit only the exact send result.
pub(super) async fn send_durable_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    text: String,
    thread_id: Option<&str>,
    output_format: OutputFormat,
    context: DurableResponseContext<'_>,
) -> Result<(), String> {
    let (content, transport) = prepare_response(adapter, text, thread_id, output_format);
    send_durable_content(adapter, user, content, transport, context).await
}

pub(super) async fn send_durable_rich_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    text: String,
    metadata: HashMap<String, serde_json::Value>,
    context: DurableResponseContext<'_>,
) -> Result<(), String> {
    send_durable_content(
        adapter,
        user,
        ChannelContent::Text(text),
        OutboundDeliveryTransport::Rich { metadata },
        context,
    )
    .await
}

async fn send_durable_content(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    content: ChannelContent,
    transport: OutboundDeliveryTransport,
    context: DurableResponseContext<'_>,
) -> Result<(), String> {
    let intent = OutboundDeliveryIntent {
        idempotency_key: delivery_idempotency_key(
            context.channel,
            &user.platform_id,
            context.source_message_id,
            context.purpose,
            &content,
            &transport,
        ),
        agent_id: context.agent_id,
        channel: context.channel.to_string(),
        recipient: user.clone(),
        content,
        transport,
        source_message_id: context.source_message_id.to_string(),
        purpose: context.purpose.to_string(),
    };
    match context
        .handle
        .prepare_outbound_delivery(intent.clone(), outbound_delivery_owner())
        .await?
    {
        OutboundDeliveryPreparation::AlreadyHandled => Ok(()),
        OutboundDeliveryPreparation::Bypass => {
            let result = execute_transport(
                adapter,
                &intent.recipient,
                intent.content.clone(),
                &intent.transport,
            )
            .await;
            record_transport_result(context.handle, &intent, &result).await;
            result.map(|_| ())
        }
        OutboundDeliveryPreparation::Claimed(claim) => {
            execute_outbound_claim(context.handle, adapter, claim).await
        }
    }
}

pub(super) async fn execute_outbound_claim(
    handle: &Arc<dyn ChannelBridgeHandle>,
    adapter: &dyn ChannelAdapter,
    claim: OutboundDeliveryClaim,
) -> Result<(), String> {
    let content = if claim.possible_duplicate {
        recovered_content(claim.intent.content.clone())
    } else {
        claim.intent.content.clone()
    };
    let result = execute_transport(
        adapter,
        &claim.intent.recipient,
        content,
        &claim.intent.transport,
    )
    .await;
    match &result {
        Ok(external_message_id) => {
            if let Err(error) = handle
                .complete_outbound_delivery(
                    &claim.delivery_id,
                    &claim.lease_token,
                    external_message_id.as_deref(),
                )
                .await
            {
                record_transport_failure(handle, &claim.intent, &error).await;
                return Err(format!(
                    "channel accepted the response but its durable receipt could not be committed: {error}"
                ));
            }
        }
        Err(error) => {
            if let Err(state_error) = handle
                .retry_outbound_delivery(&claim.delivery_id, &claim.lease_token, error)
                .await
            {
                warn!(%state_error, delivery_id = %claim.delivery_id, "failed to persist outbound delivery retry state");
            }
        }
    }
    record_transport_result(handle, &claim.intent, &result).await;
    result.clone().map(|_| ())
}

fn prepare_response(
    adapter: &dyn ChannelAdapter,
    text: String,
    thread_id: Option<&str>,
    output_format: OutputFormat,
) -> (ChannelContent, OutboundDeliveryTransport) {
    let formatted = format_channel_text(adapter, &text, output_format);
    let content = ChannelContent::Text(formatted);

    let transport = if adapter.name() == "telegram" && output_format == OutputFormat::PlainText {
        let mut metadata = HashMap::new();
        metadata.insert("telegram_plain_text".to_string(), serde_json::json!(true));
        if let Some(tid) = thread_id {
            metadata.insert("thread_id".to_string(), serde_json::json!(tid));
        }
        OutboundDeliveryTransport::Rich { metadata }
    } else if let Some(tid) = thread_id {
        OutboundDeliveryTransport::Thread {
            thread_id: tid.to_string(),
        }
    } else {
        OutboundDeliveryTransport::Standard
    };
    (content, transport)
}

async fn execute_transport(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    content: ChannelContent,
    transport: &OutboundDeliveryTransport,
) -> Result<Option<String>, String> {
    let result = match transport {
        OutboundDeliveryTransport::Standard => adapter.send(user, content).await.map(|_| None),
        OutboundDeliveryTransport::Thread { thread_id } => adapter
            .send_in_thread(user, content, thread_id)
            .await
            .map(|_| None),
        OutboundDeliveryTransport::Rich { metadata } => {
            adapter.send_rich(user, content, metadata).await
        }
    };
    result.map_err(|error| {
        let message = error.to_string();
        error!(%message, channel = %adapter.name(), "failed to send channel response");
        message
    })
}

async fn record_transport_result(
    handle: &Arc<dyn ChannelBridgeHandle>,
    intent: &OutboundDeliveryIntent,
    result: &Result<Option<String>, String>,
) {
    let Some(agent_id) = intent.agent_id else {
        return;
    };
    handle
        .record_delivery(
            agent_id,
            &intent.channel,
            &intent.recipient.platform_id,
            result.is_ok(),
            result.as_ref().err().map(String::as_str),
            intent.transport.thread_id(),
        )
        .await;
}

async fn record_transport_failure(
    handle: &Arc<dyn ChannelBridgeHandle>,
    intent: &OutboundDeliveryIntent,
    error: &str,
) {
    let result = Err::<Option<String>, String>(error.to_string());
    record_transport_result(handle, intent, &result).await;
}

fn recovered_content(content: ChannelContent) -> ChannelContent {
    match content {
        ChannelContent::Text(text) => {
            ChannelContent::Text(format!("{RECOVERED_DELIVERY_NOTICE}\n\n{text}"))
        }
        other => other,
    }
}

pub(super) fn outbound_delivery_owner() -> &'static str {
    static OWNER: OnceLock<String> = OnceLock::new();
    OWNER
        .get_or_init(|| format!("{}:{}", std::process::id(), uuid::Uuid::new_v4()))
        .as_str()
}

fn delivery_idempotency_key(
    channel: &str,
    recipient: &str,
    source_message_id: &str,
    purpose: &str,
    content: &ChannelContent,
    transport: &OutboundDeliveryTransport,
) -> String {
    let payload = serde_json::to_vec(&serde_json::json!({
        "channel": channel,
        "recipient": recipient,
        "source_message_id": source_message_id,
        "purpose": purpose,
        "content": content,
        "transport": transport,
    }))
    .unwrap_or_default();
    format!("outbound-v1:{:x}", Sha256::digest(payload))
}

/// Send a response without markdown/channel formatting.
///
/// Used for exact file/content dumps such as `/config`; Telegram still applies
/// its own HTML sanitization and chunking at the adapter boundary.
async fn send_raw_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    text: String,
    thread_id: Option<&str>,
) -> Result<(), String> {
    let content = ChannelContent::Text(text);
    let result = if adapter.name() == "telegram" {
        let mut metadata = HashMap::new();
        metadata.insert("telegram_plain_text".to_string(), serde_json::json!(true));
        if let Some(tid) = thread_id {
            metadata.insert("thread_id".to_string(), serde_json::json!(tid));
        }
        adapter
            .send_rich(user, content, &metadata)
            .await
            .map(|_| ())
    } else if let Some(tid) = thread_id {
        adapter.send_in_thread(user, content, tid).await
    } else {
        adapter.send(user, content).await
    };

    result.map_err(|error| error.to_string())
}

#[derive(Debug, Clone)]
pub(super) struct CommandResponse {
    text: String,
    reply_markup: Option<serde_json::Value>,
    raw: bool,
}

impl CommandResponse {
    pub(super) fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reply_markup: None,
            raw: false,
        }
    }

    pub(super) fn raw(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            reply_markup: None,
            raw: true,
        }
    }

    pub(super) fn with_reply_markup(
        text: impl Into<String>,
        reply_markup: serde_json::Value,
    ) -> Self {
        Self {
            text: text.into(),
            reply_markup: Some(reply_markup),
            raw: false,
        }
    }
}

impl std::ops::Deref for CommandResponse {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.text
    }
}

pub(super) async fn send_command_response(
    adapter: &dyn ChannelAdapter,
    user: &ChannelUser,
    response: CommandResponse,
    thread_id: Option<&str>,
    output_format: OutputFormat,
    durable: Option<DurableResponseContext<'_>>,
) -> Result<(), String> {
    let CommandResponse {
        text,
        reply_markup,
        raw,
    } = response;
    if raw && reply_markup.is_none() {
        if let Some(context) = durable {
            let transport = if adapter.name() == "telegram" {
                let mut metadata = HashMap::new();
                metadata.insert("telegram_plain_text".to_string(), serde_json::json!(true));
                if let Some(tid) = thread_id {
                    metadata.insert("thread_id".to_string(), serde_json::json!(tid));
                }
                OutboundDeliveryTransport::Rich { metadata }
            } else if let Some(tid) = thread_id {
                OutboundDeliveryTransport::Thread {
                    thread_id: tid.to_string(),
                }
            } else {
                OutboundDeliveryTransport::Standard
            };
            return send_durable_content(
                adapter,
                user,
                ChannelContent::Text(text),
                transport,
                context,
            )
            .await;
        }
        return send_raw_response(adapter, user, text, thread_id).await;
    }
    let Some(reply_markup) = reply_markup else {
        return if let Some(context) = durable {
            send_durable_response(adapter, user, text, thread_id, output_format, context).await
        } else {
            send_response(adapter, user, text, thread_id, output_format).await
        };
    };

    let formatted = format_channel_text(adapter, &text, output_format);
    let mut metadata = HashMap::new();
    metadata.insert("reply_markup".to_string(), reply_markup);
    if adapter.name() == "telegram" && output_format == OutputFormat::PlainText {
        metadata.insert("telegram_plain_text".to_string(), serde_json::json!(true));
    }
    if let Some(tid) = thread_id {
        metadata.insert("thread_id".to_string(), serde_json::json!(tid));
    }

    if let Some(context) = durable {
        return send_durable_content(
            adapter,
            user,
            ChannelContent::Text(formatted),
            OutboundDeliveryTransport::Rich { metadata },
            context,
        )
        .await;
    }
    adapter
        .send_rich(user, ChannelContent::Text(formatted), &metadata)
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn format_channel_text(
    adapter: &dyn ChannelAdapter,
    text: &str,
    output_format: OutputFormat,
) -> String {
    if adapter.name() == "wecom" {
        formatter::format_for_wecom(text, output_format)
    } else if adapter.name() == "telegram" && output_format == OutputFormat::TelegramHtml {
        // `TelegramHtml` remains the compatible config name. Bot API 10.2
        // now receives the original Markdown and renders it natively; the
        // adapter converts to legacy HTML only when the endpoint rejects it.
        text.to_string()
    } else {
        formatter::format_for_channel(text, output_format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::TelegramAdapter;
    use crate::types::{ChannelMessage, ChannelType};
    use async_trait::async_trait;
    use futures::{stream, Stream};
    use serde_json::json;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::time::Duration;

    fn telegram_adapter(api_url: String) -> TelegramAdapter {
        TelegramAdapter::new(
            "123:ABC".to_string(),
            vec!["*".to_string()],
            Duration::from_secs(1),
            Some(api_url),
        )
    }

    struct DurableTestHandle {
        events: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for DurableTestHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            _message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(String::new())
        }

        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(None)
        }

        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(Vec::new())
        }

        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("not available".to_string())
        }

        async fn prepare_outbound_delivery(
            &self,
            intent: OutboundDeliveryIntent,
            _lease_owner: &str,
        ) -> Result<OutboundDeliveryPreparation, String> {
            self.events.lock().unwrap().push("prepare".to_string());
            Ok(OutboundDeliveryPreparation::Claimed(
                OutboundDeliveryClaim {
                    delivery_id: "delivery-1".to_string(),
                    lease_token: "lease-1".to_string(),
                    intent,
                    attempt_count: 1,
                    possible_duplicate: false,
                },
            ))
        }

        async fn complete_outbound_delivery(
            &self,
            _delivery_id: &str,
            _lease_token: &str,
            _external_message_id: Option<&str>,
        ) -> Result<(), String> {
            self.events.lock().unwrap().push("complete".to_string());
            Ok(())
        }

        async fn retry_outbound_delivery(
            &self,
            _delivery_id: &str,
            _lease_token: &str,
            _error: &str,
        ) -> Result<(), String> {
            self.events.lock().unwrap().push("retry".to_string());
            Ok(())
        }

        async fn record_delivery(
            &self,
            _agent_id: AgentId,
            _channel: &str,
            _recipient: &str,
            success: bool,
            _error: Option<&str>,
            _thread_id: Option<&str>,
        ) {
            self.events
                .lock()
                .unwrap()
                .push(format!("receipt:{success}"));
        }
    }

    struct DurableTestAdapter {
        events: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    #[async_trait]
    impl ChannelAdapter for DurableTestAdapter {
        fn name(&self) -> &str {
            "recording"
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Custom("recording".to_string())
        }

        async fn start(
            &self,
        ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
        {
            Ok(Box::pin(stream::empty()))
        }

        async fn send(
            &self,
            _user: &ChannelUser,
            content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            let text = match content {
                ChannelContent::Text(text) => text,
                _ => "non-text".to_string(),
            };
            self.events.lock().unwrap().push(format!("send:{text}"));
            if self.fail {
                Err(std::io::Error::other("transport unavailable").into())
            } else {
                Ok(())
            }
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }
    }

    fn durable_user() -> ChannelUser {
        ChannelUser {
            platform_id: "recipient-1".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        }
    }

    #[test]
    fn command_response_text_is_formatted_response() {
        let response = CommandResponse::text("hello");

        assert_eq!(&*response, "hello");
        assert!(!response.raw);
        assert!(response.reply_markup.is_none());
    }

    #[test]
    fn command_response_raw_preserves_exact_text() {
        let response = CommandResponse::raw("config = true");

        assert_eq!(&*response, "config = true");
        assert!(response.raw);
        assert!(response.reply_markup.is_none());
    }

    #[test]
    fn command_response_with_markup_is_not_raw() {
        let markup = json!({"inline_keyboard": []});
        let response = CommandResponse::with_reply_markup("choose", markup.clone());

        assert_eq!(&*response, "choose");
        assert!(!response.raw);
        assert_eq!(response.reply_markup, Some(markup));
    }

    #[test]
    fn telegram_default_format_preserves_markdown_for_native_rich_transport() {
        let adapter = telegram_adapter("http://127.0.0.1:1".to_string());
        let input = "## Report\n\n| Metric | Value |\n|---|---:|\n| OK | **1** |";
        assert_eq!(
            format_channel_text(&adapter, input, OutputFormat::TelegramHtml),
            input
        );
    }

    #[tokio::test]
    async fn telegram_raw_and_plain_responses_use_unparsed_send_message() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendMessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "ok": true,
                "result": {"message_id": 42}
            })))
            .expect(2)
            .mount(&server)
            .await;
        let adapter = telegram_adapter(server.uri());
        let user = ChannelUser {
            platform_id: "42".to_string(),
            display_name: "Test".to_string(),
            captain_user: None,
        };

        send_command_response(
            &adapter,
            &user,
            CommandResponse::raw("**literal raw**"),
            Some("7"),
            OutputFormat::TelegramHtml,
            None,
        )
        .await
        .unwrap();
        send_response(
            &adapter,
            &user,
            "**plain request**".to_string(),
            None,
            OutputFormat::PlainText,
        )
        .await
        .unwrap();

        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 2);
        for request in requests {
            let body: serde_json::Value =
                serde_json::from_slice(&request.body).expect("plain JSON body");
            assert!(body.get("parse_mode").is_none());
            assert!(body.get("rich_message").is_none());
        }
        let raw_body: serde_json::Value =
            serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(raw_body["text"], "**literal raw**");
        assert_eq!(raw_body["message_thread_id"], 7);
    }

    #[tokio::test]
    async fn durable_response_is_persisted_before_send_and_committed_afterward() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let concrete_handle = Arc::new(DurableTestHandle {
            events: Arc::clone(&events),
        });
        let handle: Arc<dyn ChannelBridgeHandle> = concrete_handle;
        let adapter = DurableTestAdapter {
            events: Arc::clone(&events),
            fail: false,
        };
        let agent_id = AgentId::new();

        send_durable_response(
            &adapter,
            &durable_user(),
            "done".to_string(),
            None,
            OutputFormat::PlainText,
            DurableResponseContext {
                handle: &handle,
                agent_id: Some(agent_id),
                channel: "recording",
                source_message_id: "source-1",
                purpose: "agent_final",
            },
        )
        .await
        .unwrap();

        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["prepare", "send:done", "complete", "receipt:true"]
        );
    }

    #[tokio::test]
    async fn failed_transport_is_requeued_and_never_recorded_as_success() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let concrete_handle = Arc::new(DurableTestHandle {
            events: Arc::clone(&events),
        });
        let handle: Arc<dyn ChannelBridgeHandle> = concrete_handle;
        let adapter = DurableTestAdapter {
            events: Arc::clone(&events),
            fail: true,
        };

        let error = send_durable_response(
            &adapter,
            &durable_user(),
            "done".to_string(),
            None,
            OutputFormat::PlainText,
            DurableResponseContext {
                handle: &handle,
                agent_id: Some(AgentId::new()),
                channel: "recording",
                source_message_id: "source-2",
                purpose: "agent_final",
            },
        )
        .await
        .unwrap_err();

        assert!(error.contains("transport unavailable"));
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["prepare", "send:done", "retry", "receipt:false"]
        );
    }

    #[tokio::test]
    async fn recovered_ambiguous_delivery_marks_the_possible_duplicate() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let concrete_handle = Arc::new(DurableTestHandle {
            events: Arc::clone(&events),
        });
        let handle: Arc<dyn ChannelBridgeHandle> = concrete_handle;
        let adapter = DurableTestAdapter {
            events: Arc::clone(&events),
            fail: false,
        };
        let claim = OutboundDeliveryClaim {
            delivery_id: "delivery-recovered".to_string(),
            lease_token: "lease-recovered".to_string(),
            intent: OutboundDeliveryIntent {
                idempotency_key: "key-recovered".to_string(),
                agent_id: None,
                channel: "recording".to_string(),
                recipient: durable_user(),
                content: ChannelContent::Text("original".to_string()),
                transport: OutboundDeliveryTransport::Standard,
                source_message_id: "source-3".to_string(),
                purpose: "agent_final".to_string(),
            },
            attempt_count: 2,
            possible_duplicate: true,
        };

        execute_outbound_claim(&handle, &adapter, claim)
            .await
            .unwrap();

        let events = events.lock().unwrap();
        assert!(events[0].starts_with("send:⚠️ Recovered reply:"));
        assert_eq!(events[1], "complete");
    }
}

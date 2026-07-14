//! Anthropic Claude API driver.
//!
//! Full implementation of the Anthropic Messages API with tool use support,
//! system prompt extraction, and retry on 429/529 errors.

use crate::llm_driver::{
    CacheHints, CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent,
};
use async_trait::async_trait;
use captain_types::message::{ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage};
use captain_types::tool::ToolCall;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::Sender;
use tracing::{debug, warn};
use zeroize::Zeroizing;

const MAX_ANTHROPIC_RETRIES: usize = 3;

/// Anthropic Claude API driver.
pub struct AnthropicDriver {
    api_key: Zeroizing<String>,
    base_url: String,
    client: reqwest::Client,
}

impl AnthropicDriver {
    /// Create a new Anthropic driver.
    pub fn new(api_key: String, base_url: String) -> Self {
        Self {
            api_key: Zeroizing::new(api_key),
            base_url,
            client: reqwest::Client::builder()
                .user_agent(crate::USER_AGENT)
                .build()
                .unwrap_or_default(),
        }
    }

    async fn send_messages_request(
        &self,
        api_request: &ApiRequest,
        attempt: usize,
        streaming: bool,
    ) -> Result<reqwest::Response, LlmError> {
        let url = format!("{}/v1/messages", self.base_url);
        if streaming {
            debug!(url = %url, attempt, "Sending Anthropic streaming request");
        } else {
            debug!(url = %url, attempt, "Sending Anthropic API request");
        }

        self.client
            .post(&url)
            .header("x-api-key", self.api_key.as_str())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(api_request)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))
    }
}

/// Anthropic Messages API request body.
#[derive(Debug, Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<ApiSystem>,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ApiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

/// `system` field accepts either a flat string (legacy / no-cache) or an
/// array of typed text blocks (each can carry its own `cache_control`).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ApiSystem {
    Plain(String),
    Blocks(Vec<ApiSystemBlock>),
}

#[derive(Debug, Serialize)]
struct ApiSystemBlock {
    #[serde(rename = "type")]
    block_type: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<ApiCacheControl>,
}

/// Anthropic prompt-caching marker. `ephemeral` = 5 min idle TTL.
#[derive(Debug, Serialize, Clone)]
struct ApiCacheControl {
    #[serde(rename = "type")]
    cache_type: &'static str,
}

impl ApiCacheControl {
    fn ephemeral() -> Self {
        Self {
            cache_type: "ephemeral",
        }
    }
}

/// Translate the system prompt + cache hints into the Anthropic payload.
/// `cache_system: true` wraps the cacheable prefix in a text block with
/// `cache_control: ephemeral`. If the prompt builder supplied a stable prefix
/// boundary, the dynamic suffix stays in a second uncached system block.
/// Otherwise we fall back to the flat string shape so the wire format stays
/// identical to the pre-cache build.
fn build_system_field(system: Option<String>, hints: &CacheHints) -> Option<ApiSystem> {
    let s = system?;
    if !hints.cache_system {
        return Some(ApiSystem::Plain(s));
    }
    Some(ApiSystem::Blocks(build_system_cache_blocks(
        s,
        hints.cacheable_system_prefix_bytes,
    )))
}

fn build_system_cache_blocks(
    system: String,
    cacheable_prefix_bytes: Option<usize>,
) -> Vec<ApiSystemBlock> {
    match cacheable_prefix_bytes {
        Some(split) if split > 0 && split < system.len() && system.is_char_boundary(split) => {
            let (cacheable, dynamic) = system.split_at(split);
            let mut blocks = vec![ApiSystemBlock {
                block_type: "text",
                text: cacheable.to_string(),
                cache_control: Some(ApiCacheControl::ephemeral()),
            }];
            if !dynamic.is_empty() {
                blocks.push(ApiSystemBlock {
                    block_type: "text",
                    text: dynamic.to_string(),
                    cache_control: None,
                });
            }
            blocks
        }
        _ => vec![ApiSystemBlock {
            block_type: "text",
            text: system,
            cache_control: Some(ApiCacheControl::ephemeral()),
        }],
    }
}

/// Stamp `cache_control: ephemeral` on the *last* tool definition. Anthropic
/// caches everything **before** the breakpoint, so marking the last entry of
/// the tools array caches the whole tool block in one shot.
fn apply_tools_cache(tools: &mut [ApiTool], hints: &CacheHints) {
    if hints.cache_tools {
        if let Some(last) = tools.last_mut() {
            last.cache_control = Some(ApiCacheControl::ephemeral());
        }
    }
}

#[derive(Debug, Serialize)]
struct ApiMessage {
    role: String,
    content: ApiContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ApiContent {
    Text(String),
    Blocks(Vec<ApiContentBlock>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ApiContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: ApiImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

#[derive(Debug, Serialize)]
struct ApiImageSource {
    #[serde(rename = "type")]
    source_type: String,
    media_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct ApiTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<ApiCacheControl>,
}

/// Anthropic Messages API response body.
#[derive(Debug, Deserialize)]
struct ApiResponse {
    content: Vec<ResponseContentBlock>,
    stop_reason: String,
    usage: ApiUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum ResponseContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Debug, Deserialize)]
struct ApiUsage {
    input_tokens: u64,
    output_tokens: u64,
    /// Tokens served from a previously created cache entry — billed at 10%.
    /// Only present when prompt caching is engaged (cache_control markers).
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
    /// Tokens charged at 125% to populate the cache for future reads.
    /// Only present on the *first* request that creates a cache entry.
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
}

/// Anthropic API error response.
#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

/// Accumulator for content blocks during streaming.
enum ContentBlockAccum {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        input_json: String,
    },
}

struct AnthropicStreamEvent {
    event_type: String,
    data: String,
}

struct AnthropicStreamAccumulator {
    buffer: String,
    blocks: Vec<ContentBlockAccum>,
    stop_reason: StopReason,
    usage: TokenUsage,
}

impl Default for AnthropicStreamAccumulator {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            blocks: Vec::new(),
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
        }
    }
}

impl AnthropicStreamAccumulator {
    fn push_chunk_text(&mut self, chunk: &str) -> Vec<AnthropicStreamEvent> {
        self.buffer.push_str(chunk);
        let mut events = Vec::new();

        while let Some(pos) = self.buffer.find("\n\n") {
            let event_text = self.buffer[..pos].to_string();
            self.buffer = self.buffer[pos + 2..].to_string();
            if let Some(event) = parse_anthropic_sse_event(&event_text) {
                events.push(event);
            }
        }

        events
    }

    async fn ingest_event(&mut self, event: AnthropicStreamEvent, tx: &Sender<StreamEvent>) {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&event.data) else {
            return;
        };
        self.ingest_json_event(&event.event_type, &json, tx).await;
    }

    async fn ingest_json_event(
        &mut self,
        event_type: &str,
        json: &serde_json::Value,
        tx: &Sender<StreamEvent>,
    ) {
        match event_type {
            "message_start" => self.ingest_message_start(json),
            "content_block_start" => self.ingest_content_block_start(json, tx).await,
            "content_block_delta" => self.ingest_content_block_delta(json, tx).await,
            "content_block_stop" => self.ingest_content_block_stop(json, tx).await,
            "message_delta" => self.ingest_message_delta(json),
            _ => {}
        }
    }

    fn ingest_message_start(&mut self, json: &serde_json::Value) {
        if let Some(input_tokens) = json["message"]["usage"]["input_tokens"].as_u64() {
            self.usage.input_tokens = input_tokens;
        }
    }

    async fn ingest_content_block_start(
        &mut self,
        json: &serde_json::Value,
        tx: &Sender<StreamEvent>,
    ) {
        let block = &json["content_block"];
        match block["type"].as_str().unwrap_or("") {
            "text" => self.blocks.push(ContentBlockAccum::Text(String::new())),
            "tool_use" => {
                let id = block["id"].as_str().unwrap_or("").to_string();
                let name = block["name"].as_str().unwrap_or("").to_string();
                let _ = tx
                    .send(StreamEvent::ToolUseStart {
                        id: id.clone(),
                        name: name.clone(),
                    })
                    .await;
                self.blocks.push(ContentBlockAccum::ToolUse {
                    id,
                    name,
                    input_json: String::new(),
                });
            }
            "thinking" => self.blocks.push(ContentBlockAccum::Thinking(String::new())),
            _ => {}
        }
    }

    async fn ingest_content_block_delta(
        &mut self,
        json: &serde_json::Value,
        tx: &Sender<StreamEvent>,
    ) {
        let block_idx = json["index"].as_u64().unwrap_or(0) as usize;
        let delta = &json["delta"];
        match delta["type"].as_str().unwrap_or("") {
            "text_delta" => {
                if let Some(text) = delta["text"].as_str() {
                    if let Some(ContentBlockAccum::Text(ref mut existing)) =
                        self.blocks.get_mut(block_idx)
                    {
                        existing.push_str(text);
                    }
                    let _ = tx
                        .send(StreamEvent::TextDelta {
                            text: text.to_string(),
                        })
                        .await;
                }
            }
            "input_json_delta" => {
                if let Some(partial) = delta["partial_json"].as_str() {
                    if let Some(ContentBlockAccum::ToolUse {
                        ref mut input_json, ..
                    }) = self.blocks.get_mut(block_idx)
                    {
                        input_json.push_str(partial);
                    }
                    let _ = tx
                        .send(StreamEvent::ToolInputDelta {
                            text: partial.to_string(),
                        })
                        .await;
                }
            }
            "thinking_delta" => {
                if let Some(thinking) = delta["thinking"].as_str() {
                    if let Some(ContentBlockAccum::Thinking(ref mut existing)) =
                        self.blocks.get_mut(block_idx)
                    {
                        existing.push_str(thinking);
                    }
                }
            }
            _ => {}
        }
    }

    async fn ingest_content_block_stop(
        &mut self,
        json: &serde_json::Value,
        tx: &Sender<StreamEvent>,
    ) {
        let block_idx = json["index"].as_u64().unwrap_or(0) as usize;
        if let Some(ContentBlockAccum::ToolUse {
            id,
            name,
            input_json,
        }) = self.blocks.get(block_idx)
        {
            let input: serde_json::Value =
                serde_json::from_str(input_json).unwrap_or_else(|_| serde_json::json!({}));
            let _ = tx
                .send(StreamEvent::ToolUseEnd {
                    id: id.clone(),
                    name: name.clone(),
                    input,
                })
                .await;
        }
    }

    fn ingest_message_delta(&mut self, json: &serde_json::Value) {
        if let Some(stop_reason) = json["delta"]["stop_reason"].as_str() {
            self.stop_reason = parse_stop_reason(stop_reason);
        }
        if let Some(output_tokens) = json["usage"]["output_tokens"].as_u64() {
            self.usage.output_tokens = output_tokens;
        }
    }

    fn into_completion_response(self) -> CompletionResponse {
        let mut content = Vec::new();
        let mut tool_calls = Vec::new();

        for block in self.blocks {
            match block {
                ContentBlockAccum::Text(text) => {
                    content.push(ContentBlock::Text {
                        text,
                        provider_metadata: None,
                    });
                }
                ContentBlockAccum::Thinking(thinking) => {
                    content.push(ContentBlock::Thinking {
                        thinking,
                        provider_metadata: None,
                    });
                }
                ContentBlockAccum::ToolUse {
                    id,
                    name,
                    input_json,
                } => {
                    // #186 — guarantee a JSON object (Anthropic 400s on
                    // anything else, including the Null produced by an
                    // empty input_json from a no-arg tool like system_time).
                    let input = crate::drivers::util::parse_tool_input(&input_json);
                    content.push(ContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                        provider_metadata: None,
                    });
                    tool_calls.push(ToolCall { id, name, input });
                }
            }
        }

        CompletionResponse {
            content,
            stop_reason: self.stop_reason,
            tool_calls,
            usage: self.usage,
        }
    }
}

#[async_trait]
impl LlmDriver for AnthropicDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let api_request = build_api_request(&request, false);

        for attempt in 0..=MAX_ANTHROPIC_RETRIES {
            let resp = self
                .send_messages_request(&api_request, attempt, false)
                .await?;
            let Some(resp) = handle_anthropic_status(resp, attempt, false).await? else {
                continue;
            };
            return Ok(convert_response(parse_api_response(resp).await?));
        }

        Err(max_retries_exceeded())
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let api_request = build_api_request(&request, true);

        for attempt in 0..=MAX_ANTHROPIC_RETRIES {
            let resp = self
                .send_messages_request(&api_request, attempt, true)
                .await?;
            let Some(resp) = handle_anthropic_status(resp, attempt, true).await? else {
                continue;
            };
            return completion_response_from_anthropic_stream(resp, tx).await;
        }

        Err(max_retries_exceeded())
    }
}

fn build_api_request(request: &CompletionRequest, stream: bool) -> ApiRequest {
    let mut tools = build_api_tools(request);
    apply_tools_cache(&mut tools, &request.cache_hints);

    ApiRequest {
        model: request.model.clone(),
        max_tokens: request.max_tokens,
        system: build_system_field(extract_system_prompt(request), &request.cache_hints),
        messages: build_api_messages(&request.messages),
        tools,
        temperature: Some(request.temperature),
        stream,
    }
}

fn extract_system_prompt(request: &CompletionRequest) -> Option<String> {
    request.system.clone().or_else(|| {
        request.messages.iter().find_map(|message| {
            if message.role == Role::System {
                match &message.content {
                    MessageContent::Text(text) => Some(text.clone()),
                    _ => None,
                }
            } else {
                None
            }
        })
    })
}

fn build_api_messages(messages: &[Message]) -> Vec<ApiMessage> {
    messages
        .iter()
        .filter(|message| message.role != Role::System)
        .map(convert_message)
        .collect()
}

fn build_api_tools(request: &CompletionRequest) -> Vec<ApiTool> {
    request
        .tools
        .iter()
        .map(|tool| ApiTool {
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
            cache_control: None,
        })
        .collect()
}

async fn handle_anthropic_status(
    resp: reqwest::Response,
    attempt: usize,
    streaming: bool,
) -> Result<Option<reqwest::Response>, LlmError> {
    let status = resp.status().as_u16();

    if status == 429 || status == 529 {
        if attempt < MAX_ANTHROPIC_RETRIES {
            let retry_ms = (attempt + 1) as u64 * 2000;
            if streaming {
                warn!(status, retry_ms, "Rate limited (stream), retrying");
            } else {
                warn!(status, retry_ms, "Rate limited, retrying");
            }
            tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
            return Ok(None);
        }
        return Err(retry_error_for_status(status));
    }

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        let message = serde_json::from_str::<ApiErrorResponse>(&body)
            .map(|error| error.error.message)
            .unwrap_or(body);
        return Err(LlmError::Api { status, message });
    }

    Ok(Some(resp))
}

fn retry_error_for_status(status: u16) -> LlmError {
    if status == 429 {
        LlmError::RateLimited {
            retry_after_ms: 5000,
        }
    } else {
        LlmError::Overloaded {
            retry_after_ms: 5000,
        }
    }
}

async fn parse_api_response(resp: reqwest::Response) -> Result<ApiResponse, LlmError> {
    let body = resp
        .text()
        .await
        .map_err(|error| LlmError::Http(error.to_string()))?;
    serde_json::from_str(&body).map_err(|error| LlmError::Parse(error.to_string()))
}

async fn completion_response_from_anthropic_stream(
    resp: reqwest::Response,
    tx: Sender<StreamEvent>,
) -> Result<CompletionResponse, LlmError> {
    let mut accumulator = AnthropicStreamAccumulator::default();
    let mut byte_stream = resp.bytes_stream();

    while let Some(chunk_result) = byte_stream.next().await {
        let chunk = chunk_result.map_err(|error| LlmError::Http(error.to_string()))?;
        for event in accumulator.push_chunk_text(&String::from_utf8_lossy(&chunk)) {
            accumulator.ingest_event(event, &tx).await;
        }
    }

    let response = accumulator.into_completion_response();
    let _ = tx
        .send(StreamEvent::ContentComplete {
            stop_reason: response.stop_reason,
            usage: response.usage,
        })
        .await;
    Ok(response)
}

fn parse_anthropic_sse_event(event_text: &str) -> Option<AnthropicStreamEvent> {
    let mut event_type = String::new();
    let mut data = String::new();
    for line in event_text.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_type = value.trim_start().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            data = value.trim_start().to_string();
        }
    }

    if data.is_empty() {
        return None;
    }

    Some(AnthropicStreamEvent { event_type, data })
}

/// Convert an Captain Message to an Anthropic API message.
fn convert_message(msg: &Message) -> ApiMessage {
    let role = match msg.role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "user", // Should be filtered out, but handle gracefully
    };

    let content = match &msg.content {
        MessageContent::Text(text) => ApiContent::Text(text.clone()),
        MessageContent::Blocks(blocks) => {
            let api_blocks: Vec<ApiContentBlock> = blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text, .. } => {
                        Some(ApiContentBlock::Text { text: text.clone() })
                    }
                    ContentBlock::Image { media_type, data } => Some(ApiContentBlock::Image {
                        source: ApiImageSource {
                            source_type: "base64".to_string(),
                            media_type: media_type.clone(),
                            data: data.clone(),
                        },
                    }),
                    ContentBlock::ToolUse {
                        id, name, input, ..
                    } => Some(ApiContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    }),
                    ContentBlock::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                        ..
                    } => Some(ApiContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: content.clone(),
                        is_error: *is_error,
                    }),
                    ContentBlock::Thinking { .. } => None,
                    ContentBlock::Unknown => None,
                })
                .collect();
            ApiContent::Blocks(api_blocks)
        }
    };

    ApiMessage {
        role: role.to_string(),
        content,
    }
}

/// Convert an Anthropic API response to our CompletionResponse.
fn convert_response(api: ApiResponse) -> CompletionResponse {
    let mut content = Vec::new();
    let mut tool_calls = Vec::new();

    for block in api.content {
        match block {
            ResponseContentBlock::Text { text } => {
                content.push(ContentBlock::Text {
                    text,
                    provider_metadata: None,
                });
            }
            ResponseContentBlock::ToolUse { id, name, input } => {
                content.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                    provider_metadata: None,
                });
                tool_calls.push(ToolCall { id, name, input });
            }
            ResponseContentBlock::Thinking { thinking } => {
                content.push(ContentBlock::Thinking {
                    thinking,
                    provider_metadata: None,
                });
            }
        }
    }

    let stop_reason = parse_stop_reason(api.stop_reason.as_str());

    CompletionResponse {
        content,
        stop_reason,
        tool_calls,
        usage: TokenUsage {
            input_tokens: api.usage.input_tokens,
            output_tokens: api.usage.output_tokens,
            cached_input_tokens: api.usage.cache_read_input_tokens.unwrap_or(0),
            cache_creation_tokens: api.usage.cache_creation_input_tokens.unwrap_or(0),
        },
    }
}

fn parse_stop_reason(stop_reason: &str) -> StopReason {
    match stop_reason {
        "end_turn" => StopReason::EndTurn,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        _ => StopReason::EndTurn,
    }
}

fn max_retries_exceeded() -> LlmError {
    LlmError::Api {
        status: 0,
        message: "Max retries exceeded".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_system_no_hints_stays_plain_string() {
        let hints = CacheHints::default();
        let s = build_system_field(Some("hello".into()), &hints).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        // Untagged enum picks the String variant — no array, no cache_control
        assert_eq!(json, "\"hello\"");
        assert!(!json.contains("cache_control"));
    }

    #[test]
    fn build_system_with_cache_hint_emits_block_with_ephemeral() {
        let hints = CacheHints {
            cache_system: true,
            cache_tools: false,
            ..CacheHints::default()
        };
        let s = build_system_field(Some("stable prefix".into()), &hints).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"stable prefix\""));
        assert!(json.contains("\"cache_control\""));
        assert!(json.contains("\"ephemeral\""));
    }

    #[test]
    fn build_system_with_prefix_split_caches_only_stable_block() {
        let system = "stable instructions\n\n## Current Date\nToday is Monday".to_string();
        let split = "stable instructions".len();
        let hints = CacheHints {
            cache_system: true,
            cache_tools: false,
            cacheable_system_prefix_bytes: Some(split),
            prompt_cache_key: None,
        };

        let s = build_system_field(Some(system), &hints).unwrap();
        let ApiSystem::Blocks(blocks) = s else {
            panic!("expected block system payload");
        };
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].text, "stable instructions");
        assert!(blocks[0].cache_control.is_some());
        assert!(blocks[1].text.contains("## Current Date"));
        assert!(blocks[1].cache_control.is_none());
    }

    #[test]
    fn build_system_with_invalid_prefix_split_falls_back_to_whole_cache_block() {
        let hints = CacheHints {
            cache_system: true,
            cache_tools: false,
            cacheable_system_prefix_bytes: Some("é".len() - 1),
            prompt_cache_key: None,
        };

        let s = build_system_field(Some("é dynamic".into()), &hints).unwrap();
        let ApiSystem::Blocks(blocks) = s else {
            panic!("expected block system payload");
        };
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].text, "é dynamic");
        assert!(blocks[0].cache_control.is_some());
    }

    #[test]
    fn build_system_returns_none_when_system_absent() {
        let hints = CacheHints::full();
        assert!(build_system_field(None, &hints).is_none());
    }

    #[test]
    fn apply_tools_cache_marks_only_last_tool() {
        let mut tools = vec![
            ApiTool {
                name: "a".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
                cache_control: None,
            },
            ApiTool {
                name: "b".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
                cache_control: None,
            },
            ApiTool {
                name: "c".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
                cache_control: None,
            },
        ];
        let hints = CacheHints {
            cache_system: false,
            cache_tools: true,
            ..CacheHints::default()
        };
        apply_tools_cache(&mut tools, &hints);
        assert!(tools[0].cache_control.is_none());
        assert!(tools[1].cache_control.is_none());
        assert!(
            tools[2].cache_control.is_some(),
            "last tool gets the breakpoint"
        );
    }

    #[test]
    fn apply_tools_cache_no_op_when_hint_disabled() {
        let mut tools = vec![ApiTool {
            name: "a".into(),
            description: "".into(),
            input_schema: serde_json::json!({}),
            cache_control: None,
        }];
        apply_tools_cache(&mut tools, &CacheHints::default());
        assert!(tools[0].cache_control.is_none());
    }

    #[test]
    fn apply_tools_cache_handles_empty_tools() {
        let mut tools: Vec<ApiTool> = vec![];
        apply_tools_cache(&mut tools, &CacheHints::full());
        assert!(tools.is_empty());
    }

    #[tokio::test]
    async fn stream_accumulator_maps_blocks_usage_and_events() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mut accumulator = AnthropicStreamAccumulator::default();

        feed_stream_accumulator_fixture(&mut accumulator, &tx).await;
        drop(tx);
        let response = accumulator.into_completion_response();

        assert_stream_accumulator_response(&response);
        assert_stream_accumulator_events(&mut rx).await;
    }

    async fn feed_stream_accumulator_fixture(
        accumulator: &mut AnthropicStreamAccumulator,
        tx: &Sender<StreamEvent>,
    ) {
        accumulator
            .ingest_json_event(
                "message_start",
                &serde_json::json!({"message": {"usage": {"input_tokens": 7}}}),
                tx,
            )
            .await;
        feed_text_block_fixture(accumulator, tx).await;
        feed_tool_block_fixture(accumulator, tx).await;
        feed_thinking_block_fixture(accumulator, tx).await;
        accumulator
            .ingest_json_event(
                "message_delta",
                &serde_json::json!({
                    "delta": {"stop_reason": "tool_use"},
                    "usage": {"output_tokens": 11}
                }),
                tx,
            )
            .await;
    }

    async fn feed_text_block_fixture(
        accumulator: &mut AnthropicStreamAccumulator,
        tx: &Sender<StreamEvent>,
    ) {
        accumulator
            .ingest_json_event(
                "content_block_start",
                &serde_json::json!({"index": 0, "content_block": {"type": "text"}}),
                tx,
            )
            .await;
        accumulator
            .ingest_json_event(
                "content_block_delta",
                &serde_json::json!({"index": 0, "delta": {"type": "text_delta", "text": "hi"}}),
                tx,
            )
            .await;
    }

    async fn feed_tool_block_fixture(
        accumulator: &mut AnthropicStreamAccumulator,
        tx: &Sender<StreamEvent>,
    ) {
        accumulator
            .ingest_json_event(
                "content_block_start",
                &serde_json::json!({
                    "index": 1,
                    "content_block": {"type": "tool_use", "id": "tool_1", "name": "lookup"}
                }),
                tx,
            )
            .await;
        accumulator
            .ingest_json_event(
                "content_block_delta",
                &serde_json::json!({
                    "index": 1,
                    "delta": {"type": "input_json_delta", "partial_json": "{\"q\":\"rust\"}"}
                }),
                tx,
            )
            .await;
        accumulator
            .ingest_json_event("content_block_stop", &serde_json::json!({"index": 1}), tx)
            .await;
    }

    async fn feed_thinking_block_fixture(
        accumulator: &mut AnthropicStreamAccumulator,
        tx: &Sender<StreamEvent>,
    ) {
        accumulator
            .ingest_json_event(
                "content_block_start",
                &serde_json::json!({"index": 2, "content_block": {"type": "thinking"}}),
                tx,
            )
            .await;
        accumulator
            .ingest_json_event(
                "content_block_delta",
                &serde_json::json!({
                    "index": 2,
                    "delta": {"type": "thinking_delta", "thinking": "consider"}
                }),
                tx,
            )
            .await;
    }

    fn assert_stream_accumulator_response(response: &CompletionResponse) {
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.usage.input_tokens, 7);
        assert_eq!(response.usage.output_tokens, 11);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "lookup");
        assert_eq!(
            response.tool_calls[0].input,
            serde_json::json!({"q": "rust"})
        );

        match &response.content[0] {
            ContentBlock::Text { text, .. } => assert_eq!(text, "hi"),
            other => panic!("expected text block, got {other:?}"),
        }
        match &response.content[2] {
            ContentBlock::Thinking { thinking, .. } => assert_eq!(thinking, "consider"),
            other => panic!("expected thinking block, got {other:?}"),
        }
    }

    async fn assert_stream_accumulator_events(rx: &mut tokio::sync::mpsc::Receiver<StreamEvent>) {
        match rx.recv().await.unwrap() {
            StreamEvent::TextDelta { text } => assert_eq!(text, "hi"),
            other => panic!("expected text delta, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            StreamEvent::ToolUseStart { id, name } => {
                assert_eq!(id, "tool_1");
                assert_eq!(name, "lookup");
            }
            other => panic!("expected tool start, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            StreamEvent::ToolInputDelta { text } => assert_eq!(text, "{\"q\":\"rust\"}"),
            other => panic!("expected tool input delta, got {other:?}"),
        }
        match rx.recv().await.unwrap() {
            StreamEvent::ToolUseEnd { id, name, input } => {
                assert_eq!(id, "tool_1");
                assert_eq!(name, "lookup");
                assert_eq!(input, serde_json::json!({"q": "rust"}));
            }
            other => panic!("expected tool end, got {other:?}"),
        }
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn test_convert_message_text() {
        let msg = Message::user("Hello");
        let api_msg = convert_message(&msg);
        assert_eq!(api_msg.role, "user");
    }

    #[test]
    fn test_convert_response() {
        let api_response = ApiResponse {
            content: vec![
                ResponseContentBlock::Text {
                    text: "I'll help you.".to_string(),
                },
                ResponseContentBlock::ToolUse {
                    id: "tool_1".to_string(),
                    name: "web_search".to_string(),
                    input: serde_json::json!({"query": "rust lang"}),
                },
            ],
            stop_reason: "tool_use".to_string(),
            usage: ApiUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        };

        let response = convert_response(api_response);
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "web_search");
        assert_eq!(response.usage.total(), 150);
    }
}

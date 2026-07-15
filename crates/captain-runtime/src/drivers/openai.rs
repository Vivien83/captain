//! OpenAI-compatible API driver.
//!
//! Works with OpenAI, Ollama, vLLM, and any other OpenAI-compatible endpoint.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use crate::think_filter::{FilterAction, StreamingThinkFilter};
use async_trait::async_trait;
use captain_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use captain_types::tool::ToolCall;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use zeroize::Zeroizing;

/// Azure OpenAI API version query parameter.
const AZURE_API_VERSION: &str = "2024-10-21";

/// Normalize a tool_call_id to satisfy the strictest provider constraint.
///
/// Mistral rejects tool_call_ids longer than 9 chars or containing non-alphanumerics.
/// We hash the original id to a deterministic 9-char alphanumeric token so that
/// cross-provider fallback (e.g. Qwen → Mistral) preserves tool_use/tool_result pairing.
fn normalize_tool_call_id(raw: &str) -> String {
    if raw.len() <= 9 && raw.chars().all(|c| c.is_ascii_alphanumeric()) {
        return raw.to_string();
    }
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut hasher);
    let h = hasher.finish();
    const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(9);
    let mut v = h;
    for _ in 0..9 {
        out.push(ALPHA[(v as usize) % ALPHA.len()] as char);
        v /= ALPHA.len() as u64;
    }
    out
}

/// OpenAI-compatible API driver.
pub struct OpenAIDriver {
    api_key: Zeroizing<String>,
    base_url: String,
    client: reqwest::Client,
    extra_headers: Vec<(String, String)>,
    /// When true, uses Azure OpenAI URL format and `api-key` header.
    azure_mode: bool,
}

impl OpenAIDriver {
    /// Create a new OpenAI-compatible driver.
    pub fn new(api_key: String, base_url: String) -> Self {
        Self {
            api_key: Zeroizing::new(api_key),
            base_url,
            client: reqwest::Client::builder()
                .user_agent(crate::USER_AGENT)
                .build()
                .unwrap_or_default(),
            extra_headers: Vec::new(),
            azure_mode: false,
        }
    }

    /// Create a driver configured for Azure OpenAI.
    ///
    /// Azure uses a deployment-based URL scheme and `api-key` header instead of
    /// `Authorization: Bearer`.  The `base_url` should be the deployments root,
    /// e.g. `https://{resource}.openai.azure.com/openai/deployments`.
    pub fn new_azure(api_key: String, base_url: String) -> Self {
        Self {
            api_key: Zeroizing::new(api_key),
            base_url,
            client: reqwest::Client::builder()
                .user_agent(crate::USER_AGENT)
                .build()
                .unwrap_or_default(),
            extra_headers: Vec::new(),
            azure_mode: true,
        }
    }

    /// True if this provider is Moonshot/Kimi and requires reasoning_content on assistant messages with tool_calls.
    fn needs_reasoning_content(&self, model: &str) -> bool {
        self.base_url.contains("moonshot")
            || model.to_lowercase().contains("kimi")
            || model.to_lowercase().contains("reasoner")
    }

    /// Create a driver with additional HTTP headers (e.g. for Copilot IDE auth).
    pub fn with_extra_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }

    /// Build the chat completions URL for the given model.
    ///
    /// Standard OpenAI: `{base_url}/chat/completions`
    /// Azure OpenAI:    `{base_url}/{model}/chat/completions?api-version=2024-10-21`
    fn chat_url(&self, model: &str) -> String {
        if self.azure_mode {
            format!(
                "{}/{}/chat/completions?api-version={}",
                self.base_url.trim_end_matches('/'),
                model,
                AZURE_API_VERSION,
            )
        } else {
            format!("{}/chat/completions", self.base_url)
        }
    }

    /// Apply authentication headers to the request builder.
    ///
    /// Standard: `Authorization: Bearer {key}`
    /// Azure:    `api-key: {key}`
    fn apply_auth(&self, mut builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.api_key.as_str().is_empty() {
            return builder;
        }
        if self.azure_mode {
            builder = builder.header("api-key", self.api_key.as_str());
        } else {
            builder = builder.header("authorization", format!("Bearer {}", self.api_key.as_str()));
        }
        builder
    }

    fn build_oai_request(&self, request: &CompletionRequest, mode: OaiRequestMode) -> OaiRequest {
        let needs_reasoning = self.needs_reasoning_content(&request.model);
        let tools = build_oai_tools(&request.tools);
        let tool_choice = build_oai_tool_choice(&tools, &request.tool_choice);
        let (max_tokens, max_completion_tokens) =
            oai_token_limits(&request.model, request.max_tokens);

        OaiRequest {
            model: request.model.clone(),
            messages: build_oai_messages(request, needs_reasoning),
            max_tokens,
            max_completion_tokens,
            temperature: oai_temperature(&request.model, request.temperature, needs_reasoning),
            tools,
            tool_choice,
            stream: mode.is_stream(),
            stream_options: mode.stream_options(),
            thinking: oai_thinking(needs_reasoning),
        }
    }

    async fn send_chat_request(
        &self,
        model: &str,
        oai_request: &OaiRequest,
        attempt: u32,
        streaming: bool,
    ) -> Result<reqwest::Response, LlmError> {
        let url = self.chat_url(model);
        if streaming {
            debug!(url = %url, attempt, "Sending OpenAI streaming request");
        } else {
            debug!(url = %url, attempt, "Sending OpenAI API request");
        }

        let req_builder = self
            .client
            .post(&url)
            .header("content-type", "application/json")
            .json(oai_request);

        let mut req_builder = self.apply_auth(req_builder);
        for (k, v) in &self.extra_headers {
            req_builder = req_builder.header(k, v);
        }

        req_builder
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))
    }
}

#[derive(Debug, Serialize)]
struct OaiRequest {
    model: String,
    messages: Vec<OaiMessage>,
    /// Classic token limit field (used by most models).
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// New token limit field required by GPT-5 and o-series reasoning models.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OaiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
    /// Request usage stats in streaming responses (OpenAI extension, supported by Groq et al).
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<serde_json::Value>,
    /// Moonshot Kimi K2.5: disable thinking so multi-turn with tool_calls works without preserving reasoning_content.
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
}

/// Returns true if a model uses `max_completion_tokens` instead of `max_tokens`.
fn uses_completion_tokens(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("gpt-5")
        || m.starts_with("gpt5")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
}

/// Returns true if a model rejects the `temperature` parameter.
///
/// OpenAI's o-series reasoning models and GPT-5-mini variants only accept
/// `temperature=1` (the default). Sending any other value causes a 400 error.
/// We proactively omit `temperature` for these models to avoid wasting a retry.
fn rejects_temperature(model: &str) -> bool {
    let m = model.to_lowercase();
    // o-series reasoning models: o1, o1-mini, o1-preview, o3, o3-mini, o3-pro, o4-mini, etc.
    m.starts_with("o1")
        || m.starts_with("o3")
        || m.starts_with("o4")
        // GPT-5 nano/mini are reasoning models that reject temperature
        || m.starts_with("gpt-5-mini")
        || m.starts_with("gpt-5-nano")
        || m.starts_with("gpt5-mini")
        || m.starts_with("gpt5-nano")
        // DeepSeek-R1 reasoning models
        || m.contains("deepseek-r1")
        || m.contains("reasoner")
        // Catch any model explicitly tagged as "reasoning"
        || m.contains("-reasoning")
}

/// Returns true if a model only accepts temperature = 1 (e.g. Moonshot Kimi K2/K2.5).
fn temperature_must_be_one(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("kimi-k2") || m == "kimi-k2.5" || m == "kimi-k2.5-0711"
}

#[derive(Debug, Serialize)]
struct OaiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OaiMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    /// Moonshot Kimi: sent as empty string on assistant messages with tool_calls when using Kimi (thinking is disabled for multi-turn compatibility).
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
}

/// Content can be a plain string or an array of content parts (for images).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OaiMessageContent {
    Text(String),
    Parts(Vec<OaiContentPart>),
}

/// A content part for multi-modal messages.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OaiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OaiImageUrl },
}

#[derive(Debug, Serialize)]
struct OaiImageUrl {
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type", default = "default_tool_call_type")]
    call_type: String,
    function: OaiFunction,
}

fn default_tool_call_type() -> String {
    "function".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OaiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OaiToolDef,
}

#[derive(Debug, Serialize)]
struct OaiToolDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OaiRequestMode {
    Complete,
    Stream,
}

impl OaiRequestMode {
    fn is_stream(self) -> bool {
        matches!(self, Self::Stream)
    }

    fn stream_options(self) -> Option<serde_json::Value> {
        if self.is_stream() {
            Some(serde_json::json!({"include_usage": true}))
        } else {
            None
        }
    }
}

fn build_oai_messages(request: &CompletionRequest, needs_reasoning: bool) -> Vec<OaiMessage> {
    let mut messages = Vec::new();

    if let Some(system) = &request.system {
        push_text_message(&mut messages, "system", system);
    }

    for msg in &request.messages {
        match (&msg.role, &msg.content) {
            (Role::System, MessageContent::Text(text)) => {
                if request.system.is_none() {
                    push_text_message(&mut messages, "system", text);
                }
            }
            (Role::User, MessageContent::Text(text)) => {
                push_text_message(&mut messages, "user", text);
            }
            (Role::Assistant, MessageContent::Text(text)) => {
                push_text_message(&mut messages, "assistant", text);
            }
            (Role::User, MessageContent::Blocks(blocks)) => {
                push_user_block_messages(&mut messages, blocks);
            }
            (Role::Assistant, MessageContent::Blocks(blocks)) => {
                push_assistant_block_message(&mut messages, blocks, needs_reasoning);
            }
            _ => {}
        }
    }

    messages
}

fn push_text_message(messages: &mut Vec<OaiMessage>, role: &str, text: &str) {
    messages.push(OaiMessage {
        role: role.to_string(),
        content: Some(OaiMessageContent::Text(text.to_string())),
        tool_calls: None,
        tool_call_id: None,
        reasoning_content: None,
    });
}

fn push_user_block_messages(messages: &mut Vec<OaiMessage>, blocks: &[ContentBlock]) {
    let mut parts = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                messages.push(OaiMessage {
                    role: "tool".to_string(),
                    content: Some(OaiMessageContent::Text(if content.is_empty() {
                        "(empty)".to_string()
                    } else {
                        content.clone()
                    })),
                    tool_calls: None,
                    tool_call_id: Some(normalize_tool_call_id(tool_use_id)),
                    reasoning_content: None,
                });
            }
            ContentBlock::Text { text, .. } => {
                parts.push(OaiContentPart::Text { text: text.clone() });
            }
            ContentBlock::Image { media_type, data } => {
                parts.push(OaiContentPart::ImageUrl {
                    image_url: OaiImageUrl {
                        url: format!("data:{media_type};base64,{data}"),
                    },
                });
            }
            ContentBlock::Thinking { .. } => {}
            _ => {}
        }
    }

    if !parts.is_empty() {
        messages.push(OaiMessage {
            role: "user".to_string(),
            content: Some(OaiMessageContent::Parts(parts)),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
        });
    }
}

fn push_assistant_block_message(
    messages: &mut Vec<OaiMessage>,
    blocks: &[ContentBlock],
    needs_reasoning: bool,
) {
    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();
    let mut reasoning_text = String::new();

    for block in blocks {
        match block {
            ContentBlock::Text { text, .. } => text_parts.push(text.clone()),
            ContentBlock::ToolUse {
                id, name, input, ..
            } => {
                tool_calls.push(OaiToolCall {
                    id: normalize_tool_call_id(id),
                    call_type: "function".to_string(),
                    function: OaiFunction {
                        name: name.clone(),
                        arguments: serde_json::to_string(input).unwrap_or_default(),
                    },
                });
            }
            ContentBlock::Thinking { thinking, .. } => {
                reasoning_text = thinking.clone();
            }
            _ => {}
        }
    }

    let has_tool_calls = !tool_calls.is_empty();
    messages.push(OaiMessage {
        role: "assistant".to_string(),
        content: if text_parts.is_empty() {
            if has_tool_calls {
                Some(OaiMessageContent::Text(String::new()))
            } else {
                None
            }
        } else {
            Some(OaiMessageContent::Text(text_parts.join("")))
        },
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: None,
        reasoning_content: if needs_reasoning {
            Some(if reasoning_text.is_empty() {
                String::new()
            } else {
                reasoning_text
            })
        } else {
            None
        },
    });
}

fn build_oai_tools(tools: &[captain_types::tool::ToolDefinition]) -> Vec<OaiTool> {
    tools
        .iter()
        .map(|t| OaiTool {
            tool_type: "function".to_string(),
            function: OaiToolDef {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: captain_types::tool::normalize_schema_for_provider(
                    &t.input_schema,
                    "openai",
                ),
            },
        })
        .collect()
}

fn build_oai_tool_choice(
    tools: &[OaiTool],
    request_tool_choice: &Option<serde_json::Value>,
) -> Option<serde_json::Value> {
    if tools.is_empty() {
        None
    } else {
        request_tool_choice
            .clone()
            .or(Some(serde_json::json!("auto")))
    }
}

fn oai_token_limits(model: &str, max_tokens: u32) -> (Option<u32>, Option<u32>) {
    if uses_completion_tokens(model) {
        (None, Some(max_tokens))
    } else {
        (Some(max_tokens), None)
    }
}

fn oai_temperature(model: &str, requested: f32, needs_reasoning: bool) -> Option<f32> {
    if needs_reasoning {
        Some(0.6)
    } else if temperature_must_be_one(model) {
        Some(1.0)
    } else if rejects_temperature(model) {
        None
    } else {
        Some(requested)
    }
}

fn oai_thinking(needs_reasoning: bool) -> Option<serde_json::Value> {
    if needs_reasoning {
        Some(serde_json::json!({"type": "disabled"}))
    } else {
        None
    }
}

#[derive(Debug, Deserialize)]
struct OaiResponse {
    choices: Vec<OaiChoice>,
    usage: Option<OaiUsage>,
}

#[derive(Debug, Deserialize)]
struct OaiChoice {
    message: OaiResponseMessage,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiResponseMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OaiToolCall>>,
    /// Reasoning/thinking content returned by some models (DeepSeek-R1, Qwen3, etc.)
    /// via LM Studio, Ollama, and other local inference servers.
    reasoning_content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OaiUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    /// Present on OpenAI 2024-10+ models that expose automatic prompt
    /// caching (≥1024-token prefix, 50% off on cached tokens).
    #[serde(default)]
    prompt_tokens_details: Option<OaiPromptTokensDetails>,
}

#[derive(Debug, Deserialize, Default)]
struct OaiPromptTokensDetails {
    #[serde(default)]
    cached_tokens: u64,
}

fn completion_response_from_oai_response(
    oai_response: OaiResponse,
) -> Result<CompletionResponse, LlmError> {
    let mut usage = token_usage_from_oai_usage(oai_response.usage);
    let choice = first_oai_choice(oai_response.choices)?;
    let OaiChoice {
        message,
        finish_reason,
    } = choice;
    let has_tool_call_field = message.tool_calls.is_some();
    let mut content = Vec::new();
    let mut tool_calls = Vec::new();

    push_response_reasoning(&mut content, &message);
    push_response_text(&mut content, &message);
    synthesize_thinking_only_response(&mut content, !has_tool_call_field, "response");
    push_response_tool_calls(&mut content, &mut tool_calls, message.tool_calls);

    let stop_reason = stop_reason_from_finish(finish_reason.as_deref(), !tool_calls.is_empty());
    ensure_nonzero_output_usage(&content, &mut usage, "Response");

    Ok(CompletionResponse {
        content,
        stop_reason,
        tool_calls,
        usage,
    })
}

fn first_oai_choice(choices: Vec<OaiChoice>) -> Result<OaiChoice, LlmError> {
    choices
        .into_iter()
        .next()
        .ok_or_else(|| LlmError::Parse("No choices in response".to_string()))
}

fn push_response_reasoning(content: &mut Vec<ContentBlock>, message: &OaiResponseMessage) {
    if let Some(reasoning) = &message.reasoning_content {
        if !reasoning.is_empty() {
            debug!(
                len = reasoning.len(),
                "Captured reasoning_content from response"
            );
            content.push(ContentBlock::Thinking {
                thinking: reasoning.clone(),
                provider_metadata: None,
            });
        }
    }
}

fn push_response_text(content: &mut Vec<ContentBlock>, message: &OaiResponseMessage) {
    if let Some(text) = &message.content {
        if text.is_empty() {
            return;
        }

        let (cleaned, thinking) = extract_think_tags(text);
        if let Some(think_text) = thinking {
            if message.reasoning_content.is_none() {
                content.push(ContentBlock::Thinking {
                    thinking: think_text,
                    provider_metadata: None,
                });
            }
        }
        if !cleaned.is_empty() {
            content.push(ContentBlock::Text {
                text: cleaned,
                provider_metadata: None,
            });
        }
    }
}

fn synthesize_thinking_only_response(
    content: &mut Vec<ContentBlock>,
    has_no_tool_calls: bool,
    log_label: &str,
) {
    let has_text = content
        .iter()
        .any(|b| matches!(b, ContentBlock::Text { .. }));
    let has_thinking = content
        .iter()
        .any(|b| matches!(b, ContentBlock::Thinking { .. }));
    if has_thinking && !has_text && has_no_tool_calls {
        let thinking_text = content
            .iter()
            .find_map(|b| match b {
                ContentBlock::Thinking { thinking, .. } => Some(thinking.as_str()),
                _ => None,
            })
            .unwrap_or("");
        let summary = extract_thinking_summary(thinking_text);
        debug!(
            summary_len = summary.len(),
            "Synthesizing text from thinking-only {log_label}"
        );
        content.push(ContentBlock::Text {
            text: summary,
            provider_metadata: None,
        });
    }
}

fn push_response_tool_calls(
    content: &mut Vec<ContentBlock>,
    tool_calls: &mut Vec<ToolCall>,
    calls: Option<Vec<OaiToolCall>>,
) {
    if let Some(calls) = calls {
        for call in calls {
            let input: serde_json::Value =
                serde_json::from_str(&call.function.arguments).unwrap_or_default();
            let normalized_id = normalize_tool_call_id(&call.id);
            content.push(ContentBlock::ToolUse {
                id: normalized_id.clone(),
                name: call.function.name.clone(),
                input: input.clone(),
                provider_metadata: None,
            });
            tool_calls.push(ToolCall {
                id: normalized_id,
                name: call.function.name,
                input,
            });
        }
    }
}

fn stop_reason_from_finish(finish_reason: Option<&str>, has_tool_calls: bool) -> StopReason {
    match finish_reason {
        Some("stop") => StopReason::EndTurn,
        Some("tool_calls") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        _ => {
            if has_tool_calls {
                StopReason::ToolUse
            } else {
                StopReason::EndTurn
            }
        }
    }
}

fn token_usage_from_oai_usage(usage: Option<OaiUsage>) -> TokenUsage {
    usage
        .map(|u| TokenUsage {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
            cached_input_tokens: u
                .prompt_tokens_details
                .as_ref()
                .map(|d| d.cached_tokens)
                .unwrap_or(0),
            cache_creation_tokens: 0,
        })
        .unwrap_or_default()
}

fn ensure_nonzero_output_usage(
    content: &[ContentBlock],
    usage: &mut TokenUsage,
    response_label: &str,
) {
    if !content.is_empty() && usage.input_tokens == 0 && usage.output_tokens == 0 {
        debug!(
            "{response_label} has content but no usage stats — setting synthetic output_tokens=1"
        );
        usage.output_tokens = 1;
    }
}

enum OaiErrorOutcome {
    Retry,
    Recovered(CompletionResponse),
}

async fn handle_oai_error_response(
    status: u16,
    body: String,
    attempt: u32,
    max_retries: u32,
    oai_request: &mut OaiRequest,
    streaming: bool,
) -> Result<OaiErrorOutcome, LlmError> {
    if let Some(outcome) =
        handle_tool_use_failed_error(status, &body, attempt, max_retries, streaming).await
    {
        return Ok(outcome);
    }

    if should_strip_temperature(status, &body, oai_request, attempt, max_retries) {
        if streaming {
            warn!(model = %oai_request.model, "Stripping temperature for this model (stream)");
        } else {
            warn!(model = %oai_request.model, "Stripping temperature for this model");
        }
        oai_request.temperature = None;
        return Ok(OaiErrorOutcome::Retry);
    }

    if should_switch_to_completion_tokens(status, &body, oai_request, attempt, max_retries) {
        let val = oai_request.max_tokens.unwrap();
        if streaming {
            warn!(model = %oai_request.model, "Switching to max_completion_tokens for this model (stream)");
        } else {
            warn!(model = %oai_request.model, "Switching to max_completion_tokens for this model");
        }
        oai_request.max_tokens = None;
        oai_request.max_completion_tokens = Some(val);
        return Ok(OaiErrorOutcome::Retry);
    }

    if should_cap_max_tokens(status, &body, attempt, max_retries) {
        cap_oai_max_tokens(&body, oai_request, streaming);
        return Ok(OaiErrorOutcome::Retry);
    }

    if streaming && should_strip_stream_options(status, &body, oai_request, attempt, max_retries) {
        warn!(model = %oai_request.model, "Stripping stream_options (unsupported by provider)");
        oai_request.stream_options = None;
        return Ok(OaiErrorOutcome::Retry);
    }

    if should_retry_without_tools(status, &body, oai_request, attempt, max_retries) {
        warn!(
            model = %oai_request.model,
            status,
            "Model may not support tools (stream), retrying without tools"
        );
        oai_request.tools.clear();
        oai_request.tool_choice = None;
        return Ok(OaiErrorOutcome::Retry);
    }

    Err(LlmError::Api {
        status,
        message: body,
    })
}

async fn handle_tool_use_failed_error(
    status: u16,
    body: &str,
    attempt: u32,
    max_retries: u32,
    streaming: bool,
) -> Option<OaiErrorOutcome> {
    if status != 400 || !body.contains("tool_use_failed") {
        return None;
    }
    if let Some(response) = parse_groq_failed_tool_call(body) {
        if streaming {
            warn!("Recovered tool call from Groq failed_generation (stream)");
        } else {
            warn!("Recovered tool call from Groq failed_generation");
        }
        return Some(OaiErrorOutcome::Recovered(response));
    }
    if attempt < max_retries {
        let retry_ms = (attempt + 1) as u64 * 1500;
        if streaming {
            warn!(
                status,
                attempt, retry_ms, "tool_use_failed (stream), retrying"
            );
        } else {
            warn!(status, attempt, retry_ms, "tool_use_failed, retrying");
        }
        tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
        return Some(OaiErrorOutcome::Retry);
    }
    None
}

fn should_strip_temperature(
    status: u16,
    body: &str,
    oai_request: &OaiRequest,
    attempt: u32,
    max_retries: u32,
) -> bool {
    status == 400
        && body.contains("temperature")
        && body.contains("unsupported_parameter")
        && oai_request.temperature.is_some()
        && attempt < max_retries
}

fn should_switch_to_completion_tokens(
    status: u16,
    body: &str,
    oai_request: &OaiRequest,
    attempt: u32,
    max_retries: u32,
) -> bool {
    status == 400
        && body.contains("max_tokens")
        && (body.contains("unsupported_parameter") || body.contains("max_completion_tokens"))
        && oai_request.max_tokens.is_some()
        && attempt < max_retries
}

fn should_cap_max_tokens(status: u16, body: &str, attempt: u32, max_retries: u32) -> bool {
    status == 400 && body.contains("max_tokens") && attempt < max_retries
}

fn cap_oai_max_tokens(body: &str, oai_request: &mut OaiRequest, streaming: bool) {
    let current = oai_request
        .max_tokens
        .or(oai_request.max_completion_tokens)
        .unwrap_or(4096);
    let cap = extract_max_tokens_limit(body).unwrap_or(current / 2);
    if streaming {
        warn!(old = current, new = cap, "Auto-capping max_tokens (stream)");
    } else {
        warn!(
            old = current,
            new = cap,
            "Auto-capping max_tokens to model limit"
        );
    }
    if oai_request.max_completion_tokens.is_some() {
        oai_request.max_completion_tokens = Some(cap);
    } else {
        oai_request.max_tokens = Some(cap);
    }
}

fn should_retry_without_tools(
    status: u16,
    body: &str,
    oai_request: &OaiRequest,
    attempt: u32,
    max_retries: u32,
) -> bool {
    if oai_request.tools.is_empty() || attempt >= max_retries {
        return false;
    }
    let body_lower = body.to_lowercase();
    status == 500
        || body_lower.contains("internal error")
        || (status == 400
            && (body_lower.contains("does not support tools")
                || body_lower.contains("tool") && body_lower.contains("not supported")))
}

fn should_strip_stream_options(
    status: u16,
    body: &str,
    oai_request: &OaiRequest,
    attempt: u32,
    max_retries: u32,
) -> bool {
    status == 400
        && oai_request.stream_options.is_some()
        && attempt < max_retries
        && (body.contains("stream_options")
            || body.contains("stream_option")
            || body.contains("Unrecognized request argument"))
}

#[derive(Default)]
struct OaiStreamToolAccum {
    id: String,
    name: String,
    arguments: String,
}

struct OaiStreamAccumulator {
    buffer: String,
    text_content: String,
    reasoning_content: String,
    think_filter: StreamingThinkFilter,
    tool_accum: Vec<OaiStreamToolAccum>,
    finish_reason: Option<String>,
    usage: TokenUsage,
    chunk_count: u32,
    sse_line_count: u32,
}

impl OaiStreamAccumulator {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            text_content: String::new(),
            reasoning_content: String::new(),
            think_filter: StreamingThinkFilter::new(),
            tool_accum: Vec::new(),
            finish_reason: None,
            usage: TokenUsage::default(),
            chunk_count: 0,
            sse_line_count: 0,
        }
    }

    async fn consume_response(
        &mut self,
        resp: reqwest::Response,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError> {
        let mut byte_stream = resp.bytes_stream();
        while let Some(chunk_result) = byte_stream.next().await {
            let chunk = chunk_result.map_err(|e| LlmError::Http(e.to_string()))?;
            self.chunk_count += 1;
            self.consume_chunk(&chunk, tx).await;
        }
        Ok(())
    }

    async fn consume_chunk(&mut self, chunk: &[u8], tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        while let Some(line) = self.next_sse_line() {
            self.process_sse_line(line, tx).await;
        }
    }

    fn next_sse_line(&mut self) -> Option<String> {
        let pos = self.buffer.find('\n')?;
        let line = self.buffer[..pos].trim_end().to_string();
        self.buffer = self.buffer[pos + 1..].to_string();
        Some(line)
    }

    async fn process_sse_line(
        &mut self,
        line: String,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        if line.is_empty() || line.starts_with(':') {
            return;
        }

        self.sse_line_count += 1;
        let data = match line.strip_prefix("data:") {
            Some(d) => d.trim_start(),
            None => return,
        };
        if data == "[DONE]" {
            return;
        }

        let json: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => return,
        };
        self.process_sse_json(&json, tx).await;
    }

    async fn process_sse_json(
        &mut self,
        json: &serde_json::Value,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        self.apply_stream_usage(json);
        let choices = match json["choices"].as_array() {
            Some(c) => c,
            None => return,
        };

        for choice in choices {
            let delta = &choice["delta"];
            self.process_text_delta(delta, tx).await;
            self.process_reasoning_delta(delta, tx).await;
            self.process_tool_call_delta(delta, tx).await;

            if let Some(fr) = choice["finish_reason"].as_str() {
                self.finish_reason = Some(fr.to_string());
            }
        }
    }

    fn apply_stream_usage(&mut self, json: &serde_json::Value) {
        if let Some(u) = json.get("usage") {
            if let Some(pt) = u["prompt_tokens"].as_u64() {
                self.usage.input_tokens = pt;
            }
            if let Some(ct) = u["completion_tokens"].as_u64() {
                self.usage.output_tokens = ct;
            }
        }
    }

    async fn process_text_delta(
        &mut self,
        delta: &serde_json::Value,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        if let Some(text) = delta["content"].as_str() {
            if text.is_empty() {
                return;
            }
            self.text_content.push_str(text);
            for action in self.think_filter.process(text) {
                send_think_filter_action(tx, action).await;
            }
        }
    }

    async fn process_reasoning_delta(
        &mut self,
        delta: &serde_json::Value,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        if let Some(reasoning) = delta["reasoning_content"].as_str() {
            if !reasoning.is_empty() {
                self.reasoning_content.push_str(reasoning);
                let _ = tx
                    .send(StreamEvent::ThinkingDelta {
                        text: reasoning.to_string(),
                    })
                    .await;
            }
        }
    }

    async fn process_tool_call_delta(
        &mut self,
        delta: &serde_json::Value,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        if let Some(calls) = delta["tool_calls"].as_array() {
            for call in calls {
                let idx = call["index"].as_u64().unwrap_or(0) as usize;
                self.ensure_tool_slot(idx);
                if let Some(id) = call["id"].as_str() {
                    self.tool_accum[idx].id = id.to_string();
                }
                if let Some(func) = call.get("function") {
                    self.process_tool_function_delta(idx, func, tx).await;
                }
            }
        }
    }

    fn ensure_tool_slot(&mut self, idx: usize) {
        while self.tool_accum.len() <= idx {
            self.tool_accum.push(OaiStreamToolAccum::default());
        }
    }

    async fn process_tool_function_delta(
        &mut self,
        idx: usize,
        func: &serde_json::Value,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        if let Some(name) = func["name"].as_str() {
            self.tool_accum[idx].name = name.to_string();
            let _ = tx
                .send(StreamEvent::ToolUseStart {
                    id: self.tool_accum[idx].id.clone(),
                    name: name.to_string(),
                })
                .await;
        }
        if let Some(args) = func["arguments"].as_str() {
            self.tool_accum[idx].arguments.push_str(args);
            if !args.is_empty() {
                let _ = tx
                    .send(StreamEvent::ToolInputDelta {
                        text: args.to_string(),
                    })
                    .await;
            }
        }
    }

    async fn flush_think_filter(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        for action in self.think_filter.flush() {
            send_think_filter_action(tx, action).await;
        }
    }

    fn log_summary(&self) {
        let is_empty_stream = self.text_content.is_empty()
            && self.reasoning_content.is_empty()
            && self.tool_accum.is_empty()
            && self.usage.input_tokens == 0
            && self.usage.output_tokens == 0;
        if is_empty_stream {
            warn!(
                chunks = self.chunk_count,
                sse_lines = self.sse_line_count,
                finish = ?self.finish_reason,
                buffer_remaining = self.buffer.len(),
                "SSE stream returned empty: 0 content, 0 tokens — likely a silently failed request"
            );
        } else {
            debug!(
                chunks = self.chunk_count,
                sse_lines = self.sse_line_count,
                text_len = self.text_content.len(),
                reasoning_len = self.reasoning_content.len(),
                tool_count = self.tool_accum.len(),
                finish = ?self.finish_reason,
                input_tokens = self.usage.input_tokens,
                output_tokens = self.usage.output_tokens,
                buffer_remaining = self.buffer.len(),
                "SSE stream completed"
            );
        }
    }

    async fn into_response(
        mut self,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> CompletionResponse {
        let mut content = Vec::new();
        let mut tool_calls = Vec::new();
        push_stream_reasoning_and_text(&mut content, &self.reasoning_content, &self.text_content);
        synthesize_thinking_only_response(
            &mut content,
            self.tool_accum.is_empty(),
            "stream response",
        );
        self.push_stream_tool_calls(&mut content, &mut tool_calls, tx)
            .await;

        let stop_reason =
            stop_reason_from_finish(self.finish_reason.as_deref(), !tool_calls.is_empty());
        ensure_nonzero_output_usage(&content, &mut self.usage, "Stream");
        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason,
                usage: self.usage,
            })
            .await;

        CompletionResponse {
            content,
            stop_reason,
            tool_calls,
            usage: self.usage,
        }
    }

    async fn push_stream_tool_calls(
        &self,
        content: &mut Vec<ContentBlock>,
        tool_calls: &mut Vec<ToolCall>,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        for tool in &self.tool_accum {
            let input = crate::drivers::util::parse_tool_input(&tool.arguments);
            let normalized_id = normalize_tool_call_id(&tool.id);
            content.push(ContentBlock::ToolUse {
                id: normalized_id.clone(),
                name: tool.name.clone(),
                input: input.clone(),
                provider_metadata: None,
            });
            tool_calls.push(ToolCall {
                id: normalized_id.clone(),
                name: tool.name.clone(),
                input: input.clone(),
            });

            let _ = tx
                .send(StreamEvent::ToolUseEnd {
                    id: normalized_id,
                    name: tool.name.clone(),
                    input,
                })
                .await;
        }
    }
}

async fn completion_response_from_oai_stream(
    resp: reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) -> Result<CompletionResponse, LlmError> {
    let mut stream = OaiStreamAccumulator::new();
    stream.consume_response(resp, tx).await?;
    stream.flush_think_filter(tx).await;
    stream.log_summary();
    Ok(stream.into_response(tx).await)
}

async fn send_think_filter_action(
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    action: FilterAction,
) {
    match action {
        FilterAction::EmitText(text) => {
            let _ = tx.send(StreamEvent::TextDelta { text }).await;
        }
        FilterAction::EmitThinking(text) => {
            let _ = tx.send(StreamEvent::ThinkingDelta { text }).await;
        }
    }
}

fn push_stream_reasoning_and_text(
    content: &mut Vec<ContentBlock>,
    reasoning_content: &str,
    text_content: &str,
) {
    if !reasoning_content.is_empty() {
        content.push(ContentBlock::Thinking {
            thinking: reasoning_content.to_string(),
            provider_metadata: None,
        });
    }
    if !text_content.is_empty() {
        let (cleaned, thinking) = extract_think_tags(text_content);
        if let Some(think_text) = thinking {
            if reasoning_content.is_empty() {
                content.push(ContentBlock::Thinking {
                    thinking: think_text,
                    provider_metadata: None,
                });
            }
        }
        if !cleaned.is_empty() {
            content.push(ContentBlock::Text {
                text: cleaned,
                provider_metadata: None,
            });
        }
    }
}

#[async_trait]
impl LlmDriver for OpenAIDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut oai_request = self.build_oai_request(&request, OaiRequestMode::Complete);

        let max_retries = 3;
        for attempt in 0..=max_retries {
            let resp = self
                .send_chat_request(&request.model, &oai_request, attempt, false)
                .await?;

            let status = resp.status().as_u16();
            if status == 429 {
                if attempt < max_retries {
                    let retry_ms = (attempt + 1) as u64 * 2000;
                    warn!(status, retry_ms, "Rate limited, retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                    continue;
                }
                return Err(LlmError::RateLimited {
                    retry_after_ms: 5000,
                });
            }

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                match handle_oai_error_response(
                    status,
                    body,
                    attempt,
                    max_retries,
                    &mut oai_request,
                    false,
                )
                .await?
                {
                    OaiErrorOutcome::Retry => continue,
                    OaiErrorOutcome::Recovered(response) => return Ok(response),
                }
            }

            let body = resp
                .text()
                .await
                .map_err(|e| LlmError::Http(e.to_string()))?;
            let oai_response: OaiResponse =
                serde_json::from_str(&body).map_err(|e| LlmError::Parse(e.to_string()))?;

            return completion_response_from_oai_response(oai_response);
        }

        Err(LlmError::Api {
            status: 0,
            message: "Max retries exceeded".to_string(),
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut oai_request = self.build_oai_request(&request, OaiRequestMode::Stream);

        // Retry loop for the initial HTTP request
        let max_retries = 3;
        for attempt in 0..=max_retries {
            let resp = self
                .send_chat_request(&request.model, &oai_request, attempt, true)
                .await?;

            let status = resp.status().as_u16();
            if status == 429 {
                if attempt < max_retries {
                    let retry_ms = (attempt + 1) as u64 * 2000;
                    warn!(status, retry_ms, "Rate limited (stream), retrying");
                    tokio::time::sleep(std::time::Duration::from_millis(retry_ms)).await;
                    continue;
                }
                return Err(LlmError::RateLimited {
                    retry_after_ms: 5000,
                });
            }

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                match handle_oai_error_response(
                    status,
                    body,
                    attempt,
                    max_retries,
                    &mut oai_request,
                    true,
                )
                .await?
                {
                    OaiErrorOutcome::Retry => continue,
                    OaiErrorOutcome::Recovered(response) => return Ok(response),
                }
            }

            return completion_response_from_oai_stream(resp, &tx).await;
        }

        Err(LlmError::Api {
            status: 0,
            message: "Max retries exceeded".to_string(),
        })
    }
}

/// Extract `<think>...</think>` blocks from content text.
///
/// Some local LLMs (Qwen3, DeepSeek-R1) embed their reasoning directly in the
/// content field wrapped in `<think>` tags. This function separates the thinking
/// from the actual response text.
///
/// Returns `(cleaned_text, Option<thinking_text>)`.
fn extract_think_tags(text: &str) -> (String, Option<String>) {
    let mut thinking_parts = Vec::new();
    let mut cleaned = text.to_string();

    // Extract all <think>...</think> blocks (greedy within each block)
    while let Some(start) = cleaned.find("<think>") {
        if let Some(end) = cleaned.find("</think>") {
            let think_start = start + "<think>".len();
            if think_start <= end {
                let thought = cleaned[think_start..end].trim().to_string();
                if !thought.is_empty() {
                    thinking_parts.push(thought);
                }
                // Remove the entire <think>...</think> block
                cleaned = format!(
                    "{}{}",
                    &cleaned[..start],
                    &cleaned[end + "</think>".len()..]
                );
            } else {
                break;
            }
        } else {
            // Unclosed <think> tag — treat everything after as thinking
            let thought = cleaned[start + "<think>".len()..].trim().to_string();
            if !thought.is_empty() {
                thinking_parts.push(thought);
            }
            cleaned = cleaned[..start].to_string();
            break;
        }
    }

    let cleaned = cleaned.trim().to_string();
    if thinking_parts.is_empty() {
        (cleaned, None)
    } else {
        (cleaned, Some(thinking_parts.join("\n\n")))
    }
}

/// Extract a usable summary from thinking-only output.
///
/// When a local model returns only thinking/reasoning with no actual response text,
/// we extract the last meaningful paragraph as a synthesized response rather than
/// showing "empty response" to the user.
fn extract_thinking_summary(thinking: &str) -> String {
    let trimmed = thinking.trim();
    if trimmed.is_empty() {
        return "[The model produced reasoning but no final answer. Try rephrasing your question.]"
            .to_string();
    }

    // Take the last non-empty paragraph (models usually conclude with their answer)
    let paragraphs: Vec<&str> = trimmed
        .split("\n\n")
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();

    if let Some(last) = paragraphs.last() {
        // If the last paragraph is reasonably short, use it directly
        if last.len() <= 2000 {
            last.to_string()
        } else {
            // Take the last 2000 chars
            last[last.len() - 2000..].to_string()
        }
    } else {
        "[The model produced reasoning but no final answer. Try rephrasing your question.]"
            .to_string()
    }
}

/// Parse Groq's `tool_use_failed` error and extract the tool call from `failed_generation`.
/// Extract the max_tokens limit from an API error message.
/// Looks for patterns like: `must be less than or equal to \`8192\``
fn extract_max_tokens_limit(body: &str) -> Option<u32> {
    // Pattern: "must be <= `N`" or "must be less than or equal to `N`"
    let patterns = [
        "less than or equal to `",
        "must be <= `",
        "maximum value for `max_tokens` is `",
    ];
    for pat in &patterns {
        if let Some(idx) = body.find(pat) {
            let after = &body[idx + pat.len()..];
            let end = after
                .find('`')
                .or_else(|| after.find('"'))
                .unwrap_or(after.len());
            if let Ok(n) = after[..end].trim().parse::<u32>() {
                return Some(n);
            }
        }
    }
    None
}

///
/// Some models (e.g. Llama 3.3) generate tool calls as XML: `<function=NAME ARGS></function>`
/// instead of the proper JSON format. Groq rejects these with `tool_use_failed` but includes
/// the raw generation. We parse it and construct a proper CompletionResponse.
fn parse_groq_failed_tool_call(body: &str) -> Option<CompletionResponse> {
    let failed = groq_failed_generation(body)?;
    let tool_calls = parse_groq_failed_tool_calls(&failed)?;

    if tool_calls.is_empty() {
        return recover_groq_plain_text_generation(&failed);
    }

    Some(CompletionResponse {
        content: vec![],
        tool_calls,
        stop_reason: StopReason::ToolUse,
        usage: TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            ..Default::default()
        },
    })
}

fn groq_failed_generation(body: &str) -> Option<String> {
    let json_body: serde_json::Value = serde_json::from_str(body).ok()?;
    json_body
        .pointer("/error/failed_generation")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn parse_groq_failed_tool_calls(failed: &str) -> Option<Vec<ToolCall>> {
    let mut tool_calls = Vec::new();
    let mut remaining = failed;

    while let Some(start) = remaining.find("<function=") {
        remaining = &remaining[start + 10..]; // skip "<function="
        let end = remaining.find("</function>")?;
        let mut call_content = &remaining[..end];
        remaining = &remaining[end + 11..]; // skip "</function>"

        call_content = call_content.strip_suffix('>').unwrap_or(call_content);
        let (name, args) = split_groq_function_call(call_content);
        let args_value: serde_json::Value =
            serde_json::from_str(args).unwrap_or(serde_json::json!({}));

        tool_calls.push(ToolCall {
            id: format!("groq_recovered_{}", tool_calls.len()),
            name: name.to_string(),
            input: args_value,
        });
    }

    Some(tool_calls)
}

fn split_groq_function_call(call_content: &str) -> (&str, &str) {
    if let Some(brace_pos) = call_content.find('{') {
        let name = call_content[..brace_pos].trim();
        let args = &call_content[brace_pos..];
        (name, args)
    } else {
        (call_content.trim(), "{}")
    }
}

fn recover_groq_plain_text_generation(failed: &str) -> Option<CompletionResponse> {
    if failed.trim().is_empty() {
        return None;
    }
    warn!("Recovering plain text from Groq failed_generation (no tool calls)");
    Some(CompletionResponse {
        content: vec![ContentBlock::Text {
            text: failed.to_string(),
            provider_metadata: None,
        }],
        tool_calls: vec![],
        stop_reason: StopReason::EndTurn,
        usage: TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            ..Default::default()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::Message;
    use captain_types::tool::ToolDefinition;

    fn test_completion_request(messages: Vec<Message>) -> CompletionRequest {
        CompletionRequest {
            model: "gpt-4o".to_string(),
            messages,
            tools: Vec::new(),
            max_tokens: 321,
            temperature: 0.3,
            system: None,
            thinking: None,
            tool_choice: None,
            cache_hints: Default::default(),
        }
    }

    #[test]
    fn test_openai_driver_creation() {
        let driver = OpenAIDriver::new("test-key".to_string(), "http://localhost".to_string());
        assert_eq!(driver.api_key.as_str(), "test-key");
    }

    #[test]
    fn build_oai_request_complete_maps_system_user_parts_and_tools() {
        let driver = OpenAIDriver::new("test-key".to_string(), "http://localhost".to_string());
        let mut request = test_completion_request(vec![
            Message::system("message system should be skipped"),
            Message::user_with_blocks(vec![
                ContentBlock::Text {
                    text: "look".to_string(),
                    provider_metadata: None,
                },
                ContentBlock::Image {
                    media_type: "image/png".to_string(),
                    data: "abcd".to_string(),
                },
            ]),
        ]);
        request.system = Some("top system".to_string());
        request.tools = vec![ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "query": { "type": "string" } }
            }),
        }];

        let built = driver.build_oai_request(&request, OaiRequestMode::Complete);

        assert!(!built.stream);
        assert!(built.stream_options.is_none());
        assert_eq!(built.max_tokens, Some(321));
        assert_eq!(built.max_completion_tokens, None);
        assert_eq!(built.temperature, Some(0.3));
        assert_eq!(built.tool_choice, Some(serde_json::json!("auto")));
        assert_eq!(built.tools.len(), 1);
        assert_eq!(built.messages.len(), 2);
        assert_eq!(built.messages[0].role, "system");
        match built.messages[0].content.as_ref().unwrap() {
            OaiMessageContent::Text(text) => assert_eq!(text, "top system"),
            _ => panic!("system message should be text"),
        }
        match built.messages[1].content.as_ref().unwrap() {
            OaiMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                match &parts[0] {
                    OaiContentPart::Text { text } => assert_eq!(text, "look"),
                    _ => panic!("first part should be text"),
                }
                match &parts[1] {
                    OaiContentPart::ImageUrl { image_url } => {
                        assert_eq!(image_url.url, "data:image/png;base64,abcd");
                    }
                    _ => panic!("second part should be image"),
                }
            }
            _ => panic!("user message should use multimodal parts"),
        }
    }

    #[test]
    fn build_oai_request_stream_keeps_tool_results_and_multimodal_user_content() {
        let driver = OpenAIDriver::new("test-key".to_string(), "http://localhost".to_string());
        let request = test_completion_request(vec![Message::user_with_blocks(vec![
            ContentBlock::Text {
                text: "inspect the screenshot".to_string(),
                provider_metadata: None,
            },
            ContentBlock::ToolResult {
                tool_use_id: "tool_call_with_symbols_123".to_string(),
                tool_name: "web_search".to_string(),
                content: String::new(),
                is_error: false,
            },
            ContentBlock::Image {
                media_type: "image/png".to_string(),
                data: "cG5n".to_string(),
            },
        ])]);

        let built = driver.build_oai_request(&request, OaiRequestMode::Stream);

        assert!(built.stream);
        assert_eq!(
            built.stream_options,
            Some(serde_json::json!({"include_usage": true}))
        );
        assert_eq!(built.messages.len(), 2);
        assert_eq!(built.messages[0].role, "tool");
        assert_eq!(
            built.messages[0].tool_call_id.as_deref(),
            Some(normalize_tool_call_id("tool_call_with_symbols_123").as_str())
        );
        match built.messages[0].content.as_ref().unwrap() {
            OaiMessageContent::Text(text) => assert_eq!(text, "(empty)"),
            _ => panic!("tool result message should be text"),
        }
        assert_eq!(built.messages[1].role, "user");
        match built.messages[1].content.as_ref().unwrap() {
            OaiMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                assert!(matches!(
                    &parts[0],
                    OaiContentPart::Text { text } if text == "inspect the screenshot"
                ));
                assert!(matches!(
                    &parts[1],
                    OaiContentPart::ImageUrl { image_url }
                        if image_url.url == "data:image/png;base64,cG5n"
                ));
            }
            _ => panic!("streaming user message should keep multimodal parts"),
        }
    }

    #[test]
    fn completion_response_parser_maps_text_tool_calls_and_usage() {
        let response = OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    content: Some("<think>hidden</think>Hello".to_string()),
                    tool_calls: Some(vec![OaiToolCall {
                        id: "call_with_symbols_123".to_string(),
                        call_type: "function".to_string(),
                        function: OaiFunction {
                            name: "web_search".to_string(),
                            arguments: serde_json::json!({"query":"captain"}).to_string(),
                        },
                    }]),
                    reasoning_content: None,
                },
                finish_reason: Some("tool_calls".to_string()),
            }],
            usage: Some(OaiUsage {
                prompt_tokens: 12,
                completion_tokens: 3,
                prompt_tokens_details: Some(OaiPromptTokensDetails { cached_tokens: 7 }),
            }),
        };

        let parsed = completion_response_from_oai_response(response).unwrap();

        assert_eq!(parsed.stop_reason, StopReason::ToolUse);
        assert_eq!(parsed.usage.input_tokens, 12);
        assert_eq!(parsed.usage.cached_input_tokens, 7);
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "web_search");
        assert!(parsed.content.iter().any(
            |block| matches!(block, ContentBlock::Thinking { thinking, .. } if thinking == "hidden")
        ));
        assert!(parsed
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Text { text, .. } if text == "Hello")));
    }

    #[test]
    fn completion_response_parser_synthesizes_thinking_only_text() {
        let response = OaiResponse {
            choices: vec![OaiChoice {
                message: OaiResponseMessage {
                    content: None,
                    tool_calls: None,
                    reasoning_content: Some("First thought.\n\nFinal answer.".to_string()),
                },
                finish_reason: Some("stop".to_string()),
            }],
            usage: None,
        };

        let parsed = completion_response_from_oai_response(response).unwrap();

        assert_eq!(parsed.stop_reason, StopReason::EndTurn);
        assert_eq!(parsed.usage.output_tokens, 1);
        assert!(parsed.content.iter().any(
            |block| matches!(block, ContentBlock::Text { text, .. } if text == "Final answer.")
        ));
    }

    #[test]
    fn complete_error_retry_predicates_preserve_provider_fallbacks() {
        let driver = OpenAIDriver::new("test-key".to_string(), "http://localhost".to_string());
        let mut request = test_completion_request(vec![]);
        request.model = "gpt-4o".to_string();
        request.tools = vec![ToolDefinition {
            name: "shell_exec".to_string(),
            description: "Run shell".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
        }];
        let built = driver.build_oai_request(&request, OaiRequestMode::Complete);

        assert!(should_strip_temperature(
            400,
            "unsupported_parameter: temperature",
            &built,
            0,
            3
        ));
        assert!(should_switch_to_completion_tokens(
            400,
            "unsupported_parameter: use max_completion_tokens instead of max_tokens",
            &built,
            0,
            3
        ));
        assert!(should_retry_without_tools(
            500,
            "internal error",
            &built,
            0,
            3
        ));
        let stream_built = driver.build_oai_request(&request, OaiRequestMode::Stream);
        assert!(should_strip_stream_options(
            400,
            "Unrecognized request argument: stream_options",
            &stream_built,
            0,
            3
        ));
    }

    #[tokio::test]
    async fn stream_accumulator_maps_text_tool_usage_and_events() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mut stream = OaiStreamAccumulator::new();
        let arguments = serde_json::json!({"query":"captain"}).to_string();
        let chunk = serde_json::json!({
            "choices": [{
                "delta": {
                    "content": "<think>hidden</think>Hello",
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_with_symbols_123",
                        "function": {
                            "name": "web_search",
                            "arguments": arguments
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 2
            }
        });

        stream.process_sse_json(&chunk, &tx).await;
        stream.flush_think_filter(&tx).await;
        let parsed = stream.into_response(&tx).await;

        assert_eq!(parsed.stop_reason, StopReason::ToolUse);
        assert_eq!(parsed.usage.input_tokens, 5);
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "web_search");
        assert!(parsed
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::Text { text, .. } if text == "Hello")));

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        assert!(events.iter().any(
            |event| matches!(event, StreamEvent::ToolUseEnd { name, .. } if name == "web_search")
        ));
        assert!(events
            .iter()
            .any(|event| matches!(event, StreamEvent::ContentComplete { stop_reason, .. } if *stop_reason == StopReason::ToolUse)));
    }

    #[test]
    fn test_normalize_tool_call_id_short_alphanum_passthrough() {
        assert_eq!(normalize_tool_call_id("abc123"), "abc123");
        assert_eq!(normalize_tool_call_id("xyzABC789"), "xyzABC789");
    }

    #[test]
    fn test_normalize_tool_call_id_long_gets_hashed() {
        let long = "call_c135311863474e98a02c3e04fc7ee103";
        let out = normalize_tool_call_id(long);
        assert_eq!(out.len(), 9);
        assert!(out.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_normalize_tool_call_id_deterministic() {
        let raw = "call_c135311863474e98a02c3e04fc7ee103";
        assert_eq!(normalize_tool_call_id(raw), normalize_tool_call_id(raw));
    }

    #[test]
    fn test_normalize_tool_call_id_with_underscore_gets_hashed() {
        let out = normalize_tool_call_id("call_1234");
        assert_eq!(out.len(), 9);
        assert!(out.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_parse_groq_failed_tool_call() {
        let body = r#"{"error":{"message":"Failed to call a function.","type":"invalid_request_error","code":"tool_use_failed","failed_generation":"<function=web_fetch{\"url\": \"https://example.com\"}></function>\n"}}"#;
        let result = parse_groq_failed_tool_call(body);
        assert!(result.is_some());
        let resp = result.unwrap();
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "web_fetch");
        assert!(resp.tool_calls[0]
            .input
            .to_string()
            .contains("https://example.com"));
    }

    #[test]
    fn test_parse_groq_failed_tool_call_with_space() {
        let body = r#"{"error":{"message":"Failed","type":"invalid_request_error","code":"tool_use_failed","failed_generation":"<function=shell_exec {\"command\": \"ls -la\"}></function>"}}"#;
        let result = parse_groq_failed_tool_call(body);
        assert!(result.is_some());
        let resp = result.unwrap();
        assert_eq!(resp.tool_calls[0].name, "shell_exec");
    }

    // ----- rejects_temperature tests -----

    #[test]
    fn test_rejects_temperature_o1_models() {
        assert!(rejects_temperature("o1"));
        assert!(rejects_temperature("o1-mini"));
        assert!(rejects_temperature("o1-mini-2024-09-12"));
        assert!(rejects_temperature("o1-preview"));
        assert!(rejects_temperature("o1-preview-2024-09-12"));
    }

    #[test]
    fn test_rejects_temperature_o3_models() {
        assert!(rejects_temperature("o3"));
        assert!(rejects_temperature("o3-mini"));
        assert!(rejects_temperature("o3-mini-2025-01-31"));
        assert!(rejects_temperature("o3-pro"));
    }

    #[test]
    fn test_rejects_temperature_o4_models() {
        assert!(rejects_temperature("o4-mini"));
        assert!(rejects_temperature("o4-mini-2025-04-16"));
    }

    #[test]
    fn test_rejects_temperature_gpt5_mini() {
        assert!(rejects_temperature("gpt-5-mini"));
        assert!(rejects_temperature("gpt-5-mini-2025-08-07"));
        assert!(rejects_temperature("gpt5-mini"));
        assert!(rejects_temperature("GPT-5-MINI-2025-08-07"));
    }

    #[test]
    fn test_rejects_temperature_reasoning_suffix() {
        assert!(rejects_temperature("some-model-reasoning"));
        assert!(rejects_temperature("deepseek-r1-reasoning"));
    }

    #[test]
    fn test_does_not_reject_temperature_normal_models() {
        assert!(!rejects_temperature("gpt-4o"));
        assert!(!rejects_temperature("gpt-4o-mini"));
        assert!(!rejects_temperature("gpt-5"));
        assert!(!rejects_temperature("gpt-5-2025-06-01"));
        assert!(!rejects_temperature("claude-sonnet-4-20250514"));
        assert!(!rejects_temperature("llama-3.3-70b-versatile"));
        assert!(!rejects_temperature("deepseek-chat"));
    }

    // ----- uses_completion_tokens tests -----

    #[test]
    fn test_uses_completion_tokens_gpt5() {
        assert!(uses_completion_tokens("gpt-5"));
        assert!(uses_completion_tokens("gpt-5-mini"));
        assert!(uses_completion_tokens("gpt-5-mini-2025-08-07"));
        assert!(uses_completion_tokens("gpt5-mini"));
    }

    #[test]
    fn test_uses_completion_tokens_o_series() {
        assert!(uses_completion_tokens("o1"));
        assert!(uses_completion_tokens("o1-mini"));
        assert!(uses_completion_tokens("o3"));
        assert!(uses_completion_tokens("o3-mini"));
        assert!(uses_completion_tokens("o3-pro"));
        assert!(uses_completion_tokens("o4-mini"));
    }

    #[test]
    fn test_does_not_use_completion_tokens_normal_models() {
        assert!(!uses_completion_tokens("gpt-4o"));
        assert!(!uses_completion_tokens("gpt-4o-mini"));
        assert!(!uses_completion_tokens("llama-3.3-70b"));
    }

    // ----- extract_max_tokens_limit tests -----

    #[test]
    fn test_extract_max_tokens_limit() {
        let body = r#"max_tokens must be less than or equal to `8192`"#;
        assert_eq!(extract_max_tokens_limit(body), Some(8192));
    }

    #[test]
    fn test_extract_max_tokens_limit_no_match() {
        assert_eq!(extract_max_tokens_limit("some random error"), None);
    }

    // ----- extract_think_tags tests -----

    #[test]
    fn test_extract_think_tags_no_tags() {
        let (cleaned, thinking) = extract_think_tags("Hello world");
        assert_eq!(cleaned, "Hello world");
        assert!(thinking.is_none());
    }

    #[test]
    fn test_extract_think_tags_with_thinking() {
        let input = "<think>Let me reason about this...</think>The answer is 42.";
        let (cleaned, thinking) = extract_think_tags(input);
        assert_eq!(cleaned, "The answer is 42.");
        assert_eq!(thinking.unwrap(), "Let me reason about this...");
    }

    #[test]
    fn test_extract_think_tags_only_thinking() {
        let input = "<think>I need to think about this carefully.\n\nThe user wants to know about Rust.</think>";
        let (cleaned, thinking) = extract_think_tags(input);
        assert_eq!(cleaned, "");
        assert!(thinking.is_some());
        assert!(thinking.unwrap().contains("think about this carefully"));
    }

    #[test]
    fn test_extract_think_tags_multiple_blocks() {
        let input =
            "<think>First thought</think>Middle text<think>Second thought</think>Final text";
        let (cleaned, thinking) = extract_think_tags(input);
        assert_eq!(cleaned, "Middle textFinal text");
        let t = thinking.unwrap();
        assert!(t.contains("First thought"));
        assert!(t.contains("Second thought"));
    }

    #[test]
    fn test_extract_think_tags_unclosed() {
        let input = "Some text<think>unclosed thinking content";
        let (cleaned, thinking) = extract_think_tags(input);
        assert_eq!(cleaned, "Some text");
        assert_eq!(thinking.unwrap(), "unclosed thinking content");
    }

    // ----- extract_thinking_summary tests -----

    #[test]
    fn test_extract_thinking_summary_empty() {
        let summary = extract_thinking_summary("");
        assert!(summary.contains("no final answer"));
    }

    #[test]
    fn test_extract_thinking_summary_single_paragraph() {
        let summary = extract_thinking_summary("The answer is 42.");
        assert_eq!(summary, "The answer is 42.");
    }

    #[test]
    fn test_extract_thinking_summary_multiple_paragraphs() {
        let input = "First I need to consider X.\n\nThen I should check Y.\n\nThe answer is 42.";
        let summary = extract_thinking_summary(input);
        assert_eq!(summary, "The answer is 42.");
    }

    // ----- reasoning_content deserialization test -----

    #[test]
    fn test_oai_response_message_with_reasoning_content() {
        let json =
            r#"{"content": null, "reasoning_content": "Let me think...", "tool_calls": null}"#;
        let msg: OaiResponseMessage = serde_json::from_str(json).unwrap();
        assert!(msg.content.is_none());
        assert_eq!(msg.reasoning_content.as_deref(), Some("Let me think..."));
    }

    #[test]
    fn test_oai_response_message_without_reasoning_content() {
        let json = r#"{"content": "Hello", "tool_calls": null}"#;
        let msg: OaiResponseMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.content.as_deref(), Some("Hello"));
        assert!(msg.reasoning_content.is_none());
    }

    #[test]
    fn test_oai_response_message_null_content_null_reasoning() {
        let json = r#"{"content": null, "tool_calls": null}"#;
        let msg: OaiResponseMessage = serde_json::from_str(json).unwrap();
        assert!(msg.content.is_none());
        assert!(msg.reasoning_content.is_none());
    }

    // ── Azure OpenAI tests ──────────────────────────────────────────

    #[test]
    fn test_azure_driver_creation() {
        let driver = OpenAIDriver::new_azure(
            "test-key".to_string(),
            "https://myresource.openai.azure.com/openai/deployments".to_string(),
        );
        assert!(driver.azure_mode);
    }

    #[test]
    fn test_standard_driver_not_azure() {
        let driver = OpenAIDriver::new(
            "test-key".to_string(),
            "https://api.openai.com/v1".to_string(),
        );
        assert!(!driver.azure_mode);
    }

    #[test]
    fn test_azure_chat_url() {
        let driver = OpenAIDriver::new_azure(
            "test-key".to_string(),
            "https://myresource.openai.azure.com/openai/deployments".to_string(),
        );
        let url = driver.chat_url("my-gpt4o-deployment");
        assert_eq!(
            url,
            "https://myresource.openai.azure.com/openai/deployments/my-gpt4o-deployment/chat/completions?api-version=2024-10-21"
        );
    }

    #[test]
    fn test_azure_chat_url_trailing_slash() {
        let driver = OpenAIDriver::new_azure(
            "test-key".to_string(),
            "https://myresource.openai.azure.com/openai/deployments/".to_string(),
        );
        let url = driver.chat_url("gpt-4o");
        assert_eq!(
            url,
            "https://myresource.openai.azure.com/openai/deployments/gpt-4o/chat/completions?api-version=2024-10-21"
        );
    }

    #[test]
    fn test_standard_chat_url() {
        let driver = OpenAIDriver::new(
            "test-key".to_string(),
            "https://api.openai.com/v1".to_string(),
        );
        let url = driver.chat_url("gpt-4o");
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }
}

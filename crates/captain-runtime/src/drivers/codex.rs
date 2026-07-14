//! Phase O.1 — Driver dédié pour ChatGPT subscription / OpenAI Codex.
//!
//! Endpoint cible : `https://chatgpt.com/backend-api/codex/responses`
//! (Responses API, **pas** `/chat/completions`).
//!
//! Diffs majeurs vs OpenAI standard :
//!   - Payload `{model, instructions, input[], tools[], store:false, stream}`
//!     (pas `messages[]`)
//!   - Headers obligatoires `originator: codex_cli_rs` et User-Agent
//!     spécifique sinon Cloudflare bloque (403/429)
//!   - JWT contient `chatgpt_account_id` qu'il faut renvoyer en header
//!   - SSE events `response.output_text.delta` / `.completed` /
//!     `.function_call.*` (pas `data: {choices[]…}`)
//!
//! Codex provider integration helpers.
//! et du Codex CLI officiel (codex_cli_rs).

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use captain_types::message::{ContentBlock, MessageContent, Role, StopReason, TokenUsage};
use captain_types::tool::ToolCall;
use futures::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::Duration;
use tracing::{debug, warn};

/// Stable User-Agent that Cloudflare whitelists for the chatgpt.com
/// codex backend. **Do not change** — using the wrong UA causes 403.
const CODEX_UA: &str = "codex_cli_rs/0.0.0 (Captain Agent)";

/// Required header — same constraint as the User-Agent.
const CODEX_ORIGINATOR: &str = "codex_cli_rs";

const CODEX_REASONING_METADATA_KEY: &str = "codex_reasoning";

pub struct CodexDriver {
    access_token: String,
    base_url: String,
    /// Extracted from the JWT payload at construction. Sent as
    /// `ChatGPT-Account-ID` header. None if extraction failed (driver
    /// still attempts requests; the backend may reject them).
    account_id: Option<String>,
    client: reqwest::Client,
}

impl CodexDriver {
    pub fn new(access_token: String, base_url: String) -> Self {
        let account_id = extract_chatgpt_account_id(&access_token);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            access_token,
            base_url,
            account_id,
            client,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }

    fn auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut b = builder
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("User-Agent", CODEX_UA)
            .header("originator", CODEX_ORIGINATOR)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        if let Some(acc) = &self.account_id {
            b = b.header("ChatGPT-Account-ID", acc);
        }
        b
    }
}

/// Extract `chatgpt_account_id` from a JWT access token without verifying
/// the signature. The Codex backend expects this id as the
/// `ChatGPT-Account-ID` header on every `/responses` call.
fn extract_chatgpt_account_id(token: &str) -> Option<String> {
    use base64::Engine;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload_b64 = parts[1];
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("https://api.openai.com/auth")
        .and_then(|a| a.get("chatgpt_account_id"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
}

/// One element in the `input` array. Either a chat-style message,
/// a tool call (assistant), or a tool result (function output).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum InputItem {
    Message {
        role: &'static str,
        content: Vec<InputContent>,
    },
    Reasoning {
        #[serde(rename = "type")]
        kind: &'static str,
        encrypted_content: String,
        summary: Vec<Value>,
    },
    FunctionCall {
        #[serde(rename = "type")]
        kind: &'static str,
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        #[serde(rename = "type")]
        kind: &'static str,
        call_id: String,
        output: String,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum InputContent {
    InputText { text: String },
    OutputText { text: String },
}

#[derive(Debug, Serialize)]
struct ToolDef<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
}

#[derive(Debug, Serialize)]
struct CodexRequest<'a> {
    model: &'a str,
    instructions: &'a str,
    input: Vec<InputItem>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<ToolDef<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<CodexReasoning>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    include: Vec<&'static str>,
    #[serde(skip_serializing_if = "is_false")]
    parallel_tool_calls: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt_cache_key: Option<&'a str>,
    store: bool,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct CodexReasoning {
    effort: &'static str,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn build_payload(req: &CompletionRequest, stream: bool) -> CodexRequest<'_> {
    let instructions: &str = req.system.as_deref().unwrap_or("");
    let mut input: Vec<InputItem> = Vec::new();
    for msg in &req.messages {
        if matches!(msg.role, Role::System) {
            continue;
        }
        let role: &'static str = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => continue,
        };
        match &msg.content {
            MessageContent::Text(s) => input.push(InputItem::Message {
                role,
                content: vec![match msg.role {
                    Role::Assistant => InputContent::OutputText { text: s.clone() },
                    _ => InputContent::InputText { text: s.clone() },
                }],
            }),
            MessageContent::Blocks(blocks) => {
                for blk in blocks {
                    match blk {
                        ContentBlock::Text { text, .. } => input.push(InputItem::Message {
                            role,
                            content: vec![match msg.role {
                                Role::Assistant => InputContent::OutputText { text: text.clone() },
                                _ => InputContent::InputText { text: text.clone() },
                            }],
                        }),
                        ContentBlock::ToolUse {
                            id,
                            name,
                            input: ti,
                            ..
                        } => {
                            input.push(InputItem::FunctionCall {
                                kind: "function_call",
                                call_id: id.clone(),
                                name: name.clone(),
                                arguments: serde_json::to_string(ti)
                                    .unwrap_or_else(|_| "{}".into()),
                            });
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            input.push(InputItem::FunctionCallOutput {
                                kind: "function_call_output",
                                call_id: tool_use_id.clone(),
                                output: content.clone(),
                            });
                        }
                        ContentBlock::Thinking {
                            provider_metadata, ..
                        } => {
                            if let Some(reasoning) =
                                codex_reasoning_input_from_metadata(provider_metadata.as_ref())
                            {
                                input.push(reasoning);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let tools: Vec<ToolDef<'_>> = req
        .tools
        .iter()
        .map(|t| ToolDef {
            kind: "function",
            name: &t.name,
            description: &t.description,
            parameters: &t.input_schema,
        })
        .collect();
    let has_tools = !tools.is_empty();
    let supports_reasoning = codex_model_supports_reasoning(&req.model);

    CodexRequest {
        model: &req.model,
        instructions,
        input,
        tools,
        tool_choice: if has_tools {
            req.tool_choice.clone().or_else(|| Some(json!("auto")))
        } else {
            None
        },
        reasoning: supports_reasoning.then(|| CodexReasoning {
            effort: codex_reasoning_effort(req),
        }),
        include: if supports_reasoning {
            vec!["reasoning.encrypted_content"]
        } else {
            Vec::new()
        },
        parallel_tool_calls: has_tools,
        prompt_cache_key: req.cache_hints.prompt_cache_key.as_deref(),
        store: false,
        stream,
    }
}

fn codex_model_supports_reasoning(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("gpt-5") || model.starts_with('o') || model.contains("/o")
}

fn codex_reasoning_effort(req: &CompletionRequest) -> &'static str {
    match req.thinking.as_ref().map(|t| t.budget_tokens) {
        Some(1..=2_048) => "low",
        Some(30_001..) => "xhigh",
        Some(12_001..) => "high",
        _ => "medium",
    }
}

fn codex_reasoning_input_from_metadata(metadata: Option<&Value>) -> Option<InputItem> {
    let meta = metadata?.get(CODEX_REASONING_METADATA_KEY)?;
    let encrypted_content = meta
        .get("encrypted_content")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    let summary = meta
        .get("summary")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Some(InputItem::Reasoning {
        kind: "reasoning",
        encrypted_content,
        summary,
    })
}

#[derive(Debug, Clone)]
struct PendingCodexTool {
    item_id: Option<String>,
    call_id: String,
    name: String,
    arguments: String,
    started: bool,
    finalized: bool,
    arguments_done: bool,
}

impl PendingCodexTool {
    fn new(item_id: Option<String>, call_id: String, name: String) -> Self {
        Self {
            item_id,
            call_id,
            name,
            arguments: String::new(),
            started: false,
            finalized: false,
            arguments_done: false,
        }
    }

    fn can_finalize(&self) -> bool {
        !self.finalized && self.arguments_done && !self.name.trim().is_empty()
    }
}

#[derive(Debug)]
struct CodexStreamStats {
    text_delta_count: u64,
    tool_start_count: u64,
    tool_argument_delta_count: u64,
    tool_done_count: u64,
    terminal_event_seen: bool,
    unknown_event_types: Vec<String>,
}

struct CodexStreamState {
    accumulated: String,
    reasoning_blocks: Vec<ContentBlock>,
    pending_tools: Vec<PendingCodexTool>,
    tool_calls: Vec<ToolCall>,
    stop: StopReason,
    incomplete: bool,
    usage: TokenUsage,
    text_delta_count: u64,
    tool_start_count: u64,
    tool_argument_delta_count: u64,
    tool_done_count: u64,
    terminal_event_seen: bool,
    unknown_event_types: BTreeSet<String>,
}

impl CodexStreamState {
    fn new() -> Self {
        Self {
            accumulated: String::new(),
            reasoning_blocks: Vec::new(),
            pending_tools: Vec::new(),
            tool_calls: Vec::new(),
            stop: StopReason::EndTurn,
            incomplete: false,
            usage: TokenUsage::default(),
            text_delta_count: 0,
            tool_start_count: 0,
            tool_argument_delta_count: 0,
            tool_done_count: 0,
            terminal_event_seen: false,
            unknown_event_types: BTreeSet::new(),
        }
    }

    fn ingest_event(&mut self, typ: &str, value: &Value) -> Result<Vec<StreamEvent>, LlmError> {
        let mut events = Vec::new();
        match typ {
            "response.output_text.delta" => {
                if let Some(d) = value.get("delta").and_then(|v| v.as_str()) {
                    self.text_delta_count += 1;
                    self.accumulated.push_str(d);
                    events.push(StreamEvent::TextDelta {
                        text: d.to_string(),
                    });
                }
            }
            "response.reasoning.delta" => {
                if let Some(d) = value.get("delta").and_then(|v| v.as_str()) {
                    events.push(StreamEvent::ThinkingDelta {
                        text: d.to_string(),
                    });
                }
            }
            "response.output_item.added" => {
                if let Some(item) = value.get("item") {
                    events.extend(self.ingest_output_item_added(item));
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(d) = value.get("delta").and_then(|v| v.as_str()) {
                    self.tool_argument_delta_count += 1;
                    self.append_tool_arguments(
                        string_field(value, "item_id"),
                        string_field(value, "call_id"),
                        d,
                    );
                    events.push(StreamEvent::ToolInputDelta {
                        text: d.to_string(),
                    });
                }
            }
            "response.function_call_arguments.done" => {
                self.replace_tool_arguments(
                    string_field(value, "item_id"),
                    string_field(value, "call_id"),
                    string_field(value, "arguments").unwrap_or_default(),
                );
                events.extend(self.finalize_tool_for_event(value));
            }
            "response.output_item.done" => {
                if let Some(item) = value.get("item") {
                    events.extend(self.ingest_output_item_done(item));
                }
            }
            "response.completed" | "response.incomplete" => {
                self.terminal_event_seen = true;
                if typ == "response.incomplete" {
                    self.incomplete = true;
                }
                if let Some(response) = value.get("response") {
                    self.ingest_response_snapshot(response, &mut events);
                }
            }
            "response.failed" | "error" => {
                let msg = value
                    .pointer("/response/error/message")
                    .or_else(|| value.pointer("/error/message"))
                    .or_else(|| value.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("codex stream failed")
                    .to_string();
                warn!(error = %msg, "Codex stream emitted failure event");
                return Err(LlmError::Api {
                    status: 500,
                    message: msg,
                });
            }
            "" => {}
            other => {
                self.unknown_event_types.insert(other.to_string());
            }
        }
        Ok(events)
    }

    fn ingest_output_item_added(&mut self, item: &Value) -> Vec<StreamEvent> {
        if item.get("type").and_then(|v| v.as_str()) != Some("function_call") {
            return Vec::new();
        }
        let item_id = item_identifier(item);
        let call_id = string_field(item, "call_id")
            .or_else(|| item_id.clone())
            .unwrap_or_default();
        if self.tool_call_already_recorded(&call_id) {
            return Vec::new();
        }
        let name = string_field(item, "name").unwrap_or_default();
        let idx = self.ensure_pending_tool(item_id, call_id.clone(), name.clone());
        if let Some(args) = string_field(item, "arguments") {
            self.pending_tools[idx].arguments = args;
        }
        self.emit_tool_start(idx)
    }

    fn ingest_output_item_done(&mut self, item: &Value) -> Vec<StreamEvent> {
        match item.get("type").and_then(|v| v.as_str()) {
            Some("message") => {
                return self
                    .ingest_message_output_item(item)
                    .map(|text| vec![StreamEvent::TextDelta { text }])
                    .unwrap_or_default();
            }
            Some("reasoning") => {
                self.ingest_reasoning_output_item(item);
                return Vec::new();
            }
            Some("function_call") => {}
            _ => return Vec::new(),
        }
        let item_id = item_identifier(item);
        let call_id = string_field(item, "call_id")
            .or_else(|| item_id.clone())
            .unwrap_or_default();
        if self.tool_call_already_recorded(&call_id) {
            return Vec::new();
        }
        let name = string_field(item, "name").unwrap_or_default();
        let idx = self.ensure_pending_tool(item_id, call_id, name);
        if let Some(args) = string_field(item, "arguments") {
            self.pending_tools[idx].arguments = args;
            self.pending_tools[idx].arguments_done = true;
        }
        self.emit_tool_start(idx)
            .into_iter()
            .chain(self.finalize_tool_index(idx))
            .collect()
    }

    fn ingest_response_snapshot(&mut self, response: &Value, events: &mut Vec<StreamEvent>) {
        if let Some(u) = response.get("usage") {
            self.usage.input_tokens = u.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            self.usage.output_tokens = u.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
            self.usage.cached_input_tokens = u
                .get("cached_input_tokens")
                .or_else(|| u.pointer("/input_tokens_details/cached_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
        }

        if matches!(
            response.get("status").and_then(|v| v.as_str()),
            Some("incomplete" | "in_progress")
        ) {
            self.incomplete = true;
        }

        let Some(output) = response.get("output").and_then(|v| v.as_array()) else {
            return;
        };
        for item in output {
            if matches!(
                item.get("status").and_then(|v| v.as_str()),
                Some("incomplete" | "in_progress")
            ) {
                self.incomplete = true;
            }
            if matches!(
                item.get("phase").and_then(|v| v.as_str()),
                Some("commentary" | "status" | "analysis")
            ) {
                self.incomplete = true;
            }
            match item.get("type").and_then(|v| v.as_str()) {
                Some("message") => {
                    if let Some(text) = self.ingest_message_output_item(item) {
                        events.push(StreamEvent::TextDelta { text });
                    }
                }
                Some("reasoning") => {
                    self.ingest_reasoning_output_item(item);
                }
                Some("function_call") => {
                    events.extend(self.ingest_output_item_done(item));
                }
                _ => {}
            }
        }
    }

    fn ingest_message_output_item(&mut self, item: &Value) -> Option<String> {
        if !self.accumulated.is_empty() {
            return None;
        }
        let mut text = String::new();
        if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
            for block in content {
                if matches!(
                    block.get("type").and_then(|v| v.as_str()),
                    Some("output_text" | "text")
                ) {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    }
                }
            }
        }
        if !text.is_empty() {
            self.accumulated = text.clone();
            Some(text)
        } else {
            None
        }
    }

    fn ensure_pending_tool(
        &mut self,
        item_id: Option<String>,
        call_id: String,
        name: String,
    ) -> usize {
        if let Some(idx) = self.find_pending_tool(item_id.as_deref(), Some(&call_id)) {
            let tool = &mut self.pending_tools[idx];
            if tool.item_id.is_none() {
                tool.item_id = item_id;
            }
            if tool.call_id.is_empty() {
                tool.call_id = call_id;
            }
            if tool.name.is_empty() {
                tool.name = name;
            }
            return idx;
        }
        self.pending_tools
            .push(PendingCodexTool::new(item_id, call_id, name));
        self.pending_tools.len() - 1
    }

    fn find_pending_tool(&self, item_id: Option<&str>, call_id: Option<&str>) -> Option<usize> {
        if let Some(item_id) = item_id.filter(|s| !s.is_empty()) {
            if let Some(idx) = self
                .pending_tools
                .iter()
                .position(|t| t.item_id.as_deref() == Some(item_id) && !t.finalized)
            {
                return Some(idx);
            }
        }
        if let Some(call_id) = call_id.filter(|s| !s.is_empty()) {
            if let Some(idx) = self
                .pending_tools
                .iter()
                .position(|t| t.call_id == call_id && !t.finalized)
            {
                return Some(idx);
            }
        }
        None
    }

    fn ingest_reasoning_output_item(&mut self, item: &Value) {
        let encrypted_content = string_field(item, "encrypted_content");
        let summary = item
            .get("summary")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let thinking = reasoning_summary_text(&summary);
        if encrypted_content.is_none() && thinking.is_empty() {
            return;
        }
        if let Some(enc) = encrypted_content.as_deref() {
            if self.reasoning_blocks.iter().any(|block| {
                matches!(
                    block,
                    ContentBlock::Thinking {
                        provider_metadata: Some(meta),
                        ..
                    } if meta
                        .get(CODEX_REASONING_METADATA_KEY)
                        .and_then(|m| m.get("encrypted_content"))
                        .and_then(|v| v.as_str())
                        == Some(enc)
                )
            }) {
                return;
            }
        }

        let provider_metadata = encrypted_content.map(|encrypted_content| {
            json!({
                (CODEX_REASONING_METADATA_KEY): {
                    "encrypted_content": encrypted_content,
                    "summary": summary,
                }
            })
        });
        self.reasoning_blocks.push(ContentBlock::Thinking {
            thinking,
            provider_metadata,
        });
    }

    fn find_single_open_tool(&self) -> Option<usize> {
        let mut open = self
            .pending_tools
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.finalized);
        let first = open.next().map(|(idx, _)| idx)?;
        if open.next().is_none() {
            Some(first)
        } else {
            None
        }
    }

    fn tool_call_already_recorded(&self, call_id: &str) -> bool {
        !call_id.is_empty() && self.tool_calls.iter().any(|call| call.id == call_id)
    }

    fn append_tool_arguments(
        &mut self,
        item_id: Option<String>,
        call_id: Option<String>,
        delta: &str,
    ) {
        let idx = self
            .find_pending_tool(item_id.as_deref(), call_id.as_deref())
            .or_else(|| self.find_single_open_tool())
            .unwrap_or_else(|| {
                let call_id = call_id
                    .clone()
                    .or_else(|| item_id.clone())
                    .unwrap_or_default();
                self.pending_tools.push(PendingCodexTool::new(
                    item_id.clone(),
                    call_id,
                    String::new(),
                ));
                self.pending_tools.len() - 1
            });
        self.pending_tools[idx].arguments.push_str(delta);
    }

    fn replace_tool_arguments(
        &mut self,
        item_id: Option<String>,
        call_id: Option<String>,
        arguments: String,
    ) {
        let idx = self
            .find_pending_tool(item_id.as_deref(), call_id.as_deref())
            .or_else(|| self.find_single_open_tool())
            .unwrap_or_else(|| {
                let call_id = call_id
                    .clone()
                    .or_else(|| item_id.clone())
                    .unwrap_or_default();
                self.pending_tools.push(PendingCodexTool::new(
                    item_id.clone(),
                    call_id,
                    String::new(),
                ));
                self.pending_tools.len() - 1
            });
        self.pending_tools[idx].arguments = arguments;
        self.pending_tools[idx].arguments_done = true;
    }

    fn emit_tool_start(&mut self, idx: usize) -> Vec<StreamEvent> {
        let tool = &mut self.pending_tools[idx];
        if tool.started || tool.call_id.is_empty() || tool.name.is_empty() {
            return Vec::new();
        }
        tool.started = true;
        self.tool_start_count += 1;
        vec![StreamEvent::ToolUseStart {
            id: tool.call_id.clone(),
            name: tool.name.clone(),
        }]
    }

    fn finalize_tool_for_event(&mut self, value: &Value) -> Vec<StreamEvent> {
        let item_id = string_field(value, "item_id");
        let call_id = string_field(value, "call_id");
        let idx = self
            .find_pending_tool(item_id.as_deref(), call_id.as_deref())
            .or_else(|| self.find_single_open_tool());
        idx.into_iter()
            .flat_map(|idx| self.finalize_tool_index(idx))
            .collect()
    }

    fn finalize_tool_index(&mut self, idx: usize) -> Vec<StreamEvent> {
        if idx >= self.pending_tools.len() || !self.pending_tools[idx].can_finalize() {
            return Vec::new();
        }
        if self.pending_tools[idx].call_id.is_empty() {
            if let Some(item_id) = self.pending_tools[idx].item_id.clone() {
                self.pending_tools[idx].call_id = item_id;
            } else {
                self.pending_tools[idx].call_id = format!("call_{}", self.tool_calls.len() + 1);
            }
        }
        if self.tool_call_already_recorded(&self.pending_tools[idx].call_id) {
            self.pending_tools[idx].finalized = true;
            return Vec::new();
        }
        let parsed = parse_tool_arguments(&self.pending_tools[idx].arguments);
        let id = self.pending_tools[idx].call_id.clone();
        let name = self.pending_tools[idx].name.clone();
        self.pending_tools[idx].finalized = true;
        self.tool_done_count += 1;
        self.stop = StopReason::ToolUse;
        self.tool_calls.push(ToolCall {
            id: id.clone(),
            name: name.clone(),
            input: parsed.clone(),
        });
        vec![StreamEvent::ToolUseEnd {
            id,
            name,
            input: parsed,
        }]
    }

    fn finish(self) -> (CompletionResponse, CodexStreamStats) {
        let mut content: Vec<ContentBlock> = Vec::new();
        content.extend(self.reasoning_blocks);
        if !self.accumulated.is_empty() {
            content.push(ContentBlock::Text {
                text: self.accumulated,
                provider_metadata: None,
            });
        }
        for tc in &self.tool_calls {
            content.push(ContentBlock::ToolUse {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
                provider_metadata: None,
            });
        }

        let stats = CodexStreamStats {
            text_delta_count: self.text_delta_count,
            tool_start_count: self.tool_start_count,
            tool_argument_delta_count: self.tool_argument_delta_count,
            tool_done_count: self.tool_done_count,
            terminal_event_seen: self.terminal_event_seen,
            unknown_event_types: self.unknown_event_types.into_iter().collect(),
        };

        (
            CompletionResponse {
                content,
                stop_reason: if self.incomplete && self.stop != StopReason::ToolUse {
                    StopReason::Incomplete
                } else {
                    self.stop
                },
                tool_calls: self.tool_calls,
                usage: self.usage,
            },
            stats,
        )
    }
}

fn item_identifier(item: &Value) -> Option<String> {
    string_field(item, "id").or_else(|| string_field(item, "item_id"))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn reasoning_summary_text(summary: &[Value]) -> String {
    summary
        .iter()
        .filter_map(|item| item.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_tool_arguments(arguments: &str) -> Value {
    if arguments.trim().is_empty() {
        return Value::Object(Default::default());
    }
    serde_json::from_str(arguments).unwrap_or_else(|_| Value::Object(Default::default()))
}

fn is_empty_codex_contract_failure(response: &CompletionResponse) -> bool {
    !response.has_any_content()
        && response.tool_calls.is_empty()
        && response.usage.input_tokens == 0
        && response.usage.output_tokens == 0
}

fn parse_sse_event(raw_event: &str) -> Option<(String, Value)> {
    let mut event_type: Option<&str> = None;
    let mut data_lines: Vec<&str> = Vec::new();
    for line in raw_event.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            event_type = Some(rest);
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data_lines.push(rest);
        }
    }
    if data_lines.is_empty() {
        return None;
    }
    let data = data_lines.join("\n");
    if data.trim() == "[DONE]" {
        return None;
    }
    let value: Value = serde_json::from_str(&data).ok()?;
    let typ = event_type
        .map(|s| s.to_string())
        .or_else(|| {
            value
                .get("type")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default();
    Some((typ, value))
}

#[async_trait]
impl LlmDriver for CodexDriver {
    /// The chatgpt.com Codex backend rejects `stream:false` with
    /// `400 "Stream must be set to true"`. We therefore wrap `stream()`
    /// with a sink channel and reassemble the final response.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        self.stream(request, tx).await
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let payload = build_payload(&request, true);
        let mut resp = self
            .auth_headers(self.client.post(self.endpoint()))
            .header("Accept", "text/event-stream")
            .json(&payload)
            .send()
            .await
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let mut status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let first_body = resp.text().await.unwrap_or_default();
            if let Some(new_token) =
                crate::model_catalog::refresh_or_rotate_codex_credential(&self.access_token).await
            {
                let retry_driver = CodexDriver::new(new_token, self.base_url.clone());
                resp = retry_driver
                    .auth_headers(retry_driver.client.post(retry_driver.endpoint()))
                    .header("Accept", "text/event-stream")
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|e| LlmError::Http(e.to_string()))?;
                status = resp.status();
            } else {
                return Err(map_status_error(status, first_body));
            }
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(map_status_error(status, body));
        }

        let mut state = CodexStreamState::new();
        let mut byte_stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = byte_stream.next().await {
            let bytes = chunk.map_err(|e| LlmError::Http(e.to_string()))?;
            buffer.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(idx) = buffer.find("\n\n") {
                let raw_event = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();
                let Some((typ, value)) = parse_sse_event(&raw_event) else {
                    continue;
                };
                for event in state.ingest_event(&typ, &value)? {
                    let _ = tx.send(event).await;
                }
            }
        }
        if let Some((typ, value)) = parse_sse_event(&buffer) {
            for event in state.ingest_event(&typ, &value)? {
                let _ = tx.send(event).await;
            }
        }

        let (response, stats) = state.finish();
        debug!(
            model = %request.model,
            tools_offered = request.tools.len(),
            text_delta_count = stats.text_delta_count,
            tool_start_count = stats.tool_start_count,
            tool_argument_delta_count = stats.tool_argument_delta_count,
            tool_done_count = stats.tool_done_count,
            terminal_event_seen = stats.terminal_event_seen,
            unknown_event_types = ?stats.unknown_event_types,
            input_tokens = response.usage.input_tokens,
            output_tokens = response.usage.output_tokens,
            cached_input_tokens = response.usage.cached_input_tokens,
            stop_reason = ?response.stop_reason,
            "Codex stream parsed"
        );

        if is_empty_codex_contract_failure(&response) {
            return Err(LlmError::Parse(
                "Codex stream returned no text, tool calls, or usage".to_string(),
            ));
        }

        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason: response.stop_reason,
                usage: response.usage,
            })
            .await;

        Ok(response)
    }
}

fn map_status_error(status: reqwest::StatusCode, body: String) -> LlmError {
    let s = status.as_u16();
    match s {
        401 | 403 => LlmError::AuthenticationFailed(format!(
            "Codex {s}: token rejeté. Refais `captain login codex`. Body: {}",
            truncate(&body, 200)
        )),
        404 => LlmError::ModelNotFound(format!(
            "Codex 404: route ou modèle introuvable. Body: {}",
            truncate(&body, 200)
        )),
        429 => {
            let retry_ms = parse_retry_after(&body).unwrap_or(5000);
            LlmError::RateLimited {
                retry_after_ms: retry_ms,
            }
        }
        500..=599 => LlmError::Overloaded {
            retry_after_ms: 3000,
        },
        _ => LlmError::Api {
            status: s,
            message: truncate(&body, 500),
        },
    }
}

fn parse_retry_after(body: &str) -> Option<u64> {
    serde_json::from_str::<Value>(body)
        .ok()?
        .pointer("/error/retry_after_ms")
        .and_then(|v| v.as_u64())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::Message;
    use captain_types::tool::ToolDefinition;
    use serde_json::json;

    #[test]
    fn build_payload_extracts_system_to_instructions() {
        let req = CompletionRequest {
            model: "gpt-5.4".into(),
            messages: vec![Message::system("you are helpful"), Message::user("hi")],
            tools: vec![],
            max_tokens: 1024,
            temperature: 0.7,
            system: Some("you are helpful".into()),
            thinking: None,
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default(),
        };
        let payload = build_payload(&req, false);
        assert_eq!(payload.instructions, "you are helpful");
        assert_eq!(payload.input.len(), 1);
        assert!(!payload.store);
        let encoded = serde_json::to_value(&payload).unwrap();
        assert!(
            encoded.get("max_output_tokens").is_none(),
            "chatgpt.com/backend-api/codex currently rejects max_output_tokens"
        );
    }

    #[test]
    fn build_payload_preserves_tool_choice_for_responses_contract() {
        let req = CompletionRequest {
            model: "gpt-5.4".into(),
            messages: vec![Message::user("use ping")],
            tools: vec![ToolDefinition {
                name: "ping".into(),
                description: "ping a target".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {"target": {"type": "string"}},
                    "required": ["target"]
                }),
            }],
            max_tokens: 1024,
            temperature: 0.7,
            system: None,
            thinking: None,
            tool_choice: Some(json!("required")),
            cache_hints: crate::llm_driver::CacheHints::default(),
        };
        let encoded = serde_json::to_value(build_payload(&req, true)).unwrap();
        assert_eq!(encoded.get("tool_choice"), Some(&json!("required")));
        assert_eq!(encoded["tools"][0]["type"], "function");
    }

    #[test]
    fn build_payload_enables_codex_agentic_contract_defaults() {
        let req = CompletionRequest {
            model: "gpt-5.3-codex".into(),
            messages: vec![Message::user("inspect the repo")],
            tools: vec![ToolDefinition {
                name: "shell".into(),
                description: "run a shell command".into(),
                input_schema: json!({"type": "object"}),
            }],
            max_tokens: 1024,
            temperature: 0.7,
            system: None,
            thinking: None,
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default()
                .with_prompt_cache_key(Some("captain-session-test".into())),
        };
        let encoded = serde_json::to_value(build_payload(&req, true)).unwrap();
        assert_eq!(encoded["tool_choice"], json!("auto"));
        assert_eq!(encoded["parallel_tool_calls"], json!(true));
        assert_eq!(encoded["reasoning"]["effort"], json!("medium"));
        assert_eq!(encoded["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(encoded["prompt_cache_key"], json!("captain-session-test"));
    }

    #[test]
    fn build_payload_maps_large_thinking_budget_to_xhigh() {
        let req = CompletionRequest {
            model: "gpt-5.5".into(),
            messages: vec![Message::user("hard task")],
            tools: vec![],
            max_tokens: 1024,
            temperature: 0.7,
            system: None,
            thinking: Some(captain_types::config::ThinkingConfig {
                budget_tokens: 40_000,
                stream_thinking: false,
            }),
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default(),
        };
        let encoded = serde_json::to_value(build_payload(&req, true)).unwrap();
        assert_eq!(encoded["reasoning"]["effort"], json!("xhigh"));
    }

    #[test]
    fn codex_reasoning_encrypted_content_round_trips_without_response_id() {
        let mut state = CodexStreamState::new();
        state
            .ingest_event(
                "response.completed",
                &json!({
                    "response": {
                        "output": [{
                            "id": "rs_123",
                            "type": "reasoning",
                            "encrypted_content": "enc_abc",
                            "summary": [{"type": "summary_text", "text": "Checked context."}]
                        }],
                        "usage": {"input_tokens": 10, "output_tokens": 2}
                    }
                }),
            )
            .unwrap();
        let (response, _) = state.finish();
        assert!(response.has_any_content());

        let req = CompletionRequest {
            model: "gpt-5.3-codex".into(),
            messages: vec![
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(response.content),
                },
                Message::user("continue"),
            ],
            tools: vec![],
            max_tokens: 1024,
            temperature: 0.7,
            system: None,
            thinking: None,
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default(),
        };
        let encoded = serde_json::to_value(build_payload(&req, true)).unwrap();
        let reasoning = encoded["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "reasoning")
            .unwrap();
        assert_eq!(reasoning["encrypted_content"], json!("enc_abc"));
        assert!(
            reasoning.get("id").is_none(),
            "stateless replay must not keep output ids"
        );
    }

    #[test]
    fn codex_reasoning_replay_keeps_empty_summary_array() {
        let req = CompletionRequest {
            model: "gpt-5.5".into(),
            messages: vec![
                Message {
                    role: Role::Assistant,
                    content: MessageContent::Blocks(vec![ContentBlock::Thinking {
                        thinking: String::new(),
                        provider_metadata: Some(json!({
                            CODEX_REASONING_METADATA_KEY: {
                                "encrypted_content": "enc_without_summary",
                                "summary": []
                            }
                        })),
                    }]),
                },
                Message::user("hey"),
            ],
            tools: vec![],
            max_tokens: 1024,
            temperature: 0.7,
            system: None,
            thinking: None,
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default(),
        };

        let encoded = serde_json::to_value(build_payload(&req, true)).unwrap();
        let reasoning = encoded["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "reasoning")
            .unwrap();
        assert_eq!(reasoning["encrypted_content"], json!("enc_without_summary"));
        assert_eq!(reasoning["summary"], json!([]));
    }

    #[test]
    fn codex_stream_accepts_text_deltas_when_final_output_is_empty() {
        let mut state = CodexStreamState::new();
        state
            .ingest_event("response.output_text.delta", &json!({"delta": "Bon"}))
            .unwrap();
        state
            .ingest_event("response.output_text.delta", &json!({"delta": "jour"}))
            .unwrap();
        state
            .ingest_event(
                "response.completed",
                &json!({
                    "response": {
                        "output": [],
                        "usage": {"input_tokens": 12, "output_tokens": 3}
                    }
                }),
            )
            .unwrap();

        let (response, stats) = state.finish();
        assert_eq!(response.text(), "Bonjour");
        assert_eq!(response.usage.input_tokens, 12);
        assert_eq!(response.usage.output_tokens, 3);
        assert_eq!(stats.text_delta_count, 2);
        assert!(!is_empty_codex_contract_failure(&response));
    }

    #[test]
    fn codex_stream_finalizes_tool_on_arguments_done_without_output_item_done() {
        let mut state = CodexStreamState::new();
        let start = state
            .ingest_event(
                "response.output_item.added",
                &json!({
                    "item": {
                        "id": "fc_1",
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "shell"
                    }
                }),
            )
            .unwrap();
        assert!(
            matches!(start.first(), Some(StreamEvent::ToolUseStart { id, name }) if id == "call_1" && name == "shell")
        );

        state
            .ingest_event(
                "response.function_call_arguments.delta",
                &json!({"item_id": "fc_1", "delta": "{\"cmd\":"}),
            )
            .unwrap();
        let done = state
            .ingest_event(
                "response.function_call_arguments.done",
                &json!({"item_id": "fc_1", "arguments": "{\"cmd\":\"date\"}"}),
            )
            .unwrap();
        assert!(
            matches!(done.first(), Some(StreamEvent::ToolUseEnd { id, name, .. }) if id == "call_1" && name == "shell")
        );

        let (response, _) = state.finish();
        assert_eq!(response.stop_reason, StopReason::ToolUse);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "shell");
        assert_eq!(response.tool_calls[0].input, json!({"cmd": "date"}));
    }

    #[test]
    fn codex_stream_keeps_interleaved_tool_arguments_by_item_id() {
        let mut state = CodexStreamState::new();
        state
            .ingest_event(
                "response.output_item.added",
                &json!({"item": {"id": "fc_a", "type": "function_call", "call_id": "call_a", "name": "alpha"}}),
            )
            .unwrap();
        state
            .ingest_event(
                "response.output_item.added",
                &json!({"item": {"id": "fc_b", "type": "function_call", "call_id": "call_b", "name": "beta"}}),
            )
            .unwrap();
        state
            .ingest_event(
                "response.function_call_arguments.delta",
                &json!({"item_id": "fc_b", "delta": "{\"b\":2}"}),
            )
            .unwrap();
        state
            .ingest_event(
                "response.function_call_arguments.delta",
                &json!({"item_id": "fc_a", "delta": "{\"a\":1}"}),
            )
            .unwrap();
        state
            .ingest_event(
                "response.function_call_arguments.done",
                &json!({"item_id": "fc_a", "arguments": "{\"a\":1}"}),
            )
            .unwrap();
        state
            .ingest_event(
                "response.function_call_arguments.done",
                &json!({"item_id": "fc_b", "arguments": "{\"b\":2}"}),
            )
            .unwrap();

        let (response, _) = state.finish();
        assert_eq!(response.tool_calls.len(), 2);
        assert_eq!(response.tool_calls[0].id, "call_a");
        assert_eq!(response.tool_calls[0].input, json!({"a": 1}));
        assert_eq!(response.tool_calls[1].id, "call_b");
        assert_eq!(response.tool_calls[1].input, json!({"b": 2}));
    }

    #[test]
    fn codex_stream_recovers_text_and_tool_from_completed_snapshot() {
        let mut state = CodexStreamState::new();
        state
            .ingest_event(
                "response.completed",
                &json!({
                    "response": {
                        "output": [
                            {
                                "type": "message",
                                "content": [{"type": "output_text", "text": "Snapshot text"}]
                            },
                            {
                                "id": "fc_9",
                                "type": "function_call",
                                "call_id": "call_9",
                                "name": "lookup",
                                "arguments": "{\"q\":\"captain\"}"
                            }
                        ],
                        "usage": {
                            "input_tokens": 20,
                            "output_tokens": 7,
                            "input_tokens_details": {"cached_tokens": 8}
                        }
                    }
                }),
            )
            .unwrap();

        let (response, _) = state.finish();
        assert_eq!(response.text(), "Snapshot text");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].input, json!({"q": "captain"}));
        assert_eq!(response.usage.cached_input_tokens, 8);
    }

    #[test]
    fn codex_stream_completed_snapshot_does_not_duplicate_finalized_tool() {
        let mut state = CodexStreamState::new();
        state
            .ingest_event(
                "response.output_item.added",
                &json!({"item": {"id": "fc_1", "type": "function_call", "call_id": "call_1", "name": "lookup"}}),
            )
            .unwrap();
        state
            .ingest_event(
                "response.function_call_arguments.done",
                &json!({"item_id": "fc_1", "arguments": "{\"q\":\"captain\"}"}),
            )
            .unwrap();
        state
            .ingest_event(
                "response.completed",
                &json!({
                    "response": {
                        "output": [{
                            "id": "fc_1",
                            "type": "function_call",
                            "call_id": "call_1",
                            "name": "lookup",
                            "arguments": "{\"q\":\"captain\"}"
                        }],
                        "usage": {"input_tokens": 10, "output_tokens": 2}
                    }
                }),
            )
            .unwrap();

        let (response, _) = state.finish();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "call_1");
    }

    #[test]
    fn codex_stream_incomplete_uses_dedicated_stop_reason() {
        let mut state = CodexStreamState::new();
        state
            .ingest_event(
                "response.incomplete",
                &json!({
                    "response": {
                        "status": "incomplete",
                        "output": [{
                            "type": "message",
                            "content": [{"type": "output_text", "text": "I will inspect it."}]
                        }],
                        "usage": {"input_tokens": 10, "output_tokens": 3}
                    }
                }),
            )
            .unwrap();

        let (response, _) = state.finish();
        assert_eq!(response.stop_reason, StopReason::Incomplete);
        assert_eq!(response.text(), "I will inspect it.");
    }

    #[test]
    fn codex_sse_parser_accepts_final_event_without_blank_line() {
        let raw = "event: response.output_text.delta\ndata: {\"delta\":\"tail\"}";
        let (typ, value) = parse_sse_event(raw).unwrap();
        assert_eq!(typ, "response.output_text.delta");
        assert_eq!(value["delta"], json!("tail"));
    }

    #[test]
    fn codex_stream_empty_contract_failure_is_detected() {
        let state = CodexStreamState::new();
        let (response, _) = state.finish();
        assert!(is_empty_codex_contract_failure(&response));
    }

    #[test]
    fn codex_stream_does_not_finalize_tool_without_done_signal() {
        let mut state = CodexStreamState::new();
        state
            .ingest_event(
                "response.output_item.added",
                &json!({
                    "item": {
                        "id": "fc_partial",
                        "type": "function_call",
                        "call_id": "call_partial",
                        "name": "shell"
                    }
                }),
            )
            .unwrap();
        state
            .ingest_event(
                "response.function_call_arguments.delta",
                &json!({"item_id": "fc_partial", "delta": "{\"cmd\":\"date\"}"}),
            )
            .unwrap();

        let (response, _) = state.finish();
        assert!(
            response.tool_calls.is_empty(),
            "partial tool calls must not execute without arguments.done or output_item.done"
        );
    }

    #[test]
    fn extract_account_id_from_jwt_payload() {
        // header.payload.signature with payload = base64url of:
        // {"https://api.openai.com/auth":{"chatgpt_account_id":"acc_42"}}
        use base64::Engine;
        let payload = serde_json::json!({
            "https://api.openai.com/auth": {"chatgpt_account_id": "acc_42"}
        });
        let payload_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&payload).unwrap());
        let jwt = format!("h.{}.s", payload_b64);
        assert_eq!(extract_chatgpt_account_id(&jwt), Some("acc_42".into()));
    }

    #[test]
    fn extract_account_id_returns_none_for_garbage() {
        assert_eq!(extract_chatgpt_account_id("not.a.jwt"), None);
        assert_eq!(extract_chatgpt_account_id(""), None);
    }

    #[test]
    fn map_status_error_classifies_429_with_retry_after() {
        let body = r#"{"error":{"retry_after_ms":7500}}"#.to_string();
        let err = map_status_error(reqwest::StatusCode::TOO_MANY_REQUESTS, body);
        assert!(matches!(
            err,
            LlmError::RateLimited {
                retry_after_ms: 7500
            }
        ));
    }
}

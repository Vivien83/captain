//! LLM driver trait and types.
//!
//! Abstracts over multiple LLM providers (Anthropic, OpenAI, Ollama, etc.).

use async_trait::async_trait;
use captain_types::message::{ContentBlock, Message, StopReason, TokenUsage};
use captain_types::tool::{ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Error type for LLM driver operations.
#[derive(Error, Debug)]
pub enum LlmError {
    /// HTTP request failed.
    #[error("HTTP error: {0}")]
    Http(String),
    /// API returned an error.
    #[error("API error ({status}): {message}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Error message from the API.
        message: String,
    },
    /// Rate limited — should retry after delay.
    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited {
        /// How long to wait before retrying.
        retry_after_ms: u64,
    },
    /// Response parsing failed.
    #[error("Parse error: {0}")]
    Parse(String),
    /// No API key configured.
    #[error("Missing API key: {0}")]
    MissingApiKey(String),
    /// Model overloaded.
    #[error("Model overloaded, retry after {retry_after_ms}ms")]
    Overloaded {
        /// How long to wait before retrying.
        retry_after_ms: u64,
    },
    /// Authentication failed (invalid/missing API key).
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    /// Model not found.
    #[error("Model not found: {0}")]
    ModelNotFound(String),
}

/// Provider-agnostic cache hints attached to a completion request.
///
/// Each driver translates these flags into its provider's caching mechanism:
/// - **Anthropic**: appends `cache_control: {type: "ephemeral"}` on the last
///   block of the requested section (system / tools). When
///   `cacheable_system_prefix_bytes` is present, the Anthropic driver splits
///   the system prompt into cacheable-prefix and dynamic-suffix blocks.
/// - **OpenAI**: ignored — caching is automatic on prefixes ≥ 1024 tokens.
/// - **Codex**: sends `prompt_cache_key` to keep automatic prompt caching
///   stable per Captain session.
/// - **Gemini**: ignored for explicit caching today; Gemini's implicit cache
///   still benefits from stable repeated prefixes.
/// - **Groq / Mistral / unknown**: ignored (no-op, no cache prefix support).
///
/// Defaults to all-false so existing callers stay on the legacy uncached path.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheHints {
    /// Mark the system prompt block as a cache breakpoint.
    pub cache_system: bool,
    /// Mark the tools block as a cache breakpoint.
    pub cache_tools: bool,
    /// Byte length of the stable prefix inside `CompletionRequest.system`.
    ///
    /// `None` means "the whole system prompt is considered cacheable" for
    /// providers that support explicit breakpoints. `Some(n)` means bytes
    /// `[0..n]` are stable/cacheable and bytes `[n..]` are still system
    /// instructions, but intentionally not part of the explicit Anthropic
    /// cache breakpoint.
    #[serde(default)]
    pub cacheable_system_prefix_bytes: Option<usize>,
    /// Provider-specific cache affinity key. For OpenAI/Codex Responses this
    /// gives the backend a stable bucket for repeated session prefixes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

impl CacheHints {
    /// Both system and tools cached — the typical "Anthropic stable prefix" case.
    pub fn full() -> Self {
        Self {
            cache_system: true,
            cache_tools: true,
            cacheable_system_prefix_bytes: None,
            prompt_cache_key: None,
        }
    }

    /// True when at least one breakpoint is requested.
    pub fn any(&self) -> bool {
        self.cache_system || self.cache_tools
    }

    /// Pick sensible cache hints for a given provider name.
    ///
    /// Only Anthropic honors explicit `cache_control` markers on the system
    /// prompt and tools block today; OpenAI caches the prefix automatically
    /// (no opt-in needed) and the rest of the providers have no per-request
    /// cache mechanism yet, so we leave their hints empty.
    pub fn for_provider(provider: &str) -> Self {
        match provider.to_ascii_lowercase().as_str() {
            "anthropic" | "claude" => Self::full(),
            _ => Self::default(),
        }
    }

    /// Attach a system-prefix split produced by the prompt builder.
    pub fn with_system_prefix_bytes(mut self, bytes: Option<usize>) -> Self {
        self.cacheable_system_prefix_bytes = bytes;
        self
    }

    /// Attach a provider-specific prompt cache key.
    pub fn with_prompt_cache_key(mut self, key: Option<String>) -> Self {
        self.prompt_cache_key = key;
        self
    }
}

/// A request to an LLM for completion.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// Model identifier.
    pub model: String,
    /// Conversation messages.
    pub messages: Vec<Message>,
    /// Available tools the model can use.
    pub tools: Vec<ToolDefinition>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// System prompt (extracted from messages for APIs that need it separately).
    pub system: Option<String>,
    /// Extended thinking configuration (if supported by the model).
    pub thinking: Option<captain_types::config::ThinkingConfig>,
    /// Override tool_choice for this request (e.g., force a specific tool).
    /// None = "auto" (default). Some("any") = force at least one tool call.
    /// Some({"type":"function","function":{"name":"X"}}) = force specific tool.
    pub tool_choice: Option<serde_json::Value>,
    /// Provider-neutral cache hints. Each driver translates these into its
    /// own caching mechanism (or ignores them on providers without support).
    pub cache_hints: CacheHints,
}

/// A response from an LLM completion.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionResponse {
    /// The content blocks in the response.
    pub content: Vec<ContentBlock>,
    /// Why the model stopped generating.
    pub stop_reason: StopReason,
    /// Tool calls extracted from the response.
    pub tool_calls: Vec<ToolCall>,
    /// Token usage statistics.
    pub usage: TokenUsage,
}

impl CompletionResponse {
    /// Extract text content from the response.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                ContentBlock::Thinking { .. } => None,
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    /// Check if the response has any meaningful content (including Thinking blocks).
    /// Used to distinguish true empty responses from thinking-only responses.
    pub fn has_any_content(&self) -> bool {
        self.content.iter().any(|block| match block {
            ContentBlock::Text { text, .. } => !text.is_empty(),
            ContentBlock::Thinking {
                thinking,
                provider_metadata,
            } => !thinking.is_empty() || provider_metadata.is_some(),
            ContentBlock::ToolUse { .. } | ContentBlock::Image { .. } => true,
            _ => false,
        })
    }
}

/// Events emitted during streaming LLM completion.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental text content.
    TextDelta { text: String },
    /// A tool use block has started.
    ToolUseStart { id: String, name: String },
    /// Incremental JSON input for an in-progress tool use.
    ToolInputDelta { text: String },
    /// A tool use block is complete with parsed input.
    ToolUseEnd {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Incremental thinking/reasoning text.
    ThinkingDelta { text: String },
    /// The entire response is complete.
    ContentComplete {
        stop_reason: StopReason,
        usage: TokenUsage,
    },
    /// Agent lifecycle phase change (for UX indicators).
    PhaseChange {
        phase: String,
        detail: Option<String>,
    },
    /// Tool execution completed with result (emitted by agent loop, not LLM driver).
    ToolExecutionResult {
        tool_use_id: String,
        name: String,
        result_preview: String,
        is_error: bool,
    },
    /// Incremental output from a running tool (v3.9e-B).
    /// `stream` is usually "stdout" or "stderr" for exec-class tools. It may
    /// also be "progress" for semantic mid-tool heartbeat ticks emitted by
    /// long-running tools without exposing raw terminal output.
    ToolOutputDelta {
        tool_use_id: String,
        stream: &'static str, // "stdout" or "stderr"
        chunk: String,
    },
    /// Agent emits a complete intermediate message before continuing work.
    /// Creates a new visible "bubble" in the conversation — the agent thinks aloud.
    IntermediateMessage { content: String },
    /// Agent needs user input to continue. Blocks the loop until answered.
    AskUser {
        question: String,
        options: Option<Vec<String>>,
    },
    /// User responded to an AskUser request. Injected by the WS handler.
    UserResponse { content: String },
}

/// Trait for LLM drivers.
#[async_trait]
pub trait LlmDriver: Send + Sync {
    /// Send a completion request and get a response.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Stream a completion request, sending incremental events to the channel.
    /// Returns the full response when complete. Default wraps `complete()`.
    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let response = self.complete(request).await?;
        let text = response.text();
        if !text.is_empty() {
            let _ = tx.send(StreamEvent::TextDelta { text }).await;
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

/// Configuration for creating an LLM driver.
#[derive(Clone, Serialize, Deserialize)]
pub struct DriverConfig {
    /// Provider name.
    pub provider: String,
    /// API key.
    pub api_key: Option<String>,
    /// Base URL override.
    pub base_url: Option<String>,
    /// Skip interactive permission prompts (Claude Code provider only).
    ///
    /// When `true`, adds `--dangerously-skip-permissions` to the spawned
    /// `claude` CLI.  Defaults to `true` because Captain runs as a daemon
    /// with no interactive terminal, so permission prompts would block
    /// indefinitely.  Captain's own capability / RBAC layer already
    /// restricts what agents can do, making this safe.
    #[serde(default = "default_skip_permissions")]
    pub skip_permissions: bool,
}

fn default_skip_permissions() -> bool {
    true
}

/// SECURITY: Custom Debug impl redacts the API key.
impl std::fmt::Debug for DriverConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverConfig")
            .field("provider", &self.provider)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("base_url", &self.base_url)
            .field("skip_permissions", &self.skip_permissions)
            .finish()
    }
}

#[cfg(test)]
#[path = "llm_driver_tests.rs"]
mod tests;

//! Qwen Code CLI backend driver.
//!
//! Spawns the `qwen` CLI (Qwen Code) as a subprocess in print mode (`-p`),
//! which is non-interactive and handles its own authentication.
//! This allows users with Qwen Code installed to use it as an LLM provider
//! without needing a separate API key (uses Qwen OAuth by default).

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use captain_types::message::{ContentBlock, Role, StopReason, TokenUsage};
use serde::Deserialize;
use tokio::io::AsyncBufReadExt;
use tracing::{debug, warn};

/// Environment variable names to strip from the subprocess to prevent
/// leaking API keys from other providers.
const SENSITIVE_ENV_EXACT: &[&str] = &[
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "GROQ_API_KEY",
    "DEEPSEEK_API_KEY",
    "MISTRAL_API_KEY",
    "TOGETHER_API_KEY",
    "FIREWORKS_API_KEY",
    "OPENROUTER_API_KEY",
    "PERPLEXITY_API_KEY",
    "COHERE_API_KEY",
    "AI21_API_KEY",
    "CEREBRAS_API_KEY",
    "SAMBANOVA_API_KEY",
    "HUGGINGFACE_API_KEY",
    "XAI_API_KEY",
    "REPLICATE_API_TOKEN",
    "BRAVE_API_KEY",
    "TAVILY_API_KEY",
    "ELEVENLABS_API_KEY",
];

/// Suffixes that indicate a secret — remove any env var ending with these
/// unless it starts with `QWEN_`.
const SENSITIVE_SUFFIXES: &[&str] = &["_SECRET", "_TOKEN", "_PASSWORD"];

/// LLM driver that delegates to the Qwen Code CLI.
pub struct QwenCodeDriver {
    cli_path: String,
    skip_permissions: bool,
}

impl QwenCodeDriver {
    /// Create a new Qwen Code driver.
    ///
    /// `cli_path` overrides the CLI binary path; defaults to `"qwen"` on PATH.
    /// `skip_permissions` adds `--yolo` to the spawned command so that the CLI
    /// runs non-interactively (required for daemon mode).
    pub fn new(cli_path: Option<String>, skip_permissions: bool) -> Self {
        if skip_permissions {
            warn!(
                "Qwen Code driver: --yolo enabled. \
                 The CLI will not prompt for tool approvals. \
                 Captain's own capability/RBAC system enforces access control."
            );
        }

        Self {
            cli_path: cli_path
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "qwen".to_string()),
            skip_permissions,
        }
    }

    /// Detect if the Qwen Code CLI is available on PATH.
    pub fn detect() -> Option<String> {
        let output = std::process::Command::new("qwen")
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }

    /// Build the CLI arguments for a given request.
    pub fn build_args(&self, prompt: &str, model: &str, streaming: bool) -> Vec<String> {
        let mut args = vec!["-p".to_string(), prompt.to_string()];

        args.push("--output-format".to_string());
        if streaming {
            args.push("stream-json".to_string());
            args.push("--verbose".to_string());
        } else {
            args.push("json".to_string());
        }

        if self.skip_permissions {
            args.push("--yolo".to_string());
        }

        let model_flag = Self::model_flag(model);
        if let Some(ref m) = model_flag {
            args.push("--model".to_string());
            args.push(m.clone());
        }

        args
    }

    /// Build a text prompt from the completion request messages.
    fn build_prompt(request: &CompletionRequest) -> String {
        let mut parts = Vec::new();

        if let Some(ref sys) = request.system {
            parts.push(format!("[System]\n{sys}"));
        }

        for msg in &request.messages {
            let role_label = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
            };
            let text = msg.content.text_content();
            if !text.is_empty() {
                parts.push(format!("[{role_label}]\n{text}"));
            }
        }

        parts.join("\n\n")
    }

    /// Map a model ID like "qwen-code/qwen3-coder" to CLI --model flag value.
    fn model_flag(model: &str) -> Option<String> {
        let stripped = model.strip_prefix("qwen-code/").unwrap_or(model);
        match stripped {
            "qwen3-coder" | "coder" => Some("qwen3-coder".to_string()),
            "qwen-coder-plus" | "coder-plus" => Some("qwen-coder-plus".to_string()),
            "qwq-32b" | "qwq" => Some("qwq-32b".to_string()),
            _ => Some(stripped.to_string()),
        }
    }

    /// Apply security env filtering to a command.
    fn apply_env_filter(cmd: &mut tokio::process::Command) {
        for key in SENSITIVE_ENV_EXACT {
            cmd.env_remove(key);
        }
        for (key, _) in std::env::vars() {
            if key.starts_with("QWEN_") {
                continue;
            }
            let upper = key.to_uppercase();
            for suffix in SENSITIVE_SUFFIXES {
                if upper.ends_with(suffix) {
                    cmd.env_remove(&key);
                    break;
                }
            }
        }
    }

    fn command_for_request(
        &self,
        request: &CompletionRequest,
        streaming: bool,
    ) -> tokio::process::Command {
        let prompt = Self::build_prompt(request);
        let args = self.build_args(&prompt, &request.model, streaming);
        let mut cmd = tokio::process::Command::new(&self.cli_path);
        for arg in &args {
            cmd.arg(arg);
        }
        Self::apply_env_filter(&mut cmd);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd
    }
}

/// JSON output from `qwen -p --output-format json`.
#[derive(Debug, Deserialize)]
struct QwenJsonOutput {
    result: Option<String>,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    usage: Option<QwenUsage>,
    #[serde(default)]
    #[allow(dead_code)]
    cost_usd: Option<f64>,
}

/// Usage stats from Qwen CLI JSON output.
#[derive(Debug, Deserialize, Default)]
struct QwenUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

fn token_usage_from_qwen(usage: QwenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        ..Default::default()
    }
}

fn text_response(text: String, usage: TokenUsage) -> CompletionResponse {
    CompletionResponse {
        content: vec![ContentBlock::Text {
            text,
            provider_metadata: None,
        }],
        stop_reason: StopReason::EndTurn,
        tool_calls: Vec::new(),
        usage,
    }
}

fn response_from_qwen_stdout(stdout: &str) -> CompletionResponse {
    if let Ok(parsed) = serde_json::from_str::<QwenJsonOutput>(stdout) {
        let text = parsed
            .result
            .or(parsed.content)
            .or(parsed.text)
            .unwrap_or_default();
        return text_response(
            text,
            parsed.usage.map(token_usage_from_qwen).unwrap_or_default(),
        );
    }
    text_response(stdout.trim().to_string(), TokenUsage::default())
}

/// Stream JSON event from `qwen -p --output-format stream-json`.
#[derive(Debug, Deserialize)]
struct QwenStreamEvent {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    usage: Option<QwenUsage>,
}

#[derive(Default)]
struct QwenStreamState {
    full_text: String,
    final_usage: TokenUsage,
}

impl QwenStreamState {
    fn ingest_event(&mut self, event: QwenStreamEvent) -> Option<String> {
        match event.r#type.as_str() {
            "content" | "text" | "assistant" | "content_block_delta" => {
                self.push_optional_content(event.content)
            }
            "result" | "done" | "complete" => {
                let delta = if self.full_text.is_empty() {
                    event.result.inspect(|result| {
                        self.full_text = result.clone();
                    })
                } else {
                    None
                };
                if let Some(usage) = event.usage {
                    self.final_usage = token_usage_from_qwen(usage);
                }
                delta
            }
            _ => self.push_optional_content(event.content),
        }
    }

    fn ingest_plain_line(&mut self, line: String) -> String {
        self.full_text.push_str(&line);
        line
    }

    fn into_response(self) -> CompletionResponse {
        text_response(self.full_text, self.final_usage)
    }

    fn push_optional_content(&mut self, content: Option<String>) -> Option<String> {
        content.inspect(|content| {
            self.full_text.push_str(content);
        })
    }
}

fn qwen_spawn_error(error: std::io::Error) -> LlmError {
    LlmError::Http(format!(
        "Qwen Code CLI not found or failed to start ({}). \
         Install: npm install -g @qwen-code/qwen-code && qwen auth",
        error
    ))
}

fn qwen_exit_error(code: i32, detail: &str) -> LlmError {
    let message = if detail.contains("not authenticated")
        || detail.contains("auth")
        || detail.contains("login")
        || detail.contains("credentials")
    {
        format!("Qwen Code CLI is not authenticated. Run: qwen auth\nDetail: {detail}")
    } else {
        format!("Qwen Code CLI exited with code {code}: {detail}")
    };
    LlmError::Api {
        status: code as u16,
        message,
    }
}

#[async_trait]
impl LlmDriver for QwenCodeDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut cmd = self.command_for_request(&request, false);

        debug!(cli = %self.cli_path, skip_permissions = self.skip_permissions, "Spawning Qwen Code CLI");

        let output = cmd.output().await.map_err(qwen_spawn_error)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if !stderr.is_empty() { &stderr } else { &stdout };
            let code = output.status.code().unwrap_or(1);
            return Err(qwen_exit_error(code, detail));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(response_from_qwen_stdout(&stdout))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut cmd = self.command_for_request(&request, true);

        debug!(cli = %self.cli_path, skip_permissions = self.skip_permissions, "Spawning Qwen Code CLI (streaming)");

        let mut child = cmd.spawn().map_err(qwen_spawn_error)?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| LlmError::Http("No stdout from qwen CLI".to_string()))?;

        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();

        let mut state = QwenStreamState::default();

        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<QwenStreamEvent>(&line) {
                Ok(event) => {
                    if let Some(text) = state.ingest_event(event) {
                        let _ = tx.send(StreamEvent::TextDelta { text }).await;
                    }
                }
                Err(e) => {
                    warn!(line = %line, error = %e, "Non-JSON line from Qwen CLI");
                    let text = state.ingest_plain_line(line);
                    let _ = tx.send(StreamEvent::TextDelta { text }).await;
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| LlmError::Http(format!("Qwen CLI wait failed: {e}")))?;

        if !status.success() {
            warn!(code = ?status.code(), "Qwen CLI exited with error");
        }

        let response = state.into_response();
        let _ = tx
            .send(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: response.usage,
            })
            .await;

        Ok(response)
    }
}

/// Check if the Qwen Code CLI is available.
pub fn qwen_code_available() -> bool {
    QwenCodeDriver::detect().is_some() || qwen_credentials_exist()
}

/// Check if Qwen credentials exist.
fn qwen_credentials_exist() -> bool {
    if let Some(home) = home_dir() {
        let qwen_dir = home.join(".qwen");
        qwen_dir.join("credentials.json").exists()
            || qwen_dir.join(".credentials.json").exists()
            || qwen_dir.join("auth.json").exists()
    } else {
        false
    }
}

/// Cross-platform home directory.
fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(std::path::PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(std::path::PathBuf::from)
    }
}

#[cfg(test)]
#[path = "qwen_code_tests.rs"]
mod tests;

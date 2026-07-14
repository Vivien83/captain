//! Agent-related types: identity, manifests, state, and scheduling.

use crate::tool::ToolDefinition;
use crate::tool_compat::normalize_tool_name;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// Unique identifier for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UserId(pub Uuid);

impl UserId {
    /// Generate a new random UserId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for UserId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Model routing configuration — auto-selects cheap/mid/expensive models by complexity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelRoutingConfig {
    /// Model to use for simple queries.
    pub simple_model: String,
    /// Model to use for medium-complexity queries.
    pub medium_model: String,
    /// Model to use for complex queries.
    pub complex_model: String,
    /// Token count threshold: below this = simple.
    pub simple_threshold: u32,
    /// Token count threshold: above this = complex.
    pub complex_threshold: u32,
}

impl Default for ModelRoutingConfig {
    fn default() -> Self {
        Self {
            simple_model: "anthropic/claude-haiku-4.5".to_string(),
            medium_model: "anthropic/claude-sonnet-4.6".to_string(),
            complex_model: "anthropic/claude-sonnet-4.6".to_string(),
            simple_threshold: 100,
            complex_threshold: 500,
        }
    }
}

/// Autonomous agent configuration — guardrails for 24/7 agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutonomousConfig {
    /// Cron expression for quiet hours (e.g., "0 22 * * *" to "0 6 * * *").
    pub quiet_hours: Option<String>,
    /// Maximum iterations per invocation (overrides global MAX_ITERATIONS).
    pub max_iterations: u32,
    /// Maximum restarts before the agent is permanently stopped.
    pub max_restarts: u32,
    /// Heartbeat interval in seconds.
    pub heartbeat_interval_secs: u64,
    /// Channel to send heartbeat status to (e.g., "telegram", "discord").
    pub heartbeat_channel: Option<String>,
}

impl Default for AutonomousConfig {
    fn default() -> Self {
        Self {
            quiet_hours: None,
            max_iterations: 50,
            max_restarts: 10,
            heartbeat_interval_secs: 30,
            heartbeat_channel: None,
        }
    }
}

/// Hook event types that can be intercepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Fires before a tool call is executed. Handler can block the call.
    BeforeToolCall,
    /// Fires after a tool call completes.
    AfterToolCall,
    /// Fires before the system prompt is constructed.
    BeforePromptBuild,
    /// Fires after the agent loop completes.
    AgentLoopEnd,
}

/// Unique identifier for an agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub Uuid);

impl AgentId {
    /// Generate a new random AgentId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a deterministic AgentId from a string using SHA-1 namespace.
    /// Useful for hand agents that need stable IDs across restarts.
    pub fn from_string(s: &str) -> Self {
        const NAMESPACE: Uuid = Uuid::NAMESPACE_DNS;
        Self(Uuid::new_v5(&NAMESPACE, s.as_bytes()))
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for AgentId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Unique identifier for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    /// Create a new random SessionId.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The current lifecycle state of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent has been created but not yet started.
    Created,
    /// Agent is actively running and processing events.
    Running,
    /// Agent is paused and not processing events.
    Suspended,
    /// Agent has been terminated and cannot be resumed.
    Terminated,
    /// Agent crashed and is awaiting recovery.
    Crashed,
}

/// Permission-based operational mode for an agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMode {
    /// Read-only: agent can observe but cannot call any tools.
    Observe,
    /// Restricted: agent can only call read-only tools (file_read, file_list, memory_recall, web_fetch, web_search, document_extract).
    Assist,
    /// Unrestricted: agent can use all granted tools.
    #[default]
    Full,
}

impl AgentMode {
    /// Filter a tool list based on this mode.
    pub fn filter_tools(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        match self {
            Self::Observe => vec![],
            Self::Assist => {
                let read_only = [
                    "file_read",
                    "file_list",
                    "memory_recall",
                    "document_extract",
                    "web_fetch",
                    "web_search",
                    "agent_list",
                ];
                tools
                    .into_iter()
                    .filter(|t| read_only.contains(&t.name.as_str()))
                    .collect()
            }
            Self::Full => tools,
        }
    }
}

/// How an agent dispatches requests across models / sub-agents.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationMode {
    /// Automatic tier routing — cheap model for simple queries, expensive for
    /// complex ones. Uses `build_default_routing` based on the agent's model family.
    #[default]
    Routing,
    /// Primary model acts as an orchestrator that delegates multi-step work to
    /// worker sub-agents via agent_spawn/task_post. The prompt receives explicit
    /// delegation instructions.
    Delegation,
    /// No routing, no delegation — the configured model is used as-is for every
    /// request. Most predictable but most expensive on complex models.
    Pinned,
}

/// How an agent is scheduled to run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleMode {
    /// Agent wakes up when a message/event arrives (default).
    #[default]
    Reactive,
    /// Agent wakes up on a cron schedule.
    Periodic { cron: String },
    /// Agent monitors conditions and acts when thresholds are met.
    Proactive { conditions: Vec<String> },
    /// Agent runs in a persistent loop.
    Continuous {
        #[serde(default = "default_check_interval")]
        check_interval_secs: u64,
    },
}

fn default_check_interval() -> u64 {
    60
}

/// Resource limits for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResourceQuota {
    /// Maximum WASM memory in bytes.
    pub max_memory_bytes: u64,
    /// Maximum CPU time per invocation in milliseconds.
    pub max_cpu_time_ms: u64,
    /// Maximum tool calls per minute.
    pub max_tool_calls_per_minute: u32,
    /// Maximum LLM tokens per hour.
    pub max_llm_tokens_per_hour: u64,
    /// Maximum network bytes per hour.
    pub max_network_bytes_per_hour: u64,
    /// Maximum cost in USD per hour.
    pub max_cost_per_hour_usd: f64,
    /// Maximum cost in USD per day (0.0 = unlimited).
    pub max_cost_per_day_usd: f64,
    /// Maximum cost in USD per month (0.0 = unlimited).
    pub max_cost_per_month_usd: f64,
}

impl Default for ResourceQuota {
    fn default() -> Self {
        Self {
            max_memory_bytes: 256 * 1024 * 1024, // 256 MB
            max_cpu_time_ms: 30_000,             // 30 seconds
            max_tool_calls_per_minute: 60,
            // Was 0 (unlimited). A background/continuous agent left running
            // has no other automatic ceiling on LLM token consumption, so an
            // unbounded default let a single agent run unattended for weeks
            // and consume well over 100M tokens (see incident: `researcher-hand`,
            // ~131M tokens across 8k+ autonomous ticks). 200_000/hour matches
            // the value already recommended in the TUI agent scaffold template
            // (`tui/screens/agents.rs`). Explicit `0` in a manifest still means
            // unlimited for agents that genuinely need it.
            max_llm_tokens_per_hour: 200_000,
            max_network_bytes_per_hour: 100 * 1024 * 1024, // 100 MB
            max_cost_per_hour_usd: 0.0,                    // unlimited by default
            max_cost_per_day_usd: 0.0,                     // unlimited
            max_cost_per_month_usd: 0.0,                   // unlimited
        }
    }
}

/// Agent priority level for scheduling.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    /// Low priority.
    Low = 0,
    /// Normal priority (default).
    #[default]
    Normal = 1,
    /// High priority.
    High = 2,
    /// Critical priority.
    Critical = 3,
}

/// Named tool presets — expand to tool lists + derived capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolProfile {
    Minimal,
    Coding,
    Research,
    Messaging,
    Automation,
    #[default]
    Full,
    Custom,
}

impl ToolProfile {
    /// Expand profile to tool name list.
    pub fn tools(&self) -> Vec<String> {
        match self {
            Self::Minimal => vec!["file_read", "file_list"],
            Self::Coding => vec![
                "file_read",
                "file_write",
                "file_list",
                "shell_exec",
                "web_fetch",
            ],
            Self::Research => vec![
                "web_research_batch",
                "web_fetch",
                "web_download",
                "web_search",
                "document_extract",
                "document_create",
                "document_pipeline",
                "file_read",
                "file_write",
            ],
            Self::Messaging => vec!["agent_send", "agent_list", "memory_store", "memory_recall"],
            Self::Automation => vec![
                "file_read",
                "file_write",
                "file_list",
                "shell_exec",
                "web_fetch",
                "web_search",
                "agent_send",
                "agent_list",
                "memory_store",
                "memory_recall",
            ],
            Self::Full | Self::Custom => vec!["*"],
        }
        .into_iter()
        .map(String::from)
        .collect()
    }

    /// Derive ManifestCapabilities implied by this profile.
    pub fn implied_capabilities(&self) -> ManifestCapabilities {
        let tools = self.tools();
        let has_net = tools.iter().any(|t| t.starts_with("web_") || t == "*");
        let has_shell = tools.iter().any(|t| t == "shell_exec" || t == "*");
        let has_agent = tools.iter().any(|t| t.starts_with("agent_") || t == "*");
        let has_memory = tools.iter().any(|t| t.starts_with("memory_") || t == "*");
        ManifestCapabilities {
            tools,
            network: if has_net { vec!["*".into()] } else { vec![] },
            shell: if has_shell { vec!["*".into()] } else { vec![] },
            agent_spawn: has_agent,
            agent_message: if has_agent { vec!["*".into()] } else { vec![] },
            memory_read: if has_memory {
                vec!["*".into()]
            } else {
                vec!["self.*".into()]
            },
            memory_write: vec!["self.*".into()],
            ofp_discover: false,
            ofp_connect: vec![],
        }
    }
}

/// LLM model configuration for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelConfig {
    /// LLM provider name.
    pub provider: String,
    /// Model identifier.
    #[serde(alias = "name")]
    pub model: String,
    /// Maximum tokens for completion.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// System prompt for the agent.
    pub system_prompt: String,
    /// Optional API key environment variable name.
    pub api_key_env: Option<String>,
    /// Optional base URL override for the provider.
    pub base_url: Option<String>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 4096,
            temperature: 0.7,
            system_prompt: "You are a helpful AI agent.".to_string(),
            api_key_env: None,
            base_url: None,
        }
    }
}

pub const AGENT_MANIFEST_CANONICAL_EXAMPLE: &str = r#"name = "veille-technologique"
description = "Agent specialise dans la veille technologique."
module = "builtin:chat"
tool_allowlist = ["web_research_batch", "web_fetch", "memory_recall", "memory_save"]

[model]
provider = "codex"
model = "gpt-5.5"
system_prompt = "Tu es un agent de veille technologique. Utilise des sources reelles, cite-les, et signale les incertitudes."
"#;

/// Render an actionable, public-safe manifest parse error.
///
/// The original TOML is intentionally not echoed back because manifests may
/// contain local paths, provider settings, or future secret references.
pub fn format_agent_manifest_parse_error(error: &toml::de::Error, manifest_toml: &str) -> String {
    let raw_error_text = error.to_string();
    let error_text = public_toml_error_summary(&raw_error_text);
    let manifest_lower = manifest_toml.to_ascii_lowercase();
    let mut hints = Vec::new();

    if raw_error_text.contains("expected struct ModelConfig")
        || manifest_lower
            .lines()
            .any(|line| line.trim_start().starts_with("model ="))
    {
        hints.push(
            "`model` must be a TOML table, not a string. Use `[model]` with `provider` and `model`.",
        );
    }
    if manifest_lower.contains("[tools]")
        || manifest_lower.contains("allow = [")
        || manifest_lower.contains("allowlist = [")
    {
        hints.push(
            "`tools` is a map of per-tool config tables. For an agent tool surface, use top-level `tool_allowlist = [...]` or `[capabilities] tools = [...]`.",
        );
    }
    if !manifest_lower.contains("tool_allowlist") && !manifest_lower.contains("[capabilities]") {
        hints.push(
            "Sub-agents must declare an explicit non-wildcard tool surface with `tool_allowlist` or `[capabilities] tools`.",
        );
    }
    if hints.is_empty() {
        hints.push("Use the canonical manifest shape below and retry once.");
    }

    format!(
        "Invalid manifest: {error_text}\n\nRecovery:\n- {}\n\nCanonical minimal manifest:\n```toml\n{AGENT_MANIFEST_CANONICAL_EXAMPLE}```",
        hints.join("\n- ")
    )
}

/// Auto-repair the single most common malformed manifest shape observed
/// live: a flat top-level `model = "..."` string (often with a sibling flat
/// `provider = "..."`) instead of a `[model]` table. The tool description
/// and `format_agent_manifest_parse_error` hint already warn about this
/// exact mistake, but it recurred 10+ times across one session — repairing
/// the shape before parsing is more reliable than hoping the model reads
/// the hint. Returns `None` when the shape doesn't match (nothing to
/// repair, or `model` is already a table), leaving the original error path
/// untouched.
pub fn repair_flat_model_fields(manifest_toml: &str) -> Option<String> {
    let mut value: toml::Value = toml::from_str(manifest_toml).ok()?;
    let table = value.as_table_mut()?;

    let model_str = match table.get("model") {
        Some(toml::Value::String(s)) => s.clone(),
        _ => return None,
    };
    let provider = match table.remove("provider") {
        Some(toml::Value::String(s)) => s,
        _ => ModelConfig::default().provider,
    };

    let mut model_table = toml::value::Table::new();
    model_table.insert("provider".to_string(), toml::Value::String(provider));
    model_table.insert("model".to_string(), toml::Value::String(model_str));
    table.insert("model".to_string(), toml::Value::Table(model_table));

    toml::to_string(&value).ok()
}

fn public_toml_error_summary(error_text: &str) -> String {
    error_text
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            trimmed != "|" && !trimmed.contains(" | ") && !trimmed.starts_with('^')
        })
        .map(redact_toml_error_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_toml_error_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(idx) = rest.find("string \"") {
        out.push_str(&rest[..idx]);
        out.push_str("string <redacted>");
        let after_marker = &rest[idx + "string \"".len()..];
        let Some(end_quote) = after_marker.find('"') else {
            rest = "";
            break;
        };
        rest = &after_marker[end_quote + 1..];
    }
    out.push_str(rest);
    out
}

/// A fallback model entry in a chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackModel {
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Tool configuration within an agent manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    /// Tool-specific configuration parameters.
    pub params: HashMap<String, serde_json::Value>,
}

/// Complete agent manifest — defines everything about an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentManifest {
    /// Human-readable agent name.
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// Description of what this agent does.
    pub description: String,
    /// Author identifier.
    pub author: String,
    /// Path to the agent module (WASM or Python file).
    pub module: String,
    /// Scheduling mode.
    pub schedule: ScheduleMode,
    /// LLM model configuration.
    pub model: ModelConfig,
    /// Fallback model chain — tried in order if the primary model fails.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub fallback_models: Vec<FallbackModel>,
    /// Resource quotas.
    pub resources: ResourceQuota,
    /// Priority level.
    pub priority: Priority,
    /// Capability grants (parsed into Capability enum by kernel).
    pub capabilities: ManifestCapabilities,
    /// Named tool profile — expands to tool list + derived capabilities.
    #[serde(default)]
    pub profile: Option<ToolProfile>,
    /// Tool-specific configurations.
    #[serde(default, deserialize_with = "crate::serde_compat::map_lenient")]
    pub tools: HashMap<String, ToolConfig>,
    /// Installed skill references (empty = all skills available).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub skills: Vec<String>,
    /// MCP server allowlist (empty = all connected MCP servers available).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub mcp_servers: Vec<String>,
    /// Custom metadata.
    #[serde(default, deserialize_with = "crate::serde_compat::map_lenient")]
    pub metadata: HashMap<String, serde_json::Value>,
    /// Tags for agent discovery and categorization.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub tags: Vec<String>,
    /// Model routing configuration — auto-select models by complexity.
    #[serde(default)]
    pub routing: Option<ModelRoutingConfig>,
    /// Autonomous agent configuration — guardrails for 24/7 agents.
    #[serde(default)]
    pub autonomous: Option<AutonomousConfig>,
    /// Pinned model override (used in Stable mode).
    #[serde(default)]
    pub pinned_model: Option<String>,
    /// Agent workspace directory. Auto-created on spawn.
    /// Default: `{workspaces_dir}/{agent_name}-{agent_id_prefix}/`
    #[serde(default)]
    pub workspace: Option<PathBuf>,
    /// Whether to generate workspace identity files (SOUL.md, USER.md, etc.) on creation.
    #[serde(default = "default_true")]
    pub generate_identity_files: bool,
    /// Per-agent exec policy override. If None, uses global exec_policy.
    /// Accepts string shorthand ("allow", "deny", "full", "allowlist") or full table.
    #[serde(default, deserialize_with = "crate::serde_compat::exec_policy_lenient")]
    pub exec_policy: Option<crate::config::ExecPolicy>,
    /// Tool allowlist — only these tools are available (empty = all tools).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub tool_allowlist: Vec<String>,
    /// Tool blocklist — these tools are excluded (applied after allowlist).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub tool_blocklist: Vec<String>,
    /// How this agent dispatches requests (auto-routing / delegation / pinned).
    #[serde(default)]
    pub orchestration_mode: OrchestrationMode,
}

fn default_true() -> bool {
    true
}

impl Default for AgentManifest {
    fn default() -> Self {
        Self {
            name: "unnamed".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            author: String::new(),
            module: "builtin:chat".to_string(),
            schedule: ScheduleMode::default(),
            model: ModelConfig::default(),
            fallback_models: Vec::new(),
            resources: ResourceQuota::default(),
            priority: Priority::default(),
            capabilities: ManifestCapabilities::default(),
            profile: None,
            tools: HashMap::new(),
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            metadata: HashMap::new(),
            tags: Vec::new(),
            routing: None,
            autonomous: None,
            pinned_model: None,
            workspace: None,
            generate_identity_files: true,
            exec_policy: None,
            tool_allowlist: Vec::new(),
            tool_blocklist: Vec::new(),
            orchestration_mode: OrchestrationMode::default(),
        }
    }
}

/// Capability declarations in a manifest (human-readable TOML format).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ManifestCapabilities {
    /// Allowed network hosts (e.g., ["api.anthropic.com:443"]).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub network: Vec<String>,
    /// Allowed tool IDs.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub tools: Vec<String>,
    /// Memory read scopes.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub memory_read: Vec<String>,
    /// Memory write scopes.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub memory_write: Vec<String>,
    /// Whether this agent can spawn sub-agents.
    pub agent_spawn: bool,
    /// Agent message patterns (e.g., ["*"] or ["agent-name"]).
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub agent_message: Vec<String>,
    /// Allowed shell commands.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub shell: Vec<String>,
    /// Whether this agent can discover remote agents via OFP.
    pub ofp_discover: bool,
    /// Allowed OFP peer patterns.
    #[serde(default, deserialize_with = "crate::serde_compat::vec_lenient")]
    pub ofp_connect: Vec<String>,
}

/// Derive the effective manifest capabilities used for operator views and
/// kernel grants.
///
/// `tool_allowlist` is the strict tool surface introduced after legacy
/// `capabilities.tools`; when present it takes priority and drives derived
/// network/memory/shell capabilities.
pub fn effective_manifest_capabilities(manifest: &AgentManifest) -> ManifestCapabilities {
    let has_tool_allowlist = !manifest.tool_allowlist.is_empty();
    let has_capability_tools = !manifest.capabilities.tools.is_empty();
    let mut effective = if let Some(profile) = &manifest.profile {
        if !has_tool_allowlist && !has_capability_tools {
            merge_profile_capabilities(profile.implied_capabilities(), &manifest.capabilities)
        } else {
            manifest.capabilities.clone()
        }
    } else {
        manifest.capabilities.clone()
    };

    if has_tool_allowlist {
        effective.tools =
            normalized_effective_tools(&manifest.tool_allowlist, &manifest.tool_blocklist);
    } else if has_capability_tools || !manifest.tool_blocklist.is_empty() {
        effective.tools = normalized_effective_tools(&effective.tools, &manifest.tool_blocklist);
    } else if manifest.profile.is_none() {
        // No allowlist, no capabilities.tools, no profile: the real dispatcher
        // (`kernel_tool_runtime::available_tools`) treats this as unrestricted
        // and grants every builtin tool. Mirror that here with the existing
        // "*" wildcard convention (already used for network/memory scopes)
        // instead of reporting an empty list that contradicts what the agent
        // can actually do (e.g. the principal `captain` agent).
        effective.tools = vec!["*".to_string()];
    }

    let tools = effective.tools.clone();
    grant_capability_implications_for_tools(&mut effective, &tools);
    effective
}

fn merge_profile_capabilities(
    mut base: ManifestCapabilities,
    overrides: &ManifestCapabilities,
) -> ManifestCapabilities {
    if !overrides.network.is_empty() {
        base.network = overrides.network.clone();
    }
    if !overrides.shell.is_empty() {
        base.shell = overrides.shell.clone();
    }
    if !overrides.agent_message.is_empty() {
        base.agent_message = overrides.agent_message.clone();
    }
    if overrides.agent_spawn {
        base.agent_spawn = true;
    }
    if !overrides.memory_read.is_empty() {
        base.memory_read = overrides.memory_read.clone();
    }
    if !overrides.memory_write.is_empty() {
        base.memory_write = overrides.memory_write.clone();
    }
    if overrides.ofp_discover {
        base.ofp_discover = true;
    }
    if !overrides.ofp_connect.is_empty() {
        base.ofp_connect = overrides.ofp_connect.clone();
    }
    base
}

fn normalized_effective_tools(tools: &[String], blocklist: &[String]) -> Vec<String> {
    let blocked = blocklist
        .iter()
        .map(|tool| normalize_tool_name(tool))
        .collect::<Vec<_>>();
    let mut normalized = Vec::new();
    for tool in tools {
        let name = normalize_tool_name(tool);
        if blocked.contains(&name) {
            continue;
        }
        if !normalized.iter().any(|existing| existing == name) {
            normalized.push(name.to_string());
        }
    }
    normalized
}

fn grant_capability_implications_for_tools(caps: &mut ManifestCapabilities, tools: &[String]) {
    let has_tool = |name: &str| tools.iter().any(|tool| tool == name || tool == "*");
    if tools
        .iter()
        .any(|tool| tool == "*" || tool.starts_with("web_") || tool == "browser_batch")
        && caps.network.is_empty()
    {
        caps.network.push("*".to_string());
    }
    if has_tool("shell_exec") && caps.shell.is_empty() {
        caps.shell.push("*".to_string());
    }
    if tools
        .iter()
        .any(|tool| tool == "*" || tool.starts_with("agent_"))
    {
        caps.agent_spawn = caps.agent_spawn || has_tool("agent_spawn") || has_tool("*");
        if caps.agent_message.is_empty() {
            caps.agent_message.push("*".to_string());
        }
    }
    if tools
        .iter()
        .any(|tool| tool == "*" || tool == "memory_recall" || tool == "memory_store")
        && caps.memory_read.is_empty()
    {
        caps.memory_read.push("self.*".to_string());
    }
    if tools
        .iter()
        .any(|tool| tool == "*" || tool == "memory_save" || tool == "memory_store")
        && caps.memory_write.is_empty()
    {
        caps.memory_write.push("self.*".to_string());
    }
}

/// Human-readable session label (e.g., "support inbox", "research").
/// Max 128 chars, alphanumeric + spaces + hyphens + underscores only.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SessionLabel(String);

impl SessionLabel {
    /// Create a new validated session label.
    pub fn new(label: &str) -> Result<Self, crate::error::CaptainError> {
        let trimmed = label.trim();
        if trimmed.is_empty() || trimmed.len() > 128 {
            return Err(crate::error::CaptainError::InvalidInput(
                "Session label must be 1-128 chars".into(),
            ));
        }
        if !trimmed
            .chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
        {
            return Err(crate::error::CaptainError::InvalidInput(
                "Session label contains invalid chars".into(),
            ));
        }
        Ok(Self(trimmed.to_string()))
    }

    /// Get the label as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Visual identity for an agent — emoji, avatar, color, personality.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentIdentity {
    /// Single emoji character for quick visual identification.
    pub emoji: Option<String>,
    /// Avatar URL (http/https) or data URI.
    pub avatar_url: Option<String>,
    /// Hex color code (e.g., "#FF5C00") for UI accent.
    pub color: Option<String>,
    /// Archetype: "researcher", "coder", "assistant", "writer", "devops", "support", "analyst".
    pub archetype: Option<String>,
    /// Personality vibe: "professional", "friendly", "technical", "creative", "concise", "mentor".
    pub vibe: Option<String>,
    /// Greeting style: "warm", "formal", "playful", "brief".
    pub greeting_style: Option<String>,
}

/// A registered agent entry in the kernel's registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEntry {
    /// Unique agent ID.
    pub id: AgentId,
    /// Human-readable name.
    pub name: String,
    /// Full manifest.
    pub manifest: AgentManifest,
    /// Current lifecycle state.
    pub state: AgentState,
    /// Permission-based operational mode.
    #[serde(default)]
    pub mode: AgentMode,
    /// When the agent was created.
    pub created_at: DateTime<Utc>,
    /// When the agent was last active.
    pub last_active: DateTime<Utc>,
    /// Parent agent (if spawned by another agent).
    pub parent: Option<AgentId>,
    /// Child agents spawned by this agent.
    pub children: Vec<AgentId>,
    /// Active session ID.
    pub session_id: SessionId,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Visual identity for dashboard display.
    #[serde(default)]
    pub identity: AgentIdentity,
    /// Whether onboarding (bootstrap) has been completed.
    #[serde(default)]
    pub onboarding_completed: bool,
    /// When onboarding was completed.
    #[serde(default)]
    pub onboarding_completed_at: Option<DateTime<Utc>>,
    /// Current mission for managers — survives reboot so the agent can resume.
    #[serde(default)]
    pub mission: Option<String>,
    /// When the current mission was assigned.
    #[serde(default)]
    pub mission_set_at: Option<DateTime<Utc>>,
    /// Autoscale config for fleet managers (None = disabled).
    #[serde(default)]
    pub autoscale: Option<AutoScaleConfig>,
    /// Timestamp of the last autoscale spawn/kill event (for cooldown).
    #[serde(default)]
    pub last_scale_event: Option<DateTime<Utc>>,
}

/// Auto-scaling configuration for a fleet manager.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutoScaleConfig {
    /// Whether autoscaling is active.
    #[serde(default = "default_autoscale_enabled")]
    pub enabled: bool,
    /// Minimum number of workers to keep alive.
    #[serde(default)]
    pub min_workers: u32,
    /// Absolute maximum number of workers.
    #[serde(default = "default_max_workers")]
    pub max_workers: u32,
    /// Queue depth at/above which a new worker is spawned.
    #[serde(default = "default_spawn_threshold")]
    pub spawn_threshold: u32,
    /// Queue depth at/below which an idle worker is killed.
    #[serde(default)]
    pub kill_threshold: u32,
    /// Minimum seconds between two scale events (prevents thrashing).
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
    /// Optional TOML manifest fragment used when spawning a new worker.
    #[serde(default)]
    pub worker_template: Option<String>,
}

fn default_autoscale_enabled() -> bool {
    true
}
fn default_max_workers() -> u32 {
    3
}
fn default_spawn_threshold() -> u32 {
    2
}
fn default_cooldown() -> u64 {
    60
}

impl Default for AutoScaleConfig {
    fn default() -> Self {
        Self {
            enabled: default_autoscale_enabled(),
            min_workers: 0,
            max_workers: default_max_workers(),
            spawn_threshold: default_spawn_threshold(),
            kill_threshold: 0,
            cooldown_secs: default_cooldown(),
            worker_template: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_id_uniqueness() {
        let id1 = AgentId::new();
        let id2 = AgentId::new();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_agent_id_display() {
        let id = AgentId::new();
        let display = format!("{}", id);
        assert!(!display.is_empty());
        assert_eq!(display.len(), 36); // UUID v4 string length
    }

    #[test]
    fn test_agent_id_serialization() {
        let id = AgentId::new();
        let json = serde_json::to_string(&id).unwrap();
        let deserialized: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_default_resource_quota() {
        let quota = ResourceQuota::default();
        assert_eq!(quota.max_memory_bytes, 256 * 1024 * 1024);
        assert_eq!(quota.max_cpu_time_ms, 30_000);
        assert_eq!(quota.max_llm_tokens_per_hour, 200_000);
    }

    #[test]
    fn test_user_id_uniqueness() {
        let u1 = UserId::new();
        let u2 = UserId::new();
        assert_ne!(u1, u2);
    }

    #[test]
    fn test_user_id_roundtrip() {
        let u = UserId::new();
        let json = serde_json::to_string(&u).unwrap();
        let back: UserId = serde_json::from_str(&json).unwrap();
        assert_eq!(u, back);
    }

    #[test]
    fn test_model_routing_config_defaults() {
        let cfg = ModelRoutingConfig::default();
        assert!(!cfg.simple_model.is_empty());
        assert!(cfg.simple_threshold < cfg.complex_threshold);
    }

    #[test]
    fn test_model_routing_config_serde() {
        let cfg = ModelRoutingConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        let back: ModelRoutingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.simple_model, cfg.simple_model);
    }

    #[test]
    fn test_autonomous_config_defaults() {
        let cfg = AutonomousConfig::default();
        assert_eq!(cfg.max_iterations, 50);
        assert_eq!(cfg.max_restarts, 10);
        assert_eq!(cfg.heartbeat_interval_secs, 30);
        assert!(cfg.quiet_hours.is_none());
    }

    #[test]
    fn test_autonomous_config_serde() {
        let cfg = AutonomousConfig {
            quiet_hours: Some("0 22 * * *".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AutonomousConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.quiet_hours, Some("0 22 * * *".to_string()));
    }

    #[test]
    fn test_manifest_with_routing_and_autonomous() {
        let manifest = AgentManifest {
            routing: Some(ModelRoutingConfig::default()),
            autonomous: Some(AutonomousConfig::default()),
            pinned_model: Some("claude-sonnet-4-20250514".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        assert!(back.routing.is_some());
        assert!(back.autonomous.is_some());
        assert_eq!(
            back.pinned_model,
            Some("claude-sonnet-4-20250514".to_string())
        );
    }

    #[test]
    fn test_agent_manifest_serialization() {
        let manifest = AgentManifest {
            name: "test-agent".to_string(),
            version: "0.1.0".to_string(),
            description: "A test agent".to_string(),
            author: "test".to_string(),
            module: "test.wasm".to_string(),
            schedule: ScheduleMode::default(),
            model: ModelConfig::default(),
            fallback_models: vec![],
            resources: ResourceQuota::default(),
            priority: Priority::default(),
            capabilities: ManifestCapabilities::default(),
            profile: None,
            tools: HashMap::new(),
            skills: vec![],
            mcp_servers: vec![],
            metadata: HashMap::new(),
            tags: vec!["test".to_string()],
            routing: None,
            autonomous: None,
            pinned_model: None,
            workspace: None,
            generate_identity_files: true,
            exec_policy: None,
            tool_allowlist: Vec::new(),
            tool_blocklist: Vec::new(),
            orchestration_mode: OrchestrationMode::default(),
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let deserialized: AgentManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test-agent");
        assert_eq!(deserialized.tags, vec!["test".to_string()]);
    }

    // ----- ToolProfile tests -----

    #[test]
    fn test_tool_profile_minimal() {
        let tools = ToolProfile::Minimal.tools();
        assert_eq!(tools, vec!["file_read", "file_list"]);
    }

    #[test]
    fn test_tool_profile_coding() {
        let tools = ToolProfile::Coding.tools();
        assert!(tools.contains(&"file_read".to_string()));
        assert!(tools.contains(&"shell_exec".to_string()));
        assert!(tools.contains(&"web_fetch".to_string()));
        assert_eq!(tools.len(), 5);
    }

    #[test]
    fn test_tool_profile_research() {
        let tools = ToolProfile::Research.tools();
        assert!(tools.contains(&"web_research_batch".to_string()));
        assert!(tools.contains(&"web_fetch".to_string()));
        assert!(tools.contains(&"web_download".to_string()));
        assert!(tools.contains(&"web_search".to_string()));
        assert!(tools.contains(&"document_extract".to_string()));
        assert!(tools.contains(&"document_pipeline".to_string()));
        assert_eq!(tools.len(), 9);
    }

    #[test]
    fn test_tool_profile_messaging() {
        let tools = ToolProfile::Messaging.tools();
        assert!(tools.contains(&"agent_send".to_string()));
        assert!(tools.contains(&"memory_recall".to_string()));
        assert_eq!(tools.len(), 4);
    }

    #[test]
    fn test_tool_profile_automation() {
        let tools = ToolProfile::Automation.tools();
        assert_eq!(tools.len(), 10);
    }

    #[test]
    fn test_tool_profile_full() {
        let tools = ToolProfile::Full.tools();
        assert_eq!(tools, vec!["*"]);
    }

    #[test]
    fn test_tool_profile_implied_capabilities_coding() {
        let caps = ToolProfile::Coding.implied_capabilities();
        assert!(caps.network.contains(&"*".to_string())); // web_fetch
        assert!(caps.shell.contains(&"*".to_string())); // shell_exec
        assert!(!caps.agent_spawn); // no agent_* tools
        assert!(caps.agent_message.is_empty());
    }

    #[test]
    fn test_tool_profile_implied_capabilities_messaging() {
        let caps = ToolProfile::Messaging.implied_capabilities();
        assert!(caps.network.is_empty());
        assert!(caps.shell.is_empty());
        assert!(caps.agent_spawn);
        assert!(caps.agent_message.contains(&"*".to_string()));
        assert!(caps.memory_read.contains(&"*".to_string()));
    }

    #[test]
    fn test_tool_profile_implied_capabilities_minimal() {
        let caps = ToolProfile::Minimal.implied_capabilities();
        assert!(caps.network.is_empty());
        assert!(caps.shell.is_empty());
        assert!(!caps.agent_spawn);
        assert_eq!(caps.memory_read, vec!["self.*".to_string()]);
    }

    #[test]
    fn repair_flat_model_fields_lifts_real_world_broken_manifest() {
        // The exact shape observed live: flat `model`/`provider` string keys
        // instead of a `[model]` table, repeated 10+ times in one session.
        let broken = r#"
name = "ephemeral-smoke-test-20260701-2024"
description = "Agent temporaire de verification cleanup ephemeral"
model = "gpt-5.5"
provider = "codex"
tags = ["ephemeral"]
tool_allowlist = ["memory_recall", "system_time", "channel_send"]

[budget]
max_tool_calls_per_min = 10
"#;
        assert!(toml::from_str::<AgentManifest>(broken).is_err());

        let repaired = repair_flat_model_fields(broken).expect("repairable shape");
        let manifest: AgentManifest = toml::from_str(&repaired).expect("repaired manifest parses");

        assert_eq!(manifest.model.provider, "codex");
        assert_eq!(manifest.model.model, "gpt-5.5");
        assert_eq!(manifest.name, "ephemeral-smoke-test-20260701-2024");
        assert_eq!(
            manifest.tool_allowlist,
            vec!["memory_recall", "system_time", "channel_send"]
        );
    }

    #[test]
    fn repair_flat_model_fields_defaults_provider_when_absent() {
        let broken = "name = \"x\"\nmodel = \"gpt-5.5\"\n";
        let repaired = repair_flat_model_fields(broken).expect("repairable shape");
        let manifest: AgentManifest = toml::from_str(&repaired).expect("repaired manifest parses");

        assert_eq!(manifest.model.model, "gpt-5.5");
        assert_eq!(manifest.model.provider, ModelConfig::default().provider);
    }

    #[test]
    fn repair_flat_model_fields_returns_none_for_already_valid_table() {
        let valid = "name = \"x\"\n[model]\nprovider = \"codex\"\nmodel = \"gpt-5.5\"\n";
        assert!(repair_flat_model_fields(valid).is_none());
    }

    #[test]
    fn repair_flat_model_fields_returns_none_when_model_key_absent() {
        let no_model = "name = \"x\"\n";
        assert!(repair_flat_model_fields(no_model).is_none());
    }

    #[test]
    fn effective_manifest_capabilities_reports_unrestricted_agent_as_all_tools() {
        // No tool_allowlist, no capabilities.tools, no profile: this is the
        // exact shape of the principal `captain` agent's default manifest
        // (`kernel_boot_default_agent.rs`). The real dispatcher grants every
        // builtin tool in this case (`kernel_tool_runtime::available_tools`),
        // so the reported capabilities must match instead of showing "none".
        let manifest = AgentManifest::default();

        let caps = effective_manifest_capabilities(&manifest);

        assert_eq!(caps.tools, vec!["*".to_string()]);
        assert_eq!(caps.network, vec!["*".to_string()]);
        assert_eq!(caps.shell, vec!["*".to_string()]);
        assert!(caps.agent_spawn);
    }

    #[test]
    fn effective_manifest_capabilities_uses_tool_allowlist() {
        let manifest = AgentManifest {
            tool_allowlist: vec![
                "web_fetch".to_string(),
                "memory_recall".to_string(),
                "memory_save".to_string(),
            ],
            ..Default::default()
        };

        let caps = effective_manifest_capabilities(&manifest);

        assert_eq!(
            caps.tools,
            vec![
                "web_fetch".to_string(),
                "memory_recall".to_string(),
                "memory_save".to_string()
            ]
        );
        assert_eq!(caps.network, vec!["*".to_string()]);
        assert_eq!(caps.memory_read, vec!["self.*".to_string()]);
        assert_eq!(caps.memory_write, vec!["self.*".to_string()]);
    }

    #[test]
    fn effective_manifest_capabilities_blocks_and_prioritizes_tool_allowlist() {
        let manifest = AgentManifest {
            capabilities: ManifestCapabilities {
                tools: vec!["shell_exec".to_string()],
                ..Default::default()
            },
            tool_allowlist: vec!["web_fetch".to_string(), "shell_exec".to_string()],
            tool_blocklist: vec!["shell_exec".to_string()],
            ..Default::default()
        };

        let caps = effective_manifest_capabilities(&manifest);

        assert_eq!(caps.tools, vec!["web_fetch".to_string()]);
        assert_eq!(caps.network, vec!["*".to_string()]);
        assert!(caps.shell.is_empty());
    }

    #[test]
    fn test_tool_profile_serde_roundtrip() {
        let profile = ToolProfile::Coding;
        let json = serde_json::to_string(&profile).unwrap();
        assert_eq!(json, "\"coding\"");
        let back: ToolProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ToolProfile::Coding);
    }

    // ----- AgentMode tests -----

    #[test]
    fn test_agent_mode_default() {
        assert_eq!(AgentMode::default(), AgentMode::Full);
    }

    #[test]
    fn test_agent_mode_observe_filters_all() {
        let tools = vec![
            ToolDefinition {
                name: "file_read".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "shell_exec".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
        ];
        let filtered = AgentMode::Observe.filter_tools(tools);
        assert!(filtered.is_empty());
    }

    #[test]
    fn test_agent_mode_assist_filters_write_tools() {
        let tools = vec![
            ToolDefinition {
                name: "file_read".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "file_write".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "shell_exec".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "web_fetch".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "document_extract".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "memory_recall".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
        ];
        let filtered = AgentMode::Assist.filter_tools(tools);
        assert_eq!(filtered.len(), 4);
        let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"web_fetch"));
        assert!(names.contains(&"document_extract"));
        assert!(names.contains(&"memory_recall"));
        assert!(!names.contains(&"file_write"));
        assert!(!names.contains(&"shell_exec"));
    }

    #[test]
    fn test_agent_mode_full_passes_all() {
        let tools = vec![
            ToolDefinition {
                name: "file_read".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
            ToolDefinition {
                name: "shell_exec".into(),
                description: String::new(),
                input_schema: serde_json::Value::Null,
            },
        ];
        let filtered = AgentMode::Full.filter_tools(tools);
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn test_agent_mode_serde_roundtrip() {
        let mode = AgentMode::Assist;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"assist\"");
        let back: AgentMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, AgentMode::Assist);
    }

    // ----- FallbackModel tests -----

    #[test]
    fn test_fallback_model_serde() {
        let fb = FallbackModel {
            provider: "groq".to_string(),
            model: "llama-3.3-70b".to_string(),
            api_key_env: Some("GROQ_API_KEY".to_string()),
            base_url: None,
        };
        let json = serde_json::to_string(&fb).unwrap();
        let back: FallbackModel = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, "groq");
        assert_eq!(back.model, "llama-3.3-70b");
        assert_eq!(back.api_key_env, Some("GROQ_API_KEY".to_string()));
    }

    #[test]
    fn test_manifest_with_new_fields() {
        let manifest = AgentManifest {
            profile: Some(ToolProfile::Coding),
            fallback_models: vec![FallbackModel {
                provider: "groq".to_string(),
                model: "llama-3.3-70b".to_string(),
                api_key_env: None,
                base_url: None,
            }],
            ..Default::default()
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let back: AgentManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.profile, Some(ToolProfile::Coding));
        assert_eq!(back.fallback_models.len(), 1);
    }

    #[test]
    fn test_agent_entry_with_mode() {
        let entry = AgentEntry {
            id: AgentId::new(),
            name: "test".to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Running,
            mode: AgentMode::Assist,
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec![],
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            mission: None,
            mission_set_at: None,
            autoscale: None,
            last_scale_event: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: AgentEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.mode, AgentMode::Assist);
    }

    #[test]
    fn test_agent_identity_default() {
        let id = AgentIdentity::default();
        assert!(id.emoji.is_none());
        assert!(id.avatar_url.is_none());
        assert!(id.color.is_none());
        assert!(id.archetype.is_none());
        assert!(id.vibe.is_none());
        assert!(id.greeting_style.is_none());
    }

    #[test]
    fn test_agent_identity_serde_roundtrip() {
        let id = AgentIdentity {
            emoji: Some("\u{1F916}".to_string()),
            avatar_url: Some("https://example.com/avatar.png".to_string()),
            color: Some("#FF5C00".to_string()),
            archetype: Some("assistant".to_string()),
            vibe: Some("friendly".to_string()),
            greeting_style: Some("warm".to_string()),
        };
        let json = serde_json::to_string(&id).unwrap();
        let back: AgentIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(back.emoji, Some("\u{1F916}".to_string()));
        assert_eq!(back.color, Some("#FF5C00".to_string()));
    }

    #[test]
    fn test_agent_identity_deserialize_missing_fields() {
        // AgentIdentity should deserialize from empty JSON thanks to #[serde(default)]
        let id: AgentIdentity = serde_json::from_str("{}").unwrap();
        assert!(id.emoji.is_none());
    }

    #[test]
    fn test_autoscale_config_defaults() {
        let cfg = AutoScaleConfig::default();
        assert_eq!(cfg.min_workers, 0);
        assert_eq!(cfg.max_workers, 3);
        assert_eq!(cfg.spawn_threshold, 2);
        assert_eq!(cfg.kill_threshold, 0);
        assert_eq!(cfg.cooldown_secs, 60);
        assert!(cfg.enabled);
    }

    #[test]
    fn test_autoscale_config_roundtrip() {
        let cfg = AutoScaleConfig {
            enabled: true,
            min_workers: 1,
            max_workers: 5,
            spawn_threshold: 3,
            kill_threshold: 1,
            cooldown_secs: 120,
            worker_template: Some("[manifest]".to_string()),
        };
        let json = serde_json::to_string(&cfg).unwrap();
        let back: AutoScaleConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn test_mission_roundtrip_via_json() {
        let entry = AgentEntry {
            id: AgentId::new(),
            name: "mgr".to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec!["manager".into()],
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            mission: Some("Analyser le marché X".to_string()),
            mission_set_at: Some(Utc::now()),
            autoscale: Some(AutoScaleConfig::default()),
            last_scale_event: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: AgentEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.mission.as_deref(), Some("Analyser le marché X"));
        assert!(back.autoscale.is_some());
    }

    #[test]
    fn test_agent_entry_legacy_decode_without_new_fields() {
        let entry = AgentEntry {
            id: AgentId::new(),
            name: "legacy".to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec![],
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            mission: None,
            mission_set_at: None,
            autoscale: None,
            last_scale_event: None,
        };
        let mut value = serde_json::to_value(&entry).unwrap();
        let obj = value.as_object_mut().unwrap();
        obj.remove("mission");
        obj.remove("mission_set_at");
        obj.remove("autoscale");
        obj.remove("last_scale_event");
        let back: AgentEntry = serde_json::from_value(value).unwrap();
        assert!(back.mission.is_none());
        assert!(back.autoscale.is_none());
    }

    #[test]
    fn test_agent_entry_identity_in_serde() {
        let entry = AgentEntry {
            id: AgentId::new(),
            name: "bot".to_string(),
            manifest: AgentManifest::default(),
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec![],
            identity: AgentIdentity {
                emoji: Some("\u{1F525}".to_string()),
                avatar_url: None,
                color: Some("#00FF00".to_string()),
                ..Default::default()
            },
            onboarding_completed: false,
            onboarding_completed_at: None,
            mission: None,
            mission_set_at: None,
            autoscale: None,
            last_scale_event: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let back: AgentEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.identity.emoji, Some("\u{1F525}".to_string()));
        assert_eq!(back.identity.color, Some("#00FF00".to_string()));
        assert!(back.identity.avatar_url.is_none());
    }

    // ----- SessionLabel tests -----

    #[test]
    fn test_session_label_valid() {
        let label = SessionLabel::new("support inbox").unwrap();
        assert_eq!(label.as_str(), "support inbox");
    }

    #[test]
    fn test_session_label_with_hyphens_underscores() {
        let label = SessionLabel::new("my-session_2024").unwrap();
        assert_eq!(label.as_str(), "my-session_2024");
    }

    #[test]
    fn test_session_label_trims_whitespace() {
        let label = SessionLabel::new("  research  ").unwrap();
        assert_eq!(label.as_str(), "research");
    }

    #[test]
    fn test_session_label_rejects_empty() {
        assert!(SessionLabel::new("").is_err());
        assert!(SessionLabel::new("   ").is_err());
    }

    #[test]
    fn test_session_label_rejects_too_long() {
        let long = "a".repeat(129);
        assert!(SessionLabel::new(&long).is_err());
    }

    #[test]
    fn test_session_label_rejects_special_chars() {
        assert!(SessionLabel::new("hello@world").is_err());
        assert!(SessionLabel::new("path/traversal").is_err());
        assert!(SessionLabel::new("<script>").is_err());
    }

    #[test]
    fn test_session_label_serde_roundtrip() {
        let label = SessionLabel::new("test label").unwrap();
        let json = serde_json::to_string(&label).unwrap();
        let back: SessionLabel = serde_json::from_str(&json).unwrap();
        assert_eq!(label, back);
    }

    // ----- generate_identity_files field tests -----

    #[test]
    fn test_manifest_generate_identity_files_default_true() {
        let manifest = AgentManifest::default();
        assert!(manifest.generate_identity_files);
    }

    #[test]
    fn test_manifest_generate_identity_files_serde() {
        let json = r#"{"name":"test","generate_identity_files":false}"#;
        let manifest: AgentManifest = serde_json::from_str(json).unwrap();
        assert!(!manifest.generate_identity_files);
    }

    #[test]
    fn test_manifest_generate_identity_files_defaults_on_missing() {
        let json = r#"{"name":"test"}"#;
        let manifest: AgentManifest = serde_json::from_str(json).unwrap();
        assert!(manifest.generate_identity_files);
    }

    // ----- ModelConfig alias tests -----

    #[test]
    fn test_model_config_name_alias_toml() {
        let toml_str = r#"
name = "llama-3.3-70b-versatile"
provider = "groq"
"#;
        let cfg: ModelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.model, "llama-3.3-70b-versatile");
        assert_eq!(cfg.provider, "groq");
    }

    #[test]
    fn test_model_config_model_field_still_works() {
        let toml_str = r#"
model = "gpt-4o"
provider = "openai"
"#;
        let cfg: ModelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.model, "gpt-4o");
        assert_eq!(cfg.provider, "openai");
    }

    // ----- Multi-line system_prompt TOML tests (wizard generateToml output) -----

    #[test]
    fn test_manifest_multiline_system_prompt_toml() {
        // This is the exact TOML format the dashboard wizard generateToml() now produces
        let toml_str = r#"
name = "brand-guardian"
module = "builtin:chat"

[model]
provider = "google"
model = "gemini-3-flash-preview"
system_prompt = """
You are Brand Guardian, an expert brand strategist.

Your Core Mission:
- Develop brand strategy including purpose, vision, mission, values
- Design complete visual identity systems
- Establish brand voice and messaging architecture

Critical Rules:
- Establish comprehensive brand foundation before tactical implementation
- Ensure all brand elements work as a cohesive system
"""
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "brand-guardian");
        assert_eq!(manifest.model.provider, "google");
        assert_eq!(manifest.model.model, "gemini-3-flash-preview");
        assert!(manifest.model.system_prompt.contains("Brand Guardian"));
        assert!(manifest.model.system_prompt.contains("Critical Rules:"));
        // Verify newlines are preserved
        assert!(manifest.model.system_prompt.contains('\n'));
    }

    #[test]
    fn test_manifest_multiline_system_prompt_with_quotes() {
        // System prompt containing double quotes (common in persona prompts)
        let toml_str = r#"
name = "test-agent"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = """
You are a "helpful" assistant.
When users say "hello", respond warmly.
"""
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.model.system_prompt.contains("\"helpful\""));
        assert!(manifest.model.system_prompt.contains("\"hello\""));
    }

    #[test]
    fn test_manifest_multiline_system_prompt_with_code_blocks() {
        // System prompt containing markdown-style code blocks
        let toml_str = r#"
name = "coder"

[model]
provider = "deepseek"
model = "deepseek-chat"
system_prompt = """
You are a coding assistant.

Example output format:
```python
def hello():
    print("world")
```

Always use proper indentation.
"""
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert!(manifest.model.system_prompt.contains("```python"));
        assert!(manifest.model.system_prompt.contains("def hello()"));
    }

    #[test]
    fn test_manifest_single_line_system_prompt_still_works() {
        // Ensure the old single-line format still parses fine
        let toml_str = r#"
name = "simple"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are a helpful assistant."
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.model.system_prompt, "You are a helpful assistant.");
    }

    #[test]
    fn test_manifest_wizard_custom_profile_with_capabilities() {
        // Full wizard output when profile=custom with capabilities block
        let toml_str = r#"
name = "brand-guardian"
module = "builtin:chat"

[model]
provider = "google"
model = "gemini-3-flash-preview"
system_prompt = """
You are Brand Guardian.
Protect brand consistency across all touchpoints.
"""

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.name, "brand-guardian");
        assert!(manifest.model.system_prompt.contains("Brand Guardian"));
        assert_eq!(manifest.capabilities.memory_read, vec!["*".to_string()]);
        assert_eq!(
            manifest.capabilities.memory_write,
            vec!["self.*".to_string()]
        );
    }

    #[test]
    fn agent_manifest_parse_error_guides_model_table_shape() {
        let toml_str = r#"
name = "veille-technologique"
model = "codex:gpt-5.5"
tool_allowlist = ["web_research_batch"]
"#;
        let err = toml::from_str::<AgentManifest>(toml_str).unwrap_err();
        let rendered = format_agent_manifest_parse_error(&err, toml_str);

        assert!(rendered.contains("`model` must be a TOML table"));
        assert!(rendered.contains("[model]"));
        assert!(rendered.contains("provider = \"codex\""));
        assert!(rendered.contains("tool_allowlist"));
        assert!(!rendered.contains("codex:gpt-5.5"));
    }

    #[test]
    fn agent_manifest_parse_error_guides_tool_allowlist_shape() {
        let toml_str = r#"
name = "veille-technologique"

[model]
provider = "codex"
model = "gpt-5.5"

[tools]
allow = ["web_search"]
"#;
        let err = toml::from_str::<AgentManifest>(toml_str).unwrap_err();
        let rendered = format_agent_manifest_parse_error(&err, toml_str);

        assert!(rendered.contains("`tools` is a map"));
        assert!(rendered.contains("top-level `tool_allowlist = [...]`"));
        assert!(rendered.contains("[capabilities] tools"));
        assert!(!rendered.contains("web_search\"]"));
    }
}

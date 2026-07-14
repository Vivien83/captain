//! Configuration types for the Captain kernel.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

mod agent_runtime;
mod auth;
mod automation;
mod budget;
mod canvas;
mod channel_active;
mod channel_behavior;
mod channel_frozen;
mod channel_routing;
mod channels;
mod docker;
mod execution;
mod extensions;
mod learning;
mod memory;
mod model_defaults;
mod network;
mod pairing;
mod protocols;
mod secret_fields;
mod validation;
mod voice;
mod web;
pub use agent_runtime::*;
pub use auth::*;
pub use automation::*;
pub use budget::*;
pub use canvas::*;
pub use channel_active::*;
pub use channel_behavior::*;
pub use channel_frozen::*;
pub use channel_routing::*;
pub use channels::*;
pub use docker::*;
pub use execution::*;
pub use extensions::*;
pub use learning::*;
pub use memory::*;
pub use model_defaults::*;
pub use network::*;
pub use pairing::*;
pub use protocols::*;
pub use secret_fields::*;
pub use voice::*;
pub use web::*;

/// Deserialize a `Vec<String>` that tolerates both string and integer elements.
///
/// When channel configs are saved from setup/API tools, numeric IDs (e.g. Discord
/// guild snowflakes, Telegram user IDs) are stored as TOML integers. This helper
/// transparently converts integers back to strings so deserialization never fails.
fn deserialize_string_or_int_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values: Vec<serde_json::Value> = serde::Deserialize::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .map(|v| match v {
            serde_json::Value::String(s) => s,
            serde_json::Value::Number(n) => n.to_string(),
            other => other.to_string(),
        })
        .collect())
}

/// Controls what usage info appears in response footers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageFooterMode {
    /// Don't show usage info.
    Off,
    /// Show token counts only.
    Tokens,
    /// Show estimated cost only.
    Cost,
    /// Show tokens + cost (default).
    #[default]
    Full,
}

/// Kernel operating mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KernelMode {
    /// Conservative mode — no auto-updates, pinned models, stability-first.
    Stable,
    /// Default balanced mode.
    #[default]
    Default,
    /// Developer mode — experimental features enabled.
    Dev,
}

/// Top-level kernel configuration.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KernelConfig {
    /// Captain home directory (default: ~/.captain).
    pub home_dir: PathBuf,
    /// Data directory for databases (default: ~/.captain/data).
    pub data_dir: PathBuf,
    /// Log level (trace, debug, info, warn, error).
    pub log_level: String,
    /// API listen address (e.g., "0.0.0.0:4200").
    #[serde(alias = "listen_addr")]
    pub api_listen: String,
    /// Whether to enable the OFP network layer.
    pub network_enabled: bool,
    /// Default LLM provider configuration.
    pub default_model: DefaultModelConfig,
    /// Agent loop guardrails for normal turns.
    #[serde(default)]
    pub agent_loop: AgentLoopConfig,
    /// Memory substrate configuration.
    pub memory: MemoryConfig,
    /// Network configuration.
    pub network: NetworkConfig,
    /// Channel bridge configuration (Telegram, etc.).
    pub channels: ChannelsConfig,
    /// API authentication key. When set, all API endpoints (except /api/health)
    /// require a `Authorization: Bearer <key>` header.
    /// If empty, the API is unauthenticated (local development only).
    pub api_key: String,
    /// Kernel operating mode (stable, default, dev).
    #[serde(default)]
    pub mode: KernelMode,
    /// Language/locale for CLI and messages (default: "en").
    #[serde(default = "default_language")]
    pub language: String,
    /// User-facing assistant name and answer style.
    #[serde(default)]
    pub assistant: AssistantConfig,
    /// User configurations for RBAC multi-user support.
    #[serde(default)]
    pub users: Vec<UserConfig>,
    /// MCP server configurations for external tool integration.
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfigEntry>,
    /// A2A (Agent-to-Agent) protocol configuration.
    #[serde(default)]
    pub a2a: Option<A2aConfig>,
    /// Usage footer mode (what to show after each response).
    #[serde(default)]
    pub usage_footer: UsageFooterMode,
    /// Web tools configuration (search + fetch).
    #[serde(default)]
    pub web: WebConfig,
    /// Embedded browser terminal configuration.
    #[serde(default)]
    pub web_terminal: WebTerminalConfig,
    /// Deployment metadata captured during setup/install.
    #[serde(default)]
    pub deployment: DeploymentConfig,
    /// Product surface gates for the Hermes-level core refactor.
    #[serde(default)]
    pub surfaces: ProductSurfacesConfig,
    /// Fallback providers tried in order if the primary fails.
    /// Configure in config.toml as `[[fallback_providers]]`.
    #[serde(default)]
    pub fallback_providers: Vec<FallbackProviderConfig>,
    /// Browser automation configuration.
    #[serde(default)]
    pub browser: BrowserConfig,
    /// Extensions & integrations configuration.
    #[serde(default)]
    pub extensions: ExtensionsConfig,
    /// Credential vault configuration.
    #[serde(default)]
    pub vault: VaultConfig,
    /// Root directory for agent workspaces. Default: `~/.captain/workspaces`
    #[serde(default)]
    pub workspaces_dir: Option<PathBuf>,
    /// Captain principal-agent unsandbox extension. Holds extra
    /// directories the user explicitly authorized via the
    /// `workspace_add` tool. Captain can read/write inside these on
    /// top of `~/.captain/`; subagents are unaffected.
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    /// Media understanding configuration.
    #[serde(default)]
    pub media: crate::media::MediaConfig,
    /// Link understanding configuration.
    #[serde(default)]
    pub links: crate::media::LinkConfig,
    /// Config hot-reload settings.
    #[serde(default)]
    pub reload: ReloadConfig,
    /// Default IANA timezone for scheduling (e.g., "Europe/Paris").
    /// When cron jobs don't specify a tz, this is used instead of UTC.
    /// Auto-detected from system if not set in config.toml.
    #[serde(default = "default_timezone")]
    pub timezone: String,
    /// Webhook trigger configuration (external event injection).
    #[serde(default)]
    pub webhook_triggers: Option<WebhookTriggerConfig>,
    /// Native outbound webhooks for Captain lifecycle events.
    #[serde(default)]
    pub outbound_webhooks: OutboundWebhooksConfig,
    /// Execution approval policy.
    #[serde(default, alias = "approval_policy")]
    pub approval: crate::approval::ApprovalPolicy,
    /// Cron scheduler max total jobs across all agents. Default: 500.
    #[serde(default = "default_max_cron_jobs")]
    pub max_cron_jobs: usize,
    /// Config include files — loaded and deep-merged before the root config.
    /// Paths are relative to the root config file's directory.
    /// Security: absolute paths and `..` components are rejected.
    #[serde(default)]
    pub include: Vec<String>,
    /// Shell/exec security policy.
    #[serde(default)]
    pub exec_policy: ExecPolicy,
    /// Agent bindings for multi-account routing.
    #[serde(default)]
    pub bindings: Vec<AgentBinding>,
    /// Broadcast routing configuration.
    #[serde(default)]
    pub broadcast: BroadcastConfig,
    /// Auto-reply background engine configuration.
    #[serde(default)]
    pub auto_reply: AutoReplyConfig,
    /// Canvas (A2UI) configuration.
    #[serde(default)]
    pub canvas: CanvasConfig,
    /// Text-to-speech configuration.
    #[serde(default)]
    pub tts: TtsConfig,
    /// Live browser voice-call configuration.
    #[serde(default)]
    pub voice_call: VoiceCallConfig,
    /// Docker container sandbox configuration.
    #[serde(default)]
    pub docker: DockerSandboxConfig,
    /// Device pairing configuration.
    #[serde(default)]
    pub pairing: PairingConfig,
    /// Auth profiles for key rotation (provider name → profiles).
    #[serde(default)]
    pub auth_profiles: HashMap<String, Vec<AuthProfile>>,
    /// Extended thinking configuration.
    #[serde(default)]
    pub thinking: Option<ThinkingConfig>,
    /// Global spending budget configuration.
    #[serde(default)]
    pub budget: BudgetConfig,
    /// Provider base URL overrides (provider ID → custom base URL).
    /// e.g. `ollama = "http://192.168.1.100:11434/v1"`
    #[serde(default)]
    pub provider_urls: HashMap<String, String>,
    /// Provider API key env var overrides (provider ID → env var name).
    /// For custom/unknown providers, maps the provider name to the environment
    /// variable holding the API key. e.g. `nvidia = "NVIDIA_API_KEY"`.
    /// If not set, the convention `{PROVIDER_UPPER}_API_KEY` is used automatically.
    #[serde(default)]
    pub provider_api_keys: HashMap<String, String>,
    /// OAuth client ID overrides for PKCE flows.
    #[serde(default)]
    pub oauth: OAuthConfig,
    /// Web authentication (username/password login).
    #[serde(default)]
    pub auth: AuthConfig,
    /// Directory for auto-loading workflow JSON files on startup.
    /// Defaults to `~/.captain/workflows`. Set to empty string to disable.
    #[serde(default)]
    pub workflows_dir: Option<PathBuf>,
    /// STT model for voice transcription (whisper-small, whisper-large-v3, voxtral-4b, mistral-api).
    #[serde(default = "default_stt_model")]
    pub stt_model: String,
    /// Neural pulse interval in seconds (default: 300 = 5 min).
    #[serde(default = "default_pulse_interval_secs")]
    pub pulse_interval_secs: u64,
    /// Dream cycle interval in hours (default: 6).
    #[serde(default = "default_dream_interval_hours")]
    pub dream_interval_hours: u64,
    /// Minimum hours between Telegram digests (default: 2).
    #[serde(default = "default_digest_min_interval_hours")]
    pub digest_min_interval_hours: u64,
    /// Enable Telegram consciousness digests (default: false).
    #[serde(default)]
    pub digest_enabled: bool,
    /// v3.12 LearningEngine configuration.
    #[serde(default)]
    pub learning: LearningConfig,
    /// Session checkpoint summarizer configuration.
    #[serde(default)]
    pub checkpoints: CheckpointConfig,
    /// v3.13 SkillSynthesizer configuration.
    #[serde(default)]
    pub skills: SkillsConfig,
}

fn default_max_cron_jobs() -> usize {
    500
}

fn default_language() -> String {
    "en".to_string()
}

fn default_stt_model() -> String {
    "whisper-small".to_string()
}

/// Closed set of values accepted by `stt_model` — shared by the voice API
/// route and the config_write tool so both reject the same way.
pub const ALLOWED_STT_MODELS: &[&str] = &[
    "whisper-small",
    "whisper-large-v3",
    "voxtral-4b",
    "mistral-api",
];

fn default_pulse_interval_secs() -> u64 {
    120 // 2 minutes
}

fn default_dream_interval_hours() -> u64 {
    6
}

fn default_digest_min_interval_hours() -> u64 {
    2
}

fn default_timezone() -> String {
    // Try to detect system timezone from /etc/localtime or TZ env var
    if let Ok(tz) = std::env::var("TZ") {
        if !tz.is_empty() {
            return tz;
        }
    }
    // macOS/Linux: read /etc/localtime symlink target
    #[cfg(unix)]
    {
        if let Ok(link) = std::fs::read_link("/etc/localtime") {
            let path = link.to_string_lossy();
            if let Some(pos) = path.find("/zoneinfo/") {
                return path[pos + 10..].to_string();
            }
        }
    }
    "UTC".to_string()
}

fn default_true() -> bool {
    true
}

fn default_thread_ttl() -> u64 {
    24
}

impl Default for KernelConfig {
    fn default() -> Self {
        let home_dir = captain_home_dir();
        Self {
            data_dir: home_dir.join("data"),
            home_dir,
            log_level: "info".to_string(),
            api_listen: "127.0.0.1:50051".to_string(),
            network_enabled: false,
            default_model: DefaultModelConfig::default(),
            agent_loop: AgentLoopConfig::default(),
            memory: MemoryConfig::default(),
            network: NetworkConfig::default(),
            channels: ChannelsConfig::default(),
            api_key: String::new(),
            mode: KernelMode::default(),
            language: "en".to_string(),
            assistant: AssistantConfig::default(),
            users: Vec::new(),
            mcp_servers: Vec::new(),
            a2a: None,
            usage_footer: UsageFooterMode::default(),
            web: WebConfig::default(),
            web_terminal: WebTerminalConfig::default(),
            deployment: DeploymentConfig::default(),
            surfaces: ProductSurfacesConfig::default(),
            fallback_providers: Vec::new(),
            browser: BrowserConfig::default(),
            extensions: ExtensionsConfig::default(),
            vault: VaultConfig::default(),
            workspaces_dir: None,
            workspace: WorkspaceConfig::default(),
            media: crate::media::MediaConfig::default(),
            links: crate::media::LinkConfig::default(),
            reload: ReloadConfig::default(),
            timezone: default_timezone(),
            webhook_triggers: None,
            outbound_webhooks: OutboundWebhooksConfig::default(),
            approval: crate::approval::ApprovalPolicy::default(),
            max_cron_jobs: default_max_cron_jobs(),
            include: Vec::new(),
            exec_policy: ExecPolicy::default(),
            bindings: Vec::new(),
            broadcast: BroadcastConfig::default(),
            auto_reply: AutoReplyConfig::default(),
            canvas: CanvasConfig::default(),
            tts: TtsConfig::default(),
            voice_call: VoiceCallConfig::default(),
            docker: DockerSandboxConfig::default(),
            pairing: PairingConfig::default(),
            auth_profiles: HashMap::new(),
            thinking: None,
            budget: BudgetConfig::default(),
            provider_urls: HashMap::new(),
            provider_api_keys: HashMap::new(),
            oauth: OAuthConfig::default(),
            auth: AuthConfig::default(),
            workflows_dir: None,
            stt_model: default_stt_model(),
            pulse_interval_secs: default_pulse_interval_secs(),
            dream_interval_hours: default_dream_interval_hours(),
            digest_min_interval_hours: default_digest_min_interval_hours(),
            digest_enabled: false,
            learning: LearningConfig::default(),
            checkpoints: CheckpointConfig::default(),
            skills: SkillsConfig::default(),
        }
    }
}

impl KernelConfig {
    /// Resolved workspaces root directory.
    pub fn effective_workspaces_dir(&self) -> PathBuf {
        self.workspaces_dir
            .clone()
            .unwrap_or_else(|| self.home_dir.join("workspaces"))
    }

    /// Resolve the API key env var name for a provider.
    ///
    /// Checks: 1) explicit `provider_api_keys` mapping, 2) `auth_profiles` first entry,
    /// 3) convention `{PROVIDER_UPPER}_API_KEY`.
    pub fn resolve_api_key_env(&self, provider: &str) -> String {
        // 1. Explicit mapping in [provider_api_keys]
        if let Some(env_var) = self.provider_api_keys.get(provider) {
            return env_var.clone();
        }
        // 2. Auth profiles (first profile by priority)
        if let Some(profiles) = self.auth_profiles.get(provider) {
            let mut sorted: Vec<_> = profiles.iter().collect();
            sorted.sort_by_key(|p| p.priority);
            if let Some(best) = sorted.first() {
                return best.api_key_env.clone();
            }
        }
        // 3. Convention: NVIDIA → NVIDIA_API_KEY
        format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"))
    }
}

/// SECURITY: Custom Debug impl redacts sensitive fields (api_key).
impl std::fmt::Debug for KernelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelConfig")
            .field("home_dir", &self.home_dir)
            .field("data_dir", &self.data_dir)
            .field("log_level", &self.log_level)
            .field("api_listen", &self.api_listen)
            .field("network_enabled", &self.network_enabled)
            .field("default_model", &self.default_model)
            .field("agent_loop", &self.agent_loop)
            .field("memory", &self.memory)
            .field("network", &self.network)
            .field("channels", &self.channels)
            .field(
                "api_key",
                &if self.api_key.is_empty() {
                    "<empty>"
                } else {
                    "<redacted>"
                },
            )
            .field("mode", &self.mode)
            .field("language", &self.language)
            .field("assistant", &self.assistant)
            .field("users", &format!("{} user(s)", self.users.len()))
            .field(
                "mcp_servers",
                &format!("{} server(s)", self.mcp_servers.len()),
            )
            .field("a2a", &self.a2a.as_ref().map(|a| a.enabled))
            .field("usage_footer", &self.usage_footer)
            .field("web", &self.web)
            .field("web_terminal", &self.web_terminal)
            .field("deployment", &self.deployment)
            .field("surfaces", &self.surfaces)
            .field(
                "fallback_providers",
                &format!("{} provider(s)", self.fallback_providers.len()),
            )
            .field("browser", &self.browser)
            .field("extensions", &self.extensions)
            .field("vault", &format!("enabled={}", self.vault.enabled))
            .field("workspaces_dir", &self.workspaces_dir)
            .field(
                "media",
                &format!(
                    "image={} audio={} video={}",
                    self.media.image_description,
                    self.media.audio_transcription,
                    self.media.video_description
                ),
            )
            .field("links", &format!("enabled={}", self.links.enabled))
            .field("reload", &self.reload.mode)
            .field(
                "webhook_triggers",
                &self.webhook_triggers.as_ref().map(|w| w.enabled),
            )
            .field("outbound_webhooks", &self.outbound_webhooks.enabled)
            .field(
                "approval",
                &format!("{} tool(s)", self.approval.require_approval.len()),
            )
            .field("max_cron_jobs", &self.max_cron_jobs)
            .field("include", &format!("{} file(s)", self.include.len()))
            .field("exec_policy", &self.exec_policy.mode)
            .field("bindings", &format!("{} binding(s)", self.bindings.len()))
            .field(
                "broadcast",
                &format!("{} route(s)", self.broadcast.routes.len()),
            )
            .field(
                "auto_reply",
                &format!("enabled={}", self.auto_reply.enabled),
            )
            .field("canvas", &format!("enabled={}", self.canvas.enabled))
            .field("tts", &format!("enabled={}", self.tts.enabled))
            .field(
                "voice_call",
                &format!(
                    "enabled={}, provider={}, model={}",
                    self.voice_call.enabled, self.voice_call.provider, self.voice_call.model
                ),
            )
            .field("docker", &format!("enabled={}", self.docker.enabled))
            .field("pairing", &format!("enabled={}", self.pairing.enabled))
            .field(
                "auth_profiles",
                &format!("{} provider(s)", self.auth_profiles.len()),
            )
            .field("thinking", &self.thinking.is_some())
            .field(
                "provider_api_keys",
                &format!("{} mapping(s)", self.provider_api_keys.len()),
            )
            .field("checkpoints", &self.checkpoints)
            .field("auth", &format!("enabled={}", self.auth.enabled))
            .finish()
    }
}

/// Resolve the Captain home directory.
///
/// Priority: `CAPTAIN_HOME` env var > `~/.captain`.
fn captain_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("CAPTAIN_HOME") {
        return PathBuf::from(home);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".captain")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = KernelConfig::default();
        assert_eq!(config.log_level, "info");
        assert_eq!(config.api_listen, "127.0.0.1:50051");
        assert!(!config.network_enabled);
    }

    #[test]
    fn test_config_serialization() {
        let config = KernelConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("log_level"));
    }

    #[test]
    fn test_validate_no_channels() {
        let config = KernelConfig::default();
        let warnings = config.validate();
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_kernel_mode_default() {
        let mode = KernelMode::default();
        assert_eq!(mode, KernelMode::Default);
    }

    #[test]
    fn test_kernel_mode_serde() {
        let stable = KernelMode::Stable;
        let json = serde_json::to_string(&stable).unwrap();
        assert_eq!(json, "\"stable\"");
        let back: KernelMode = serde_json::from_str(&json).unwrap();
        assert_eq!(back, KernelMode::Stable);
    }

    #[test]
    fn test_user_config_serde() {
        let uc = UserConfig {
            name: "Alice".to_string(),
            role: "owner".to_string(),
            channel_bindings: {
                let mut m = std::collections::HashMap::new();
                m.insert("telegram".to_string(), "123456".to_string());
                m
            },
            api_key_hash: None,
        };
        let json = serde_json::to_string(&uc).unwrap();
        let back: UserConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "Alice");
        assert_eq!(back.role, "owner");
        assert_eq!(back.channel_bindings.get("telegram").unwrap(), "123456");
    }

    #[test]
    fn test_config_with_mode_and_language() {
        let config = KernelConfig {
            mode: KernelMode::Stable,
            language: "ar".to_string(),
            ..Default::default()
        };
        assert_eq!(config.mode, KernelMode::Stable);
        assert_eq!(config.language, "ar");
    }

    #[test]
    fn test_validate_missing_env_vars() {
        let mut config = KernelConfig::default();
        config.channels.discord = Some(DiscordConfig {
            bot_token_env: "CAPTAIN_TEST_NONEXISTENT_VAR_DC".to_string(),
            ..Default::default()
        });
        let warnings = config.validate();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Discord"));
    }

    #[test]
    fn test_clamp_bounds_zero_browser_timeout() {
        let mut config = KernelConfig::default();
        config.browser.timeout_secs = 0;
        config.clamp_bounds();
        assert_eq!(config.browser.timeout_secs, 30);
    }

    #[test]
    fn test_clamp_bounds_excessive_browser_sessions() {
        let mut config = KernelConfig::default();
        config.browser.max_sessions = 999;
        config.clamp_bounds();
        assert_eq!(config.browser.max_sessions, 100);
    }

    #[test]
    fn test_clamp_bounds_zero_fetch_bytes() {
        let mut config = KernelConfig::default();
        config.web.fetch.max_response_bytes = 0;
        config.clamp_bounds();
        assert_eq!(config.web.fetch.max_response_bytes, 5_000_000);
    }

    #[test]
    fn test_clamp_bounds_zero_fetch_timeout() {
        let mut config = KernelConfig::default();
        config.web.fetch.timeout_secs = 0;
        config.clamp_bounds();
        assert_eq!(config.web.fetch.timeout_secs, 30);
    }

    #[test]
    fn test_clamp_bounds_defaults_unchanged() {
        let mut config = KernelConfig::default();
        let browser_timeout = config.browser.timeout_secs;
        let browser_sessions = config.browser.max_sessions;
        let fetch_bytes = config.web.fetch.max_response_bytes;
        let fetch_timeout = config.web.fetch.timeout_secs;
        config.clamp_bounds();
        assert_eq!(config.browser.timeout_secs, browser_timeout);
        assert_eq!(config.browser.max_sessions, browser_sessions);
        assert_eq!(config.web.fetch.max_response_bytes, fetch_bytes);
        assert_eq!(config.web.fetch.timeout_secs, fetch_timeout);
    }

    #[test]
    fn test_resolve_api_key_env_convention() {
        let config = KernelConfig::default();
        // Unknown provider falls back to convention
        assert_eq!(config.resolve_api_key_env("nvidia"), "NVIDIA_API_KEY");
        assert_eq!(config.resolve_api_key_env("my-custom"), "MY_CUSTOM_API_KEY");
    }

    #[test]
    fn test_resolve_api_key_env_explicit_mapping() {
        let mut config = KernelConfig::default();
        config
            .provider_api_keys
            .insert("nvidia".to_string(), "NIM_KEY".to_string());
        // Explicit mapping takes precedence over convention
        assert_eq!(config.resolve_api_key_env("nvidia"), "NIM_KEY");
    }

    #[test]
    fn test_resolve_api_key_env_auth_profiles() {
        let mut config = KernelConfig::default();
        config.auth_profiles.insert(
            "nvidia".to_string(),
            vec![AuthProfile {
                name: "primary".to_string(),
                api_key_env: "NVIDIA_PRIMARY_KEY".to_string(),
                priority: 0,
            }],
        );
        // Auth profiles take precedence over convention (but not explicit mapping)
        assert_eq!(config.resolve_api_key_env("nvidia"), "NVIDIA_PRIMARY_KEY");
    }

    #[test]
    fn test_resolve_api_key_env_explicit_over_auth_profile() {
        let mut config = KernelConfig::default();
        config
            .provider_api_keys
            .insert("nvidia".to_string(), "NIM_KEY".to_string());
        config.auth_profiles.insert(
            "nvidia".to_string(),
            vec![AuthProfile {
                name: "primary".to_string(),
                api_key_env: "NVIDIA_PRIMARY_KEY".to_string(),
                priority: 0,
            }],
        );
        // Explicit mapping wins over auth profiles
        assert_eq!(config.resolve_api_key_env("nvidia"), "NIM_KEY");
    }

    #[test]
    fn test_provider_api_keys_toml_roundtrip() {
        let toml_str = r#"
            [provider_api_keys]
            nvidia = "NVIDIA_NIM_KEY"
            azure = "AZURE_OPENAI_KEY"
        "#;
        let config: KernelConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.provider_api_keys.len(), 2);
        assert_eq!(
            config.provider_api_keys.get("nvidia").unwrap(),
            "NVIDIA_NIM_KEY"
        );
        assert_eq!(
            config.provider_api_keys.get("azure").unwrap(),
            "AZURE_OPENAI_KEY"
        );
    }
}

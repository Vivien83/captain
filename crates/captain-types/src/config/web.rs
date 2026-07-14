use serde::{Deserialize, Serialize};

/// Web search provider selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchProvider {
    /// Brave Search API.
    Brave,
    /// Tavily AI-agent-native search.
    Tavily,
    /// Perplexity AI search.
    Perplexity,
    /// DuckDuckGo HTML (no API key needed).
    DuckDuckGo,
    /// Auto-select based on available API keys (Tavily -> Brave -> Perplexity -> DuckDuckGo).
    #[default]
    Auto,
}

/// Web tools configuration (search + fetch).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebConfig {
    /// Which search provider to use.
    pub search_provider: SearchProvider,
    /// Cache TTL in minutes (0 = disabled).
    pub cache_ttl_minutes: u64,
    /// Brave Search configuration.
    pub brave: BraveSearchConfig,
    /// Tavily Search configuration.
    pub tavily: TavilySearchConfig,
    /// Perplexity Search configuration.
    pub perplexity: PerplexitySearchConfig,
    /// Web fetch configuration.
    pub fetch: WebFetchConfig,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            search_provider: SearchProvider::default(),
            cache_ttl_minutes: 15,
            brave: BraveSearchConfig::default(),
            tavily: TavilySearchConfig::default(),
            perplexity: PerplexitySearchConfig::default(),
            fetch: WebFetchConfig::default(),
        }
    }
}

/// Browser terminal configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebTerminalConfig {
    /// Enable the embedded web terminal page and PTY WebSocket.
    pub enabled: bool,
    /// Allow `mode=shell` to spawn the user's default shell.
    ///
    /// The product default is `false`: `/terminal` starts the Captain TUI,
    /// not an unrestricted shell. Raw shell access should be an explicit VPS
    /// administrator decision.
    pub allow_raw_shell: bool,
    /// Maximum live PTY sessions kept in the daemon process.
    pub max_sessions: usize,
    /// Default terminal mode: `captain` or `shell`.
    pub default_mode: String,
}

impl Default for WebTerminalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_raw_shell: false,
            max_sessions: 4,
            default_mode: "captain".to_string(),
        }
    }
}

/// Product deployment metadata written by setup/install flows.
///
/// The daemon does not need this to boot, but keeping it in `config.toml`
/// makes the file the source of truth for VPS/domain/HTTPS choices and gives
/// doctor/status surfaces a stable place to report deployment intent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DeploymentConfig {
    /// Installation profile used by setup: core, vps, desktop, full-media.
    pub profile: String,
    /// Public URL for browser access, usually `https://captain.example.com`.
    pub public_url: String,
    /// Whether the public URL is expected to be served over HTTPS.
    pub https: bool,
    /// Reverse proxy expected in front of Captain, for example `caddy`.
    pub reverse_proxy: String,
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            profile: "core".to_string(),
            public_url: String::new(),
            https: true,
            reverse_proxy: "caddy".to_string(),
        }
    }
}

/// Product surfaces that are allowed to participate in the active core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActiveSurface {
    Cli,
    Web,
    Telegram,
    Projects,
    Memory,
    Skills,
    Automation,
    Status,
}

impl ActiveSurface {
    pub fn tier1_defaults() -> Vec<Self> {
        vec![
            Self::Cli,
            Self::Web,
            Self::Telegram,
            Self::Projects,
            Self::Memory,
            Self::Skills,
            Self::Automation,
            Self::Status,
        ]
    }
}

/// Product surface gates. Frozen surfaces stay compiled but should not be
/// advertised in prompts, discovery, or primary UX unless explicitly re-enabled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProductSurfacesConfig {
    pub active: Vec<ActiveSurface>,
    pub frozen: Vec<String>,
}

impl Default for ProductSurfacesConfig {
    fn default() -> Self {
        Self {
            active: ActiveSurface::tier1_defaults(),
            frozen: vec![
                "hands".to_string(),
                "a2a".to_string(),
                "peers".to_string(),
                "fleets".to_string(),
                "desktop".to_string(),
                "long-tail-channels".to_string(),
                "roadmap-dashboard".to_string(),
                "experimental-integrations".to_string(),
            ],
        }
    }
}

impl ProductSurfacesConfig {
    pub fn is_active(&self, surface: ActiveSurface) -> bool {
        self.active.contains(&surface)
    }

    pub fn is_frozen_name(&self, surface: &str) -> bool {
        self.frozen
            .iter()
            .any(|name| name.eq_ignore_ascii_case(surface))
    }
}

/// Brave Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BraveSearchConfig {
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Maximum results to return.
    pub max_results: usize,
    /// Country code for search localization (e.g., "US").
    pub country: String,
    /// Search language (e.g., "en").
    pub search_lang: String,
    /// Freshness filter (e.g., "pd" = past day, "pw" = past week).
    pub freshness: String,
}

impl Default for BraveSearchConfig {
    fn default() -> Self {
        Self {
            api_key_env: "BRAVE_API_KEY".to_string(),
            max_results: 5,
            country: String::new(),
            search_lang: String::new(),
            freshness: String::new(),
        }
    }
}

/// Tavily Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TavilySearchConfig {
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Search depth: "basic" or "advanced".
    pub search_depth: String,
    /// Maximum results to return.
    pub max_results: usize,
    /// Include AI-generated answer summary.
    pub include_answer: bool,
}

impl Default for TavilySearchConfig {
    fn default() -> Self {
        Self {
            api_key_env: "TAVILY_API_KEY".to_string(),
            search_depth: "basic".to_string(),
            max_results: 5,
            include_answer: true,
        }
    }
}

/// Perplexity Search API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PerplexitySearchConfig {
    /// Env var name holding the API key.
    pub api_key_env: String,
    /// Model to use for search (e.g., "sonar").
    pub model: String,
}

impl Default for PerplexitySearchConfig {
    fn default() -> Self {
        Self {
            api_key_env: "PERPLEXITY_API_KEY".to_string(),
            model: "sonar".to_string(),
        }
    }
}

/// Web fetch configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebFetchConfig {
    /// Maximum characters to return in content.
    pub max_chars: usize,
    /// Maximum response body size in bytes.
    pub max_response_bytes: usize,
    /// HTTP request timeout in seconds.
    pub timeout_secs: u64,
    /// Enable HTML to Markdown readability extraction.
    pub readability: bool,
}

impl Default for WebFetchConfig {
    fn default() -> Self {
        Self {
            max_chars: 50_000,
            max_response_bytes: 10 * 1024 * 1024, // 10 MB
            timeout_secs: 30,
            readability: true,
        }
    }
}

/// Browser automation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserConfig {
    /// Run browser in headless mode (no visible window).
    pub headless: bool,
    /// Viewport width in pixels.
    pub viewport_width: u32,
    /// Viewport height in pixels.
    pub viewport_height: u32,
    /// Per-action timeout in seconds.
    pub timeout_secs: u64,
    /// Idle timeout: auto-close session after this many seconds of inactivity.
    pub idle_timeout_secs: u64,
    /// Maximum concurrent browser sessions.
    pub max_sessions: usize,
    /// Path to Chromium/Chrome binary. Auto-detected if None.
    pub chromium_path: Option<String>,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            headless: true,
            viewport_width: 1280,
            viewport_height: 720,
            timeout_secs: 30,
            idle_timeout_secs: 300,
            max_sessions: 5,
            chromium_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_config_defaults_keep_provider_chain() {
        let cfg = WebConfig::default();

        assert_eq!(cfg.search_provider, SearchProvider::Auto);
        assert_eq!(cfg.cache_ttl_minutes, 15);
        assert_eq!(cfg.brave.api_key_env, "BRAVE_API_KEY");
        assert_eq!(cfg.tavily.api_key_env, "TAVILY_API_KEY");
        assert_eq!(cfg.perplexity.api_key_env, "PERPLEXITY_API_KEY");
        assert_eq!(cfg.fetch.max_response_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn product_surfaces_keep_core_active_and_frozen_long_tail() {
        let cfg = ProductSurfacesConfig::default();

        for surface in ActiveSurface::tier1_defaults() {
            assert!(cfg.is_active(surface));
        }
        assert!(cfg.is_frozen_name("Hands"));
        assert!(cfg.is_frozen_name("long-tail-channels"));
        assert!(!cfg.is_frozen_name("telegram"));
    }

    #[test]
    fn product_surfaces_serde_accepts_kebab_case() {
        let cfg: ProductSurfacesConfig = toml::from_str(
            r#"
            active = ["cli", "web", "status"]
            frozen = ["roadmap-dashboard"]
            "#,
        )
        .unwrap();

        assert!(cfg.is_active(ActiveSurface::Cli));
        assert!(cfg.is_active(ActiveSurface::Web));
        assert!(cfg.is_active(ActiveSurface::Status));
        assert!(!cfg.is_active(ActiveSurface::Telegram));
        assert!(cfg.is_frozen_name("ROADMAP-DASHBOARD"));
    }
}

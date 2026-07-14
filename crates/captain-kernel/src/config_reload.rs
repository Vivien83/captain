//! Config reload planning — diffs two `KernelConfig` instances and produces a `ReloadPlan`.
//!
//! **Hot-reload candidates**: channels, skills, usage footer, web config, browser,
//! approval policy, cron settings, webhook triggers, extensions.
//!
//! **Auto-applied by `kernel.reload_config()` today**: approval policy, cron
//! settings, provider URL overrides, default model, TTS config.
//!
//! Other candidates are detected in the plan but must not be reported as
//! applied unless their owning subsystem actually reinitializes them; if they
//! remain deferred, the returned runtime plan must require an operator action.
//!
//! **No-op** (informational only): log_level, language, mode.
//!
//! **Restart required**: api_listen, api_key, network, memory, outbound webhooks.

use captain_types::config::{KernelConfig, ReloadMode};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// HotAction — what can be changed at runtime without restart
// ---------------------------------------------------------------------------

/// An individual action that may be hot-reloadable at runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotAction {
    /// Channel configuration changed — reload channel bridges.
    ReloadChannels,
    /// Skill configuration changed — reload skill registry.
    ReloadSkills,
    /// Usage footer mode changed.
    UpdateUsageFooter,
    /// Web config changed — rebuild web tools context.
    ReloadWebConfig,
    /// Browser config changed.
    ReloadBrowserConfig,
    /// Approval policy changed.
    UpdateApprovalPolicy,
    /// Cron max jobs changed.
    UpdateCronConfig,
    /// Webhook trigger config changed.
    UpdateWebhookConfig,
    /// Extension config changed.
    ReloadExtensions,
    /// MCP server list changed — reconnect MCP clients.
    ReloadMcpServers,
    /// A2A config changed.
    ReloadA2aConfig,
    /// Fallback provider chain changed.
    ReloadFallbackProviders,
    /// Provider base URL overrides changed.
    ReloadProviderUrls,
    /// Default model changed — update in-place without restart.
    UpdateDefaultModel,
    /// Text-to-speech config changed — update the runtime TTS engine.
    UpdateTtsConfig,
}

// ---------------------------------------------------------------------------
// ReloadPlan — the output of diffing two configs
// ---------------------------------------------------------------------------

/// A categorized plan for applying config changes.
///
/// After building a plan via [`build_reload_plan`], callers inspect
/// `restart_required` to decide whether a full restart is needed. `hot_actions`
/// are candidates until the caller has actually applied or deferred them.
#[derive(Debug, Clone)]
pub struct ReloadPlan {
    /// Whether a full restart is needed.
    pub restart_required: bool,
    /// Human-readable reasons why restart is required.
    pub restart_reasons: Vec<String>,
    /// Actions that can be hot-reloaded without restart.
    pub hot_actions: Vec<HotAction>,
    /// Fields that changed but are no-ops (informational only).
    pub noop_changes: Vec<String>,
}

impl ReloadPlan {
    fn empty() -> Self {
        Self {
            restart_required: false,
            restart_reasons: Vec::new(),
            hot_actions: Vec::new(),
            noop_changes: Vec::new(),
        }
    }

    fn require_restart(&mut self, reason: impl Into<String>) {
        self.restart_required = true;
        self.restart_reasons.push(reason.into());
    }

    fn push_hot_action(&mut self, action: HotAction) {
        self.hot_actions.push(action);
    }

    fn note_noop(&mut self, change: impl Into<String>) {
        self.noop_changes.push(change.into());
    }

    /// Whether any changes were detected at all.
    pub fn has_changes(&self) -> bool {
        self.restart_required || !self.hot_actions.is_empty() || !self.noop_changes.is_empty()
    }

    /// Whether the plan can be applied without restart.
    pub fn is_hot_reloadable(&self) -> bool {
        !self.restart_required
    }

    /// Log a human-readable summary of the plan.
    pub fn log_summary(&self) {
        if !self.has_changes() {
            info!("config reload: no changes detected");
            return;
        }
        if self.restart_required {
            warn!(
                "config reload: restart required — {}",
                self.restart_reasons.join("; ")
            );
        }
        for action in &self.hot_actions {
            info!("config reload: hot-reload candidate detected — {action:?}");
        }
        for noop in &self.noop_changes {
            info!("config reload: no-op change — {noop}");
        }
    }
}

// ---------------------------------------------------------------------------
// build_reload_plan
// ---------------------------------------------------------------------------

/// Compare JSON-serialized forms of a field. Returns `true` when the
/// serialized representations differ (or if one side fails to serialize).
fn field_changed<T: serde::Serialize>(old: &T, new: &T) -> bool {
    let old_json = serde_json::to_string(old).ok();
    let new_json = serde_json::to_string(new).ok();
    old_json != new_json
}

/// Diff two configurations and produce a reload plan.
///
/// The plan categorizes every detected change into one of three buckets:
///
/// 1. **restart_required** — the change touches something that cannot be
///    patched at runtime (e.g. the listen address or database path).
/// 2. **hot_actions** — the change is a hot-reload candidate. Callers that
///    actually apply a subset should replace this list with applied actions
///    before returning an operator/API response.
/// 3. **noop_changes** — the change is informational; no action needed.
pub fn build_reload_plan(old: &KernelConfig, new: &KernelConfig) -> ReloadPlan {
    let mut plan = ReloadPlan::empty();
    add_restart_required_changes(&mut plan, old, new);
    add_hot_reload_actions(&mut plan, old, new);
    add_noop_changes(&mut plan, old, new);
    plan
}

fn add_restart_required_changes(plan: &mut ReloadPlan, old: &KernelConfig, new: &KernelConfig) {
    if old.api_listen != new.api_listen {
        plan.require_restart(format!(
            "api_listen changed: {} -> {}",
            old.api_listen, new.api_listen
        ));
    }

    if old.api_key != new.api_key {
        plan.require_restart("api_key changed");
    }

    if old.network_enabled != new.network_enabled {
        plan.require_restart("network_enabled changed");
    }

    if field_changed(&old.network, &new.network) {
        plan.require_restart("network config changed");
    }

    if field_changed(&old.memory, &new.memory) {
        plan.require_restart("memory config changed");
    }

    if field_changed(&old.outbound_webhooks, &new.outbound_webhooks) {
        plan.require_restart("outbound webhooks changed");
    }

    if old.home_dir != new.home_dir {
        plan.require_restart(format!(
            "home_dir changed: {:?} -> {:?}",
            old.home_dir, new.home_dir
        ));
    }
    if old.data_dir != new.data_dir {
        plan.require_restart(format!(
            "data_dir changed: {:?} -> {:?}",
            old.data_dir, new.data_dir
        ));
    }

    if field_changed(&old.vault, &new.vault) {
        plan.require_restart("vault config changed");
    }
}

fn add_hot_reload_actions(plan: &mut ReloadPlan, old: &KernelConfig, new: &KernelConfig) {
    if field_changed(&old.default_model, &new.default_model) {
        plan.push_hot_action(HotAction::UpdateDefaultModel);
    }

    if field_changed(&old.channels, &new.channels) {
        plan.push_hot_action(HotAction::ReloadChannels);
    }

    if old.usage_footer != new.usage_footer {
        plan.push_hot_action(HotAction::UpdateUsageFooter);
    }

    if field_changed(&old.web, &new.web) {
        plan.push_hot_action(HotAction::ReloadWebConfig);
    }

    if field_changed(&old.browser, &new.browser) {
        plan.push_hot_action(HotAction::ReloadBrowserConfig);
    }

    if field_changed(&old.approval, &new.approval) {
        plan.push_hot_action(HotAction::UpdateApprovalPolicy);
    }

    if old.max_cron_jobs != new.max_cron_jobs {
        plan.push_hot_action(HotAction::UpdateCronConfig);
    }

    if field_changed(&old.webhook_triggers, &new.webhook_triggers) {
        plan.push_hot_action(HotAction::UpdateWebhookConfig);
    }

    if field_changed(&old.extensions, &new.extensions) {
        plan.push_hot_action(HotAction::ReloadExtensions);
    }

    if field_changed(&old.mcp_servers, &new.mcp_servers) {
        plan.push_hot_action(HotAction::ReloadMcpServers);
    }

    if field_changed(&old.a2a, &new.a2a) {
        plan.push_hot_action(HotAction::ReloadA2aConfig);
    }

    if field_changed(&old.fallback_providers, &new.fallback_providers) {
        plan.push_hot_action(HotAction::ReloadFallbackProviders);
    }

    if field_changed(&old.provider_urls, &new.provider_urls) {
        plan.push_hot_action(HotAction::ReloadProviderUrls);
    }

    if field_changed(&old.tts, &new.tts) {
        plan.push_hot_action(HotAction::UpdateTtsConfig);
    }
}

fn add_noop_changes(plan: &mut ReloadPlan, old: &KernelConfig, new: &KernelConfig) {
    if field_changed(&old.provider_api_keys, &new.provider_api_keys) {
        plan.note_noop("provider_api_keys changed (takes effect on next driver init)");
    }

    if old.log_level != new.log_level {
        plan.note_noop(format!("log_level: {} -> {}", old.log_level, new.log_level));
    }

    if old.language != new.language {
        plan.note_noop(format!("language: {} -> {}", old.language, new.language));
    }

    if old.mode != new.mode {
        plan.note_noop(format!("mode: {:?} -> {:?}", old.mode, new.mode));
    }
}

// ---------------------------------------------------------------------------
// validate_config_for_reload
// ---------------------------------------------------------------------------

/// Validate a new config before applying it.
///
/// Returns `Ok(())` if the config passes basic sanity checks, or `Err` with
/// a list of human-readable error messages.
pub fn validate_config_for_reload(config: &KernelConfig) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if config.api_listen.is_empty() {
        errors.push("api_listen cannot be empty".to_string());
    }

    if config.max_cron_jobs > 10_000 {
        errors.push("max_cron_jobs exceeds reasonable limit (10000)".to_string());
    }

    // Validate approval policy
    if let Err(e) = config.approval.validate() {
        errors.push(format!("approval policy: {e}"));
    }

    // Network config: if network is enabled, shared_secret must be set
    if config.network_enabled && config.network.shared_secret.is_empty() {
        errors.push("network_enabled is true but network.shared_secret is empty".to_string());
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// should_reload — convenience helper for the reload mode
// ---------------------------------------------------------------------------

/// Given the configured [`ReloadMode`] and a [`ReloadPlan`], decide whether
/// the caller should apply hot actions.
///
/// Returns `true` if hot-reload actions should be applied.
pub fn should_apply_hot(mode: ReloadMode, plan: &ReloadPlan) -> bool {
    match mode {
        ReloadMode::Off => false,
        ReloadMode::Restart => false, // caller must do a full restart
        ReloadMode::Hot => !plan.hot_actions.is_empty(),
        ReloadMode::Hybrid => !plan.hot_actions.is_empty(),
    }
}

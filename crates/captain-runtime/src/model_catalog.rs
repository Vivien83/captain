//! Model catalog — registry of known models with metadata, pricing, and auth detection.
//!
//! Provides a comprehensive catalog of 130+ builtin models across 28 providers,
//! with alias resolution, auth status detection, and pricing lookups.

use captain_types::model_catalog::{AuthStatus, ModelCatalogEntry, ModelTier, ProviderInfo};
use std::collections::HashMap;

pub use crate::model_catalog_codex::{
    codex_cached_model_entries, codex_cached_model_ids, codex_model_choices,
    refresh_codex_models_cache, refresh_codex_models_cache_with_token,
};
pub use crate::model_catalog_codex_auth::{
    codex_oauth_readiness_error, codex_token_scopes, read_codex_credential,
    read_codex_credential_with_refresh, refresh_codex_credential_now,
    refresh_or_rotate_codex_credential,
};

/// The model catalog — registry of all known models and providers.
pub struct ModelCatalog {
    models: Vec<ModelCatalogEntry>,
    aliases: HashMap<String, String>,
    providers: Vec<ProviderInfo>,
}

impl ModelCatalog {
    /// Create a new catalog populated with builtin models and providers.
    pub fn new() -> Self {
        let mut models = builtin_models();
        let mut aliases = crate::model_catalog_aliases::builtin_aliases();
        let mut providers = crate::model_catalog_providers::builtin_providers();

        crate::model_catalog_codex::apply_codex_models_cache(&mut models, &mut aliases);

        // Auto-register aliases defined on model entries
        for model in &models {
            for alias in &model.aliases {
                let lower = alias.to_lowercase();
                aliases.entry(lower).or_insert_with(|| model.id.clone());
            }
        }

        // Set model counts on providers
        for provider in &mut providers {
            provider.model_count = models.iter().filter(|m| m.provider == provider.id).count();
        }

        Self {
            models,
            aliases,
            providers,
        }
    }

    /// Detect which providers have API keys configured.
    ///
    /// Checks `std::env::var()` for each provider's API key env var.
    /// Only checks presence — never reads or stores the actual secret.
    pub fn detect_auth(&mut self) {
        for provider in &mut self.providers {
            // Claude Code is special: no API key needed, but we probe for CLI
            // installation so the dashboard shows "Configured" vs "Not Installed".
            if provider.id == "claude-code" {
                provider.auth_status = if crate::drivers::claude_code::claude_code_available() {
                    AuthStatus::Configured
                } else {
                    AuthStatus::Missing
                };
                continue;
            }
            if provider.id == "qwen-code" {
                provider.auth_status = if crate::drivers::qwen_code::qwen_code_available() {
                    AuthStatus::Configured
                } else {
                    AuthStatus::Missing
                };
                continue;
            }

            if !provider.key_required {
                provider.auth_status = AuthStatus::NotRequired;
                continue;
            }

            // Primary: check the provider's declared env var
            let has_key =
                !provider.api_key_env.is_empty() && std::env::var(&provider.api_key_env).is_ok();

            // Secondary: provider-specific fallback auth
            let has_fallback = match provider.id.as_str() {
                "gemini" => std::env::var("GOOGLE_API_KEY").is_ok(),
                "codex" => read_codex_credential().is_some(),
                // claude-code is handled above (before key_required check)
                _ => false,
            };

            provider.auth_status = if has_key || has_fallback {
                AuthStatus::Configured
            } else {
                AuthStatus::Missing
            };
        }
    }

    /// Reload only the Codex family from the official local cache.
    ///
    /// Other providers, custom models, URL overrides, and runtime pricing
    /// adjustments remain untouched while newly visible Codex aliases become
    /// immediately available to routing and safe model-switch preflight.
    pub fn reload_codex_models_cache(&mut self) -> usize {
        crate::model_catalog_codex::apply_codex_models_cache(&mut self.models, &mut self.aliases);
        for model in self.models.iter().filter(|model| model.provider == "codex") {
            for alias in &model.aliases {
                self.aliases
                    .insert(alias.to_ascii_lowercase(), model.id.clone());
            }
        }
        let count = self
            .models
            .iter()
            .filter(|model| model.provider == "codex")
            .count();
        if let Some(provider) = self.providers.iter_mut().find(|p| p.id == "codex") {
            provider.model_count = count;
        }
        count
    }

    /// List all models in the catalog.
    pub fn list_models(&self) -> &[ModelCatalogEntry] {
        &self.models
    }

    /// Find a model by its canonical ID or by alias.
    pub fn find_model(&self, id_or_alias: &str) -> Option<&ModelCatalogEntry> {
        let lower = id_or_alias.to_lowercase();
        // Direct ID match first
        if let Some(entry) = self.models.iter().find(|m| m.id.to_lowercase() == lower) {
            return Some(entry);
        }
        // Strip provider prefix (e.g. "anthropic/claude-haiku-4.5" → "claude-haiku-4.5")
        if let Some((_provider, model_part)) = lower.split_once('/') {
            // Try with dots replaced by dashes (e.g. "claude-haiku-4.5" → "claude-haiku-4-5")
            let dashed = model_part.replace('.', "-");
            if let Some(entry) = self.models.iter().find(|m| {
                let mid = m.id.to_lowercase();
                mid == model_part || mid.starts_with(&dashed)
            }) {
                return Some(entry);
            }
        }
        // Alias resolution
        if let Some(canonical) = self.aliases.get(&lower) {
            return self.models.iter().find(|m| m.id == *canonical);
        }
        None
    }

    /// Resolve an alias to a canonical model ID, or None if not an alias.
    pub fn resolve_alias(&self, alias: &str) -> Option<&str> {
        self.aliases.get(&alias.to_lowercase()).map(|s| s.as_str())
    }

    /// List all providers.
    pub fn list_providers(&self) -> &[ProviderInfo] {
        &self.providers
    }

    /// Get a provider by ID.
    pub fn get_provider(&self, provider_id: &str) -> Option<&ProviderInfo> {
        self.providers.iter().find(|p| p.id == provider_id)
    }

    /// List models from a specific provider.
    pub fn models_by_provider(&self, provider: &str) -> Vec<&ModelCatalogEntry> {
        self.models
            .iter()
            .filter(|m| m.provider == provider)
            .collect()
    }

    /// Return the default model ID for a provider (first model in catalog order).
    pub fn default_model_for_provider(&self, provider: &str) -> Option<String> {
        // Check aliases first — e.g. "minimax" alias resolves to "MiniMax-M2.5"
        if let Some(model_id) = self.aliases.get(provider) {
            return Some(model_id.clone());
        }
        // Fall back to the first model registered for this provider
        self.models
            .iter()
            .find(|m| m.provider == provider)
            .map(|m| m.id.clone())
    }

    /// List models that are available (from configured providers only).
    pub fn available_models(&self) -> Vec<&ModelCatalogEntry> {
        let configured: Vec<&str> = self
            .providers
            .iter()
            .filter(|p| p.auth_status != AuthStatus::Missing)
            .map(|p| p.id.as_str())
            .collect();
        self.models
            .iter()
            .filter(|m| configured.contains(&m.provider.as_str()))
            .collect()
    }

    /// Get pricing for a model: (input_cost_per_million, output_cost_per_million).
    pub fn pricing(&self, model_id: &str) -> Option<(f64, f64)> {
        self.find_model(model_id)
            .map(|m| (m.input_cost_per_m, m.output_cost_per_m))
    }

    /// List all alias mappings.
    pub fn list_aliases(&self) -> &HashMap<String, String> {
        &self.aliases
    }

    /// Set a custom base URL for a provider, overriding the default.
    ///
    /// Returns `true` if the provider was found and updated.
    pub fn set_provider_url(&mut self, provider: &str, url: &str) -> bool {
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.base_url = url.to_string();
            true
        } else {
            // Custom provider — add a new entry so it appears in /api/providers
            let env_var = format!("{}_API_KEY", provider.to_uppercase().replace('-', "_"));
            self.providers.push(ProviderInfo {
                id: provider.to_string(),
                display_name: provider.to_string(),
                api_key_env: env_var,
                base_url: url.to_string(),
                key_required: true,
                auth_status: AuthStatus::Missing,
                model_count: 0,
            });
            // Re-detect auth for the newly added provider
            self.detect_auth();
            true
        }
    }

    /// Apply a batch of provider URL overrides from config.
    ///
    /// Each entry maps a provider ID to a custom base URL.
    /// Unknown providers are automatically added as custom OpenAI-compatible entries.
    /// Providers with explicit URL overrides are marked as configured since
    /// the user intentionally set them up (e.g. local proxies, custom endpoints).
    pub fn apply_url_overrides(&mut self, overrides: &HashMap<String, String>) {
        for (provider, url) in overrides {
            if self.set_provider_url(provider, url) {
                // Mark as configured so models from this provider show as available
                if let Some(p) = self.providers.iter_mut().find(|p| p.id == *provider) {
                    if p.auth_status == AuthStatus::Missing {
                        p.auth_status = AuthStatus::Configured;
                    }
                }
            }
        }
    }

    /// List models filtered by tier.
    pub fn models_by_tier(&self, tier: ModelTier) -> Vec<&ModelCatalogEntry> {
        self.models.iter().filter(|m| m.tier == tier).collect()
    }

    /// Merge dynamically discovered models from a local provider.
    ///
    /// Adds models not already in the catalog with `Local` tier and zero cost.
    /// Also updates the provider's `model_count`.
    pub fn merge_discovered_models(&mut self, provider: &str, model_ids: &[String]) {
        let existing_ids: std::collections::HashSet<String> = self
            .models
            .iter()
            .filter(|m| m.provider == provider)
            .map(|m| m.id.to_lowercase())
            .collect();

        let mut added = 0usize;
        for id in model_ids {
            if existing_ids.contains(&id.to_lowercase()) {
                continue;
            }
            // Generate a human-friendly display name
            let display = format!("{} ({})", id, provider);
            self.models.push(ModelCatalogEntry {
                id: id.clone(),
                display_name: display,
                provider: provider.to_string(),
                tier: ModelTier::Local,
                context_window: 32_768,
                max_output_tokens: 4_096,
                input_cost_per_m: 0.0,
                output_cost_per_m: 0.0,
                supports_tools: true,
                supports_vision: false,
                supports_streaming: true,
                aliases: Vec::new(),
            });
            added += 1;
        }

        // Update model count on the provider
        if added > 0 {
            if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
                p.model_count = self
                    .models
                    .iter()
                    .filter(|m| m.provider == provider)
                    .count();
            }
        }
    }

    /// Add a custom model at runtime.
    ///
    /// Returns `true` if the model was added, `false` if a model with the same
    /// ID **and** provider already exists (case-insensitive).
    pub fn add_custom_model(&mut self, entry: ModelCatalogEntry) -> bool {
        let lower_id = entry.id.to_lowercase();
        let lower_provider = entry.provider.to_lowercase();
        if self
            .models
            .iter()
            .any(|m| m.id.to_lowercase() == lower_id && m.provider.to_lowercase() == lower_provider)
        {
            return false;
        }
        let provider = entry.provider.clone();
        self.models.push(entry);

        // Update provider model count
        if let Some(p) = self.providers.iter_mut().find(|p| p.id == provider) {
            p.model_count = self
                .models
                .iter()
                .filter(|m| m.provider == provider)
                .count();
        }
        true
    }

    /// Update pricing for a model (any model, not just custom).
    /// Returns true if the model was found and updated.
    pub fn update_pricing(&mut self, model_id: &str, input_cost: f64, output_cost: f64) -> bool {
        let lower = model_id.to_lowercase();
        if let Some(m) = self
            .models
            .iter_mut()
            .find(|m| m.id.to_lowercase() == lower)
        {
            m.input_cost_per_m = input_cost;
            m.output_cost_per_m = output_cost;
            true
        } else {
            false
        }
    }

    /// Remove a custom model by ID.
    ///
    /// Only removes models with `Custom` tier to prevent accidental deletion
    /// of builtin models. Returns `true` if removed.
    pub fn remove_custom_model(&mut self, model_id: &str) -> bool {
        let lower = model_id.to_lowercase();
        let before = self.models.len();
        self.models
            .retain(|m| !(m.id.to_lowercase() == lower && m.tier == ModelTier::Custom));
        self.models.len() < before
    }

    /// Load custom models from a JSON file.
    ///
    /// Merges them into the catalog. Skips models that already exist.
    pub fn load_custom_models(&mut self, path: &std::path::Path) {
        if !path.exists() {
            return;
        }
        let Ok(data) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(entries) = serde_json::from_str::<Vec<ModelCatalogEntry>>(&data) else {
            return;
        };
        for entry in entries {
            self.add_custom_model(entry);
        }
    }

    /// Save all custom-tier models to a JSON file.
    pub fn save_custom_models(&self, path: &std::path::Path) -> Result<(), String> {
        let custom: Vec<&ModelCatalogEntry> = self
            .models
            .iter()
            .filter(|m| m.tier == ModelTier::Custom)
            .collect();
        let json = serde_json::to_string_pretty(&custom)
            .map_err(|e| format!("Failed to serialize custom models: {e}"))?;
        captain_types::durable_fs::atomic_write(path, json.as_bytes())
            .map_err(|e| format!("Failed to persist custom models file: {e}"))?;
        Ok(())
    }
}

impl Default for ModelCatalog {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Builtin data
// ---------------------------------------------------------------------------

fn builtin_models() -> Vec<ModelCatalogEntry> {
    let mut models = crate::model_catalog_models_anthropic::anthropic_models();
    models.extend(crate::model_catalog_models_openai::openai_models());
    models.extend(crate::model_catalog_models_gemini::gemini_models());
    models.extend(crate::model_catalog_models_deepseek::deepseek_models());
    models.extend(crate::model_catalog_models_azure_openai::azure_openai_models());
    models.extend(crate::model_catalog_models_groq::groq_models());
    models.extend(crate::model_catalog_models_openrouter::openrouter_models_before_mistral());
    models.extend(crate::model_catalog_models_mistral::mistral_models());
    models.extend(crate::model_catalog_models_openrouter::openrouter_models_after_mistral());
    models.extend(crate::model_catalog_models_together::together_models());
    models.extend(crate::model_catalog_models_fireworks::fireworks_models());
    models.extend(crate::model_catalog_models_nvidia::nvidia_models());
    models.extend(crate::model_catalog_models_ollama::ollama_models());
    models.extend(crate::model_catalog_models_vllm::vllm_models());
    models.extend(crate::model_catalog_models_lmstudio::lmstudio_models());
    models.extend(crate::model_catalog_models_perplexity::perplexity_models());
    models.extend(crate::model_catalog_models_cohere::cohere_models());
    models.extend(crate::model_catalog_models_ai21::ai21_models());
    models.extend(crate::model_catalog_models_cerebras::cerebras_models());
    models.extend(crate::model_catalog_models_sambanova::sambanova_models());
    models.extend(crate::model_catalog_models_xai::xai_models());
    models.extend(crate::model_catalog_models_huggingface::huggingface_models());
    models.extend(crate::model_catalog_models_replicate::replicate_models());
    models.extend(crate::model_catalog_models_github_copilot::github_copilot_models());
    models.extend(crate::model_catalog_models_qwen::qwen_models());
    models.extend(crate::model_catalog_models_minimax::minimax_models());
    models.extend(crate::model_catalog_models_zhipu::zhipu_models());
    models.extend(crate::model_catalog_models_zhipu_coding::zhipu_coding_models());
    models.extend(crate::model_catalog_models_zai_coding::zai_coding_models());
    models.extend(crate::model_catalog_models_moonshot::moonshot_models());
    models.extend(crate::model_catalog_models_kimi_coding::kimi_coding_models());
    models.extend(crate::model_catalog_models_qianfan::qianfan_models());
    models.extend(crate::model_catalog_models_volcengine::volcengine_models());
    models.extend(crate::model_catalog_models_bedrock::bedrock_models());
    models.extend(crate::model_catalog_models_codex::codex_models());
    models.extend(crate::model_catalog_models_claude_code::claude_code_models());
    models.extend(crate::model_catalog_models_qwen_code::qwen_code_models());
    models.extend(crate::model_catalog_models_chutes::chutes_models());
    models.extend(crate::model_catalog_models_venice::venice_models());
    models
}

//! Durable Codex model discovery and explicit user decisions.
//!
//! The live Codex endpoint remains the source of truth. This module only
//! records which visible models have already been seen and which additions
//! still need a user decision; it never changes an agent model by itself.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use captain_runtime::audit::AuditAction;
use captain_runtime::model_catalog::{codex_cached_model_entries, refresh_codex_models_cache};
use captain_runtime::model_switch_pending::pending_model_switch_key;
use captain_types::agent::AgentId;
use captain_types::error::CaptainError;
use captain_types::model_catalog::ModelCatalogEntry;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::{KernelError, KernelResult};
use crate::{shared_memory_agent_id, CaptainKernel};

#[path = "codex_model_updates_delivery.rs"]
mod delivery;
#[cfg(test)]
#[path = "codex_model_updates_tests.rs"]
mod tests;

const STATE_KEY: &str = "__captain_codex_model_updates_v1";
const STATE_SCHEMA_VERSION: u32 = 1;
const INITIAL_SCAN_DELAY_SECS: u64 = 15;
const SCAN_INTERVAL_SECS: u64 = 60 * 60;
const MAX_DECISION_HISTORY: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexModelUpdate {
    pub model_id: String,
    pub display_name: String,
    pub discovered_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_notified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodexModelUpdateDecisionRecord {
    pub model_id: String,
    pub decision: String,
    pub decided_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexModelUpdateAgent {
    pub agent_id: String,
    pub agent_name: String,
    pub current_model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexModelUpdateSnapshot {
    pub provider: &'static str,
    pub active: bool,
    pub baseline_ready: bool,
    pub known_model_count: usize,
    pub last_checked_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub pending: Vec<CodexModelUpdate>,
    pub recent_decisions: Vec<CodexModelUpdateDecisionRecord>,
    pub agents: Vec<CodexModelUpdateAgent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct CodexModelUpdateState {
    schema_version: u32,
    baseline_ready: bool,
    initialized_at: String,
    last_checked_at: Option<String>,
    last_success_at: Option<String>,
    last_error: Option<String>,
    consecutive_failures: u32,
    known_model_ids: BTreeSet<String>,
    pending: Vec<CodexModelUpdate>,
    recent_decisions: Vec<CodexModelUpdateDecisionRecord>,
}

impl Default for CodexModelUpdateState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            baseline_ready: false,
            initialized_at: String::new(),
            last_checked_at: None,
            last_success_at: None,
            last_error: None,
            consecutive_failures: 0,
            known_model_ids: BTreeSet::new(),
            pending: Vec::new(),
            recent_decisions: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexModelScanOutcome {
    pub active: bool,
    pub available_models: usize,
    pub newly_discovered: Vec<String>,
}

pub fn spawn_codex_model_catalog_monitor(kernel: Arc<CaptainKernel>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(INITIAL_SCAN_DELAY_SECS)).await;
        loop {
            if let Err(error) = scan_codex_model_catalog_once(&kernel).await {
                warn!(error = %error, "Codex model catalog refresh deferred");
            }
            tokio::time::sleep(Duration::from_secs(SCAN_INTERVAL_SECS)).await;
        }
    });
}

pub async fn scan_codex_model_catalog_once(
    kernel: &Arc<CaptainKernel>,
) -> Result<CodexModelScanOutcome, String> {
    if kernel.codex_model_update_agents().is_empty() {
        return Ok(CodexModelScanOutcome {
            active: false,
            available_models: 0,
            newly_discovered: Vec::new(),
        });
    }

    let baseline = codex_cached_model_entries();
    let now = chrono::Utc::now().to_rfc3339();
    if let Err(error) = refresh_codex_models_cache().await {
        kernel
            .record_codex_model_scan_failure(&baseline, &now, &error)
            .map_err(|state_error| format!("{error}; state update failed: {state_error}"))?;
        return Err(error);
    }

    let available = codex_cached_model_entries();
    let newly_discovered = kernel
        .record_codex_model_scan_success(&baseline, &available, &now)
        .map_err(|error| error.to_string())?;
    kernel.reload_live_codex_model_catalog();

    if newly_discovered.is_empty() {
        debug!(models = available.len(), "Codex model catalog is current");
    } else {
        info!(
            models = available.len(),
            new_models = ?newly_discovered,
            "new Codex model availability detected"
        );
    }

    delivery::deliver_pending_telegram_notifications(kernel).await;
    Ok(CodexModelScanOutcome {
        active: true,
        available_models: available.len(),
        newly_discovered,
    })
}

impl CaptainKernel {
    pub fn codex_model_update_snapshot(&self) -> KernelResult<CodexModelUpdateSnapshot> {
        let agents = self.codex_model_update_agents();
        let state = self.load_codex_model_update_state()?.unwrap_or_default();
        Ok(CodexModelUpdateSnapshot {
            provider: "codex",
            active: !agents.is_empty(),
            baseline_ready: state.baseline_ready,
            known_model_count: state.known_model_ids.len(),
            last_checked_at: state.last_checked_at,
            last_success_at: state.last_success_at,
            last_error: state.last_error,
            consecutive_failures: state.consecutive_failures,
            pending: state.pending,
            recent_decisions: state.recent_decisions,
            agents,
        })
    }

    pub fn keep_codex_model_update(&self, model_id: Option<&str>) -> KernelResult<Vec<String>> {
        self.keep_codex_model_update_for_agent(model_id, None)
    }

    fn keep_codex_model_update_for_agent(
        &self,
        model_id: Option<&str>,
        agent_id: Option<AgentId>,
    ) -> KernelResult<Vec<String>> {
        let normalized = model_id.map(normalize_codex_model_id);
        let decided_at = chrono::Utc::now().to_rfc3339();
        let resolved = self.mutate_codex_model_update_state(|state| {
            resolve_pending_updates(
                state,
                normalized.as_deref(),
                "kept",
                &decided_at,
                agent_id.map(|id| id.to_string()),
            )
        })?;
        if resolved.is_empty() {
            return Err(KernelError::Captain(CaptainError::Config(format!(
                "No pending Codex model update matches {}",
                normalized.unwrap_or_else(|| "the request".to_string())
            ))));
        }
        self.audit_log.record(
            agent_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "captain".to_string()),
            AuditAction::ConfigChange,
            "Codex model update decision",
            format!("kept current model; dismissed={}", resolved.join(",")),
        );
        Ok(resolved)
    }

    pub fn consume_codex_model_update_keep_request(
        &self,
        agent_id: AgentId,
        user_message: &str,
    ) -> KernelResult<Option<String>> {
        if !has_keep_intent(user_message) {
            return Ok(None);
        }
        let snapshot = self.codex_model_update_snapshot()?;
        if snapshot.pending.is_empty() {
            return Ok(None);
        }

        let lower = user_message.to_ascii_lowercase();
        let mentioned = snapshot.pending.iter().find(|update| {
            lower.contains(&update.model_id.to_ascii_lowercase())
                || lower.contains(
                    &update
                        .model_id
                        .trim_start_matches("codex/")
                        .to_ascii_lowercase(),
                )
        });
        let resolved = self.keep_codex_model_update_for_agent(
            mentioned.map(|update| update.model_id.as_str()),
            Some(agent_id),
        )?;
        let _ = self.memory.structured_delete(
            shared_memory_agent_id(),
            &pending_model_switch_key(&agent_id.to_string()),
        );
        let models = resolved
            .iter()
            .map(|model| format!("`{model}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let response = if self.config.language.to_ascii_lowercase().starts_with("fr") {
            format!(
                "Décision enregistrée : je conserve le modèle actuel. La notification {models} est fermée et Captain ne basculera rien automatiquement."
            )
        } else {
            format!(
                "Decision recorded: the current model stays active. The {models} notification is closed and Captain will not switch automatically."
            )
        };
        Ok(Some(response))
    }

    pub(crate) fn resolve_codex_model_update_after_switch(
        &self,
        agent_id: AgentId,
        provider: &str,
        model: &str,
    ) {
        if !is_codex_provider(provider) {
            return;
        }
        let model_id = normalize_codex_model_id(model);
        let decided_at = chrono::Utc::now().to_rfc3339();
        let result = self.mutate_codex_model_update_state(|state| {
            resolve_pending_updates(
                state,
                Some(&model_id),
                "switched",
                &decided_at,
                Some(agent_id.to_string()),
            )
        });
        match result {
            Ok(resolved) if !resolved.is_empty() => {
                self.audit_log.record(
                    agent_id.to_string(),
                    AuditAction::ConfigChange,
                    "Codex model update decision",
                    format!("switched to {}", resolved.join(",")),
                );
            }
            Ok(_) => {}
            Err(error) => warn!(
                agent_id = %agent_id,
                model = %model_id,
                error = %error,
                "model switched but Codex update state could not be resolved"
            ),
        }
    }

    fn codex_model_update_agents(&self) -> Vec<CodexModelUpdateAgent> {
        let mut agents = self
            .registry
            .list()
            .into_iter()
            .filter(|entry| is_codex_provider(&entry.manifest.model.provider))
            .map(|entry| CodexModelUpdateAgent {
                agent_id: entry.id.to_string(),
                agent_name: entry.name,
                current_model: normalize_codex_model_id(&entry.manifest.model.model),
            })
            .collect::<Vec<_>>();
        agents.sort_by(|a, b| {
            (a.agent_name != "captain", a.agent_name.as_str())
                .cmp(&(b.agent_name != "captain", b.agent_name.as_str()))
        });
        agents
    }

    fn record_codex_model_scan_success(
        &self,
        baseline: &[ModelCatalogEntry],
        available: &[ModelCatalogEntry],
        now: &str,
    ) -> KernelResult<Vec<String>> {
        self.mutate_codex_model_update_state(|state| {
            reconcile_success(state, baseline, available, now)
        })
    }

    fn record_codex_model_scan_failure(
        &self,
        baseline: &[ModelCatalogEntry],
        now: &str,
        error: &str,
    ) -> KernelResult<()> {
        self.mutate_codex_model_update_state(|state| {
            establish_baseline(state, baseline, now);
            state.last_checked_at = Some(now.to_string());
            state.last_error = Some(error.to_string());
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        })
    }

    fn reload_live_codex_model_catalog(&self) {
        let mut catalog = self
            .model_catalog
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let count = catalog.reload_codex_models_cache();
        catalog.detect_auth();
        debug!(
            models = count,
            "reloaded live Codex entries in model catalog"
        );
    }

    fn load_codex_model_update_state(&self) -> KernelResult<Option<CodexModelUpdateState>> {
        let value = self
            .memory
            .structured_get(shared_memory_agent_id(), STATE_KEY)
            .map_err(KernelError::Captain)?;
        value
            .map(|value| {
                serde_json::from_value(value).map_err(|error| {
                    KernelError::Captain(CaptainError::Internal(format!(
                        "Invalid persisted Codex model update state: {error}"
                    )))
                })
            })
            .transpose()
    }

    fn mutate_codex_model_update_state<T>(
        &self,
        mutate: impl FnOnce(&mut CodexModelUpdateState) -> T,
    ) -> KernelResult<T> {
        let _guard = self
            .codex_model_update_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut state = self.load_codex_model_update_state()?.unwrap_or_default();
        let output = mutate(&mut state);
        self.memory
            .structured_set(
                shared_memory_agent_id(),
                STATE_KEY,
                serde_json::to_value(state).map_err(|error| {
                    KernelError::Captain(CaptainError::Internal(format!(
                        "Failed to serialize Codex model update state: {error}"
                    )))
                })?,
            )
            .map_err(KernelError::Captain)?;
        Ok(output)
    }
}

fn reconcile_success(
    state: &mut CodexModelUpdateState,
    baseline: &[ModelCatalogEntry],
    available: &[ModelCatalogEntry],
    now: &str,
) -> Vec<String> {
    establish_baseline_from_success(state, baseline, available, now);
    let mut newly_discovered = Vec::new();
    for entry in available {
        let model_id = normalize_codex_model_id(&entry.id);
        if state.known_model_ids.insert(model_id.clone()) {
            state.pending.push(CodexModelUpdate {
                model_id: model_id.clone(),
                display_name: entry.display_name.clone(),
                discovered_at: now.to_string(),
                telegram_notified_at: None,
            });
            newly_discovered.push(model_id);
        }
    }
    state.last_checked_at = Some(now.to_string());
    state.last_success_at = Some(now.to_string());
    state.last_error = None;
    state.consecutive_failures = 0;
    newly_discovered
}

fn establish_baseline_from_success(
    state: &mut CodexModelUpdateState,
    baseline: &[ModelCatalogEntry],
    available: &[ModelCatalogEntry],
    now: &str,
) {
    if state.baseline_ready {
        return;
    }
    let seed = if baseline.is_empty() {
        available
    } else {
        baseline
    };
    establish_baseline(state, seed, now);
}

fn establish_baseline(
    state: &mut CodexModelUpdateState,
    baseline: &[ModelCatalogEntry],
    now: &str,
) {
    if state.baseline_ready || baseline.is_empty() {
        if state.initialized_at.is_empty() {
            state.initialized_at = now.to_string();
        }
        return;
    }
    state.known_model_ids.extend(
        baseline
            .iter()
            .map(|entry| normalize_codex_model_id(&entry.id)),
    );
    state.baseline_ready = true;
    if state.initialized_at.is_empty() {
        state.initialized_at = now.to_string();
    }
}

fn resolve_pending_updates(
    state: &mut CodexModelUpdateState,
    model_id: Option<&str>,
    decision: &str,
    decided_at: &str,
    agent_id: Option<String>,
) -> Vec<String> {
    let mut resolved = Vec::new();
    state.pending.retain(|update| {
        let matches =
            model_id.is_none_or(|model_id| update.model_id.eq_ignore_ascii_case(model_id));
        if matches {
            resolved.push(update.model_id.clone());
            false
        } else {
            true
        }
    });
    for model_id in &resolved {
        state.recent_decisions.push(CodexModelUpdateDecisionRecord {
            model_id: model_id.clone(),
            decision: decision.to_string(),
            decided_at: decided_at.to_string(),
            agent_id: agent_id.clone(),
        });
    }
    if state.recent_decisions.len() > MAX_DECISION_HISTORY {
        let overflow = state.recent_decisions.len() - MAX_DECISION_HISTORY;
        state.recent_decisions.drain(0..overflow);
    }
    resolved
}

fn has_keep_intent(message: &str) -> bool {
    if message.contains('?') {
        return false;
    }
    let normalized = normalize_decision_text(message);
    [
        "garder le modèle actuel",
        "garder le modele actuel",
        "conserver le modèle actuel",
        "conserver le modele actuel",
        "keep the current model",
    ]
    .iter()
    .any(|decision| {
        normalized == *decision
            || ["oui ", "ok ", "okay ", "je veux "]
                .iter()
                .any(|prefix| normalized == format!("{prefix}{decision}"))
    })
}

fn normalize_codex_model_id(model: &str) -> String {
    let trimmed = model.trim();
    let slug = trimmed
        .split_once('/')
        .filter(|(provider, _)| {
            provider.eq_ignore_ascii_case("codex") || provider.eq_ignore_ascii_case("openai-codex")
        })
        .map(|(_, slug)| slug)
        .unwrap_or(trimmed);
    format!("codex/{}", slug.to_ascii_lowercase())
}

fn normalize_decision_text(message: &str) -> String {
    message
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_codex_provider(provider: &str) -> bool {
    provider.eq_ignore_ascii_case("codex") || provider.eq_ignore_ascii_case("openai-codex")
}

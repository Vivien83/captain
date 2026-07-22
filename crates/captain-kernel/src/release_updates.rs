//! Durable Captain release discovery, notification and operator decisions.

use std::sync::Arc;
use std::time::Duration;

use captain_channels::telegram::parse_runtime_update_callback;
use captain_runtime::audit::AuditAction;
use captain_types::error::CaptainError;
use captain_types::release_update::{
    RuntimeUpdateCard, RuntimeUpdateInstallMode, RuntimeUpdateOperatorContext,
    RuntimeUpdateOperatorResolution,
};
use captain_types::version::canonical_version;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::{KernelError, KernelResult};
use crate::{shared_memory_agent_id, CaptainKernel};

#[path = "release_updates_delivery.rs"]
mod delivery;
#[path = "release_updates_operator.rs"]
mod operator;
#[path = "release_updates_process.rs"]
mod process;
#[path = "release_updates_source.rs"]
mod source;
#[path = "release_updates_state.rs"]
mod state;
#[cfg(test)]
#[path = "release_updates_tests.rs"]
mod tests;

use operator::*;
use process::*;
use source::*;
use state::*;

const STATE_KEY: &str = "__captain_runtime_updates_v1";
const STATE_SCHEMA_VERSION: u16 = 1;
const INITIAL_CHECK_DELAY: Duration = Duration::from_secs(15);
const MONITOR_TICK: Duration = Duration::from_secs(30);
const CHECK_INTERVAL_MS: i64 = 12 * 60 * 60 * 1_000;
const DEFER_INTERVAL_MS: i64 = 24 * 60 * 60 * 1_000;
const FIRST_FAILURE_RETRY_MS: i64 = 15 * 60 * 1_000;
const UPDATE_RESTART_GRACE_MS: i64 = 5 * 60 * 1_000;
const UPDATE_ATTEMPT_STALE_MS: i64 = 30 * 60 * 1_000;
const OUTBOX_LEASE_MS: i64 = 120_000;
const OUTBOX_MAX_ATTEMPTS: u32 = 24;
const MAX_HISTORY: usize = 32;
const DEFAULT_GITHUB_REPO: &str = "Vivien83/captain";
const UPDATE_ATTEMPT_ID_ENV: &str = "CAPTAIN_UPDATE_ATTEMPT_ID";
const UPDATE_RESULT_PATH_ENV: &str = "CAPTAIN_UPDATE_RESULT_PATH";

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeUpdateSnapshot {
    pub last_checked_at: Option<String>,
    pub last_success_at: Option<String>,
    pub next_check_at: String,
    pub last_error: Option<String>,
    pub consecutive_failures: u32,
    pub pending_version: Option<String>,
    pub update_in_progress: bool,
    pub undelivered_notifications: usize,
    pub dead_notifications: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeUpdateCandidate {
    token: String,
    decision_version: u64,
    current_version: String,
    available_version: String,
    release_tag: String,
    release_url: String,
    published_at: Option<String>,
    prerelease: bool,
    install_mode: RuntimeUpdateInstallMode,
    discovered_at_unix_ms: i64,
    deferred_until_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeUpdateAttempt {
    attempt_id: String,
    requested_version: String,
    started_at_unix_ms: i64,
    log_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RuntimeUpdateDecisionRecord {
    available_version: String,
    decision: String,
    actor: String,
    decided_at_unix_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RuntimeUpdateOutboxStatus {
    Pending,
    Delivering,
    Delivered,
    Suppressed,
    Dead,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct RuntimeUpdateOutbox {
    id: String,
    card: RuntimeUpdateCard,
    status: RuntimeUpdateOutboxStatus,
    attempt_count: u32,
    max_attempts: u32,
    run_after_unix_ms: i64,
    lease_owner: Option<String>,
    lease_expires_at_unix_ms: Option<i64>,
    external_message_id: Option<String>,
    last_error: Option<String>,
    delivered_at_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct RuntimeUpdateState {
    schema_version: u16,
    last_checked_at_unix_ms: Option<i64>,
    last_success_at_unix_ms: Option<i64>,
    next_check_at_unix_ms: i64,
    last_error: Option<String>,
    consecutive_failures: u32,
    next_decision_version: u64,
    pending: Option<RuntimeUpdateCandidate>,
    active_attempt: Option<RuntimeUpdateAttempt>,
    decisions: Vec<RuntimeUpdateDecisionRecord>,
    consumed_attempt_ids: Vec<String>,
    outbox: Vec<RuntimeUpdateOutbox>,
}

impl Default for RuntimeUpdateState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            last_checked_at_unix_ms: None,
            last_success_at_unix_ms: None,
            next_check_at_unix_ms: 0,
            last_error: None,
            consecutive_failures: 0,
            next_decision_version: 1,
            pending: None,
            active_attempt: None,
            decisions: Vec::new(),
            consumed_attempt_ids: Vec::new(),
            outbox: Vec::new(),
        }
    }
}

pub fn spawn_runtime_update_monitor(kernel: Arc<CaptainKernel>) {
    delivery::spawn_runtime_update_delivery_worker(Arc::clone(&kernel));
    tokio::spawn(async move {
        tokio::time::sleep(INITIAL_CHECK_DELAY).await;
        let mut state_error_logged = false;
        loop {
            if kernel.supervisor.is_shutting_down() {
                break;
            }
            if let Err(error) = reconcile_update_attempt_result(&kernel) {
                if !state_error_logged {
                    warn!(error = %error, "runtime update result reconciliation deferred");
                    state_error_logged = true;
                }
                tokio::time::sleep(MONITOR_TICK).await;
                continue;
            }
            match runtime_update_check_due(&kernel, now_unix_ms()) {
                Ok(true) => {
                    state_error_logged = false;
                    if let Err(error) = scan_runtime_update_once(&kernel).await {
                        warn!(error = %error, "Captain release check deferred");
                    }
                }
                Ok(false) => state_error_logged = false,
                Err(error) if !state_error_logged => {
                    state_error_logged = true;
                    warn!(error = %error, "Captain release monitor paused on invalid durable state");
                }
                Err(_) => {}
            }
            tokio::time::sleep(MONITOR_TICK).await;
        }
    });
}

pub async fn scan_runtime_update_once(kernel: &CaptainKernel) -> Result<Option<String>, String> {
    let current = captain_types::version::captain_version();
    let install_mode = runtime_update_install_mode();
    let now = now_unix_ms();
    match fetch_release_candidate(
        &current,
        install_mode == RuntimeUpdateInstallMode::SelfUpdate,
    )
    .await
    {
        Ok(candidate) => {
            let version = candidate
                .as_ref()
                .map(|release| canonical_version(&release.tag_name).to_string());
            kernel
                .mutate_runtime_update_state(|state| {
                    reconcile_scan_success(state, &current, candidate.as_ref(), install_mode, now)
                })
                .map_err(|error| error.to_string())?;
            if let Some(version) = version.as_deref() {
                info!(
                    current,
                    available = version,
                    "Captain release update detected"
                );
            } else {
                debug!(current, "Captain release channel is current");
            }
            Ok(version)
        }
        Err(error) => {
            kernel
                .mutate_runtime_update_state(|state| reconcile_scan_failure(state, &error, now))
                .map_err(|state_error| format!("{error}; state update failed: {state_error}"))?;
            Err(error)
        }
    }
}

impl CaptainKernel {
    pub fn runtime_update_snapshot(&self) -> KernelResult<RuntimeUpdateSnapshot> {
        let state = self.load_runtime_update_state()?.unwrap_or_default();
        let next_check_at = if state.next_check_at_unix_ms <= 0 {
            now_unix_ms().saturating_add(INITIAL_CHECK_DELAY.as_millis() as i64)
        } else {
            state.next_check_at_unix_ms
        };
        Ok(RuntimeUpdateSnapshot {
            last_checked_at: state.last_checked_at_unix_ms.map(format_unix_ms),
            last_success_at: state.last_success_at_unix_ms.map(format_unix_ms),
            next_check_at: format_unix_ms(next_check_at),
            last_error: state.last_error,
            consecutive_failures: state.consecutive_failures,
            pending_version: state
                .pending
                .as_ref()
                .map(|candidate| candidate.available_version.clone()),
            update_in_progress: state.active_attempt.is_some(),
            undelivered_notifications: state
                .outbox
                .iter()
                .filter(|item| {
                    matches!(
                        item.status,
                        RuntimeUpdateOutboxStatus::Pending | RuntimeUpdateOutboxStatus::Delivering
                    )
                })
                .count(),
            dead_notifications: state
                .outbox
                .iter()
                .filter(|item| {
                    item.status == RuntimeUpdateOutboxStatus::Dead
                        && state.pending.as_ref().is_some_and(|candidate| {
                            candidate.token == item.card.token
                                && candidate.decision_version == item.card.decision_version
                        })
                })
                .count(),
        })
    }

    pub async fn runtime_update_resolve_telegram_callback(
        &self,
        callback_data: &str,
        actor: &str,
        context: &RuntimeUpdateOperatorContext,
    ) -> Result<RuntimeUpdateOperatorResolution, String> {
        authorize_runtime_update_operator(self, actor, context)?;
        let callback = parse_runtime_update_callback(callback_data)
            .ok_or_else(|| "Le bouton de mise à jour est invalide.".to_string())?;
        let now = now_unix_ms();
        let effect = self
            .mutate_runtime_update_state(|state| {
                apply_operator_decision(state, &callback, actor, &self.config.home_dir, now)
            })
            .map_err(|error| error.to_string())??;

        match effect {
            OperatorDecisionEffect::Resolved(resolution) => {
                self.audit_runtime_update(actor, &resolution);
                Ok(resolution)
            }
            OperatorDecisionEffect::Launch {
                attempt,
                resolution,
            } => match spawn_detached_update(self, &attempt) {
                Ok(()) => {
                    self.audit_runtime_update(actor, &resolution);
                    Ok(resolution)
                }
                Err(error) => {
                    self.recover_failed_update_launch(&attempt.attempt_id, &error, now)?;
                    self.audit_log.record(
                        actor,
                        AuditAction::ConfigChange,
                        "Captain runtime update launch failed",
                        bound(&error, 2_048),
                    );
                    Err(format!(
                            "La mise à jour n'a pas pu démarrer : {error}. Une nouvelle carte a été mise en file d'attente."
                        ))
                }
            },
        }
    }

    fn audit_runtime_update(&self, actor: &str, resolution: &RuntimeUpdateOperatorResolution) {
        self.audit_log.record(
            actor,
            AuditAction::ConfigChange,
            "Captain runtime update decision",
            format!(
                "status={:?}; current={}; available={}",
                resolution.status, resolution.current_version, resolution.available_version
            ),
        );
    }

    fn recover_failed_update_launch(
        &self,
        attempt_id: &str,
        error: &str,
        now: i64,
    ) -> Result<(), String> {
        self.mutate_runtime_update_state(|state| {
            if state
                .active_attempt
                .as_ref()
                .is_some_and(|attempt| attempt.attempt_id == attempt_id)
            {
                state.active_attempt = None;
                requeue_candidate_after_failure(state, error, now);
            }
        })
        .map_err(|state_error| state_error.to_string())
    }

    fn load_runtime_update_state(&self) -> KernelResult<Option<RuntimeUpdateState>> {
        let value = self
            .memory
            .structured_get(shared_memory_agent_id(), STATE_KEY)
            .map_err(KernelError::Captain)?;
        value
            .map(|value| {
                decode_runtime_update_state(value).map_err(|error| {
                    KernelError::Captain(CaptainError::Internal(format!(
                        "Invalid persisted runtime update state: {error}"
                    )))
                })
            })
            .transpose()
    }

    fn mutate_runtime_update_state<T>(
        &self,
        mutate: impl FnOnce(&mut RuntimeUpdateState) -> T,
    ) -> KernelResult<T> {
        let _guard = self
            .runtime_update_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut state = self.load_runtime_update_state()?.unwrap_or_default();
        let output = mutate(&mut state);
        trim_state(&mut state);
        self.memory
            .structured_set(
                shared_memory_agent_id(),
                STATE_KEY,
                serde_json::to_value(state).map_err(|error| {
                    KernelError::Captain(CaptainError::Internal(format!(
                        "Failed to serialize runtime update state: {error}"
                    )))
                })?,
            )
            .map_err(KernelError::Captain)?;
        Ok(output)
    }

    pub(super) fn claim_runtime_update_outbox(
        &self,
        lease_owner: &str,
        now: i64,
    ) -> KernelResult<Option<RuntimeUpdateOutbox>> {
        self.mutate_runtime_update_state(|state| claim_outbox(state, lease_owner, now))
    }

    pub(super) fn complete_runtime_update_outbox(
        &self,
        claimed: &RuntimeUpdateOutbox,
        external_message_id: Option<String>,
        now: i64,
    ) -> KernelResult<()> {
        self.mutate_runtime_update_state(|state| {
            if let Some(item) = state.outbox.iter_mut().find(|item| item.id == claimed.id) {
                if item.status == RuntimeUpdateOutboxStatus::Delivering
                    && item.lease_owner == claimed.lease_owner
                {
                    item.status = RuntimeUpdateOutboxStatus::Delivered;
                    item.external_message_id = external_message_id;
                    item.delivered_at_unix_ms = Some(now);
                    item.lease_owner = None;
                    item.lease_expires_at_unix_ms = None;
                    item.last_error = None;
                }
            }
        })
    }

    pub(super) fn retry_runtime_update_outbox(
        &self,
        claimed: &RuntimeUpdateOutbox,
        error: &str,
        now: i64,
    ) -> KernelResult<()> {
        self.mutate_runtime_update_state(|state| {
            if let Some(item) = state.outbox.iter_mut().find(|item| item.id == claimed.id) {
                if item.status != RuntimeUpdateOutboxStatus::Delivering
                    || item.lease_owner != claimed.lease_owner
                {
                    return;
                }
                item.last_error = Some(bound(error, 2_048));
                item.lease_owner = None;
                item.lease_expires_at_unix_ms = None;
                if item.attempt_count >= item.max_attempts {
                    item.status = RuntimeUpdateOutboxStatus::Dead;
                } else {
                    item.status = RuntimeUpdateOutboxStatus::Pending;
                    item.run_after_unix_ms =
                        now.saturating_add(outbox_retry_delay_ms(item.attempt_count));
                }
            }
        })
    }
}

fn decode_runtime_update_state(value: serde_json::Value) -> Result<RuntimeUpdateState, String> {
    let state = serde_json::from_value::<RuntimeUpdateState>(value)
        .map_err(|error| format!("invalid JSON contract: {error}"))?;
    if state.schema_version != STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema version {} (runtime supports {})",
            state.schema_version, STATE_SCHEMA_VERSION
        ));
    }
    Ok(state)
}

fn runtime_update_check_due(kernel: &CaptainKernel, now: i64) -> KernelResult<bool> {
    Ok(kernel
        .load_runtime_update_state()?
        .unwrap_or_default()
        .next_check_at_unix_ms
        .saturating_sub(now)
        <= 0)
}

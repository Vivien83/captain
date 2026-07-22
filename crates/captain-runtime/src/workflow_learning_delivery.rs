//! Durable preparation and settlement of Skill Learning V2 notifications.
//!
//! This layer owns no channel transport. It claims one outbox lease, verifies
//! the immutable proposal identity, projects the shared operator card, and
//! settles stale or malformed work without involving an LLM.

use captain_memory::workflow_learning_control::{
    WorkflowArtifactKind, WorkflowIsolatedTestStatus, WorkflowLearningControlError,
    WorkflowLearningStore, WorkflowProposalState,
};
use captain_memory::workflow_learning_installation::WorkflowInstallationPhase;
use captain_memory::workflow_learning_outbox::WorkflowOutboxRecord;
use captain_memory::workflow_learning_queue::{
    WorkflowJobEffectState, WorkflowJobKind, WorkflowJobStatus,
};
use captain_types::workflow_learning::{
    ProposalCard, WorkflowLifecycleCard, WorkflowLifecycleEvent,
    WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION,
};
use serde::Deserialize;

use crate::workflow_learning_card::{map_artifact_kind, map_state, project_workflow_proposal_card};
use crate::workflow_learning_staging::WorkflowStagingRoot;

pub const WORKFLOW_PROPOSAL_OUTBOX_TOPIC: &str = "workflow_learning.proposed";
pub const WORKFLOW_LIFECYCLE_OUTBOX_TOPIC: &str = "workflow_learning.lifecycle";
const PAYLOAD_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, thiserror::Error)]
pub enum WorkflowDeliveryError {
    #[error("invalid workflow delivery configuration: {0}")]
    InvalidConfiguration(String),
    #[error(transparent)]
    Control(#[from] WorkflowLearningControlError),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct WorkflowProposalDelivery {
    pub outbox: WorkflowOutboxRecord,
    pub card: ProposalCard,
    pub event: WorkflowDeliveryEvent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowDeliveryEvent {
    Proposed,
    IsolatedTestCompleted { passed: bool },
    Lifecycle(WorkflowLifecycleCard),
}

#[derive(Debug, Clone)]
pub enum WorkflowDeliveryDisposition {
    Idle,
    Ready(WorkflowProposalDelivery),
    Suppressed {
        outbox_id: String,
        proposal_id: String,
        proposal_state: String,
    },
    DeadLettered {
        outbox_id: String,
        reason: String,
    },
}

#[derive(Clone)]
pub struct WorkflowDeliveryPlanner {
    control: WorkflowLearningStore,
    staging: WorkflowStagingRoot,
    worker_id: String,
    lease_duration_ms: i64,
}

impl WorkflowDeliveryPlanner {
    pub fn new(
        control: WorkflowLearningStore,
        staging: WorkflowStagingRoot,
        worker_id: impl Into<String>,
        lease_duration_ms: i64,
    ) -> Result<Self, WorkflowDeliveryError> {
        let worker_id = worker_id.into();
        if worker_id.trim().is_empty() || worker_id.len() > 96 {
            return Err(WorkflowDeliveryError::InvalidConfiguration(
                "worker id must contain 1 to 96 characters".to_string(),
            ));
        }
        if !(1_000..=3_600_000).contains(&lease_duration_ms) {
            return Err(WorkflowDeliveryError::InvalidConfiguration(
                "lease must be between 1 second and 1 hour".to_string(),
            ));
        }
        Ok(Self {
            control,
            staging,
            worker_id,
            lease_duration_ms,
        })
    }

    pub fn claim_next(
        &self,
        now_unix_ms: i64,
    ) -> Result<WorkflowDeliveryDisposition, WorkflowDeliveryError> {
        let Some(outbox) = self.control.claim_due_outbox_for_topics(
            &self.worker_id,
            &[
                WORKFLOW_PROPOSAL_OUTBOX_TOPIC,
                WORKFLOW_LIFECYCLE_OUTBOX_TOPIC,
            ],
            now_unix_ms,
            self.lease_duration_ms,
        )?
        else {
            return Ok(WorkflowDeliveryDisposition::Idle);
        };

        match self.prepare(&outbox) {
            Ok(Preparation::Ready { card, event }) => Ok(WorkflowDeliveryDisposition::Ready(
                WorkflowProposalDelivery {
                    outbox,
                    card,
                    event,
                },
            )),
            Ok(Preparation::Suppress { proposal_state }) => {
                let receipt = serde_json::to_string(&serde_json::json!({
                    "schema_version": 1,
                    "status": "suppressed",
                    "reason": "proposal_not_actionable",
                    "proposal_state": proposal_state,
                }))?;
                self.control.complete_outbox(
                    &outbox.id,
                    &self.worker_id,
                    Some(&receipt),
                    now_unix_ms,
                )?;
                Ok(WorkflowDeliveryDisposition::Suppressed {
                    outbox_id: outbox.id,
                    proposal_id: outbox.proposal_id,
                    proposal_state,
                })
            }
            Err(PreparationError::Permanent(reason)) => {
                let reason = bounded_error(&reason);
                self.control.dead_letter_outbox(
                    &outbox.id,
                    &self.worker_id,
                    &reason,
                    now_unix_ms,
                )?;
                Ok(WorkflowDeliveryDisposition::DeadLettered {
                    outbox_id: outbox.id,
                    reason,
                })
            }
            Err(PreparationError::Control(error)) => Err(error.into()),
        }
    }

    pub fn complete(
        &self,
        delivery: &WorkflowProposalDelivery,
        delivery_result_json: &str,
        completed_at_unix_ms: i64,
    ) -> Result<WorkflowOutboxRecord, WorkflowDeliveryError> {
        Ok(self.control.complete_outbox(
            &delivery.outbox.id,
            &self.worker_id,
            Some(delivery_result_json),
            completed_at_unix_ms,
        )?)
    }

    pub fn retry(
        &self,
        delivery: &WorkflowProposalDelivery,
        error: &str,
        failed_at_unix_ms: i64,
    ) -> Result<WorkflowOutboxRecord, WorkflowDeliveryError> {
        let retry_at =
            failed_at_unix_ms.saturating_add(retry_backoff_ms(delivery.outbox.attempt_count));
        Ok(self.control.fail_outbox(
            &delivery.outbox.id,
            &self.worker_id,
            &bounded_error(error),
            retry_at,
            failed_at_unix_ms,
        )?)
    }

    fn prepare(&self, outbox: &WorkflowOutboxRecord) -> Result<Preparation, PreparationError> {
        match outbox.topic.as_str() {
            WORKFLOW_PROPOSAL_OUTBOX_TOPIC => self.prepare_proposed(outbox),
            WORKFLOW_LIFECYCLE_OUTBOX_TOPIC => self.prepare_lifecycle(outbox),
            _ => Err(PreparationError::Permanent(format!(
                "unsupported outbox topic {}",
                outbox.topic
            ))),
        }
    }

    fn prepare_proposed(
        &self,
        outbox: &WorkflowOutboxRecord,
    ) -> Result<Preparation, PreparationError> {
        let payload: ProposedNotificationPayload = serde_json::from_str(&outbox.payload_json)
            .map_err(|error| {
                PreparationError::Permanent(format!(
                    "invalid proposed notification payload: {error}"
                ))
            })?;
        if payload.schema_version != PAYLOAD_SCHEMA_VERSION || payload.state != "proposed" {
            return Err(PreparationError::Permanent(
                "unsupported proposed notification contract".to_string(),
            ));
        }
        if payload.proposal_id != outbox.proposal_id
            || outbox.revision_sha256.as_deref() != Some(payload.revision_sha256.as_str())
        {
            return Err(PreparationError::Permanent(
                "outbox payload identity differs from its durable envelope".to_string(),
            ));
        }
        let proposal = self
            .control
            .get(&outbox.proposal_id)
            .map_err(PreparationError::Control)?
            .ok_or_else(|| {
                PreparationError::Permanent("outbox proposal no longer exists".to_string())
            })?;
        if proposal.revision_sha256.as_deref() != Some(payload.revision_sha256.as_str()) {
            return Err(PreparationError::Permanent(
                "outbox revision differs from the durable proposal".to_string(),
            ));
        }
        if proposal.state != WorkflowProposalState::Proposed {
            return Ok(Preparation::Suppress {
                proposal_state: proposal.state.as_str().to_string(),
            });
        }
        let card = project_workflow_proposal_card(&proposal, &self.staging).map_err(|error| {
            PreparationError::Permanent(format!("operator card projection failed: {error}"))
        })?;
        Ok(Preparation::Ready {
            card,
            event: WorkflowDeliveryEvent::Proposed,
        })
    }

    fn prepare_lifecycle(
        &self,
        outbox: &WorkflowOutboxRecord,
    ) -> Result<Preparation, PreparationError> {
        let envelope: LifecycleNotificationEnvelope = serde_json::from_str(&outbox.payload_json)
            .map_err(|error| {
                PreparationError::Permanent(format!(
                    "invalid lifecycle notification envelope: {error}"
                ))
            })?;
        if envelope.schema_version != PAYLOAD_SCHEMA_VERSION {
            return Err(PreparationError::Permanent(
                "unsupported lifecycle notification schema".to_string(),
            ));
        }
        if envelope.event == "isolated_test_completed" {
            let payload: IsolatedTestNotificationPayload =
                serde_json::from_str(&outbox.payload_json).map_err(|error| {
                    PreparationError::Permanent(format!(
                        "invalid isolated-test notification payload: {error}"
                    ))
                })?;
            return self.prepare_isolated_test_lifecycle(outbox, payload);
        }
        let card: WorkflowLifecycleCard =
            serde_json::from_str(&outbox.payload_json).map_err(|error| {
                PreparationError::Permanent(format!("invalid activation lifecycle card: {error}"))
            })?;
        self.prepare_activation_lifecycle(outbox, card)
    }

    fn prepare_isolated_test_lifecycle(
        &self,
        outbox: &WorkflowOutboxRecord,
        payload: IsolatedTestNotificationPayload,
    ) -> Result<Preparation, PreparationError> {
        if payload.schema_version != PAYLOAD_SCHEMA_VERSION
            || payload.event != "isolated_test_completed"
            || payload.state != "proposed"
            || payload.proposal_id != outbox.proposal_id
            || outbox.revision_sha256.as_deref() != Some(payload.revision_sha256.as_str())
        {
            return Err(PreparationError::Permanent(
                "unsupported or inconsistent isolated-test notification contract".to_string(),
            ));
        }
        let proposal = self
            .control
            .get(&outbox.proposal_id)
            .map_err(PreparationError::Control)?
            .ok_or_else(|| {
                PreparationError::Permanent("lifecycle proposal no longer exists".to_string())
            })?;
        if proposal.revision_sha256.as_deref() != Some(payload.revision_sha256.as_str()) {
            return Ok(Preparation::Suppress {
                proposal_state: proposal.state.as_str().to_string(),
            });
        }
        let exact_test = self
            .control
            .isolated_test_by_job_id(&payload.test_job_id)
            .map_err(PreparationError::Control)?
            .ok_or_else(|| {
                PreparationError::Permanent(
                    "lifecycle notification has no durable isolated-test evidence".to_string(),
                )
            })?;
        let expected_status = if payload.passed {
            WorkflowIsolatedTestStatus::Passed
        } else {
            WorkflowIsolatedTestStatus::Failed
        };
        if exact_test.proposal_id != payload.proposal_id
            || exact_test.revision_sha256 != payload.revision_sha256
            || exact_test.status != expected_status
        {
            return Err(PreparationError::Permanent(
                "lifecycle notification differs from isolated-test evidence".to_string(),
            ));
        }
        if proposal.state != WorkflowProposalState::Proposed {
            return Ok(Preparation::Suppress {
                proposal_state: proposal.state.as_str().to_string(),
            });
        }
        let latest_test = proposal.isolated_test.as_ref().ok_or_else(|| {
            PreparationError::Permanent(
                "lifecycle notification has no durable isolated-test evidence".to_string(),
            )
        })?;
        if latest_test.job_id != payload.test_job_id {
            return Ok(Preparation::Suppress {
                proposal_state: "newer_isolated_test".to_string(),
            });
        }
        let card = project_workflow_proposal_card(&proposal, &self.staging).map_err(|error| {
            PreparationError::Permanent(format!("lifecycle card projection failed: {error}"))
        })?;
        Ok(Preparation::Ready {
            card,
            event: WorkflowDeliveryEvent::IsolatedTestCompleted {
                passed: payload.passed,
            },
        })
    }

    fn prepare_activation_lifecycle(
        &self,
        outbox: &WorkflowOutboxRecord,
        lifecycle: WorkflowLifecycleCard,
    ) -> Result<Preparation, PreparationError> {
        if lifecycle.schema_version != WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION
            || lifecycle.proposal_id != outbox.proposal_id
            || outbox.revision_sha256.as_deref() != Some(lifecycle.revision_sha256.as_str())
        {
            return Err(PreparationError::Permanent(
                "activation lifecycle card differs from its durable envelope".to_string(),
            ));
        }
        let proposal = self
            .control
            .get(&outbox.proposal_id)
            .map_err(PreparationError::Control)?
            .ok_or_else(|| {
                PreparationError::Permanent("lifecycle proposal no longer exists".to_string())
            })?;
        if proposal.revision_sha256.as_deref() != Some(lifecycle.revision_sha256.as_str())
            || proposal.kind.map(map_artifact_kind) != Some(lifecycle.kind)
            || proposal.name.as_deref() != Some(lifecycle.name.as_str())
        {
            return Err(PreparationError::Permanent(
                "activation lifecycle identity differs from the durable proposal".to_string(),
            ));
        }
        let (expected_state, expected_kind, expected_status, expected_result_event) =
            lifecycle_expectations(lifecycle.event)?;
        if lifecycle.state != map_state(expected_state) {
            return Err(PreparationError::Permanent(
                "activation lifecycle state does not match its event".to_string(),
            ));
        }
        let transition_exists = if lifecycle.event == WorkflowLifecycleEvent::RollbackFailed {
            proposal.state == expected_state && proposal.state_version == lifecycle.decision_version
        } else {
            self.control
                .events(&proposal.id)
                .map_err(PreparationError::Control)?
                .into_iter()
                .any(|event| {
                    event.to_state == expected_state
                        && event.resulting_version == lifecycle.decision_version
                        && event.revision_sha256.as_deref()
                            == Some(lifecycle.revision_sha256.as_str())
                        && event.created_at_unix_ms == lifecycle.occurred_at_unix_ms
                })
        };
        if !transition_exists {
            return Err(PreparationError::Permanent(
                "activation lifecycle card has no exact durable proposal transition".to_string(),
            ));
        }
        let job = self
            .control
            .get_job(&lifecycle.lifecycle_job_id)
            .map_err(PreparationError::Control)?
            .ok_or_else(|| {
                PreparationError::Permanent("activation lifecycle job is missing".to_string())
            })?;
        if job.proposal_id != proposal.id
            || job.revision_sha256.as_deref() != Some(lifecycle.revision_sha256.as_str())
            || !expected_kind.contains(&job.kind)
            || job.status != expected_status
            || job.effect_state != WorkflowJobEffectState::Completed
            || job.updated_at_unix_ms != lifecycle.occurred_at_unix_ms
        {
            return Err(PreparationError::Permanent(
                "activation lifecycle card differs from its settled job".to_string(),
            ));
        }
        verify_lifecycle_continuation(&self.control, &lifecycle, &proposal.id)?;
        verify_lifecycle_installation(&self.control, &lifecycle, proposal.kind)?;
        if let Some(result_event) = expected_result_event {
            verify_lifecycle_job_result(&job, &lifecycle, result_event)?;
        } else {
            let proposal_error_mismatch = lifecycle.event
                == WorkflowLifecycleEvent::ActivationFailed
                && (proposal.last_error_code.as_deref() != lifecycle.failure_code.as_deref()
                    || proposal.last_error_message.as_deref()
                        != lifecycle.failure_message.as_deref());
            if job.error_code.as_deref() != lifecycle.failure_code.as_deref()
                || job.error_message.as_deref() != lifecycle.failure_message.as_deref()
                || proposal_error_mismatch
            {
                return Err(PreparationError::Permanent(
                    "activation failure card differs from durable failure evidence".to_string(),
                ));
            }
        }
        let card = project_workflow_proposal_card(&proposal, &self.staging).map_err(|error| {
            PreparationError::Permanent(format!("lifecycle card projection failed: {error}"))
        })?;
        Ok(Preparation::Ready {
            card,
            event: WorkflowDeliveryEvent::Lifecycle(lifecycle),
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposedNotificationPayload {
    schema_version: u16,
    proposal_id: String,
    revision_sha256: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct LifecycleNotificationEnvelope {
    schema_version: u16,
    event: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IsolatedTestNotificationPayload {
    schema_version: u16,
    event: String,
    proposal_id: String,
    revision_sha256: String,
    test_job_id: String,
    state: String,
    passed: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleJobResult {
    schema_version: u16,
    event: String,
    proposal_id: String,
    revision_sha256: String,
    target_locator: String,
}

enum Preparation {
    Ready {
        card: ProposalCard,
        event: WorkflowDeliveryEvent,
    },
    Suppress {
        proposal_state: String,
    },
}

enum PreparationError {
    Permanent(String),
    Control(WorkflowLearningControlError),
}

type LifecycleExpectations = (
    WorkflowProposalState,
    &'static [WorkflowJobKind],
    WorkflowJobStatus,
    Option<&'static str>,
);

fn lifecycle_expectations(
    event: WorkflowLifecycleEvent,
) -> Result<LifecycleExpectations, PreparationError> {
    match event {
        WorkflowLifecycleEvent::InstallationVerified => Ok((
            WorkflowProposalState::ActiveCanary,
            &[WorkflowJobKind::Install],
            WorkflowJobStatus::Succeeded,
            Some("installation_verified"),
        )),
        WorkflowLifecycleEvent::ActivationCompleted => Ok((
            WorkflowProposalState::Active,
            &[WorkflowJobKind::Canary],
            WorkflowJobStatus::Succeeded,
            Some("canary_passed"),
        )),
        WorkflowLifecycleEvent::ActivationFailed => Ok((
            WorkflowProposalState::InstallFailed,
            &[WorkflowJobKind::Install, WorkflowJobKind::Canary],
            WorkflowJobStatus::Dead,
            None,
        )),
        WorkflowLifecycleEvent::RollbackCompleted => Ok((
            WorkflowProposalState::RolledBack,
            &[WorkflowJobKind::Rollback],
            WorkflowJobStatus::Succeeded,
            Some("rollback_verified"),
        )),
        WorkflowLifecycleEvent::RollbackFailed => Ok((
            WorkflowProposalState::InstallFailed,
            &[WorkflowJobKind::Rollback],
            WorkflowJobStatus::Dead,
            None,
        )),
    }
}

fn verify_lifecycle_continuation(
    control: &WorkflowLearningStore,
    lifecycle: &WorkflowLifecycleCard,
    proposal_id: &str,
) -> Result<(), PreparationError> {
    let continuation = lifecycle
        .continuation_job_id
        .as_deref()
        .map(|id| {
            control
                .get_job(id)
                .map_err(PreparationError::Control)?
                .ok_or_else(|| {
                    PreparationError::Permanent("lifecycle continuation job is missing".to_string())
                })
        })
        .transpose()?;
    let rollback = lifecycle
        .rollback_job_id
        .as_deref()
        .map(|id| {
            control
                .get_job(id)
                .map_err(PreparationError::Control)?
                .ok_or_else(|| {
                    PreparationError::Permanent("lifecycle rollback job is missing".to_string())
                })
        })
        .transpose()?;
    let exact = |job: &captain_memory::workflow_learning_queue::WorkflowJobRecord,
                 kind: WorkflowJobKind| {
        job.proposal_id == proposal_id
            && job.revision_sha256.as_deref() == Some(lifecycle.revision_sha256.as_str())
            && job.kind == kind
    };
    let valid = match lifecycle.event {
        WorkflowLifecycleEvent::InstallationVerified => {
            continuation
                .as_ref()
                .is_some_and(|job| exact(job, WorkflowJobKind::Canary))
                && rollback.is_none()
        }
        WorkflowLifecycleEvent::ActivationFailed => {
            continuation.is_none()
                && match rollback.as_ref() {
                    Some(job) => exact(job, WorkflowJobKind::Rollback),
                    None => true,
                }
        }
        WorkflowLifecycleEvent::ActivationCompleted
        | WorkflowLifecycleEvent::RollbackCompleted
        | WorkflowLifecycleEvent::RollbackFailed => continuation.is_none() && rollback.is_none(),
    };
    if valid {
        Ok(())
    } else {
        Err(PreparationError::Permanent(
            "lifecycle continuation identity is inconsistent".to_string(),
        ))
    }
}

fn verify_lifecycle_installation(
    control: &WorkflowLearningStore,
    lifecycle: &WorkflowLifecycleCard,
    proposal_kind: Option<WorkflowArtifactKind>,
) -> Result<(), PreparationError> {
    let installation = control
        .get_installation(&lifecycle.proposal_id, &lifecycle.revision_sha256)
        .map_err(PreparationError::Control)?;
    let required = lifecycle.event != WorkflowLifecycleEvent::ActivationFailed
        || lifecycle.rollback_job_id.is_some();
    if !required && installation.is_none() && lifecycle.target_locator.is_none() {
        return Ok(());
    }
    let installation = installation.ok_or_else(|| {
        PreparationError::Permanent("lifecycle installation mirror is missing".to_string())
    })?;
    if Some(installation.kind) != proposal_kind
        || lifecycle.target_locator.as_deref() != Some(installation.target_locator.as_str())
    {
        return Err(PreparationError::Permanent(
            "lifecycle card differs from the installation mirror".to_string(),
        ));
    }
    let phase_valid = match lifecycle.event {
        WorkflowLifecycleEvent::InstallationVerified => matches!(
            installation.phase,
            WorkflowInstallationPhase::Verified
                | WorkflowInstallationPhase::Active
                | WorkflowInstallationPhase::Failed
                | WorkflowInstallationPhase::RollbackPending
                | WorkflowInstallationPhase::RolledBack
        ),
        WorkflowLifecycleEvent::ActivationCompleted => matches!(
            installation.phase,
            WorkflowInstallationPhase::Active
                | WorkflowInstallationPhase::RollbackPending
                | WorkflowInstallationPhase::RolledBack
        ),
        WorkflowLifecycleEvent::ActivationFailed => matches!(
            installation.phase,
            WorkflowInstallationPhase::Failed
                | WorkflowInstallationPhase::RollbackPending
                | WorkflowInstallationPhase::RolledBack
        ),
        WorkflowLifecycleEvent::RollbackCompleted => {
            installation.phase == WorkflowInstallationPhase::RolledBack
        }
        WorkflowLifecycleEvent::RollbackFailed => {
            matches!(
                installation.phase,
                WorkflowInstallationPhase::Failed | WorkflowInstallationPhase::RollbackPending
            )
        }
    };
    if phase_valid {
        Ok(())
    } else {
        Err(PreparationError::Permanent(
            "lifecycle event is incompatible with the installation phase".to_string(),
        ))
    }
}

fn verify_lifecycle_job_result(
    job: &captain_memory::workflow_learning_queue::WorkflowJobRecord,
    lifecycle: &WorkflowLifecycleCard,
    expected_event: &str,
) -> Result<(), PreparationError> {
    if lifecycle.failure_code.is_some()
        || lifecycle.failure_message.is_some()
        || lifecycle.rollback_job_id.is_some()
    {
        return Err(PreparationError::Permanent(
            "successful lifecycle card contains failure evidence".to_string(),
        ));
    }
    let result: LifecycleJobResult =
        serde_json::from_str(job.result_json.as_deref().ok_or_else(|| {
            PreparationError::Permanent("successful lifecycle job has no result".to_string())
        })?)
        .map_err(|error| {
            PreparationError::Permanent(format!("invalid lifecycle job result: {error}"))
        })?;
    if result.schema_version != PAYLOAD_SCHEMA_VERSION
        || result.event != expected_event
        || result.proposal_id != lifecycle.proposal_id
        || result.revision_sha256 != lifecycle.revision_sha256
        || Some(result.target_locator.as_str()) != lifecycle.target_locator.as_deref()
    {
        return Err(PreparationError::Permanent(
            "lifecycle card differs from the exact job result".to_string(),
        ));
    }
    Ok(())
}

fn retry_backoff_ms(attempt_count: u32) -> i64 {
    let exponent = attempt_count.saturating_sub(1).min(7);
    (5_000_i64.saturating_mul(1_i64 << exponent)).min(15 * 60 * 1_000)
}

fn bounded_error(error: &str) -> String {
    let cleaned = error.trim();
    let bounded = captain_types::truncate_str(cleaned, 2_048).trim();
    if bounded.is_empty() {
        "workflow delivery failed".to_string()
    } else {
        bounded.to_string()
    }
}

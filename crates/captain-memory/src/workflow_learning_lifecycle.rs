//! Atomic lifecycle commits for workflow-learning effects.
//!
//! Filesystem and scheduler journals prove external effects. These operations
//! advance the leased job, proposal, installation mirror, next job, and outbox
//! together so no user-visible state can claim success without its durable
//! continuation.

use rusqlite::{params, Transaction, TransactionBehavior};

use crate::workflow_learning_control::{
    proposal_by_id, transition_in_tx, validate_transition, WorkflowLearningControlError,
    WorkflowLearningStore, WorkflowProposalRecord, WorkflowProposalState,
};
use crate::workflow_learning_installation::{
    installation_by_id, transition_installation_in_tx, WorkflowInstallationPhase,
};
pub use crate::workflow_learning_lifecycle_types::{
    WorkflowCanaryCompletion, WorkflowEffectFailure, WorkflowInstallCompletion,
    WorkflowLifecycleResult, WorkflowRollbackCompletion,
};
use crate::workflow_learning_lifecycle_validation::{
    proposal_transition_is_fresh, require_installation_pair, require_job_identity,
    require_next_job, require_notification, require_proposal_pair, validate_known_failure,
};
use crate::workflow_learning_outbox::insert_outbox_in_tx;
use crate::workflow_learning_queue::{
    complete_job_in_tx, insert_job_in_tx, job_by_id, WorkflowJobEffectState, WorkflowJobKind,
    WorkflowJobStatus,
};

impl WorkflowLearningStore {
    pub fn complete_install_and_enqueue_canary(
        &self,
        request: &WorkflowInstallCompletion,
    ) -> Result<WorkflowLifecycleResult, WorkflowLearningControlError> {
        validate_transition(&request.proposal_transition)?;
        require_proposal_pair(
            &request.proposal_transition,
            &[WorkflowProposalState::ApprovedPendingInstall],
            WorkflowProposalState::ActiveCanary,
        )?;
        let revision = required_revision(&request.proposal_transition)?;
        require_next_job(
            &request.canary_job,
            &request.proposal_transition.proposal_id,
            revision,
            WorkflowJobKind::Canary,
        )?;
        require_notification(
            request.notification.as_ref(),
            &request.proposal_transition.proposal_id,
            revision,
        )?;

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fresh = proposal_transition_is_fresh(&tx, &request.proposal_transition)?;
        let installation =
            installation_by_id(&tx, &request.proposal_transition.proposal_id, revision)?
                .ok_or_else(|| {
                    WorkflowLearningControlError::Conflict(
                        "install completion requires a durable installation mirror".to_string(),
                    )
                })?;
        if fresh && installation.phase != WorkflowInstallationPhase::Verified {
            return Err(WorkflowLearningControlError::Conflict(
                "fresh install completion requires registry-verified bytes".to_string(),
            ));
        }
        require_job_identity(
            &tx,
            &request.job_id,
            &request.proposal_transition.proposal_id,
            revision,
            WorkflowJobKind::Install,
            fresh,
        )?;

        let proposal = transition_in_tx(&tx, &request.proposal_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &request.job_id,
            &request.worker,
            request.result_json.as_deref(),
            request.completed_at_unix_ms,
        )?;
        let next_job = insert_job_in_tx(&tx, &request.canary_job)?;
        let notification = request
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        tx.commit()?;
        Ok(WorkflowLifecycleResult {
            proposal,
            job,
            installation: Some(installation),
            next_job: Some(next_job),
            notification,
        })
    }

    pub fn complete_canary_activation(
        &self,
        request: &WorkflowCanaryCompletion,
    ) -> Result<WorkflowLifecycleResult, WorkflowLearningControlError> {
        validate_transition(&request.proposal_transition)?;
        require_proposal_pair(
            &request.proposal_transition,
            &[WorkflowProposalState::ActiveCanary],
            WorkflowProposalState::Active,
        )?;
        require_installation_pair(
            &request.installation_transition,
            &[WorkflowInstallationPhase::Verified],
            WorkflowInstallationPhase::Active,
        )?;
        validate_linked_installation(
            &request.proposal_transition,
            &request.installation_transition.proposal_id,
            &request.installation_transition.revision_sha256,
        )?;
        let revision = required_revision(&request.proposal_transition)?;
        require_notification(
            request.notification.as_ref(),
            &request.proposal_transition.proposal_id,
            revision,
        )?;

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fresh = proposal_transition_is_fresh(&tx, &request.proposal_transition)?;
        require_job_identity(
            &tx,
            &request.job_id,
            &request.proposal_transition.proposal_id,
            revision,
            WorkflowJobKind::Canary,
            fresh,
        )?;
        let installation = transition_installation_in_tx(&tx, &request.installation_transition)?;
        let proposal = transition_in_tx(&tx, &request.proposal_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &request.job_id,
            &request.worker,
            request.result_json.as_deref(),
            request.completed_at_unix_ms,
        )?;
        let notification = request
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        tx.commit()?;
        Ok(WorkflowLifecycleResult {
            proposal,
            job,
            installation: Some(installation),
            next_job: None,
            notification,
        })
    }

    pub fn fail_known_effect_and_schedule_rollback(
        &self,
        request: &WorkflowEffectFailure,
    ) -> Result<WorkflowLifecycleResult, WorkflowLearningControlError> {
        validate_transition(&request.proposal_transition)?;
        validate_known_failure(&request.error_code, &request.error_message)?;
        let expected_state = match request.job_kind {
            WorkflowJobKind::Install => WorkflowProposalState::ApprovedPendingInstall,
            WorkflowJobKind::Canary => WorkflowProposalState::ActiveCanary,
            _ => {
                return Err(WorkflowLearningControlError::InvalidInput(
                    "known lifecycle failure only accepts install or canary jobs".to_string(),
                ))
            }
        };
        require_proposal_pair(
            &request.proposal_transition,
            &[expected_state],
            WorkflowProposalState::InstallFailed,
        )?;
        let revision = required_revision(&request.proposal_transition)?;
        match (&request.installation_transition, &request.rollback_job) {
            (Some(installation), Some(rollback)) => {
                require_installation_pair(
                    installation,
                    &[
                        WorkflowInstallationPhase::Prepared,
                        WorkflowInstallationPhase::Promoted,
                        WorkflowInstallationPhase::Verified,
                    ],
                    WorkflowInstallationPhase::Failed,
                )?;
                validate_linked_installation(
                    &request.proposal_transition,
                    &installation.proposal_id,
                    &installation.revision_sha256,
                )?;
                if installation.last_error.as_deref() != Some(request.error_message.as_str()) {
                    return Err(WorkflowLearningControlError::InvalidInput(
                        "failed installation must preserve the exact lifecycle error".to_string(),
                    ));
                }
                require_next_job(
                    rollback,
                    &request.proposal_transition.proposal_id,
                    revision,
                    WorkflowJobKind::Rollback,
                )?;
            }
            (None, None) => {}
            _ => {
                return Err(WorkflowLearningControlError::InvalidInput(
                    "rollback job and installation failure transition must be provided together"
                        .to_string(),
                ))
            }
        }
        require_notification(
            request.notification.as_ref(),
            &request.proposal_transition.proposal_id,
            revision,
        )?;

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fresh = proposal_transition_is_fresh(&tx, &request.proposal_transition)?;
        require_job_identity(
            &tx,
            &request.job_id,
            &request.proposal_transition.proposal_id,
            revision,
            request.job_kind,
            fresh,
        )?;
        let installation = request
            .installation_transition
            .as_ref()
            .map(|transition| transition_installation_in_tx(&tx, transition))
            .transpose()?;
        let proposal = transition_in_tx(&tx, &request.proposal_transition)?;
        let proposal = record_proposal_failure_in_tx(
            &tx,
            &request.proposal_transition,
            &request.error_code,
            &request.error_message,
        )?
        .unwrap_or(proposal);
        let job = settle_known_failure_in_tx(&tx, request)?;
        let next_job = request
            .rollback_job
            .as_ref()
            .map(|job| insert_job_in_tx(&tx, job))
            .transpose()?;
        let notification = request
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        tx.commit()?;
        Ok(WorkflowLifecycleResult {
            proposal,
            job,
            installation,
            next_job,
            notification,
        })
    }

    pub fn complete_rollback(
        &self,
        request: &WorkflowRollbackCompletion,
    ) -> Result<WorkflowLifecycleResult, WorkflowLearningControlError> {
        validate_transition(&request.proposal_transition)?;
        require_proposal_pair(
            &request.proposal_transition,
            &[
                WorkflowProposalState::InstallFailed,
                WorkflowProposalState::ActiveCanary,
                WorkflowProposalState::Active,
            ],
            WorkflowProposalState::RolledBack,
        )?;
        require_installation_pair(
            &request.installation_transition,
            &[WorkflowInstallationPhase::RollbackPending],
            WorkflowInstallationPhase::RolledBack,
        )?;
        validate_linked_installation(
            &request.proposal_transition,
            &request.installation_transition.proposal_id,
            &request.installation_transition.revision_sha256,
        )?;
        let revision = required_revision(&request.proposal_transition)?;
        require_notification(
            request.notification.as_ref(),
            &request.proposal_transition.proposal_id,
            revision,
        )?;

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fresh = proposal_transition_is_fresh(&tx, &request.proposal_transition)?;
        require_job_identity(
            &tx,
            &request.job_id,
            &request.proposal_transition.proposal_id,
            revision,
            WorkflowJobKind::Rollback,
            fresh,
        )?;
        let installation = transition_installation_in_tx(&tx, &request.installation_transition)?;
        let proposal = transition_in_tx(&tx, &request.proposal_transition)?;
        let job = complete_job_in_tx(
            &tx,
            &request.job_id,
            &request.worker,
            request.result_json.as_deref(),
            request.completed_at_unix_ms,
        )?;
        let notification = request
            .notification
            .as_ref()
            .map(|item| insert_outbox_in_tx(&tx, item))
            .transpose()?;
        tx.commit()?;
        Ok(WorkflowLifecycleResult {
            proposal,
            job,
            installation: Some(installation),
            next_job: None,
            notification,
        })
    }
}

fn required_revision(
    transition: &crate::workflow_learning_control::WorkflowProposalTransition,
) -> Result<&str, WorkflowLearningControlError> {
    transition
        .expected_revision_sha256
        .as_deref()
        .ok_or_else(|| WorkflowLearningControlError::InvalidInput("revision is required".into()))
}

fn validate_linked_installation(
    proposal: &crate::workflow_learning_control::WorkflowProposalTransition,
    installation_proposal_id: &str,
    installation_revision: &str,
) -> Result<(), WorkflowLearningControlError> {
    if proposal.proposal_id == installation_proposal_id
        && proposal.expected_revision_sha256.as_deref() == Some(installation_revision)
    {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(
            "installation transition does not match proposal revision".to_string(),
        ))
    }
}

fn settle_known_failure_in_tx(
    tx: &Transaction<'_>,
    request: &WorkflowEffectFailure,
) -> Result<crate::workflow_learning_queue::WorkflowJobRecord, WorkflowLearningControlError> {
    let current = job_by_id(tx, &request.job_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(request.job_id.clone()))?;
    if current.status == WorkflowJobStatus::Dead
        && current.effect_state == WorkflowJobEffectState::Completed
        && current.error_code.as_deref() == Some(request.error_code.as_str())
        && current.error_message.as_deref() == Some(request.error_message.as_str())
    {
        return Ok(current);
    }
    let changed = tx.execute(
        "UPDATE workflow_learning_jobs
         SET status = 'dead', effect_state = 'completed', result_json = NULL,
             error_code = ?1, error_message = ?2, lease_owner = NULL,
             lease_expires_at = NULL, run_after = ?3, updated_at = ?3
         WHERE id = ?4 AND status = 'running' AND lease_owner = ?5
           AND effect_state = 'started' AND lease_expires_at > ?3",
        params![
            request.error_code,
            request.error_message,
            request.failed_at_unix_ms,
            request.job_id,
            request.worker,
        ],
    )?;
    if changed != 1 {
        return Err(WorkflowLearningControlError::Conflict(
            "known effect failure requires the current started job lease".to_string(),
        ));
    }
    job_by_id(tx, &request.job_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(request.job_id.clone()))
}

fn record_proposal_failure_in_tx(
    tx: &Transaction<'_>,
    transition: &crate::workflow_learning_control::WorkflowProposalTransition,
    error_code: &str,
    error_message: &str,
) -> Result<Option<WorkflowProposalRecord>, WorkflowLearningControlError> {
    let expected_version = transition.expected_version.saturating_add(1);
    let current = proposal_by_id(tx, &transition.proposal_id)?
        .ok_or_else(|| WorkflowLearningControlError::NotFound(transition.proposal_id.clone()))?;
    if current.state != WorkflowProposalState::InstallFailed
        || current.state_version != expected_version
    {
        return Ok(None);
    }
    if current
        .last_error_code
        .as_deref()
        .is_some_and(|value| value != error_code)
        || current
            .last_error_message
            .as_deref()
            .is_some_and(|value| value != error_message)
    {
        return Err(WorkflowLearningControlError::Conflict(
            "proposal failure evidence changed for an idempotent transition".to_string(),
        ));
    }
    tx.execute(
        "UPDATE workflow_learning_proposals
         SET last_error_code = ?1, last_error_message = ?2, updated_at = ?3
         WHERE id = ?4 AND state = 'install_failed' AND state_version = ?5",
        params![
            error_code,
            error_message,
            transition.occurred_at_unix_ms,
            transition.proposal_id,
            expected_version as i64,
        ],
    )?;
    proposal_by_id(tx, &transition.proposal_id).map_err(Into::into)
}

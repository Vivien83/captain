use std::path::{Component, Path};

use crate::workflow_learning_control::WorkflowLearningControlError;
use crate::workflow_learning_installation_types::{
    NewWorkflowInstallation, WorkflowInstallationPhase, WorkflowInstallationRecord,
    WorkflowInstallationTransition,
};
use crate::workflow_learning_validation::{validate_hash, validate_text, validate_token};

pub(crate) fn validate_new_installation(
    input: &NewWorkflowInstallation,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("proposal_id", &input.proposal_id, 96)?;
    validate_hash("installation revision_sha256", &input.revision_sha256)?;
    validate_hash("installation installed_sha256", &input.installed_sha256)?;
    validate_locator("target_locator", &input.target_locator)?;
    if let Some(locator) = &input.backup_locator {
        validate_locator("backup_locator", locator)?;
    }
    match (&input.backup_locator, &input.backup_sha256) {
        (Some(_), Some(hash)) => validate_hash("installation backup_sha256", hash)?,
        (None, None) => {}
        _ => {
            return Err(WorkflowLearningControlError::InvalidInput(
                "backup locator and hash must either both be present or both be absent".to_string(),
            ))
        }
    }
    validate_token("installation actor", &input.actor, 128)?;
    validate_text("installation reason", &input.reason, 1, 2_048)?;
    validate_token("installation idempotency_key", &input.idempotency_key, 192)
}

pub(crate) fn validate_installation_transition(
    request: &WorkflowInstallationTransition,
) -> Result<(), WorkflowLearningControlError> {
    validate_token("proposal_id", &request.proposal_id, 96)?;
    validate_hash("installation revision_sha256", &request.revision_sha256)?;
    validate_token("installation actor", &request.actor, 128)?;
    validate_text("installation reason", &request.reason, 1, 2_048)?;
    validate_token(
        "installation transition idempotency_key",
        &request.idempotency_key,
        192,
    )?;
    if let Some(error) = &request.last_error {
        validate_text("installation last_error", error, 1, 2_048)?;
    }
    Ok(())
}

pub(crate) fn matches_prepared_metadata(
    existing: &WorkflowInstallationRecord,
    input: &NewWorkflowInstallation,
) -> bool {
    existing.kind == input.kind
        && existing.target_locator == input.target_locator
        && existing.backup_locator == input.backup_locator
        && existing.backup_sha256 == input.backup_sha256
        && existing.installed_sha256 == input.installed_sha256
}

pub(crate) fn require_phase_pair(
    request: &WorkflowInstallationTransition,
    from: WorkflowInstallationPhase,
    to: WorkflowInstallationPhase,
) -> Result<(), WorkflowLearningControlError> {
    if request.expected_phase == from && request.to_phase == to {
        Ok(())
    } else {
        Err(WorkflowLearningControlError::InvalidInput(format!(
            "operation requires installation transition {} -> {}",
            from.as_str(),
            to.as_str()
        )))
    }
}

pub(crate) fn is_legal_installation_transition(
    from: WorkflowInstallationPhase,
    to: WorkflowInstallationPhase,
) -> bool {
    use WorkflowInstallationPhase::*;
    matches!(
        (from, to),
        (Prepared, Promoted)
            | (Prepared, Failed)
            | (Promoted, Verified | Failed | RollbackPending)
            | (Verified, Active | Failed | RollbackPending)
            | (Active, RollbackPending)
            | (Failed, RollbackPending)
            | (RollbackPending, RolledBack)
            | (RolledBack, Quarantined)
    )
}

fn validate_locator(label: &str, locator: &str) -> Result<(), WorkflowLearningControlError> {
    validate_text(label, locator, 1, 1_024)?;
    let path = Path::new(locator);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir
                    | Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
    {
        return Err(WorkflowLearningControlError::InvalidInput(format!(
            "{label} must be a relative non-escaping locator"
        )));
    }
    Ok(())
}

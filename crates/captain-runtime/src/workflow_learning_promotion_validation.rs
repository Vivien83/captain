use std::path::{Path, PathBuf};

use crate::workflow_learning_promotion_fs::{
    optional_file_hash, validate_hash, validate_identifier,
};
use crate::workflow_learning_promotion_types::{
    PromoteWorkflowDraftRequest, VerifiedWorkflowPromotion, WorkflowPromotionError,
    WorkflowPromotionManifest, WorkflowPromotionTargetKind, WORKFLOW_PROMOTION_MANIFEST_VERSION,
};
use crate::workflow_learning_proposer::{
    RefinementTargetKind, WorkflowDraftArtifact, WorkflowDraftKind,
};
use crate::workflow_learning_staging::LoadedStagedWorkflowDraft;

pub(super) fn promotion_target(
    staged: &LoadedStagedWorkflowDraft,
) -> Result<(WorkflowPromotionTargetKind, String), WorkflowPromotionError> {
    match &staged.manifest.draft.artifact {
        WorkflowDraftArtifact::SkillMarkdown { .. } => Ok((
            WorkflowPromotionTargetKind::Skill,
            staged.manifest.name.clone(),
        )),
        WorkflowDraftArtifact::CapspecToml { .. } => Ok((
            WorkflowPromotionTargetKind::Capspec,
            staged.manifest.name.clone(),
        )),
        WorkflowDraftArtifact::Refinement {
            target_kind,
            target_name,
            ..
        } => Ok((
            match target_kind {
                RefinementTargetKind::Skill => WorkflowPromotionTargetKind::Skill,
                RefinementTargetKind::Capspec => WorkflowPromotionTargetKind::Capspec,
            },
            target_name.clone(),
        )),
        WorkflowDraftArtifact::Automation { .. } => {
            Err(WorkflowPromotionError::ExternalActivationRequired)
        }
    }
}

pub(super) fn target_relative_path(kind: WorkflowPromotionTargetKind, name: &str) -> PathBuf {
    match kind {
        WorkflowPromotionTargetKind::Skill => PathBuf::from("skills")
            .join("learned")
            .join(format!("{name}.md")),
        WorkflowPromotionTargetKind::Capspec => {
            PathBuf::from("capabilities").join(format!("{name}.captain"))
        }
    }
}

pub(super) fn validate_manifest(
    manifest: &WorkflowPromotionManifest,
    proposal_id: &str,
    revision_sha256: &str,
) -> Result<(), WorkflowPromotionError> {
    if manifest.manifest_version != WORKFLOW_PROMOTION_MANIFEST_VERSION
        || manifest.proposal_id != proposal_id
        || manifest.revision_sha256 != revision_sha256
    {
        return Err(WorkflowPromotionError::Conflict(
            "promotion manifest identity mismatch".to_string(),
        ));
    }
    validate_identifier("proposal_id", &manifest.proposal_id, 96)?;
    validate_identifier("staging_job_id", &manifest.staging_job_id, 96)?;
    validate_identifier("target_name", &manifest.target_name, 96)?;
    validate_hash("revision_sha256", &manifest.revision_sha256)?;
    validate_hash("artifact_sha256", &manifest.artifact_sha256)?;
    if let Some(hash) = &manifest.previous_sha256 {
        validate_hash("previous_sha256", hash)?;
    }
    if manifest.draft_kind == WorkflowDraftKind::Automation {
        return Err(WorkflowPromotionError::Conflict(
            "automation cannot use a filesystem promotion manifest".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn validate_backup_identity(
    manifest: &WorkflowPromotionManifest,
    proposal_id: &str,
    revision_sha256: &str,
) -> Result<(), WorkflowPromotionError> {
    let expected = PathBuf::from("learning/rollback")
        .join(proposal_id)
        .join(revision_sha256)
        .join("previous.bin");
    match (
        manifest.previous_sha256.is_some(),
        manifest.previous_backup_relative_path.as_ref(),
    ) {
        (true, Some(actual)) if actual == &expected => Ok(()),
        (false, None) => Ok(()),
        _ => Err(WorkflowPromotionError::Conflict(
            "promotion rollback identity mismatch".to_string(),
        )),
    }
}

pub(super) fn ensure_request_matches(
    manifest: &WorkflowPromotionManifest,
    request: &PromoteWorkflowDraftRequest<'_>,
) -> Result<(), WorkflowPromotionError> {
    if manifest.proposal_id == request.proposal_id
        && manifest.staging_job_id == request.staging_job_id
        && manifest.revision_sha256 == request.revision_sha256
        && manifest.artifact_sha256 == request.artifact_sha256
    {
        Ok(())
    } else {
        Err(WorkflowPromotionError::Conflict(
            "existing promotion journal belongs to different approved bytes".to_string(),
        ))
    }
}

pub(super) fn ensure_verification_matches(
    manifest: &WorkflowPromotionManifest,
    verification: &VerifiedWorkflowPromotion,
) -> Result<(), WorkflowPromotionError> {
    if manifest.proposal_id == verification.proposal_id
        && manifest.revision_sha256 == verification.revision_sha256
        && manifest.artifact_sha256 == verification.artifact_sha256
        && manifest.target_kind == verification.target_kind
        && manifest.target_name == verification.target_name
    {
        Ok(())
    } else {
        Err(WorkflowPromotionError::RegistryVerification(
            "verification token does not identify the promoted revision".to_string(),
        ))
    }
}

pub(super) fn ensure_target_matches_prior(
    path: &Path,
    manifest: &WorkflowPromotionManifest,
) -> Result<(), WorkflowPromotionError> {
    let actual = optional_file_hash(path)?;
    if actual.as_deref() == manifest.previous_sha256.as_deref() {
        Ok(())
    } else {
        Err(WorkflowPromotionError::Conflict(format!(
            "target {} changed outside the promotion transaction",
            path.display()
        )))
    }
}

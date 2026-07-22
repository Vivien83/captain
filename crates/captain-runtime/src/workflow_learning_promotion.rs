//! Crash-safe filesystem promotion for approved workflow-learning drafts.
//!
//! SQLite owns the operator decision and job lease. This module owns the
//! recoverable filesystem side effect: immutable backup, durable phase journal,
//! exact target hash checks, activation, rollback, and quarantine.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::workflow_learning_promotion_fs::{
    ensure_descendant, ensure_exact_file, ensure_existing_directory_or_absent,
    ensure_regular_file_or_absent, make_private_directory, optional_file_hash,
    read_optional_regular, read_required_regular, sha256_hex, validate_hash, validate_identifier,
    write_immutable,
};
use crate::workflow_learning_promotion_types::{
    PreparedWorkflowPromotion, PromoteWorkflowDraftRequest, VerifiedWorkflowPromotion,
    WorkflowPromotionError, WorkflowPromotionManifest, WorkflowPromotionPhase,
    WorkflowPromotionTargetKind, WORKFLOW_PROMOTION_MANIFEST_VERSION,
};
use crate::workflow_learning_promotion_validation::{
    ensure_request_matches, ensure_target_matches_prior, ensure_verification_matches,
    promotion_target, target_relative_path, validate_backup_identity, validate_manifest,
};
use crate::workflow_learning_staging::{LoadedStagedWorkflowDraft, WorkflowStagingRoot};

#[derive(Debug, Clone)]
pub struct WorkflowPromotionRoot {
    captain_home: PathBuf,
    staging: WorkflowStagingRoot,
}

impl WorkflowPromotionRoot {
    pub fn new(captain_home: impl Into<PathBuf>) -> Result<Self, WorkflowPromotionError> {
        let captain_home = captain_home.into();
        if !captain_home.is_absolute() {
            return Err(WorkflowPromotionError::InvalidRequest(
                "Captain home must be absolute".to_string(),
            ));
        }
        let staging = WorkflowStagingRoot::new(&captain_home)
            .map_err(|error| WorkflowPromotionError::InvalidStaging(error.to_string()))?;
        Ok(Self {
            captain_home,
            staging,
        })
    }

    pub fn prepare(
        &self,
        request: PromoteWorkflowDraftRequest<'_>,
    ) -> Result<PreparedWorkflowPromotion, WorkflowPromotionError> {
        validate_identifier("proposal_id", request.proposal_id, 96)?;
        validate_identifier("staging_job_id", request.staging_job_id, 96)?;
        validate_hash("revision_sha256", request.revision_sha256)?;
        validate_hash("artifact_sha256", request.artifact_sha256)?;
        self.ensure_safe_roots()?;

        let staged = self.load_exact_staged(&request)?;
        let (target_kind, target_name) = promotion_target(&staged)?;
        let target_relative_path = target_relative_path(target_kind, &target_name);
        let target_path = self.safe_target_path(target_kind, &target_name)?;
        let journal_path = self.journal_path(request.proposal_id, request.revision_sha256);
        self.ensure_private_state_file_path(&journal_path)?;

        if journal_path.exists() {
            let prepared = self.load(request.proposal_id, request.revision_sha256)?;
            ensure_request_matches(&prepared.manifest, &request)?;
            return Ok(prepared);
        }

        let previous = read_optional_regular(&target_path)?;
        let previous_sha256 = previous.as_deref().map(sha256_hex);
        let previous_backup_relative_path = if let Some(previous) = previous {
            let relative = self.backup_relative_path(request.proposal_id, request.revision_sha256);
            let path = self.captain_home.join(&relative);
            self.ensure_private_state_file_path(&path)?;
            write_immutable(&path, &previous)?;
            Some(relative)
        } else {
            None
        };

        let manifest = WorkflowPromotionManifest {
            manifest_version: WORKFLOW_PROMOTION_MANIFEST_VERSION,
            proposal_id: request.proposal_id.to_string(),
            staging_job_id: request.staging_job_id.to_string(),
            revision_sha256: request.revision_sha256.to_string(),
            artifact_sha256: request.artifact_sha256.to_string(),
            draft_kind: staged.manifest.kind,
            target_kind,
            target_name,
            target_relative_path,
            previous_sha256,
            previous_backup_relative_path,
            phase: WorkflowPromotionPhase::Prepared,
        };
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        write_immutable(&journal_path, &bytes)?;
        self.load(request.proposal_id, request.revision_sha256)
    }

    pub fn promote(
        &self,
        proposal_id: &str,
        revision_sha256: &str,
    ) -> Result<PreparedWorkflowPromotion, WorkflowPromotionError> {
        let mut prepared = self.reconcile(proposal_id, revision_sha256)?;
        match prepared.manifest.phase {
            WorkflowPromotionPhase::Prepared => {}
            WorkflowPromotionPhase::Promoted
            | WorkflowPromotionPhase::RegistryVerified
            | WorkflowPromotionPhase::Active => return Ok(prepared),
            actual => {
                return Err(WorkflowPromotionError::InvalidPhase {
                    expected: "prepared, promoted, registry_verified, or active",
                    actual,
                })
            }
        }

        let staged = self.load_staged_for_manifest(&prepared.manifest)?;
        ensure_target_matches_prior(&prepared.target_path, &prepared.manifest)?;
        captain_types::durable_fs::atomic_write(&prepared.target_path, &staged.artifact_bytes)?;
        ensure_exact_file(&prepared.target_path, &prepared.manifest.artifact_sha256)?;
        prepared.manifest.phase = WorkflowPromotionPhase::Promoted;
        self.write_manifest(&prepared.manifest)?;
        Ok(prepared)
    }

    pub fn reconcile(
        &self,
        proposal_id: &str,
        revision_sha256: &str,
    ) -> Result<PreparedWorkflowPromotion, WorkflowPromotionError> {
        let mut prepared = self.load(proposal_id, revision_sha256)?;
        let current_hash = optional_file_hash(&prepared.target_path)?;
        let expected = Some(prepared.manifest.artifact_sha256.as_str());
        let previous = prepared.manifest.previous_sha256.as_deref();

        match prepared.manifest.phase {
            WorkflowPromotionPhase::Prepared if current_hash.as_deref() == expected => {
                prepared.manifest.phase = WorkflowPromotionPhase::Promoted;
                self.write_manifest(&prepared.manifest)?;
            }
            WorkflowPromotionPhase::Prepared if current_hash.as_deref() == previous => {}
            WorkflowPromotionPhase::Promoted
            | WorkflowPromotionPhase::RegistryVerified
            | WorkflowPromotionPhase::Active
                if current_hash.as_deref() == expected => {}
            WorkflowPromotionPhase::RollbackPending if current_hash.as_deref() == expected => {}
            WorkflowPromotionPhase::RollbackPending if current_hash.as_deref() == previous => {
                prepared.manifest.phase = WorkflowPromotionPhase::RolledBack;
                self.write_manifest(&prepared.manifest)?;
            }
            WorkflowPromotionPhase::RolledBack | WorkflowPromotionPhase::Quarantined
                if current_hash.as_deref() == previous => {}
            phase => {
                return Err(WorkflowPromotionError::Conflict(format!(
                    "target {} has an unexpected hash while phase is {phase:?}",
                    prepared.target_path.display()
                )))
            }
        }
        Ok(prepared)
    }

    pub fn record_registry_verified(
        &self,
        verification: &VerifiedWorkflowPromotion,
    ) -> Result<PreparedWorkflowPromotion, WorkflowPromotionError> {
        let mut prepared =
            self.reconcile(&verification.proposal_id, &verification.revision_sha256)?;
        ensure_verification_matches(&prepared.manifest, verification)?;
        match prepared.manifest.phase {
            WorkflowPromotionPhase::Promoted => {
                prepared.manifest.phase = WorkflowPromotionPhase::RegistryVerified;
                self.write_manifest(&prepared.manifest)?;
            }
            WorkflowPromotionPhase::RegistryVerified | WorkflowPromotionPhase::Active => {}
            actual => {
                return Err(WorkflowPromotionError::InvalidPhase {
                    expected: "promoted, registry_verified, or active",
                    actual,
                })
            }
        }
        Ok(prepared)
    }

    pub fn mark_active(
        &self,
        proposal_id: &str,
        revision_sha256: &str,
    ) -> Result<PreparedWorkflowPromotion, WorkflowPromotionError> {
        let mut prepared = self.reconcile(proposal_id, revision_sha256)?;
        match prepared.manifest.phase {
            WorkflowPromotionPhase::RegistryVerified => {
                prepared.manifest.phase = WorkflowPromotionPhase::Active;
                self.write_manifest(&prepared.manifest)?;
            }
            WorkflowPromotionPhase::Active => {}
            actual => {
                return Err(WorkflowPromotionError::InvalidPhase {
                    expected: "registry_verified or active",
                    actual,
                })
            }
        }
        Ok(prepared)
    }

    pub fn rollback(
        &self,
        proposal_id: &str,
        revision_sha256: &str,
    ) -> Result<PreparedWorkflowPromotion, WorkflowPromotionError> {
        let mut prepared = self.reconcile(proposal_id, revision_sha256)?;
        match prepared.manifest.phase {
            WorkflowPromotionPhase::Prepared => {
                prepared.manifest.phase = WorkflowPromotionPhase::RolledBack;
                self.write_manifest(&prepared.manifest)?;
                return Ok(prepared);
            }
            WorkflowPromotionPhase::RolledBack | WorkflowPromotionPhase::Quarantined => {
                return Ok(prepared)
            }
            WorkflowPromotionPhase::Promoted
            | WorkflowPromotionPhase::RegistryVerified
            | WorkflowPromotionPhase::Active
            | WorkflowPromotionPhase::RollbackPending => {}
        }

        prepared.manifest.phase = WorkflowPromotionPhase::RollbackPending;
        self.write_manifest(&prepared.manifest)?;
        self.restore_previous(&prepared.manifest, &prepared.target_path)?;
        ensure_target_matches_prior(&prepared.target_path, &prepared.manifest)?;
        prepared.manifest.phase = WorkflowPromotionPhase::RolledBack;
        self.write_manifest(&prepared.manifest)?;
        Ok(prepared)
    }

    pub fn quarantine(
        &self,
        proposal_id: &str,
        revision_sha256: &str,
        reason: &str,
    ) -> Result<PreparedWorkflowPromotion, WorkflowPromotionError> {
        let reason = reason.trim();
        if reason.is_empty() || reason.chars().count() > 500 {
            return Err(WorkflowPromotionError::InvalidRequest(
                "quarantine reason must contain 1..=500 characters".to_string(),
            ));
        }
        let mut prepared = self.rollback(proposal_id, revision_sha256)?;
        let marker = QuarantineMarker {
            proposal_id,
            revision_sha256,
            artifact_sha256: &prepared.manifest.artifact_sha256,
            reason,
        };
        let marker_path = self.quarantine_marker_path(proposal_id, revision_sha256);
        self.ensure_private_state_file_path(&marker_path)?;
        let marker_bytes = serde_json::to_vec_pretty(&marker)?;
        write_immutable(&marker_path, &marker_bytes)?;
        prepared.manifest.phase = WorkflowPromotionPhase::Quarantined;
        self.write_manifest(&prepared.manifest)?;
        Ok(prepared)
    }

    pub fn load(
        &self,
        proposal_id: &str,
        revision_sha256: &str,
    ) -> Result<PreparedWorkflowPromotion, WorkflowPromotionError> {
        validate_identifier("proposal_id", proposal_id, 96)?;
        validate_hash("revision_sha256", revision_sha256)?;
        self.ensure_safe_roots()?;
        let journal_path = self.journal_path(proposal_id, revision_sha256);
        self.ensure_private_state_file_path(&journal_path)?;
        let bytes = read_required_regular(&journal_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                WorkflowPromotionError::NotPrepared(journal_path.display().to_string())
            } else {
                WorkflowPromotionError::Io(error)
            }
        })?;
        let manifest: WorkflowPromotionManifest = serde_json::from_slice(&bytes)?;
        validate_manifest(&manifest, proposal_id, revision_sha256)?;
        let target_path = self.safe_target_path(manifest.target_kind, &manifest.target_name)?;
        if manifest.target_relative_path
            != target_relative_path(manifest.target_kind, &manifest.target_name)
        {
            return Err(WorkflowPromotionError::Conflict(
                "promotion target path does not match its typed identity".to_string(),
            ));
        }
        validate_backup_identity(&manifest, proposal_id, revision_sha256)?;
        Ok(PreparedWorkflowPromotion {
            manifest,
            target_path,
            journal_path,
        })
    }

    fn load_exact_staged(
        &self,
        request: &PromoteWorkflowDraftRequest<'_>,
    ) -> Result<LoadedStagedWorkflowDraft, WorkflowPromotionError> {
        let staged = self
            .staging
            .load_exact(request.staging_job_id, request.revision_sha256)
            .map_err(|error| WorkflowPromotionError::InvalidStaging(error.to_string()))?;
        if staged.manifest.artifact_sha256 != request.artifact_sha256 {
            return Err(WorkflowPromotionError::Conflict(
                "approved artifact hash differs from staged artifact".to_string(),
            ));
        }
        Ok(staged)
    }

    fn load_staged_for_manifest(
        &self,
        manifest: &WorkflowPromotionManifest,
    ) -> Result<LoadedStagedWorkflowDraft, WorkflowPromotionError> {
        let request = PromoteWorkflowDraftRequest {
            proposal_id: &manifest.proposal_id,
            staging_job_id: &manifest.staging_job_id,
            revision_sha256: &manifest.revision_sha256,
            artifact_sha256: &manifest.artifact_sha256,
        };
        let staged = self.load_exact_staged(&request)?;
        let (kind, name) = promotion_target(&staged)?;
        if kind != manifest.target_kind || name != manifest.target_name {
            return Err(WorkflowPromotionError::Conflict(
                "staged target identity changed after preparation".to_string(),
            ));
        }
        Ok(staged)
    }

    fn write_manifest(
        &self,
        manifest: &WorkflowPromotionManifest,
    ) -> Result<(), WorkflowPromotionError> {
        validate_manifest(manifest, &manifest.proposal_id, &manifest.revision_sha256)?;
        let path = self.journal_path(&manifest.proposal_id, &manifest.revision_sha256);
        self.ensure_private_state_file_path(&path)?;
        let bytes = serde_json::to_vec_pretty(manifest)?;
        captain_types::durable_fs::atomic_write(&path, &bytes)?;
        Ok(())
    }

    fn restore_previous(
        &self,
        manifest: &WorkflowPromotionManifest,
        target_path: &Path,
    ) -> Result<(), WorkflowPromotionError> {
        ensure_exact_file(target_path, &manifest.artifact_sha256)?;
        match (
            manifest.previous_sha256.as_deref(),
            manifest.previous_backup_relative_path.as_deref(),
        ) {
            (Some(previous_hash), Some(relative)) => {
                let expected =
                    self.backup_relative_path(&manifest.proposal_id, &manifest.revision_sha256);
                if relative != expected {
                    return Err(WorkflowPromotionError::Conflict(
                        "rollback backup path does not match installation identity".to_string(),
                    ));
                }
                let backup = self.captain_home.join(relative);
                self.ensure_private_state_file_path(&backup)?;
                ensure_exact_file(&backup, previous_hash)?;
                captain_types::durable_fs::atomic_copy(&backup, target_path)?;
            }
            (None, None) => {
                captain_types::durable_fs::remove_file(target_path)?;
            }
            _ => {
                return Err(WorkflowPromotionError::Conflict(
                    "rollback metadata is incomplete".to_string(),
                ))
            }
        }
        Ok(())
    }

    fn ensure_safe_roots(&self) -> Result<(), WorkflowPromotionError> {
        ensure_existing_directory_or_absent(&self.captain_home)?;
        captain_types::durable_fs::create_dir_all(&self.captain_home)?;
        ensure_existing_directory_or_absent(&self.captain_home)?;
        for relative in [
            "learning",
            "learning/installations",
            "learning/rollback",
            "learning/quarantine",
        ] {
            let path = self.captain_home.join(relative);
            ensure_descendant(&self.captain_home, &path)?;
            captain_types::durable_fs::create_dir_all(&path)?;
            ensure_descendant(&self.captain_home, &path)?;
            ensure_existing_directory_or_absent(&path)?;
            make_private_directory(&path)?;
        }
        Ok(())
    }

    fn safe_target_path(
        &self,
        kind: WorkflowPromotionTargetKind,
        name: &str,
    ) -> Result<PathBuf, WorkflowPromotionError> {
        validate_identifier("target_name", name, 96)?;
        let relative = target_relative_path(kind, name);
        let parent = self
            .captain_home
            .join(relative.parent().expect("target has parent"));
        ensure_descendant(&self.captain_home, &parent)?;
        ensure_existing_directory_or_absent(&parent)?;
        captain_types::durable_fs::create_dir_all(&parent)?;
        ensure_descendant(&self.captain_home, &parent)?;
        ensure_existing_directory_or_absent(&parent)?;
        let target = self.captain_home.join(relative);
        ensure_descendant(&self.captain_home, &target)?;
        ensure_regular_file_or_absent(&target)?;
        Ok(target)
    }

    fn ensure_private_state_file_path(&self, path: &Path) -> Result<(), WorkflowPromotionError> {
        ensure_descendant(&self.captain_home, path)?;
        let parent = path.parent().ok_or_else(|| {
            WorkflowPromotionError::UnsafeFilesystem(
                "promotion state file has no parent".to_string(),
            )
        })?;
        ensure_existing_directory_or_absent(parent)?;
        captain_types::durable_fs::create_dir_all(parent)?;
        ensure_descendant(&self.captain_home, parent)?;
        ensure_existing_directory_or_absent(parent)?;
        make_private_directory(parent)?;
        ensure_regular_file_or_absent(path)
    }

    fn journal_path(&self, proposal_id: &str, revision_sha256: &str) -> PathBuf {
        self.captain_home
            .join("learning/installations")
            .join(proposal_id)
            .join(revision_sha256)
            .join("promotion.json")
    }

    fn backup_relative_path(&self, proposal_id: &str, revision_sha256: &str) -> PathBuf {
        PathBuf::from("learning/rollback")
            .join(proposal_id)
            .join(revision_sha256)
            .join("previous.bin")
    }

    fn quarantine_marker_path(&self, proposal_id: &str, revision_sha256: &str) -> PathBuf {
        self.captain_home
            .join("learning/quarantine")
            .join(proposal_id)
            .join(revision_sha256)
            .join("quarantine.json")
    }
}

#[derive(Serialize)]
struct QuarantineMarker<'a> {
    proposal_id: &'a str,
    revision_sha256: &'a str,
    artifact_sha256: &'a str,
    reason: &'a str,
}

//! Model-independent activation lifecycle for Skill Learning V2.
//!
//! Every external mutation has an exact durable journal. Interrupted jobs are
//! reclaimed only to reconcile that journal, never to guess whether an effect
//! happened.

use captain_memory::workflow_learning_control::{
    WorkflowArtifactKind, WorkflowLearningControlError, WorkflowLearningStore,
    WorkflowProposalRecord, WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_installation::{
    NewWorkflowInstallation, WorkflowInstallationPhase, WorkflowInstallationRecord,
    WorkflowInstallationTransition,
};
use captain_memory::workflow_learning_lifecycle::{
    WorkflowCanaryCompletion, WorkflowEffectFailure, WorkflowInstallCompletion,
    WorkflowRollbackCompletion,
};
use captain_memory::workflow_learning_outbox::NewWorkflowOutboxItem;
use captain_memory::workflow_learning_queue::{
    NewWorkflowJob, WorkflowJobEffectState, WorkflowJobKind, WorkflowJobRecord, WorkflowJobStatus,
};
use captain_runtime::workflow_learning_automation::build_disabled_automation_job;
use captain_runtime::workflow_learning_delivery::WORKFLOW_LIFECYCLE_OUTBOX_TOPIC;
use captain_runtime::workflow_learning_operator::WorkflowInstallRequestPayload;
use captain_runtime::workflow_learning_promotion::WorkflowPromotionRoot;
use captain_runtime::workflow_learning_promotion_types::{
    PromoteWorkflowDraftRequest, WorkflowPromotionTargetKind,
};
use captain_runtime::workflow_learning_registry::{
    verify_capspec_rollback, verify_promoted_capspec, verify_promoted_skill, verify_skill_rollback,
};
use captain_runtime::workflow_learning_staging::WorkflowStagingRoot;
use captain_types::agent::AgentId;
use captain_types::scheduler::{CronJob, CronJobId};
use captain_types::workflow_learning::{
    ProposalCardKind, ProposalCardState, ProposalInstallMode, WorkflowLifecycleCard,
    WorkflowLifecycleEvent, WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION,
};
use chrono::{TimeZone, Utc};
use uuid::Uuid;

use crate::cron::CronScheduler;

const ACTOR: &str = "captain:workflow-activation";

type ActivationResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub(super) struct WorkflowActivationExecutor<'a> {
    control: WorkflowLearningStore,
    staging: WorkflowStagingRoot,
    promotions: WorkflowPromotionRoot,
    skills: &'a std::sync::RwLock<captain_skills::registry::SkillRegistry>,
    capspecs: &'a captain_capspec::CapabilityRegistry,
    scheduler: &'a CronScheduler,
}

impl WorkflowActivationExecutor<'_> {
    pub(super) fn new<'a>(
        control: WorkflowLearningStore,
        staging: WorkflowStagingRoot,
        promotions: WorkflowPromotionRoot,
        skills: &'a std::sync::RwLock<captain_skills::registry::SkillRegistry>,
        capspecs: &'a captain_capspec::CapabilityRegistry,
        scheduler: &'a CronScheduler,
    ) -> WorkflowActivationExecutor<'a> {
        WorkflowActivationExecutor {
            control,
            staging,
            promotions,
            skills,
            capspecs,
            scheduler,
        }
    }

    pub(super) fn execute(
        &self,
        worker: &str,
        job: &WorkflowJobRecord,
        now_unix_ms: i64,
    ) -> ActivationResult<WorkflowProposalState> {
        if job.status != WorkflowJobStatus::Running || job.lease_owner.as_deref() != Some(worker) {
            return Err(invalid_data("activation job does not hold the current lease").into());
        }
        let job = if job.effect_state == WorkflowJobEffectState::None {
            self.control
                .mark_job_effect_started(&job.id, worker, now_unix_ms)?
        } else if job.effect_state == WorkflowJobEffectState::Started {
            job.clone()
        } else {
            return Err(invalid_data("activation job effect is already completed").into());
        };
        let payload = exact_payload(&job)?;
        let proposal = exact_proposal(&self.control, &job, &payload)?;
        match job.kind {
            WorkflowJobKind::Install => self.install(worker, &job, &payload, &proposal),
            WorkflowJobKind::Canary => self.canary(worker, &job, &payload, &proposal),
            WorkflowJobKind::Rollback => self.rollback(worker, &job, &payload, &proposal),
            _ => Err(invalid_data("non-lifecycle job reached activation worker").into()),
        }
    }

    fn install(
        &self,
        worker: &str,
        job: &WorkflowJobRecord,
        payload: &WorkflowInstallRequestPayload,
        proposal: &WorkflowProposalRecord,
    ) -> ActivationResult<WorkflowProposalState> {
        require_state(proposal, WorkflowProposalState::ApprovedPendingInstall)?;
        let installation = match required_kind(proposal)? {
            WorkflowArtifactKind::Automation => self.install_automation(proposal, payload, job)?,
            WorkflowArtifactKind::Skill
            | WorkflowArtifactKind::Capspec
            | WorkflowArtifactKind::Refinement => {
                self.install_filesystem_artifact(proposal, payload, job)?
            }
        };
        if installation.phase != WorkflowInstallationPhase::Verified {
            return Err(invalid_data("install did not reach verified state").into());
        }
        let completed_at = stable_time(job, 30);
        let canary_job =
            lifecycle_job(job, payload, WorkflowJobKind::Canary, stable_time(job, 40))?;
        let notification = lifecycle_notification(
            job,
            proposal,
            WorkflowLifecycleEvent::InstallationVerified,
            WorkflowProposalState::ActiveCanary,
            proposal.state_version.saturating_add(1),
            Some(canary_job.id.clone()),
            Some(installation.target_locator.clone()),
            None,
            None,
            None,
            completed_at,
        )?;
        let result_json = serde_json::json!({
            "schema_version": 1,
            "event": "installation_verified",
            "proposal_id": proposal.id,
            "revision_sha256": payload.revision_sha256,
            "target_locator": installation.target_locator,
        })
        .to_string();
        let result =
            self.control
                .complete_install_and_enqueue_canary(&WorkflowInstallCompletion {
                    job_id: job.id.clone(),
                    worker: worker.to_string(),
                    result_json: Some(result_json),
                    proposal_transition: proposal_transition(
                        proposal,
                        WorkflowProposalState::ActiveCanary,
                        "install-complete",
                        "exact installation verified; canary queued",
                        completed_at,
                    ),
                    canary_job,
                    notification: Some(notification),
                    completed_at_unix_ms: completed_at,
                })?;
        Ok(result.proposal.state)
    }

    fn install_filesystem_artifact(
        &self,
        proposal: &WorkflowProposalRecord,
        payload: &WorkflowInstallRequestPayload,
        job: &WorkflowJobRecord,
    ) -> ActivationResult<WorkflowInstallationRecord> {
        let prepared = self.promotions.prepare(PromoteWorkflowDraftRequest {
            proposal_id: &proposal.id,
            staging_job_id: required(&proposal.staging_job_id, "staging_job_id")?,
            revision_sha256: &payload.revision_sha256,
            artifact_sha256: required(&proposal.artifact_sha256, "artifact_sha256")?,
        })?;
        let prepared_record =
            self.control
                .record_installation_prepared(&NewWorkflowInstallation {
                    proposal_id: proposal.id.clone(),
                    revision_sha256: payload.revision_sha256.clone(),
                    kind: required_kind(proposal)?,
                    target_locator: path_text(&prepared.manifest.target_relative_path)?,
                    backup_locator: prepared
                        .manifest
                        .previous_backup_relative_path
                        .as_deref()
                        .map(path_text)
                        .transpose()?,
                    backup_sha256: prepared.manifest.previous_sha256.clone(),
                    installed_sha256: required(&proposal.artifact_sha256, "artifact_sha256")?
                        .clone(),
                    actor: ACTOR.to_string(),
                    reason: "filesystem promotion journal prepared".to_string(),
                    idempotency_key: operation_key(proposal, "prepared"),
                    occurred_at_unix_ms: stable_time(job, 1),
                })?;
        if prepared_record.phase == WorkflowInstallationPhase::Failed {
            return Err(invalid_data("installation was already marked failed").into());
        }
        let promoted = self
            .promotions
            .promote(&proposal.id, &payload.revision_sha256)?;
        self.control
            .record_installation_promoted(&installation_transition(
                proposal,
                WorkflowInstallationPhase::Prepared,
                0,
                WorkflowInstallationPhase::Promoted,
                "promoted",
                "exact active bytes committed",
                stable_time(job, 2),
                None,
            ))?;
        let verification = match promoted.manifest.target_kind {
            WorkflowPromotionTargetKind::Skill => {
                let mut registry = self
                    .skills
                    .write()
                    .map_err(|_| invalid_data("skill registry lock is poisoned"))?;
                verify_promoted_skill(&promoted, &mut registry)?
            }
            WorkflowPromotionTargetKind::Capspec => {
                verify_promoted_capspec(&promoted, self.capspecs, ACTOR)?
            }
        };
        self.promotions.record_registry_verified(&verification)?;
        self.control
            .record_installation_verified(&installation_transition(
                proposal,
                WorkflowInstallationPhase::Promoted,
                1,
                WorkflowInstallationPhase::Verified,
                "verified",
                "native registry owns the exact promoted revision",
                stable_time(job, 3),
                None,
            ))
            .map_err(Into::into)
    }

    fn install_automation(
        &self,
        proposal: &WorkflowProposalRecord,
        payload: &WorkflowInstallRequestPayload,
        job: &WorkflowJobRecord,
    ) -> ActivationResult<WorkflowInstallationRecord> {
        let cron_job = self.exact_automation_job(proposal, payload)?;
        let target_locator = format!("cron/{}", cron_job.id);
        self.control
            .record_installation_prepared(&NewWorkflowInstallation {
                proposal_id: proposal.id.clone(),
                revision_sha256: payload.revision_sha256.clone(),
                kind: WorkflowArtifactKind::Automation,
                target_locator,
                backup_locator: None,
                backup_sha256: None,
                installed_sha256: required(&proposal.artifact_sha256, "artifact_sha256")?.clone(),
                actor: ACTOR.to_string(),
                reason: "scheduler installation prepared".to_string(),
                idempotency_key: operation_key(proposal, "prepared"),
                occurred_at_unix_ms: stable_time(job, 1),
            })?;
        self.scheduler
            .install_job_durable_exact(cron_job.clone(), false)?;
        self.control
            .record_installation_promoted(&installation_transition(
                proposal,
                WorkflowInstallationPhase::Prepared,
                0,
                WorkflowInstallationPhase::Promoted,
                "promoted",
                "disabled scheduler job persisted",
                stable_time(job, 2),
                None,
            ))?;
        let installed = self
            .scheduler
            .get_job(cron_job.id)
            .ok_or_else(|| invalid_data("installed scheduler job vanished"))?;
        if installed.enabled {
            return Err(invalid_data("automation became enabled before canary").into());
        }
        self.scheduler.install_job_durable_exact(cron_job, false)?;
        self.control
            .record_installation_verified(&installation_transition(
                proposal,
                WorkflowInstallationPhase::Promoted,
                1,
                WorkflowInstallationPhase::Verified,
                "verified",
                "disabled scheduler job matches the exact staged contract",
                stable_time(job, 3),
                None,
            ))
            .map_err(Into::into)
    }

    fn canary(
        &self,
        worker: &str,
        job: &WorkflowJobRecord,
        payload: &WorkflowInstallRequestPayload,
        proposal: &WorkflowProposalRecord,
    ) -> ActivationResult<WorkflowProposalState> {
        require_state(proposal, WorkflowProposalState::ActiveCanary)?;
        let installation = exact_installation(&self.control, proposal, payload)?;
        if installation.phase != WorkflowInstallationPhase::Verified {
            return Err(invalid_data("canary requires a verified installation").into());
        }
        match required_kind(proposal)? {
            WorkflowArtifactKind::Automation => {
                let cron_job = self.exact_automation_job(proposal, payload)?;
                require_installation_target(&installation, &format!("cron/{}", cron_job.id))?;
                self.scheduler
                    .set_job_enabled_durable_exact(&cron_job, false, true)?;
                if !self
                    .scheduler
                    .get_job(cron_job.id)
                    .is_some_and(|installed| installed.enabled)
                {
                    return Err(
                        invalid_data("automation canary did not persist enabled state").into(),
                    );
                }
            }
            WorkflowArtifactKind::Skill
            | WorkflowArtifactKind::Capspec
            | WorkflowArtifactKind::Refinement => {
                let promoted = self
                    .promotions
                    .reconcile(&proposal.id, &payload.revision_sha256)?;
                require_installation_target(
                    &installation,
                    &path_text(&promoted.manifest.target_relative_path)?,
                )?;
                match promoted.manifest.target_kind {
                    WorkflowPromotionTargetKind::Skill => {
                        let mut registry = self
                            .skills
                            .write()
                            .map_err(|_| invalid_data("skill registry lock is poisoned"))?;
                        verify_promoted_skill(&promoted, &mut registry)?;
                    }
                    WorkflowPromotionTargetKind::Capspec => {
                        verify_promoted_capspec(&promoted, self.capspecs, ACTOR)?;
                    }
                }
                self.promotions
                    .mark_active(&proposal.id, &payload.revision_sha256)?;
            }
        }
        let completed_at = stable_time(job, 10);
        let notification = lifecycle_notification(
            job,
            proposal,
            WorkflowLifecycleEvent::ActivationCompleted,
            WorkflowProposalState::Active,
            proposal.state_version.saturating_add(1),
            None,
            Some(installation.target_locator.clone()),
            None,
            None,
            None,
            completed_at,
        )?;
        let result = self
            .control
            .complete_canary_activation(&WorkflowCanaryCompletion {
                job_id: job.id.clone(),
                worker: worker.to_string(),
                result_json: Some(
                    serde_json::json!({
                        "schema_version": 1,
                        "event": "canary_passed",
                        "proposal_id": proposal.id,
                        "revision_sha256": payload.revision_sha256,
                        "target_locator": installation.target_locator.clone(),
                    })
                    .to_string(),
                ),
                proposal_transition: proposal_transition(
                    proposal,
                    WorkflowProposalState::Active,
                    "canary-complete",
                    "exact canary verification passed",
                    completed_at,
                ),
                installation_transition: installation_transition(
                    proposal,
                    WorkflowInstallationPhase::Verified,
                    installation.phase_version,
                    WorkflowInstallationPhase::Active,
                    "active",
                    "canary passed; installation active",
                    completed_at,
                    None,
                ),
                notification: Some(notification),
                completed_at_unix_ms: completed_at,
            })?;
        Ok(result.proposal.state)
    }

    fn rollback(
        &self,
        worker: &str,
        job: &WorkflowJobRecord,
        payload: &WorkflowInstallRequestPayload,
        proposal: &WorkflowProposalRecord,
    ) -> ActivationResult<WorkflowProposalState> {
        if !matches!(
            proposal.state,
            WorkflowProposalState::InstallFailed
                | WorkflowProposalState::ActiveCanary
                | WorkflowProposalState::Active
        ) {
            return Err(invalid_data("rollback proposal is not rollback-eligible").into());
        }
        let mut installation = exact_installation(&self.control, proposal, payload)?;
        if installation.phase != WorkflowInstallationPhase::RollbackPending {
            installation =
                self.control
                    .record_installation_rollback_pending(&installation_transition(
                        proposal,
                        installation.phase,
                        installation.phase_version,
                        WorkflowInstallationPhase::RollbackPending,
                        "rollback-pending",
                        "rollback claimed exact installation ownership",
                        stable_time(job, 1),
                        installation.last_error.clone(),
                    ))?;
        }

        match required_kind(proposal)? {
            WorkflowArtifactKind::Automation => {
                let cron_job = self.exact_automation_job(proposal, payload)?;
                require_installation_target(&installation, &format!("cron/{}", cron_job.id))?;
                self.scheduler.remove_job_durable_exact(&cron_job, false)?;
            }
            WorkflowArtifactKind::Skill
            | WorkflowArtifactKind::Capspec
            | WorkflowArtifactKind::Refinement => {
                let prepared = self
                    .promotions
                    .reconcile(&proposal.id, &payload.revision_sha256)?;
                require_installation_target(
                    &installation,
                    &path_text(&prepared.manifest.target_relative_path)?,
                )?;
                let rolled_back = self
                    .promotions
                    .rollback(&proposal.id, &payload.revision_sha256)?;
                match rolled_back.manifest.target_kind {
                    WorkflowPromotionTargetKind::Skill => {
                        let mut registry = self
                            .skills
                            .write()
                            .map_err(|_| invalid_data("skill registry lock is poisoned"))?;
                        verify_skill_rollback(&rolled_back, &mut registry)?;
                    }
                    WorkflowPromotionTargetKind::Capspec => {
                        verify_capspec_rollback(&rolled_back, self.capspecs, ACTOR)?;
                    }
                }
            }
        }

        let completed_at = stable_time(job, 10);
        let notification = lifecycle_notification(
            job,
            proposal,
            WorkflowLifecycleEvent::RollbackCompleted,
            WorkflowProposalState::RolledBack,
            proposal.state_version.saturating_add(1),
            None,
            Some(installation.target_locator.clone()),
            None,
            None,
            None,
            completed_at,
        )?;
        let result = self
            .control
            .complete_rollback(&WorkflowRollbackCompletion {
                job_id: job.id.clone(),
                worker: worker.to_string(),
                result_json: Some(
                    serde_json::json!({
                        "schema_version": 1,
                        "event": "rollback_verified",
                        "proposal_id": proposal.id,
                        "revision_sha256": payload.revision_sha256,
                        "target_locator": installation.target_locator.clone(),
                    })
                    .to_string(),
                ),
                proposal_transition: proposal_transition(
                    proposal,
                    WorkflowProposalState::RolledBack,
                    "rollback-complete",
                    "exact rollback verified",
                    completed_at,
                ),
                installation_transition: installation_transition(
                    proposal,
                    WorkflowInstallationPhase::RollbackPending,
                    installation.phase_version,
                    WorkflowInstallationPhase::RolledBack,
                    "rolled-back",
                    "external rollback and registry state verified",
                    completed_at,
                    None,
                ),
                notification: Some(notification),
                completed_at_unix_ms: completed_at,
            })?;
        Ok(result.proposal.state)
    }

    fn exact_automation_job(
        &self,
        proposal: &WorkflowProposalRecord,
        payload: &WorkflowInstallRequestPayload,
    ) -> ActivationResult<CronJob> {
        let staged = self.staging.load_exact(
            required(&proposal.staging_job_id, "staging_job_id")?,
            &payload.revision_sha256,
        )?;
        if staged.manifest.artifact_sha256.as_str()
            != required(&proposal.artifact_sha256, "artifact_sha256")?.as_str()
            || staged.manifest.name.as_str() != required(&proposal.name, "name")?.as_str()
        {
            return Err(invalid_data("automation staging identity differs from SQLite").into());
        }
        let created_at = Utc
            .timestamp_millis_opt(proposal.created_at_unix_ms)
            .single()
            .ok_or_else(|| invalid_data("proposal timestamp cannot identify scheduler owner"))?;
        build_disabled_automation_job(
            &staged.artifact_bytes,
            required(&proposal.name, "name")?,
            automation_job_id(proposal, &payload.revision_sha256),
            AgentId::from_string(&proposal.source_agent_id),
            created_at,
        )
        .map_err(|error| invalid_data(error).into())
    }
}

pub(super) fn settle_activation_failure(
    control: &WorkflowLearningStore,
    worker: &str,
    job: &WorkflowJobRecord,
    error_message: &str,
    failed_at_unix_ms: i64,
) -> ActivationResult<WorkflowJobStatus> {
    if job.kind == WorkflowJobKind::Rollback {
        let notification = if job.attempt_count >= job.max_attempts {
            let payload = exact_payload(job)?;
            let proposal = exact_proposal(control, job, &payload)?;
            let installation = exact_installation(control, &proposal, &payload)?;
            Some(lifecycle_notification(
                job,
                &proposal,
                WorkflowLifecycleEvent::RollbackFailed,
                proposal.state,
                proposal.state_version,
                None,
                Some(installation.target_locator),
                Some("workflow_rollback_failed".to_string()),
                Some(error_message.to_string()),
                None,
                failed_at_unix_ms,
            )?)
        } else {
            None
        };
        let failed = control.fail_job_after_known_effect(
            &job.id,
            worker,
            "workflow_rollback_failed",
            error_message,
            true,
            failed_at_unix_ms.saturating_add(retry_backoff_ms(job.attempt_count)),
            failed_at_unix_ms,
            notification.as_ref(),
        )?;
        return Ok(failed.status);
    }
    let proposal = control
        .get(&job.proposal_id)?
        .ok_or_else(|| invalid_data("failed activation proposal vanished"))?;
    let expected = match job.kind {
        WorkflowJobKind::Install => WorkflowProposalState::ApprovedPendingInstall,
        WorkflowJobKind::Canary => WorkflowProposalState::ActiveCanary,
        _ => return Err(invalid_data("unsupported activation failure kind").into()),
    };
    require_state(&proposal, expected)?;
    let revision = required(&proposal.revision_sha256, "revision_sha256")?.clone();
    let installation = control.get_installation(&proposal.id, &revision)?;
    let installation_transition = installation.as_ref().map(|installation| {
        installation_transition(
            &proposal,
            installation.phase,
            installation.phase_version,
            WorkflowInstallationPhase::Failed,
            "failed",
            "activation effect returned a known failure",
            failed_at_unix_ms,
            Some(error_message.to_string()),
        )
    });
    let rollback_job = installation.as_ref().map(|_| {
        lifecycle_job(
            job,
            &WorkflowInstallRequestPayload {
                schema_version: 1,
                requested_mode: ProposalInstallMode::Activate,
                proposal_id: proposal.id.clone(),
                revision_sha256: revision.clone(),
                operator_actor: ACTOR.to_string(),
            },
            WorkflowJobKind::Rollback,
            stable_time(job, 50),
        )
    });
    let rollback_job = rollback_job.transpose()?;
    let notification = lifecycle_notification(
        job,
        &proposal,
        WorkflowLifecycleEvent::ActivationFailed,
        WorkflowProposalState::InstallFailed,
        proposal.state_version.saturating_add(1),
        None,
        installation
            .as_ref()
            .map(|installation| installation.target_locator.clone()),
        Some("workflow_activation_failed".to_string()),
        Some(error_message.to_string()),
        rollback_job.as_ref().map(|job| job.id.clone()),
        failed_at_unix_ms,
    )?;
    let result = control.fail_known_effect_and_schedule_rollback(&WorkflowEffectFailure {
        job_id: job.id.clone(),
        worker: worker.to_string(),
        job_kind: job.kind,
        error_code: "workflow_activation_failed".to_string(),
        error_message: error_message.to_string(),
        proposal_transition: proposal_transition(
            &proposal,
            WorkflowProposalState::InstallFailed,
            "activation-failed",
            "activation failed; rollback scheduled when an effect exists",
            failed_at_unix_ms,
        ),
        installation_transition,
        rollback_job,
        notification: Some(notification),
        failed_at_unix_ms,
    })?;
    Ok(result.job.status)
}

pub(super) fn retry_transient_activation_failure(
    control: &WorkflowLearningStore,
    worker: &str,
    job: &WorkflowJobRecord,
    error: &(dyn std::error::Error + Send + Sync + 'static),
    error_message: &str,
    failed_at_unix_ms: i64,
) -> ActivationResult<Option<WorkflowJobStatus>> {
    if job.kind == WorkflowJobKind::Rollback
        || job.attempt_count >= job.max_attempts
        || !matches!(
            error.downcast_ref::<WorkflowLearningControlError>(),
            Some(WorkflowLearningControlError::Sqlite(_))
        )
    {
        return Ok(None);
    }
    let failed = control.fail_job_after_known_effect(
        &job.id,
        worker,
        "workflow_control_transient",
        error_message,
        true,
        failed_at_unix_ms.saturating_add(retry_backoff_ms(job.attempt_count)),
        failed_at_unix_ms,
        None,
    )?;
    Ok(Some(failed.status))
}

fn exact_payload(job: &WorkflowJobRecord) -> ActivationResult<WorkflowInstallRequestPayload> {
    let payload: WorkflowInstallRequestPayload = serde_json::from_str(&job.payload_json)?;
    if payload.schema_version != 1
        || payload.requested_mode != ProposalInstallMode::Activate
        || payload.proposal_id != job.proposal_id
        || job.revision_sha256.as_deref() != Some(payload.revision_sha256.as_str())
        || payload.operator_actor.trim().is_empty()
    {
        return Err(invalid_data("claimed job is not an exact activation request").into());
    }
    Ok(payload)
}

fn exact_proposal(
    control: &WorkflowLearningStore,
    job: &WorkflowJobRecord,
    payload: &WorkflowInstallRequestPayload,
) -> ActivationResult<WorkflowProposalRecord> {
    let proposal = control
        .get(&job.proposal_id)?
        .ok_or_else(|| invalid_data("activation proposal no longer exists"))?;
    if proposal.revision_sha256.as_deref() != Some(payload.revision_sha256.as_str()) {
        return Err(invalid_data("activation proposal revision changed").into());
    }
    Ok(proposal)
}

fn exact_installation(
    control: &WorkflowLearningStore,
    proposal: &WorkflowProposalRecord,
    payload: &WorkflowInstallRequestPayload,
) -> ActivationResult<WorkflowInstallationRecord> {
    let installation = control
        .get_installation(&proposal.id, &payload.revision_sha256)?
        .ok_or_else(|| invalid_data("activation installation mirror is missing"))?;
    if installation.kind != required_kind(proposal)?
        || installation.installed_sha256.as_str()
            != required(&proposal.artifact_sha256, "artifact_sha256")?.as_str()
    {
        return Err(invalid_data("activation installation identity changed").into());
    }
    Ok(installation)
}

fn require_installation_target(
    installation: &WorkflowInstallationRecord,
    expected: &str,
) -> ActivationResult<()> {
    if installation.target_locator == expected {
        Ok(())
    } else {
        Err(invalid_data("activation installation target locator changed").into())
    }
}

fn lifecycle_job(
    parent: &WorkflowJobRecord,
    payload: &WorkflowInstallRequestPayload,
    kind: WorkflowJobKind,
    at: i64,
) -> ActivationResult<NewWorkflowJob> {
    let seed = format!(
        "captain:workflow-learning:{}:{}:{}",
        kind.as_str(),
        payload.proposal_id,
        payload.revision_sha256
    );
    let id = format!(
        "workflow-{}-{}",
        kind.as_str(),
        Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes())
    );
    Ok(NewWorkflowJob {
        id: id.clone(),
        idempotency_key: format!("{id}:enqueue"),
        proposal_id: payload.proposal_id.clone(),
        revision_sha256: Some(payload.revision_sha256.clone()),
        kind,
        payload_json: serde_json::to_string(payload)?,
        max_attempts: parent.max_attempts.clamp(1, 20),
        run_after_unix_ms: at,
        created_at_unix_ms: at,
    })
}

#[allow(clippy::too_many_arguments)]
fn lifecycle_notification(
    job: &WorkflowJobRecord,
    proposal: &WorkflowProposalRecord,
    event: WorkflowLifecycleEvent,
    state: WorkflowProposalState,
    decision_version: u64,
    continuation_job_id: Option<String>,
    target_locator: Option<String>,
    failure_code: Option<String>,
    failure_message: Option<String>,
    rollback_job_id: Option<String>,
    occurred_at_unix_ms: i64,
) -> ActivationResult<NewWorkflowOutboxItem> {
    let revision = required(&proposal.revision_sha256, "revision_sha256")?.clone();
    let card = WorkflowLifecycleCard {
        schema_version: WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION,
        event,
        proposal_id: proposal.id.clone(),
        revision_sha256: revision.clone(),
        decision_version,
        state: proposal_card_state(state),
        kind: proposal_card_kind(required_kind(proposal)?),
        name: required(&proposal.name, "name")?.clone(),
        lifecycle_job_id: job.id.clone(),
        continuation_job_id,
        target_locator,
        failure_code,
        failure_message: failure_message.map(|message| bounded_error(&message)),
        rollback_job_id,
        occurred_at_unix_ms,
    };
    Ok(NewWorkflowOutboxItem {
        id: format!("{}:notice", job.id),
        idempotency_key: format!("{}:notice", job.id),
        proposal_id: proposal.id.clone(),
        revision_sha256: Some(revision),
        topic: WORKFLOW_LIFECYCLE_OUTBOX_TOPIC.to_string(),
        payload_json: serde_json::to_string(&card)?,
        max_attempts: 8,
        run_after_unix_ms: occurred_at_unix_ms,
        created_at_unix_ms: occurred_at_unix_ms,
    })
}

fn proposal_card_kind(kind: WorkflowArtifactKind) -> ProposalCardKind {
    match kind {
        WorkflowArtifactKind::Skill => ProposalCardKind::Skill,
        WorkflowArtifactKind::Capspec => ProposalCardKind::Capspec,
        WorkflowArtifactKind::Automation => ProposalCardKind::Automation,
        WorkflowArtifactKind::Refinement => ProposalCardKind::Refinement,
    }
}

fn proposal_card_state(state: WorkflowProposalState) -> ProposalCardState {
    match state {
        WorkflowProposalState::Observed => ProposalCardState::Observed,
        WorkflowProposalState::Eligible => ProposalCardState::Eligible,
        WorkflowProposalState::Drafting => ProposalCardState::Drafting,
        WorkflowProposalState::Validating => ProposalCardState::Validating,
        WorkflowProposalState::Proposed => ProposalCardState::Proposed,
        WorkflowProposalState::Dismissed => ProposalCardState::Dismissed,
        WorkflowProposalState::Snoozed => ProposalCardState::Snoozed,
        WorkflowProposalState::Superseded => ProposalCardState::Superseded,
        WorkflowProposalState::ApprovedPendingInstall => ProposalCardState::ApprovedPendingInstall,
        WorkflowProposalState::ActiveCanary => ProposalCardState::ActiveCanary,
        WorkflowProposalState::Active => ProposalCardState::Active,
        WorkflowProposalState::Rejected => ProposalCardState::Rejected,
        WorkflowProposalState::InstallFailed => ProposalCardState::InstallFailed,
        WorkflowProposalState::RolledBack => ProposalCardState::RolledBack,
    }
}

fn automation_job_id(proposal: &WorkflowProposalRecord, revision: &str) -> CronJobId {
    let seed = format!("captain:workflow-automation:{}:{revision}", proposal.id);
    CronJobId(Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes()))
}

fn proposal_transition(
    proposal: &WorkflowProposalRecord,
    to_state: WorkflowProposalState,
    suffix: &str,
    reason: &str,
    occurred_at_unix_ms: i64,
) -> WorkflowProposalTransition {
    WorkflowProposalTransition {
        proposal_id: proposal.id.clone(),
        expected_state: proposal.state,
        expected_version: proposal.state_version,
        expected_revision_sha256: proposal.revision_sha256.clone(),
        to_state,
        actor: ACTOR.to_string(),
        reason: reason.to_string(),
        idempotency_key: operation_key(proposal, suffix),
        snoozed_until_unix_ms: None,
        occurred_at_unix_ms,
    }
}

#[allow(clippy::too_many_arguments)]
fn installation_transition(
    proposal: &WorkflowProposalRecord,
    from: WorkflowInstallationPhase,
    version: u64,
    to: WorkflowInstallationPhase,
    suffix: &str,
    reason: &str,
    occurred_at_unix_ms: i64,
    last_error: Option<String>,
) -> WorkflowInstallationTransition {
    WorkflowInstallationTransition {
        proposal_id: proposal.id.clone(),
        revision_sha256: proposal.revision_sha256.clone().unwrap_or_default(),
        expected_phase: from,
        expected_version: version,
        to_phase: to,
        last_error,
        actor: ACTOR.to_string(),
        reason: reason.to_string(),
        idempotency_key: operation_key(proposal, &format!("installation-{suffix}")),
        occurred_at_unix_ms,
    }
}

fn operation_key(proposal: &WorkflowProposalRecord, suffix: &str) -> String {
    let revision = proposal.revision_sha256.as_deref().unwrap_or("missing");
    format!(
        "{}:{}:{suffix}",
        proposal.id,
        &revision[..revision.len().min(16)]
    )
}

fn require_state(
    proposal: &WorkflowProposalRecord,
    expected: WorkflowProposalState,
) -> ActivationResult<()> {
    if proposal.state == expected {
        Ok(())
    } else {
        Err(invalid_data(format!(
            "expected proposal state {}, found {}",
            expected.as_str(),
            proposal.state.as_str()
        ))
        .into())
    }
}

fn required_kind(proposal: &WorkflowProposalRecord) -> ActivationResult<WorkflowArtifactKind> {
    proposal
        .kind
        .ok_or_else(|| invalid_data("proposal kind is missing").into())
}

fn required<'a, T>(value: &'a Option<T>, field: &str) -> ActivationResult<&'a T> {
    value
        .as_ref()
        .ok_or_else(|| invalid_data(format!("proposal {field} is missing")).into())
}

fn path_text(path: &std::path::Path) -> ActivationResult<String> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| invalid_data("promotion path is not UTF-8").into())
}

fn stable_time(job: &WorkflowJobRecord, offset_ms: i64) -> i64 {
    job.updated_at_unix_ms
        .max(job.created_at_unix_ms)
        .saturating_add(offset_ms)
}

fn retry_backoff_ms(attempt_count: u32) -> i64 {
    let exponent = attempt_count.saturating_sub(1).min(6);
    (5_000_i64.saturating_mul(1_i64 << exponent)).min(5 * 60 * 1_000)
}

pub(super) fn bounded_error(error: &str) -> String {
    let bounded = captain_types::truncate_str(error.trim(), 2_048).trim();
    if bounded.is_empty() {
        "workflow activation failed".to_string()
    } else {
        bounded.to_string()
    }
}

fn invalid_data(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
}

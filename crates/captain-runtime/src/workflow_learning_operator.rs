//! Exact control-plane decisions for one durable workflow proposal.

use captain_memory::workflow_learning_control::{
    WorkflowLearningControlError, WorkflowLearningStore, WorkflowProposalEvent,
    WorkflowProposalRecord, WorkflowProposalState, WorkflowProposalTransition,
};
use captain_memory::workflow_learning_queue::{NewWorkflowJob, WorkflowJobKind};
use captain_memory::workflow_learning_refinement::NewWorkflowRefinementRequest;
use captain_memory::workflow_learning_test::NewWorkflowIsolatedTest;
use captain_types::workflow_learning::{
    ProposalCardAction, ProposalInstallMode, ProposalOperatorContext, ProposalOperatorOutcome,
    ProposalOperatorResolution,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workflow_learning_card::{project_workflow_proposal_card, WorkflowProposalCardError};
use crate::workflow_learning_staging::WorkflowStagingRoot;

pub const WORKFLOW_OPERATOR_SNOOZE_MS: i64 = 24 * 60 * 60 * 1_000;
pub const WORKFLOW_REFINEMENT_INPUT_WINDOW_MS: i64 = 15 * 60 * 1_000;
const INSTALL_PAYLOAD_SCHEMA_VERSION: u16 = 1;
const INSTALL_MAX_ATTEMPTS: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowInstallRequestPayload {
    pub schema_version: u16,
    pub requested_mode: ProposalInstallMode,
    pub proposal_id: String,
    pub revision_sha256: String,
    pub operator_actor: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowOperatorError {
    #[error("invalid workflow operator actor")]
    InvalidActor,
    #[error("workflow proposal callback is unknown or expired")]
    UnknownToken,
    #[error("workflow action {action} is unavailable while proposal is {state}")]
    ActionUnavailable { action: String, state: String },
    #[error("workflow proposal callback is stale: {0}")]
    Stale(String),
    #[error("workflow operator replay is inconsistent: {0}")]
    InconsistentReplay(String),
    #[error("workflow operator clock overflow")]
    ClockOverflow,
    #[error("workflow install payload could not be encoded: {0}")]
    PayloadEncoding(String),
    #[error("workflow edit requires an exact surface and conversation context")]
    MissingRefinementContext,
    #[error(transparent)]
    Control(#[from] WorkflowLearningControlError),
    #[error(transparent)]
    Card(#[from] WorkflowProposalCardError),
}

#[derive(Clone)]
pub struct WorkflowLearningOperator {
    control: WorkflowLearningStore,
    staging: WorkflowStagingRoot,
}

impl WorkflowLearningOperator {
    pub fn new(control: WorkflowLearningStore, staging: WorkflowStagingRoot) -> Self {
        Self { control, staging }
    }

    pub fn resolve_at_version(
        &self,
        operator_token: &str,
        decision_version: u64,
        action: ProposalCardAction,
        actor: &str,
        now_unix_ms: i64,
    ) -> Result<ProposalOperatorResolution, WorkflowOperatorError> {
        self.resolve_inner(
            operator_token,
            action,
            actor,
            None,
            Some(decision_version),
            now_unix_ms,
        )
    }

    pub fn resolve_with_context_at_version(
        &self,
        operator_token: &str,
        decision_version: u64,
        action: ProposalCardAction,
        actor: &str,
        context: &ProposalOperatorContext,
        now_unix_ms: i64,
    ) -> Result<ProposalOperatorResolution, WorkflowOperatorError> {
        self.resolve_inner(
            operator_token,
            action,
            actor,
            Some(context),
            Some(decision_version),
            now_unix_ms,
        )
    }

    fn resolve_inner(
        &self,
        operator_token: &str,
        action: ProposalCardAction,
        actor: &str,
        context: Option<&ProposalOperatorContext>,
        expected_decision_version: Option<u64>,
        now_unix_ms: i64,
    ) -> Result<ProposalOperatorResolution, WorkflowOperatorError> {
        validate_actor(actor)?;
        let proposal = self
            .control
            .get_by_operator_token(operator_token)?
            .ok_or(WorkflowOperatorError::UnknownToken)?;
        if let Some(expected_version) = expected_decision_version {
            require_current_or_exact_replay(&self.control, &proposal, action, expected_version)?;
        }
        let card = project_workflow_proposal_card(&proposal, &self.staging)?;

        match action {
            ProposalCardAction::Details => Ok(resolution(
                card,
                ProposalOperatorOutcome::Details,
                false,
                false,
            )),
            ProposalCardAction::Edit => {
                require_available(&proposal, &card.available_actions, action)?;
                self.resolve_edit(
                    proposal,
                    card,
                    actor,
                    context.ok_or(WorkflowOperatorError::MissingRefinementContext)?,
                    now_unix_ms,
                )
            }
            ProposalCardAction::Activate | ProposalCardAction::Test => {
                self.resolve_install(proposal, card, action, actor, now_unix_ms)
            }
            ProposalCardAction::Later => self.resolve_snooze(proposal, card, actor, now_unix_ms),
            ProposalCardAction::Ignore => self.resolve_dismiss(proposal, card, actor, now_unix_ms),
        }
    }

    fn resolve_edit(
        &self,
        proposal: WorkflowProposalRecord,
        card: captain_types::workflow_learning::ProposalCard,
        actor: &str,
        context: &ProposalOperatorContext,
        now_unix_ms: i64,
    ) -> Result<ProposalOperatorResolution, WorkflowOperatorError> {
        if let Some(existing) = self.control.pending_refinement_for_binding(
            &context.surface,
            &context.conversation_key,
            actor,
            now_unix_ms,
        )? {
            if existing.proposal_id == proposal.id
                && existing.revision_sha256 == card.revision_sha256
                && existing.expected_proposal_version == proposal.state_version
            {
                return Ok(resolution(
                    card,
                    ProposalOperatorOutcome::EditRequested {
                        request_id: existing.id,
                        expires_at_unix_ms: existing.expires_at_unix_ms,
                    },
                    true,
                    false,
                ));
            }
            return Err(WorkflowOperatorError::Stale(
                "another proposal is already awaiting input in this conversation".to_string(),
            ));
        }
        let expires_at_unix_ms = now_unix_ms
            .checked_add(WORKFLOW_REFINEMENT_INPUT_WINDOW_MS)
            .ok_or(WorkflowOperatorError::ClockOverflow)?;
        let mut seed = Sha256::new();
        let timestamp = now_unix_ms.to_string();
        for value in [
            proposal.id.as_str(),
            card.revision_sha256.as_str(),
            actor,
            context.surface.as_str(),
            context.conversation_key.as_str(),
            timestamp.as_str(),
        ] {
            seed.update(value.as_bytes());
            seed.update(b"\0");
        }
        let request_id = format!("wr-{}", hex::encode(seed.finalize()));
        let request = self
            .control
            .begin_refinement_request(&NewWorkflowRefinementRequest {
                id: request_id.clone(),
                idempotency_key: format!("{request_id}:begin"),
                proposal_id: proposal.id,
                revision_sha256: card.revision_sha256.clone(),
                expected_proposal_version: proposal.state_version,
                actor: actor.to_string(),
                surface: context.surface.clone(),
                conversation_key: context.conversation_key.clone(),
                source_message_id: context.source_message_id.clone(),
                language: context.language.clone(),
                expires_at_unix_ms,
                created_at_unix_ms: now_unix_ms,
            })?;
        let replayed = request.id != request_id;
        Ok(resolution(
            card,
            ProposalOperatorOutcome::EditRequested {
                request_id: request.id,
                expires_at_unix_ms: request.expires_at_unix_ms,
            },
            replayed,
            false,
        ))
    }

    fn resolve_install(
        &self,
        proposal: WorkflowProposalRecord,
        card: captain_types::workflow_learning::ProposalCard,
        action: ProposalCardAction,
        actor: &str,
        now_unix_ms: i64,
    ) -> Result<ProposalOperatorResolution, WorkflowOperatorError> {
        let mode = match action {
            ProposalCardAction::Activate => ProposalInstallMode::Activate,
            ProposalCardAction::Test => ProposalInstallMode::Test,
            _ => unreachable!(),
        };
        if proposal.state != WorkflowProposalState::Proposed {
            self.verify_install_replay(&proposal, action, mode)?;
            return Ok(resolution(
                card,
                ProposalOperatorOutcome::InstallQueued { mode },
                true,
                true,
            ));
        }
        require_available(&proposal, &card.available_actions, action)?;
        let revision = required_revision(&proposal)?;
        let transition_key = operator_key(action, &card.lookup_token, proposal.state_version);
        let install_id = install_job_id(action, &card.lookup_token, proposal.state_version);
        let transition = WorkflowProposalTransition {
            proposal_id: proposal.id.clone(),
            expected_state: WorkflowProposalState::Proposed,
            expected_version: proposal.state_version,
            expected_revision_sha256: Some(revision.clone()),
            to_state: WorkflowProposalState::ApprovedPendingInstall,
            actor: actor.to_string(),
            reason: format!("operator requested {}", action.as_str()),
            idempotency_key: transition_key,
            snoozed_until_unix_ms: None,
            occurred_at_unix_ms: now_unix_ms,
        };
        let payload = WorkflowInstallRequestPayload {
            schema_version: INSTALL_PAYLOAD_SCHEMA_VERSION,
            requested_mode: mode,
            proposal_id: proposal.id.clone(),
            revision_sha256: revision.clone(),
            operator_actor: actor.to_string(),
        };
        let payload_json = serde_json::to_string(&payload)
            .map_err(|error| WorkflowOperatorError::PayloadEncoding(error.to_string()))?;
        let install_job = NewWorkflowJob {
            id: install_id,
            idempotency_key: operator_install_key(
                action,
                &card.lookup_token,
                proposal.state_version,
            ),
            proposal_id: proposal.id.clone(),
            revision_sha256: Some(revision.clone()),
            kind: WorkflowJobKind::Install,
            payload_json,
            max_attempts: INSTALL_MAX_ATTEMPTS,
            run_after_unix_ms: now_unix_ms,
            created_at_unix_ms: now_unix_ms,
        };
        let updated = if mode == ProposalInstallMode::Test {
            self.control
                .approve_and_enqueue_isolated_test(
                    &transition,
                    &install_job,
                    &NewWorkflowIsolatedTest {
                        id: isolated_test_id(&card.lookup_token, proposal.state_version),
                        idempotency_key: operator_test_key(
                            &card.lookup_token,
                            proposal.state_version,
                        ),
                        proposal_id: proposal.id.clone(),
                        revision_sha256: revision,
                        job_id: install_job.id.clone(),
                        requested_by: actor.to_string(),
                        requested_at_unix_ms: now_unix_ms,
                    },
                )?
                .proposal
        } else {
            self.control
                .approve_and_enqueue_install(&transition, &install_job, None)?
        };
        let updated_card = project_workflow_proposal_card(&updated, &self.staging)?;
        Ok(resolution(
            updated_card,
            ProposalOperatorOutcome::InstallQueued { mode },
            false,
            true,
        ))
    }

    fn resolve_snooze(
        &self,
        proposal: WorkflowProposalRecord,
        card: captain_types::workflow_learning::ProposalCard,
        actor: &str,
        now_unix_ms: i64,
    ) -> Result<ProposalOperatorResolution, WorkflowOperatorError> {
        if proposal.state == WorkflowProposalState::Snoozed
            && matching_event(&self.control, &proposal, ProposalCardAction::Later)?.is_some()
        {
            return Ok(resolution(
                card,
                ProposalOperatorOutcome::Snoozed {
                    until_unix_ms: proposal.snoozed_until_unix_ms.ok_or_else(|| {
                        WorkflowOperatorError::InconsistentReplay(
                            "snoozed proposal has no deadline".to_string(),
                        )
                    })?,
                },
                true,
                true,
            ));
        }
        require_proposed_and_available(
            &proposal,
            &card.available_actions,
            ProposalCardAction::Later,
        )?;
        let until_unix_ms = now_unix_ms
            .checked_add(WORKFLOW_OPERATOR_SNOOZE_MS)
            .ok_or(WorkflowOperatorError::ClockOverflow)?;
        let updated = self.control.transition(&operator_transition(
            &proposal,
            &card.lookup_token,
            ProposalCardAction::Later,
            WorkflowProposalState::Snoozed,
            actor,
            "operator postponed proposal",
            Some(until_unix_ms),
            now_unix_ms,
        )?)?;
        let updated_card = project_workflow_proposal_card(&updated, &self.staging)?;
        Ok(resolution(
            updated_card,
            ProposalOperatorOutcome::Snoozed { until_unix_ms },
            false,
            true,
        ))
    }

    fn resolve_dismiss(
        &self,
        proposal: WorkflowProposalRecord,
        card: captain_types::workflow_learning::ProposalCard,
        actor: &str,
        now_unix_ms: i64,
    ) -> Result<ProposalOperatorResolution, WorkflowOperatorError> {
        if proposal.state == WorkflowProposalState::Dismissed
            && matching_event(&self.control, &proposal, ProposalCardAction::Ignore)?.is_some()
        {
            return Ok(resolution(
                card,
                ProposalOperatorOutcome::Dismissed,
                true,
                true,
            ));
        }
        require_proposed_and_available(
            &proposal,
            &card.available_actions,
            ProposalCardAction::Ignore,
        )?;
        let updated = self.control.transition(&operator_transition(
            &proposal,
            &card.lookup_token,
            ProposalCardAction::Ignore,
            WorkflowProposalState::Dismissed,
            actor,
            "operator dismissed proposal",
            None,
            now_unix_ms,
        )?)?;
        let updated_card = project_workflow_proposal_card(&updated, &self.staging)?;
        Ok(resolution(
            updated_card,
            ProposalOperatorOutcome::Dismissed,
            false,
            true,
        ))
    }

    fn verify_install_replay(
        &self,
        proposal: &WorkflowProposalRecord,
        action: ProposalCardAction,
        mode: ProposalInstallMode,
    ) -> Result<(), WorkflowOperatorError> {
        let event = matching_event(&self.control, proposal, action)?.ok_or_else(|| {
            WorkflowOperatorError::Stale(format!(
                "{} was not the decision recorded for state {}",
                action.as_str(),
                proposal.state.as_str()
            ))
        })?;
        if event.from_state != Some(WorkflowProposalState::Proposed)
            || event.to_state != WorkflowProposalState::ApprovedPendingInstall
        {
            return Err(WorkflowOperatorError::InconsistentReplay(
                "operator event is not an approval".to_string(),
            ));
        }
        let expected_version = event.resulting_version.checked_sub(1).ok_or_else(|| {
            WorkflowOperatorError::InconsistentReplay(
                "approval event has no predecessor version".to_string(),
            )
        })?;
        let token = proposal.operator_token.as_deref().ok_or_else(|| {
            WorkflowOperatorError::InconsistentReplay("operator token disappeared".to_string())
        })?;
        let job_id = install_job_id(action, token, expected_version);
        let job_key = operator_install_key(action, token, expected_version);
        let job = self.control.get_job(&job_id)?.ok_or_else(|| {
            WorkflowOperatorError::InconsistentReplay("install job is missing".to_string())
        })?;
        let payload: WorkflowInstallRequestPayload = serde_json::from_str(&job.payload_json)
            .map_err(|error| WorkflowOperatorError::InconsistentReplay(error.to_string()))?;
        if job.kind != WorkflowJobKind::Install
            || job.idempotency_key != job_key
            || job.proposal_id != proposal.id
            || job.revision_sha256 != proposal.revision_sha256
            || payload.schema_version != INSTALL_PAYLOAD_SCHEMA_VERSION
            || payload.requested_mode != mode
            || payload.proposal_id != proposal.id
            || Some(payload.revision_sha256.as_str()) != proposal.revision_sha256.as_deref()
            || payload.operator_actor != event.actor
        {
            return Err(WorkflowOperatorError::InconsistentReplay(
                "install job does not match the recorded operator decision".to_string(),
            ));
        }
        if mode == ProposalInstallMode::Test {
            let isolated_test = proposal.isolated_test.as_ref().ok_or_else(|| {
                WorkflowOperatorError::InconsistentReplay(
                    "isolated test record is missing".to_string(),
                )
            })?;
            if isolated_test.id != isolated_test_id(token, expected_version)
                || isolated_test.idempotency_key != operator_test_key(token, expected_version)
                || isolated_test.job_id != job_id
                || isolated_test.proposal_id != proposal.id
                || Some(isolated_test.revision_sha256.as_str())
                    != proposal.revision_sha256.as_deref()
                || isolated_test.requested_by != event.actor
            {
                return Err(WorkflowOperatorError::InconsistentReplay(
                    "isolated test record does not match the operator decision".to_string(),
                ));
            }
        }
        Ok(())
    }
}

fn operator_transition(
    proposal: &WorkflowProposalRecord,
    token: &str,
    action: ProposalCardAction,
    to_state: WorkflowProposalState,
    actor: &str,
    reason: &str,
    snoozed_until_unix_ms: Option<i64>,
    occurred_at_unix_ms: i64,
) -> Result<WorkflowProposalTransition, WorkflowOperatorError> {
    Ok(WorkflowProposalTransition {
        proposal_id: proposal.id.clone(),
        expected_state: WorkflowProposalState::Proposed,
        expected_version: proposal.state_version,
        expected_revision_sha256: Some(required_revision(proposal)?),
        to_state,
        actor: actor.to_string(),
        reason: reason.to_string(),
        idempotency_key: operator_key(action, token, proposal.state_version),
        snoozed_until_unix_ms,
        occurred_at_unix_ms,
    })
}

fn matching_event(
    control: &WorkflowLearningStore,
    proposal: &WorkflowProposalRecord,
    action: ProposalCardAction,
) -> Result<Option<WorkflowProposalEvent>, WorkflowOperatorError> {
    let token = proposal.operator_token.as_deref().ok_or_else(|| {
        WorkflowOperatorError::InconsistentReplay("operator token disappeared".to_string())
    })?;
    let prefix = format!("operator:{}:{token}:v", action.as_str());
    Ok(control
        .events(&proposal.id)?
        .into_iter()
        .rev()
        .find(|event| {
            event.idempotency_key.starts_with(&prefix)
                && event.revision_sha256 == proposal.revision_sha256
        }))
}

fn require_current_or_exact_replay(
    control: &WorkflowLearningStore,
    proposal: &WorkflowProposalRecord,
    action: ProposalCardAction,
    expected_version: u64,
) -> Result<(), WorkflowOperatorError> {
    if proposal.state_version == expected_version {
        return Ok(());
    }
    if proposal.state == WorkflowProposalState::Proposed
        || matches!(
            action,
            ProposalCardAction::Details | ProposalCardAction::Edit
        )
    {
        return Err(stale_decision_version(proposal, expected_version));
    }
    let token = proposal.operator_token.as_deref().ok_or_else(|| {
        WorkflowOperatorError::InconsistentReplay("operator token disappeared".to_string())
    })?;
    let expected_resulting_version = expected_version.checked_add(1).ok_or_else(|| {
        WorkflowOperatorError::InconsistentReplay(
            "operator decision version cannot advance".to_string(),
        )
    })?;
    let expected_target = match action {
        ProposalCardAction::Activate | ProposalCardAction::Test => {
            WorkflowProposalState::ApprovedPendingInstall
        }
        ProposalCardAction::Later => WorkflowProposalState::Snoozed,
        ProposalCardAction::Ignore => WorkflowProposalState::Dismissed,
        ProposalCardAction::Details | ProposalCardAction::Edit => unreachable!(),
    };
    let exact_key = operator_key(action, token, expected_version);
    let exact_replay = control.events(&proposal.id)?.into_iter().any(|event| {
        event.idempotency_key == exact_key
            && event.from_state == Some(WorkflowProposalState::Proposed)
            && event.to_state == expected_target
            && event.resulting_version == expected_resulting_version
            && event.revision_sha256 == proposal.revision_sha256
    });
    if exact_replay {
        Ok(())
    } else {
        Err(stale_decision_version(proposal, expected_version))
    }
}

fn stale_decision_version(
    proposal: &WorkflowProposalRecord,
    expected_version: u64,
) -> WorkflowOperatorError {
    WorkflowOperatorError::Stale(format!(
        "card version {expected_version} differs from current version {}",
        proposal.state_version
    ))
}

fn require_proposed_and_available(
    proposal: &WorkflowProposalRecord,
    available: &[ProposalCardAction],
    action: ProposalCardAction,
) -> Result<(), WorkflowOperatorError> {
    if proposal.state != WorkflowProposalState::Proposed {
        return Err(WorkflowOperatorError::Stale(format!(
            "proposal is {}",
            proposal.state.as_str()
        )));
    }
    require_available(proposal, available, action)
}

fn require_available(
    proposal: &WorkflowProposalRecord,
    available: &[ProposalCardAction],
    action: ProposalCardAction,
) -> Result<(), WorkflowOperatorError> {
    if available.contains(&action) {
        Ok(())
    } else {
        Err(WorkflowOperatorError::ActionUnavailable {
            action: action.as_str().to_string(),
            state: proposal.state.as_str().to_string(),
        })
    }
}

fn required_revision(proposal: &WorkflowProposalRecord) -> Result<String, WorkflowOperatorError> {
    proposal.revision_sha256.clone().ok_or_else(|| {
        WorkflowOperatorError::InconsistentReplay("proposal revision disappeared".to_string())
    })
}

fn operator_key(action: ProposalCardAction, token: &str, version: u64) -> String {
    format!("operator:{}:{token}:v{version}", action.as_str())
}

fn operator_install_key(action: ProposalCardAction, token: &str, version: u64) -> String {
    format!("operator-install:{}:{token}:v{version}", action.as_str())
}

fn install_job_id(action: ProposalCardAction, token: &str, version: u64) -> String {
    format!("install-{}-{token}-v{version}", action.as_str())
}

fn isolated_test_id(token: &str, version: u64) -> String {
    format!("test-{token}-v{version}")
}

fn operator_test_key(token: &str, version: u64) -> String {
    format!("operator-test:{token}:v{version}")
}

fn validate_actor(actor: &str) -> Result<(), WorkflowOperatorError> {
    let valid = !actor.is_empty()
        && actor.len() <= 128
        && actor
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    if valid {
        Ok(())
    } else {
        Err(WorkflowOperatorError::InvalidActor)
    }
}

fn resolution(
    card: captain_types::workflow_learning::ProposalCard,
    outcome: ProposalOperatorOutcome,
    replayed: bool,
    retire_keyboard: bool,
) -> ProposalOperatorResolution {
    ProposalOperatorResolution {
        card,
        outcome,
        replayed,
        retire_keyboard,
    }
}

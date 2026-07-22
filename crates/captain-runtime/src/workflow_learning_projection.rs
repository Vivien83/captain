//! Durable workflow-learning read model shared by every operator surface.

use captain_memory::workflow_learning_control::{
    WorkflowLearningControlError, WorkflowLearningStore, WorkflowProposalRecord,
    WorkflowProposalSnapshot, WorkflowProposalState,
};
use captain_memory::workflow_learning_installation::{
    WorkflowInstallationPhase, WorkflowInstallationRecord,
};
use captain_types::workflow_learning::{
    WorkflowInstallationView, WorkflowInstallationViewPhase, WorkflowLearningList,
    WorkflowLearningView, WorkflowProjectionStatus, WorkflowTimelineEntry,
    WORKFLOW_LEARNING_VIEW_SCHEMA_VERSION,
};

use crate::workflow_learning_card::{map_artifact_kind, map_state, project_workflow_proposal_card};
use crate::workflow_learning_staging::WorkflowStagingRoot;

pub fn project_workflow_learning_list(
    control: &WorkflowLearningStore,
    staging: &WorkflowStagingRoot,
    limit: usize,
) -> Result<WorkflowLearningList, WorkflowLearningControlError> {
    let mut workflows = Vec::new();
    for snapshot in control.list_snapshots(limit)? {
        workflows.push(project_workflow_learning_view(staging, snapshot));
    }
    Ok(WorkflowLearningList {
        schema_version: WORKFLOW_LEARNING_VIEW_SCHEMA_VERSION,
        returned: workflows.len(),
        workflows,
    })
}

pub fn project_workflow_learning_view(
    staging: &WorkflowStagingRoot,
    snapshot: WorkflowProposalSnapshot,
) -> WorkflowLearningView {
    let proposal = &snapshot.proposal;
    let mut projection_issues = Vec::new();
    let should_have_card = state_requires_verified_card(proposal.state);
    let can_attempt_card = should_have_card || has_complete_card_identity(proposal);
    let card = if can_attempt_card {
        match project_workflow_proposal_card(proposal, staging) {
            Ok(card) => Some(card),
            Err(error) => {
                projection_issues.push(format!("operator card unavailable: {error}"));
                None
            }
        }
    } else {
        None
    };
    if should_have_card && card.is_none() && projection_issues.is_empty() {
        projection_issues.push("operator card is missing for a decisionable revision".to_string());
    }

    let installation = snapshot.installation;
    if state_requires_installation(proposal.state) && installation.is_none() {
        projection_issues
            .push("installation mirror is missing for an activated proposal revision".to_string());
    }
    if let (Some(expected_kind), Some(actual)) = (proposal.kind, installation.as_ref()) {
        if actual.kind != expected_kind {
            projection_issues
                .push("installation mirror kind differs from the proposal revision".to_string());
        }
    }

    let mut timeline = snapshot
        .proposal_events
        .into_iter()
        .map(|event| WorkflowTimelineEntry::Proposal {
            sequence: event.sequence,
            from_state: event.from_state.map(map_state),
            to_state: map_state(event.to_state),
            resulting_version: event.resulting_version,
            actor: event.actor,
            reason: event.reason,
            occurred_at_unix_ms: event.created_at_unix_ms,
        })
        .collect::<Vec<_>>();
    timeline.extend(snapshot.installation_events.into_iter().map(|event| {
        WorkflowTimelineEntry::Installation {
            sequence: event.sequence,
            from_phase: event.from_phase.map(map_installation_phase),
            to_phase: map_installation_phase(event.to_phase),
            resulting_version: event.resulting_version,
            actor: event.actor,
            reason: event.reason,
            last_error: event.last_error,
            occurred_at_unix_ms: event.created_at_unix_ms,
        }
    }));
    timeline.sort_by_key(|entry| {
        let domain = match entry {
            WorkflowTimelineEntry::Proposal { .. } => 0_u8,
            WorkflowTimelineEntry::Installation { .. } => 1_u8,
        };
        (entry.occurred_at_unix_ms(), domain, entry.sequence())
    });

    let projection_status = if projection_issues.is_empty() {
        if card.is_some() {
            WorkflowProjectionStatus::Verified
        } else {
            WorkflowProjectionStatus::Building
        }
    } else {
        WorkflowProjectionStatus::Invalid
    };
    let projection_error = (!projection_issues.is_empty()).then(|| projection_issues.join("; "));

    WorkflowLearningView {
        schema_version: WORKFLOW_LEARNING_VIEW_SCHEMA_VERSION,
        proposal_id: proposal.id.clone(),
        decision_version: proposal.state_version,
        state: map_state(proposal.state),
        revision_sha256: proposal.revision_sha256.clone(),
        kind: proposal.kind.map(map_artifact_kind),
        name: proposal.name.clone(),
        source_agent_id: proposal.source_agent_id.clone(),
        origin_channel: proposal.origin_channel.clone(),
        created_at_unix_ms: proposal.created_at_unix_ms,
        updated_at_unix_ms: proposal.updated_at_unix_ms,
        last_error_code: proposal.last_error_code.clone(),
        last_error_message: proposal.last_error_message.clone(),
        projection_status,
        projection_error,
        card,
        installation: installation.as_ref().map(project_installation),
        timeline,
    }
}

fn state_requires_verified_card(state: WorkflowProposalState) -> bool {
    matches!(
        state,
        WorkflowProposalState::Proposed
            | WorkflowProposalState::Dismissed
            | WorkflowProposalState::Snoozed
            | WorkflowProposalState::Superseded
            | WorkflowProposalState::ApprovedPendingInstall
            | WorkflowProposalState::ActiveCanary
            | WorkflowProposalState::Active
            | WorkflowProposalState::InstallFailed
            | WorkflowProposalState::RolledBack
    )
}

fn state_requires_installation(state: WorkflowProposalState) -> bool {
    matches!(
        state,
        WorkflowProposalState::ActiveCanary
            | WorkflowProposalState::Active
            | WorkflowProposalState::InstallFailed
            | WorkflowProposalState::RolledBack
    )
}

fn has_complete_card_identity(proposal: &WorkflowProposalRecord) -> bool {
    proposal.revision_sha256.is_some()
        && proposal.operator_token.is_some()
        && proposal.artifact_sha256.is_some()
        && proposal.staging_job_id.is_some()
        && proposal.kind.is_some()
        && proposal.name.is_some()
        && proposal.validation_json.is_some()
}

fn project_installation(installation: &WorkflowInstallationRecord) -> WorkflowInstallationView {
    WorkflowInstallationView {
        phase: map_installation_phase(installation.phase),
        phase_version: installation.phase_version,
        target_locator: installation.target_locator.clone(),
        last_error: installation.last_error.clone(),
        updated_at_unix_ms: installation.updated_at_unix_ms,
    }
}

fn map_installation_phase(phase: WorkflowInstallationPhase) -> WorkflowInstallationViewPhase {
    match phase {
        WorkflowInstallationPhase::Prepared => WorkflowInstallationViewPhase::Prepared,
        WorkflowInstallationPhase::Promoted => WorkflowInstallationViewPhase::Promoted,
        WorkflowInstallationPhase::Verified => WorkflowInstallationViewPhase::Verified,
        WorkflowInstallationPhase::Active => WorkflowInstallationViewPhase::Active,
        WorkflowInstallationPhase::RollbackPending => {
            WorkflowInstallationViewPhase::RollbackPending
        }
        WorkflowInstallationPhase::RolledBack => WorkflowInstallationViewPhase::RolledBack,
        WorkflowInstallationPhase::Quarantined => WorkflowInstallationViewPhase::Quarantined,
        WorkflowInstallationPhase::Failed => WorkflowInstallationViewPhase::Failed,
    }
}

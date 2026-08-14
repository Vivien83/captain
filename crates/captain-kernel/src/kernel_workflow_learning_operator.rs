//! Authenticated operator decisions for Skill Learning V2.

use captain_channels::telegram::parse_workflow_learning_callback;
use captain_memory::workflow_learning_control::WorkflowLearningStore;
use captain_memory::workflow_learning_status::WorkflowLearningOperationalSnapshot;
use captain_runtime::audit::AuditAction;
use captain_runtime::workflow_learning_operator::WorkflowLearningOperator;
use captain_runtime::workflow_learning_projection::project_workflow_learning_list;
use captain_runtime::workflow_learning_refinement::{
    WorkflowRefinementCaptureInput, WorkflowRefinementCoordinator,
};
use captain_runtime::workflow_learning_staging::WorkflowStagingRoot;
use captain_types::workflow_learning::{
    ProposalCardAction, ProposalOperatorContext, ProposalOperatorResolution,
    ProposalRefinementCaptureResolution, ProposalRefinementMessage, WorkflowLearningJobStage,
    WorkflowLearningList, WorkflowLearningModelIdentity, WorkflowLearningRecoveryState,
    WorkflowLearningRetryResolution, WorkflowLearningRuntimeState, WorkflowLearningStatus,
    WorkflowLearningWorkerPhase, WorkflowLearningWorkerView,
    WORKFLOW_LEARNING_RETRY_SCHEMA_VERSION, WORKFLOW_LEARNING_STATUS_SCHEMA_VERSION,
};

use super::CaptainKernel;

const WORKFLOW_LEARNING_WORKER_STALE_AFTER_MS: u64 = 90_000;

impl CaptainKernel {
    pub fn workflow_learning_list(&self, limit: usize) -> Result<WorkflowLearningList, String> {
        let control = WorkflowLearningStore::new(self.memory.usage_conn());
        let staging = WorkflowStagingRoot::new(self.config.home_dir.clone())
            .map_err(|error| error.to_string())?;
        project_workflow_learning_list(&control, &staging, limit).map_err(|error| error.to_string())
    }

    pub fn workflow_learning_status(&self) -> Result<WorkflowLearningStatus, String> {
        let now_unix_ms = chrono::Utc::now().timestamp_millis();
        let control = WorkflowLearningStore::new(self.memory.usage_conn());
        let snapshot = control
            .operational_snapshot()
            .map_err(|error| error.to_string())?;
        let expected = self.workflow_learning_active_model();
        Ok(project_workflow_learning_status(
            self.config.skills.enabled
                && self.config.skills.mode != captain_types::config::LearningMode::Off,
            self.config.skills.mode,
            WorkflowLearningModelIdentity {
                provider: expected.provider,
                model: expected.model,
            },
            snapshot,
            now_unix_ms,
        ))
    }

    pub fn workflow_learning_retry_dead_proposal(
        &self,
        proposal_id: &str,
        expected_error_code: &str,
        actor: &str,
    ) -> Result<WorkflowLearningRetryResolution, String> {
        let now_unix_ms = chrono::Utc::now().timestamp_millis();
        let control = WorkflowLearningStore::new(self.memory.usage_conn());
        let result =
            control.retry_dead_preapproval_job(proposal_id, expected_error_code, now_unix_ms);
        match result {
            Ok(result) => {
                let resolution = WorkflowLearningRetryResolution {
                    schema_version: WORKFLOW_LEARNING_RETRY_SCHEMA_VERSION,
                    proposal_id: result.job.proposal_id.clone(),
                    stage: public_job_stage(result.job.kind),
                    queued_at_unix_ms: result.job.updated_at_unix_ms,
                    replayed: result.replayed,
                };
                self.audit_log.record_or_alert(
                    actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow learning retry proposal={} stage={} error_code={} replayed={}",
                        resolution.proposal_id,
                        result.job.kind.as_str(),
                        expected_error_code,
                        resolution.replayed
                    ),
                    "accepted",
                );
                Ok(resolution)
            }
            Err(error) => {
                self.audit_log.record_or_alert(
                    actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow learning retry proposal={} error_code={}",
                        proposal_id, expected_error_code
                    ),
                    format!("rejected: {error}"),
                );
                Err(error.to_string())
            }
        }
    }

    pub fn workflow_learning_resolve_surface_action(
        &self,
        operator_token: &str,
        decision_version: u64,
        action: ProposalCardAction,
        actor: &str,
    ) -> Result<ProposalOperatorResolution, String> {
        let control = WorkflowLearningStore::new(self.memory.usage_conn());
        let staging = WorkflowStagingRoot::new(self.config.home_dir.clone()).map_err(|error| {
            self.audit_log.record_or_alert(
                actor,
                AuditAction::LearningDecision,
                format!(
                    "workflow proposal surface action={} token={} staging unavailable",
                    action.as_str(),
                    operator_token
                ),
                format!("rejected: {error}"),
            );
            error.to_string()
        })?;
        let operator = WorkflowLearningOperator::new(control, staging);
        let result = operator.resolve_at_version(
            operator_token,
            decision_version,
            action,
            actor,
            chrono::Utc::now().timestamp_millis(),
        );
        match result {
            Ok(resolution) => {
                self.audit_log.record_or_alert(
                    actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow proposal surface action={} proposal={} revision={} token={} replayed={}",
                        action.as_str(),
                        resolution.card.proposal_id,
                        resolution.card.revision_sha256,
                        operator_token,
                        resolution.replayed
                    ),
                    "accepted",
                );
                Ok(resolution)
            }
            Err(error) => {
                self.audit_log.record_or_alert(
                    actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow proposal surface action={} token={}",
                        action.as_str(),
                        operator_token
                    ),
                    format!("rejected: {error}"),
                );
                Err(error.to_string())
            }
        }
    }

    pub fn workflow_learning_resolve_telegram_callback(
        &self,
        callback_data: &str,
        actor: &str,
        context: &ProposalOperatorContext,
    ) -> Result<ProposalOperatorResolution, String> {
        if !authenticated_telegram_actor(actor) || !valid_telegram_context(context) {
            self.audit_log.record_or_alert(
                actor,
                AuditAction::LearningDecision,
                "workflow learning Telegram decision rejected before parsing",
                "denied",
            );
            return Err(
                "Workflow learning requires an authenticated Telegram operator".to_string(),
            );
        }
        let callback = parse_workflow_learning_callback(callback_data).ok_or_else(|| {
            self.audit_log.record_or_alert(
                actor,
                AuditAction::LearningDecision,
                "invalid workflow learning Telegram callback",
                "denied",
            );
            "Invalid or expired workflow learning callback".to_string()
        })?;
        let control = WorkflowLearningStore::new(self.memory.usage_conn());
        let staging = match WorkflowStagingRoot::new(self.config.home_dir.clone()) {
            Ok(staging) => staging,
            Err(error) => {
                self.audit_log.record_or_alert(
                    actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow proposal action={} token={} staging unavailable",
                        callback.action.as_str(),
                        callback.token
                    ),
                    format!("rejected: {error}"),
                );
                return Err(error.to_string());
            }
        };
        let operator = WorkflowLearningOperator::new(control, staging);
        let result = operator.resolve_with_context_at_version(
            &callback.token,
            callback.decision_version,
            callback.action,
            actor,
            context,
            chrono::Utc::now().timestamp_millis(),
        );
        match result {
            Ok(resolution) => {
                self.audit_log.record_or_alert(
                    actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow proposal action={} proposal={} revision={} token={} replayed={}",
                        callback.action.as_str(),
                        resolution.card.proposal_id,
                        resolution.card.revision_sha256,
                        callback.token,
                        resolution.replayed
                    ),
                    "accepted",
                );
                Ok(resolution)
            }
            Err(error) => {
                self.audit_log.record_or_alert(
                    actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow proposal action={} token={}",
                        callback.action.as_str(),
                        callback.token
                    ),
                    format!("rejected: {error}"),
                );
                Err(error.to_string())
            }
        }
    }

    pub fn workflow_learning_capture_refinement(
        &self,
        message: &ProposalRefinementMessage,
    ) -> Result<Option<ProposalRefinementCaptureResolution>, String> {
        if !authenticated_telegram_actor(&message.actor)
            || message.surface != "telegram"
            || !message.conversation_key.starts_with("telegram:chat:")
        {
            self.audit_log.record_or_alert(
                &message.actor,
                AuditAction::LearningDecision,
                "workflow learning Telegram refinement rejected before capture",
                "denied",
            );
            return Err(
                "Workflow learning requires an authenticated Telegram conversation".to_string(),
            );
        }
        let control = WorkflowLearningStore::new(self.memory.usage_conn());
        let staging = WorkflowStagingRoot::new(self.config.home_dir.clone()).map_err(|error| {
            self.audit_log.record_or_alert(
                &message.actor,
                AuditAction::LearningDecision,
                "workflow learning Telegram refinement staging unavailable",
                format!("rejected: {error}"),
            );
            error.to_string()
        })?;
        let coordinator = WorkflowRefinementCoordinator::new(control, staging);
        let result = coordinator.capture_pending_with_status(&WorkflowRefinementCaptureInput {
            actor: message.actor.clone(),
            surface: message.surface.clone(),
            conversation_key: message.conversation_key.clone(),
            captured_message_id: message.message_id.clone(),
            instruction: message.instruction.clone(),
            captured_at_unix_ms: chrono::Utc::now().timestamp_millis(),
        });
        match result {
            Ok(Some(result)) => {
                let request = &result.capture.request;
                let resolution = ProposalRefinementCaptureResolution {
                    request_id: request.id.clone(),
                    parent_proposal_id: request.proposal_id.clone(),
                    child_proposal_id: result.capture.child_proposal.id.clone(),
                    language: request.language.clone(),
                    replayed: result.replayed,
                };
                self.audit_log.record_or_alert(
                    &message.actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow refinement request={} parent={} child={} message={} replayed={}",
                        resolution.request_id,
                        resolution.parent_proposal_id,
                        resolution.child_proposal_id,
                        message.message_id,
                        resolution.replayed
                    ),
                    "accepted",
                );
                Ok(Some(resolution))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                self.audit_log.record_or_alert(
                    &message.actor,
                    AuditAction::LearningDecision,
                    format!(
                        "workflow refinement conversation={} message={}",
                        message.conversation_key, message.message_id
                    ),
                    format!("rejected: {error}"),
                );
                Err(error.to_string())
            }
        }
    }
}

fn project_workflow_learning_status(
    enabled: bool,
    mode: captain_types::config::LearningMode,
    expected_model: WorkflowLearningModelIdentity,
    snapshot: WorkflowLearningOperationalSnapshot,
    now_unix_ms: i64,
) -> WorkflowLearningStatus {
    let worker = snapshot.worker.as_ref().map(|heartbeat| {
        let heartbeat_age_ms = now_unix_ms
            .saturating_sub(heartbeat.heartbeat_at_unix_ms)
            .max(0) as u64;
        WorkflowLearningWorkerView {
            phase: heartbeat.phase,
            bound_model: heartbeat.bound_model.clone(),
            started_at_unix_ms: heartbeat.started_at_unix_ms,
            heartbeat_at_unix_ms: heartbeat.heartbeat_at_unix_ms,
            heartbeat_age_ms,
            last_scan_at_unix_ms: heartbeat.last_scan_at_unix_ms,
            last_progress_at_unix_ms: heartbeat.last_progress_at_unix_ms,
            last_error_scope: heartbeat.last_error_scope.clone(),
        }
    });
    let state =
        workflow_learning_runtime_state(enabled, &expected_model, worker.as_ref(), &snapshot);
    let recovery = match state {
        WorkflowLearningRuntimeState::Disabled => WorkflowLearningRecoveryState::Disabled,
        WorkflowLearningRuntimeState::Starting => WorkflowLearningRecoveryState::Starting,
        WorkflowLearningRuntimeState::Recovering => {
            WorkflowLearningRecoveryState::AutomaticRetryActive
        }
        WorkflowLearningRuntimeState::Degraded | WorkflowLearningRuntimeState::Stalled => {
            WorkflowLearningRecoveryState::OperatorAttention
        }
        WorkflowLearningRuntimeState::Healthy | WorkflowLearningRuntimeState::Active => {
            WorkflowLearningRecoveryState::InSync
        }
    };
    WorkflowLearningStatus {
        schema_version: WORKFLOW_LEARNING_STATUS_SCHEMA_VERSION,
        enabled,
        mode,
        state,
        recovery,
        expected_model,
        worker,
        jobs: snapshot.jobs,
        notifications: snapshot.notifications,
        workflows: snapshot.workflows,
        attention: snapshot.attention,
        generated_at_unix_ms: now_unix_ms,
    }
}

fn public_job_stage(
    kind: captain_memory::workflow_learning_queue::WorkflowJobKind,
) -> WorkflowLearningJobStage {
    use captain_memory::workflow_learning_queue::WorkflowJobKind;
    match kind {
        WorkflowJobKind::Analyze => WorkflowLearningJobStage::Analyze,
        WorkflowJobKind::Draft => WorkflowLearningJobStage::Draft,
        WorkflowJobKind::Validate => WorkflowLearningJobStage::Validate,
        WorkflowJobKind::Install => WorkflowLearningJobStage::Install,
        WorkflowJobKind::Canary => WorkflowLearningJobStage::Canary,
        WorkflowJobKind::Rollback => WorkflowLearningJobStage::Rollback,
    }
}

fn workflow_learning_runtime_state(
    enabled: bool,
    expected_model: &WorkflowLearningModelIdentity,
    worker: Option<&WorkflowLearningWorkerView>,
    snapshot: &WorkflowLearningOperationalSnapshot,
) -> WorkflowLearningRuntimeState {
    if !enabled {
        return WorkflowLearningRuntimeState::Disabled;
    }
    let Some(worker) = worker else {
        return WorkflowLearningRuntimeState::Starting;
    };
    if worker.heartbeat_age_ms > WORKFLOW_LEARNING_WORKER_STALE_AFTER_MS {
        return WorkflowLearningRuntimeState::Stalled;
    }
    if worker.phase == WorkflowLearningWorkerPhase::Degraded
        || worker.last_error_scope.is_some()
        || snapshot.jobs.uncertain > 0
        || snapshot.jobs.dead > 0
        || snapshot.notifications.dead > 0
    {
        return WorkflowLearningRuntimeState::Degraded;
    }
    if worker.phase == WorkflowLearningWorkerPhase::Starting
        || worker.bound_model.as_ref() != Some(expected_model)
    {
        return WorkflowLearningRuntimeState::Starting;
    }
    if snapshot.jobs.retry_wait > 0 || snapshot.notifications.retry_wait > 0 {
        return WorkflowLearningRuntimeState::Recovering;
    }
    if snapshot.jobs.pending > 0
        || snapshot.jobs.running > 0
        || snapshot.notifications.pending > 0
        || snapshot.notifications.delivering > 0
        || snapshot.workflows.processing > 0
    {
        return WorkflowLearningRuntimeState::Active;
    }
    WorkflowLearningRuntimeState::Healthy
}

fn authenticated_telegram_actor(actor: &str) -> bool {
    let Some(user_id) = actor.strip_prefix("telegram:") else {
        return false;
    };
    !user_id.is_empty() && user_id.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_telegram_context(context: &ProposalOperatorContext) -> bool {
    context.surface == "telegram"
        && context.conversation_key.starts_with("telegram:chat:")
        && !context.language.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::{
        authenticated_telegram_actor, project_workflow_learning_status, valid_telegram_context,
    };
    use captain_memory::workflow_learning_status::{
        WorkflowLearningOperationalSnapshot, WorkflowLearningWorkerHeartbeat,
    };
    use captain_types::config::LearningMode;
    use captain_types::workflow_learning::{
        ProposalOperatorContext, WorkflowLearningJobQueueView, WorkflowLearningModelIdentity,
        WorkflowLearningNotificationQueueView, WorkflowLearningRecoveryState,
        WorkflowLearningRuntimeState, WorkflowLearningWorkerPhase, WorkflowLearningWorkloadView,
    };

    #[test]
    fn telegram_operator_identity_requires_a_numeric_user_id() {
        assert!(authenticated_telegram_actor("telegram:42"));
        assert!(!authenticated_telegram_actor("telegram:unknown"));
        assert!(!authenticated_telegram_actor("telegram:"));
        assert!(!authenticated_telegram_actor("web:42"));
    }

    #[test]
    fn telegram_operator_context_is_bound_to_one_conversation() {
        assert!(valid_telegram_context(&ProposalOperatorContext {
            surface: "telegram".to_string(),
            conversation_key: "telegram:chat:-1001:thread:root".to_string(),
            source_message_id: Some("42".to_string()),
            language: "fr".to_string(),
        }));
        assert!(!valid_telegram_context(&ProposalOperatorContext {
            surface: "web".to_string(),
            conversation_key: "telegram:chat:-1001:thread:root".to_string(),
            source_message_id: None,
            language: "fr".to_string(),
        }));
    }

    fn snapshot() -> WorkflowLearningOperationalSnapshot {
        WorkflowLearningOperationalSnapshot {
            worker: Some(WorkflowLearningWorkerHeartbeat {
                worker_id: "worker:test".to_string(),
                phase: WorkflowLearningWorkerPhase::Running,
                bound_model: Some(WorkflowLearningModelIdentity {
                    provider: "codex".to_string(),
                    model: "gpt-5.6-sol".to_string(),
                }),
                started_at_unix_ms: 1_000,
                heartbeat_at_unix_ms: 10_000,
                last_scan_at_unix_ms: Some(9_000),
                last_progress_at_unix_ms: None,
                last_error_scope: None,
            }),
            jobs: WorkflowLearningJobQueueView::default(),
            notifications: WorkflowLearningNotificationQueueView::default(),
            workflows: WorkflowLearningWorkloadView::default(),
            attention: Vec::new(),
        }
    }

    #[test]
    fn status_distinguishes_healthy_retry_and_stalled_without_fake_progress() {
        let model = WorkflowLearningModelIdentity {
            provider: "codex".to_string(),
            model: "gpt-5.6-sol".to_string(),
        };
        let healthy = project_workflow_learning_status(
            true,
            LearningMode::Approval,
            model.clone(),
            snapshot(),
            10_100,
        );
        assert_eq!(healthy.state, WorkflowLearningRuntimeState::Healthy);
        assert_eq!(healthy.recovery, WorkflowLearningRecoveryState::InSync);
        assert_eq!(healthy.worker.unwrap().heartbeat_age_ms, 100);

        let mut retrying_snapshot = snapshot();
        retrying_snapshot.jobs.retry_wait = 1;
        let retrying = project_workflow_learning_status(
            true,
            LearningMode::Approval,
            model.clone(),
            retrying_snapshot,
            10_100,
        );
        assert_eq!(retrying.state, WorkflowLearningRuntimeState::Recovering);
        assert_eq!(
            retrying.recovery,
            WorkflowLearningRecoveryState::AutomaticRetryActive
        );

        let stalled = project_workflow_learning_status(
            true,
            LearningMode::Approval,
            model,
            snapshot(),
            100_001,
        );
        assert_eq!(stalled.state, WorkflowLearningRuntimeState::Stalled);
        assert_eq!(
            stalled.recovery,
            WorkflowLearningRecoveryState::OperatorAttention
        );

        let mut model_failure = snapshot();
        let worker = model_failure.worker.as_mut().unwrap();
        worker.phase = WorkflowLearningWorkerPhase::Degraded;
        worker.bound_model = None;
        worker.last_error_scope = Some("model".to_string());
        let degraded = project_workflow_learning_status(
            true,
            LearningMode::Approval,
            WorkflowLearningModelIdentity {
                provider: "codex".to_string(),
                model: "gpt-5.6-sol".to_string(),
            },
            model_failure,
            10_100,
        );
        assert_eq!(degraded.state, WorkflowLearningRuntimeState::Degraded);
    }
}

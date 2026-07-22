//! Authenticated operator decisions for Skill Learning V2.

use captain_channels::telegram::parse_workflow_learning_callback;
use captain_memory::workflow_learning_control::WorkflowLearningStore;
use captain_runtime::audit::AuditAction;
use captain_runtime::workflow_learning_operator::WorkflowLearningOperator;
use captain_runtime::workflow_learning_projection::project_workflow_learning_list;
use captain_runtime::workflow_learning_refinement::{
    WorkflowRefinementCaptureInput, WorkflowRefinementCoordinator,
};
use captain_runtime::workflow_learning_staging::WorkflowStagingRoot;
use captain_types::workflow_learning::{
    ProposalCardAction, ProposalOperatorContext, ProposalOperatorResolution,
    ProposalRefinementCaptureResolution, ProposalRefinementMessage, WorkflowLearningList,
};

use super::CaptainKernel;

impl CaptainKernel {
    pub fn workflow_learning_list(&self, limit: usize) -> Result<WorkflowLearningList, String> {
        let control = WorkflowLearningStore::new(self.memory.usage_conn());
        let staging = WorkflowStagingRoot::new(self.config.home_dir.clone())
            .map_err(|error| error.to_string())?;
        project_workflow_learning_list(&control, &staging, limit).map_err(|error| error.to_string())
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
            self.audit_log.record(
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
                self.audit_log.record(
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
                self.audit_log.record(
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
            self.audit_log.record(
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
            self.audit_log.record(
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
                self.audit_log.record(
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
                self.audit_log.record(
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
                self.audit_log.record(
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
            self.audit_log.record(
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
            self.audit_log.record(
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
                self.audit_log.record(
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
                self.audit_log.record(
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
    use super::{authenticated_telegram_actor, valid_telegram_context};
    use captain_types::workflow_learning::ProposalOperatorContext;

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
}

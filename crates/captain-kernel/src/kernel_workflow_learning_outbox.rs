//! Model-independent delivery worker for Skill Learning V2 operator cards.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use captain_channels::telegram::{
    build_workflow_learning_keyboard, format_workflow_isolated_test_result,
    format_workflow_learning_card, format_workflow_lifecycle_card,
};
use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_memory::workflow_learning_control::WorkflowLearningStore;
use captain_runtime::workflow_learning_delivery::{
    WorkflowDeliveryDisposition, WorkflowDeliveryEvent, WorkflowDeliveryPlanner,
    WorkflowProposalDelivery,
};
use captain_runtime::workflow_learning_staging::WorkflowStagingRoot;
use serde::Serialize;
use tracing::{info, warn};

use super::CaptainKernel;

const IDLE_DELAY: Duration = Duration::from_secs(2);
const TARGET_DELAY: Duration = Duration::from_secs(15);
const ACTIVE_DELAY: Duration = Duration::from_millis(25);
const ERROR_DELAY: Duration = Duration::from_secs(10);
const DELIVERY_LEASE_MS: i64 = 120_000;

pub(super) fn spawn_workflow_learning_outbox_worker(kernel: Arc<CaptainKernel>) {
    if !super::kernel_workflow_learning_worker::workflow_learning_enabled(
        kernel.config.skills.enabled,
        kernel.config.skills.mode,
    ) {
        return;
    }
    tokio::spawn(run_workflow_learning_outbox_worker(kernel));
}

async fn run_workflow_learning_outbox_worker(kernel: Arc<CaptainKernel>) {
    let staging = match WorkflowStagingRoot::new(kernel.config.home_dir.clone()) {
        Ok(staging) => staging,
        Err(error) => {
            warn!(error = %error, "workflow learning notification staging is unavailable");
            return;
        }
    };
    let planner = match WorkflowDeliveryPlanner::new(
        WorkflowLearningStore::new(kernel.memory.usage_conn()),
        staging,
        format!("captain:workflow-delivery:{}", std::process::id()),
        DELIVERY_LEASE_MS,
    ) {
        Ok(planner) => planner,
        Err(error) => {
            warn!(error = %error, "workflow learning notification worker cannot start");
            return;
        }
    };
    let mut target_was_ready = false;
    let mut last_error = None::<String>;

    loop {
        if kernel.supervisor.is_shutting_down() {
            break;
        }
        let Some((recipient, adapter)) = telegram_target(&kernel) else {
            if target_was_ready {
                info!("workflow learning notifications paused until Telegram is ready");
            }
            target_was_ready = false;
            tokio::time::sleep(TARGET_DELAY).await;
            continue;
        };
        if !target_was_ready {
            info!("workflow learning Telegram notification worker ready");
            target_was_ready = true;
        }

        let now_unix_ms = chrono::Utc::now().timestamp_millis();
        let delay = match planner.claim_next(now_unix_ms) {
            Ok(WorkflowDeliveryDisposition::Idle) => IDLE_DELAY,
            Ok(WorkflowDeliveryDisposition::Suppressed {
                outbox_id,
                proposal_id,
                proposal_state,
            }) => {
                clear_error(&mut last_error);
                info!(
                    outbox_id,
                    proposal_id, proposal_state, "stale workflow learning notification suppressed"
                );
                ACTIVE_DELAY
            }
            Ok(WorkflowDeliveryDisposition::DeadLettered { outbox_id, reason }) => {
                clear_error(&mut last_error);
                warn!(
                    outbox_id,
                    reason, "workflow learning notification dead-lettered"
                );
                ACTIVE_DELAY
            }
            Ok(WorkflowDeliveryDisposition::Ready(delivery)) => {
                match send_workflow_proposal_notification(
                    adapter,
                    &recipient,
                    &kernel.config.language,
                    &delivery,
                )
                .await
                {
                    Ok(receipt) => {
                        let completed_at = chrono::Utc::now().timestamp_millis();
                        match serde_json::to_string(&receipt)
                            .map_err(|error| error.to_string())
                            .and_then(|receipt_json| {
                                planner
                                    .complete(&delivery, &receipt_json, completed_at)
                                    .map_err(|error| error.to_string())
                            }) {
                            Ok(_) => {
                                clear_error(&mut last_error);
                                info!(
                                    outbox_id = delivery.outbox.id,
                                    proposal_id = delivery.outbox.proposal_id,
                                    external_message_id = ?receipt.external_message_id,
                                    "workflow learning proposal delivered to Telegram"
                                );
                                ACTIVE_DELAY
                            }
                            Err(error) => {
                                report_error(
                                    &mut last_error,
                                    format!(
                                        "Telegram accepted outbox {} but its receipt could not be persisted: {error}",
                                        delivery.outbox.id
                                    ),
                                );
                                ERROR_DELAY
                            }
                        }
                    }
                    Err(error) => {
                        let failed_at = chrono::Utc::now().timestamp_millis();
                        match planner.retry(&delivery, &error, failed_at) {
                            Ok(item) => report_error(
                                &mut last_error,
                                format!(
                                    "outbox {} delivery failed; status={} attempt={}/{}: {error}",
                                    item.id,
                                    item.status.as_str(),
                                    item.attempt_count,
                                    item.max_attempts
                                ),
                            ),
                            Err(settle_error) => report_error(
                                &mut last_error,
                                format!(
                                    "outbox {} delivery and retry settlement failed: {error}; {settle_error}",
                                    delivery.outbox.id
                                ),
                            ),
                        }
                        ERROR_DELAY
                    }
                }
            }
            Err(error) => {
                report_error(&mut last_error, error.to_string());
                ERROR_DELAY
            }
        };
        tokio::time::sleep(delay).await;
    }
}

fn telegram_target(kernel: &CaptainKernel) -> Option<(String, Arc<dyn ChannelAdapter>)> {
    let recipient = kernel
        .config
        .channels
        .telegram
        .as_ref()?
        .default_chat_id
        .as_deref()?
        .trim();
    if recipient.is_empty() {
        return None;
    }
    let adapter = kernel.channel_adapters.get("telegram")?;
    Some((recipient.to_string(), Arc::clone(adapter.value())))
}

async fn send_workflow_proposal_notification(
    adapter: Arc<dyn ChannelAdapter>,
    recipient: &str,
    language: &str,
    delivery: &WorkflowProposalDelivery,
) -> Result<WorkflowTelegramDeliveryReceipt, String> {
    let user = ChannelUser {
        platform_id: recipient.to_string(),
        display_name: "Captain operator".to_string(),
        captain_user: None,
    };
    let text = match &delivery.event {
        WorkflowDeliveryEvent::Proposed => format_workflow_learning_card(&delivery.card, language),
        WorkflowDeliveryEvent::IsolatedTestCompleted { passed } => {
            format_workflow_isolated_test_result(&delivery.card, *passed, language)
        }
        WorkflowDeliveryEvent::Lifecycle(lifecycle) => {
            format_workflow_lifecycle_card(lifecycle, language)
        }
    };
    let content = ChannelContent::Text(text);
    let mut metadata = HashMap::new();
    metadata.insert(
        "reply_markup".to_string(),
        build_workflow_learning_keyboard(&delivery.card, language),
    );
    let external_message_id = adapter
        .send_rich(&user, content, &metadata)
        .await
        .map_err(|error| error.to_string())?
        .map(|message_id| bounded_external_id(&message_id));
    Ok(WorkflowTelegramDeliveryReceipt {
        schema_version: 1,
        status: "sent",
        channel: "telegram",
        external_message_id,
        idempotency_key: delivery.outbox.idempotency_key.clone(),
        delivery_semantics: "at_least_once",
    })
}

#[derive(Debug, Serialize)]
struct WorkflowTelegramDeliveryReceipt {
    schema_version: u16,
    status: &'static str,
    channel: &'static str,
    external_message_id: Option<String>,
    idempotency_key: String,
    delivery_semantics: &'static str,
}

fn bounded_external_id(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

fn report_error(last_error: &mut Option<String>, error: String) {
    if last_error.as_ref() != Some(&error) {
        warn!(error, "workflow learning notification worker error");
        *last_error = Some(error);
    }
}

fn clear_error(last_error: &mut Option<String>) {
    if last_error.take().is_some() {
        info!("workflow learning notification worker recovered");
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use captain_channels::types::{ChannelMessage, ChannelStatus, ChannelType, LifecycleReaction};
    use captain_memory::workflow_learning_outbox::{WorkflowOutboxRecord, WorkflowOutboxStatus};
    use captain_types::workflow_learning::{
        ProposalCard, ProposalCardAction, ProposalCardEvidence, ProposalCardKind,
        ProposalCardModel, ProposalCardRisk, ProposalCardState, ProposalCardStep,
        ProposalCardValidationFact, WorkflowLifecycleCard, WorkflowLifecycleEvent,
        PROPOSAL_CARD_SCHEMA_VERSION, WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION,
    };
    use futures::{stream, Stream};

    use super::*;

    #[derive(Default)]
    struct RecordingAdapter {
        deliveries: Mutex<
            Vec<(
                ChannelUser,
                ChannelContent,
                HashMap<String, serde_json::Value>,
            )>,
        >,
    }

    #[async_trait]
    impl ChannelAdapter for RecordingAdapter {
        fn name(&self) -> &str {
            "recording-telegram"
        }

        fn channel_type(&self) -> ChannelType {
            ChannelType::Telegram
        }

        async fn start(
            &self,
        ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
        {
            Ok(Box::pin(stream::empty()))
        }

        async fn send(
            &self,
            _user: &ChannelUser,
            _content: ChannelContent,
        ) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        async fn send_rich(
            &self,
            user: &ChannelUser,
            content: ChannelContent,
            metadata: &HashMap<String, serde_json::Value>,
        ) -> Result<Option<String>, Box<dyn std::error::Error>> {
            self.deliveries
                .lock()
                .unwrap()
                .push((user.clone(), content, metadata.clone()));
            Ok(Some("telegram-message-42".to_string()))
        }

        async fn send_reaction(
            &self,
            _user: &ChannelUser,
            _message_id: &str,
            _reaction: &LifecycleReaction,
        ) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
            Ok(())
        }

        fn status(&self) -> ChannelStatus {
            ChannelStatus {
                connected: true,
                ..ChannelStatus::default()
            }
        }
    }

    #[tokio::test]
    async fn delivery_uses_the_shared_rich_card_and_exact_callback_keyboard() {
        let adapter = Arc::new(RecordingAdapter::default());
        let delivery = proposal_delivery();

        let receipt =
            send_workflow_proposal_notification(adapter.clone(), "12345", "fr-FR", &delivery)
                .await
                .unwrap();

        assert_eq!(
            receipt.external_message_id.as_deref(),
            Some("telegram-message-42")
        );
        assert_eq!(receipt.idempotency_key, "proposal-1:proposed-notification");
        let sent = adapter.deliveries.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0.platform_id, "12345");
        let ChannelContent::Text(text) = &sent[0].1 else {
            panic!("expected a rich text proposal");
        };
        assert!(text.contains("Nouvelle capacite proposee"));
        let callbacks = sent[0].2["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();
        assert!(callbacks.contains(&"workflow:activate:00000000000000000000:4"));
        assert!(callbacks.contains(&"workflow:details:00000000000000000000:4"));
        assert!(callbacks.contains(&"workflow:ignore:00000000000000000000:4"));
    }

    #[tokio::test]
    async fn lifecycle_delivery_uses_rich_status_without_stale_decision_buttons() {
        let adapter = Arc::new(RecordingAdapter::default());
        let mut delivery = proposal_delivery();
        delivery.card.state = ProposalCardState::Active;
        delivery.card.decision_version = 7;
        delivery.card.available_actions = vec![ProposalCardAction::Details];
        delivery.event = WorkflowDeliveryEvent::Lifecycle(WorkflowLifecycleCard {
            schema_version: WORKFLOW_LIFECYCLE_CARD_SCHEMA_VERSION,
            event: WorkflowLifecycleEvent::ActivationCompleted,
            proposal_id: delivery.card.proposal_id.clone(),
            revision_sha256: delivery.card.revision_sha256.clone(),
            decision_version: 7,
            state: ProposalCardState::Active,
            kind: ProposalCardKind::Skill,
            name: delivery.card.name.clone(),
            lifecycle_job_id: "canary-1".to_string(),
            continuation_job_id: None,
            target_locator: Some("skills/learned/sourced-research.md".to_string()),
            failure_code: None,
            failure_message: None,
            rollback_job_id: None,
            occurred_at_unix_ms: 1_750_000_000_000,
        });

        send_workflow_proposal_notification(adapter.clone(), "12345", "en", &delivery)
            .await
            .unwrap();

        let sent = adapter.deliveries.lock().unwrap();
        let ChannelContent::Text(text) = &sent[0].1 else {
            panic!("expected a rich lifecycle text");
        };
        assert!(text.contains("Native capability active"));
        let rows = sent[0].2["reply_markup"]["inline_keyboard"]
            .as_array()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0]["text"], "Details");
        assert_eq!(
            rows[0][0]["callback_data"],
            "workflow:details:00000000000000000000:7"
        );
    }

    fn proposal_delivery() -> WorkflowProposalDelivery {
        WorkflowProposalDelivery {
            outbox: WorkflowOutboxRecord {
                id: "proposal-1-proposed".to_string(),
                idempotency_key: "proposal-1:proposed-notification".to_string(),
                proposal_id: "proposal-1".to_string(),
                revision_sha256: Some("a".repeat(64)),
                topic: "workflow_learning.proposed".to_string(),
                payload_json: "{}".to_string(),
                status: WorkflowOutboxStatus::Delivering,
                attempt_count: 1,
                max_attempts: 8,
                run_after_unix_ms: 1_000,
                lease_owner: Some("worker".to_string()),
                lease_expires_at_unix_ms: Some(121_000),
                delivery_result_json: None,
                last_error: None,
                delivered_at_unix_ms: None,
                created_at_unix_ms: 1_000,
                updated_at_unix_ms: 1_000,
            },
            card: ProposalCard {
                schema_version: PROPOSAL_CARD_SCHEMA_VERSION,
                proposal_id: "proposal-1".to_string(),
                lookup_token: "00000000000000000000".to_string(),
                decision_version: 4,
                revision_sha256: "a".repeat(64),
                state: ProposalCardState::Proposed,
                kind: ProposalCardKind::Skill,
                name: "sourced-research".to_string(),
                purpose: "Research current sources safely.".to_string(),
                trigger: "A current answer needs sources.".to_string(),
                evidence: ProposalCardEvidence {
                    occurrences: 3,
                    distinct_turns: 3,
                    distinct_sessions: 2,
                    explicit_reuse_request: false,
                },
                steps: vec![ProposalCardStep {
                    index: 0,
                    tool_name: "web_search".to_string(),
                    role: "research".to_string(),
                    dependencies: vec![],
                }],
                validation: vec![ProposalCardValidationFact {
                    code: "secret_scan".to_string(),
                    passed: true,
                }],
                validation_limitations: vec!["Review high-stakes claims.".to_string()],
                isolated_test: None,
                validated_by: ProposalCardModel {
                    provider: "codex".to_string(),
                    model: "gpt-5.6-sol".to_string(),
                },
                required_authority: vec!["web_search".to_string()],
                expected_benefit: "Repeatable sourced answers.".to_string(),
                risk: ProposalCardRisk::ReadOnly,
                recommended_action: ProposalCardAction::Activate,
                available_actions: vec![
                    ProposalCardAction::Activate,
                    ProposalCardAction::Details,
                    ProposalCardAction::Edit,
                    ProposalCardAction::Later,
                    ProposalCardAction::Ignore,
                ],
            },
            event: WorkflowDeliveryEvent::Proposed,
        }
    }
}

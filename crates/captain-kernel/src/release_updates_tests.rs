use std::collections::HashMap;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use captain_channels::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
    LifecycleReaction,
};
use captain_types::release_update::{
    ReleaseAsset, ReleaseDescriptor, RuntimeUpdateAttemptResult, RuntimeUpdateAttemptStatus,
    RuntimeUpdateNoticeKind, RuntimeUpdateResolutionStatus, RuntimeUpdateTelegramAction,
    RUNTIME_UPDATE_RESULT_FILENAME, RUNTIME_UPDATE_RESULT_SCHEMA_VERSION,
};
use futures::{stream, Stream};

use super::*;

fn release(tag: &str) -> ReleaseDescriptor {
    ReleaseDescriptor {
        tag_name: tag.to_string(),
        html_url: format!("https://example.test/{tag}"),
        draft: false,
        prerelease: tag.contains('-'),
        published_at: Some("2026-07-20T08:00:00Z".to_string()),
        assets: vec![ReleaseAsset {
            name: "captain-aarch64-apple-darwin.tar.gz".to_string(),
        }],
    }
}

fn state_with_alpha9(now: i64) -> RuntimeUpdateState {
    let mut state = RuntimeUpdateState::default();
    reconcile_scan_success(
        &mut state,
        "0.1.0-alpha.8",
        Some(&release("v0.1.0-alpha.9")),
        RuntimeUpdateInstallMode::SelfUpdate,
        now,
    );
    state
}

fn callback(
    state: &RuntimeUpdateState,
    action: RuntimeUpdateTelegramAction,
) -> captain_channels::telegram::RuntimeUpdateTelegramCallback {
    let candidate = state.pending.as_ref().unwrap();
    captain_channels::telegram::RuntimeUpdateTelegramCallback {
        action,
        token: candidate.token.clone(),
        decision_version: candidate.decision_version,
    }
}

#[test]
fn successful_scan_is_twelve_hourly_and_enqueues_once() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    assert_eq!(state.next_check_at_unix_ms, now + CHECK_INTERVAL_MS);
    assert_eq!(state.outbox.len(), 1);
    assert_eq!(
        state.outbox[0].card.notice,
        RuntimeUpdateNoticeKind::Available
    );

    reconcile_scan_success(
        &mut state,
        "0.1.0-alpha.8",
        Some(&release("v0.1.0-alpha.9")),
        RuntimeUpdateInstallMode::SelfUpdate,
        now + CHECK_INTERVAL_MS,
    );
    assert_eq!(state.outbox.len(), 1);
    assert_eq!(state.pending.unwrap().available_version, "0.1.0-alpha.9");
}

#[test]
fn twelve_hour_scan_reopens_a_dead_telegram_delivery() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let old_decision_version = state.pending.as_ref().unwrap().decision_version;
    state.outbox[0].status = RuntimeUpdateOutboxStatus::Dead;

    reconcile_scan_success(
        &mut state,
        "0.1.0-alpha.8",
        Some(&release("v0.1.0-alpha.9")),
        RuntimeUpdateInstallMode::SelfUpdate,
        now + CHECK_INTERVAL_MS,
    );

    assert!(state.pending.as_ref().unwrap().decision_version > old_decision_version);
    let reopened = state
        .outbox
        .iter()
        .find(|item| item.status == RuntimeUpdateOutboxStatus::Pending)
        .unwrap();
    assert_eq!(reopened.max_attempts, OUTBOX_MAX_ATTEMPTS);
    assert!(reopened.card.detail.is_none());
}

#[test]
fn failed_checks_back_off_without_erasing_a_pending_decision() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    reconcile_scan_failure(&mut state, "offline", now + 1);
    assert_eq!(
        state.next_check_at_unix_ms,
        now + 1 + FIRST_FAILURE_RETRY_MS
    );
    assert_eq!(
        state.pending.as_ref().unwrap().available_version,
        "0.1.0-alpha.9"
    );
    reconcile_scan_failure(&mut state, "still offline", now + 2);
    assert_eq!(
        state.next_check_at_unix_ms,
        now + 2 + 2 * FIRST_FAILURE_RETRY_MS
    );
    assert_eq!(state.consecutive_failures, 2);
}

#[test]
fn defer_retires_old_card_and_releases_exact_reminder_after_24_hours() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let stale = callback(&state, RuntimeUpdateTelegramAction::Install);
    let defer = callback(&state, RuntimeUpdateTelegramAction::Defer);
    let effect = apply_operator_decision(
        &mut state,
        &defer,
        "telegram:42",
        Path::new("/tmp/captain-release-update-tests"),
        now + 1,
    )
    .unwrap();
    let OperatorDecisionEffect::Resolved(resolution) = effect else {
        panic!("defer must not launch a process");
    };
    assert_eq!(resolution.status, RuntimeUpdateResolutionStatus::Deferred);
    let due = now + 1 + DEFER_INTERVAL_MS;
    assert!(claim_outbox(&mut state, "worker", due - 1).is_none());
    let reminder = claim_outbox(&mut state, "worker", due).unwrap();
    assert_eq!(reminder.card.notice, RuntimeUpdateNoticeKind::Reminder);
    assert!(apply_operator_decision(
        &mut state,
        &stale,
        "telegram:42",
        Path::new("/tmp/captain-release-update-tests"),
        due,
    )
    .unwrap_err()
    .contains("périmée"));
}

#[test]
fn refusal_suppresses_only_the_exact_version() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let refuse = callback(&state, RuntimeUpdateTelegramAction::Refuse);
    let effect = apply_operator_decision(
        &mut state,
        &refuse,
        "telegram:42",
        Path::new("/tmp/captain-release-update-tests"),
        now + 1,
    )
    .unwrap();
    assert!(matches!(effect, OperatorDecisionEffect::Resolved(_)));
    assert!(state.pending.is_none());

    reconcile_scan_success(
        &mut state,
        "0.1.0-alpha.8",
        Some(&release("v0.1.0-alpha.9")),
        RuntimeUpdateInstallMode::SelfUpdate,
        now + CHECK_INTERVAL_MS,
    );
    assert!(state.pending.is_none());

    reconcile_scan_success(
        &mut state,
        "0.1.0-alpha.8",
        Some(&release("v0.1.0-alpha.10")),
        RuntimeUpdateInstallMode::SelfUpdate,
        now + 2 * CHECK_INTERVAL_MS,
    );
    assert_eq!(state.pending.unwrap().available_version, "0.1.0-alpha.10");
}

#[test]
fn expired_delivery_lease_is_reclaimed_after_restart() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let first = claim_outbox(&mut state, "worker-a", now).unwrap();
    assert_eq!(first.attempt_count, 1);
    assert!(claim_outbox(&mut state, "worker-b", now + OUTBOX_LEASE_MS - 1).is_none());

    let replay = claim_outbox(&mut state, "worker-b", now + OUTBOX_LEASE_MS).unwrap();
    assert_eq!(replay.id, first.id);
    assert_eq!(replay.attempt_count, 2);
    assert_eq!(replay.lease_owner.as_deref(), Some("worker-b"));
}

#[test]
fn delivering_state_survives_serialization_and_is_reclaimed() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let first = claim_outbox(&mut state, "worker-a", now).unwrap();
    let serialized = serde_json::to_vec(&state).unwrap();
    let mut reopened: RuntimeUpdateState = serde_json::from_slice(&serialized).unwrap();

    let replay = claim_outbox(&mut reopened, "worker-b", now + OUTBOX_LEASE_MS).unwrap();
    assert_eq!(replay.id, first.id);
    assert_eq!(replay.attempt_count, 2);
}

#[test]
fn container_install_is_explicitly_manual_and_remains_observable() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    state.pending.as_mut().unwrap().install_mode = RuntimeUpdateInstallMode::Container;
    let install = callback(&state, RuntimeUpdateTelegramAction::Install);
    let effect = apply_operator_decision(
        &mut state,
        &install,
        "telegram:42",
        Path::new("/tmp/captain-release-update-tests"),
        now + 1,
    )
    .unwrap();
    let OperatorDecisionEffect::Resolved(resolution) = effect else {
        panic!("container install must stay orchestrator-owned");
    };
    assert_eq!(
        resolution.status,
        RuntimeUpdateResolutionStatus::ContainerManual
    );
    assert!(state.pending.is_some());
    assert!(!decision_suppresses_version(&state, "0.1.0-alpha.9"));
    let due = now + 1 + DEFER_INTERVAL_MS;
    assert_eq!(
        resolution.next_prompt_at.as_deref(),
        Some(format_unix_ms(due).as_str())
    );
    assert_eq!(
        claim_outbox(&mut state, "worker", due).unwrap().card.notice,
        RuntimeUpdateNoticeKind::Reminder
    );
}

#[test]
fn unsupported_platform_uses_manual_procedure_without_mutating_the_host() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    state.pending.as_mut().unwrap().install_mode = RuntimeUpdateInstallMode::Manual;
    let install = callback(&state, RuntimeUpdateTelegramAction::Install);

    let effect = apply_operator_decision(
        &mut state,
        &install,
        "telegram:42",
        Path::new("/tmp/captain-release-update-tests"),
        now + 1,
    )
    .unwrap();

    let OperatorDecisionEffect::Resolved(resolution) = effect else {
        panic!("manual platform must not launch a process");
    };
    assert_eq!(
        resolution.status,
        RuntimeUpdateResolutionStatus::PlatformManual
    );
    assert!(state.active_attempt.is_none());
    assert!(state.pending.is_some());
}

#[test]
fn failed_detached_attempt_requeues_a_new_exact_decision() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let install = callback(&state, RuntimeUpdateTelegramAction::Install);
    let effect = apply_operator_decision(
        &mut state,
        &install,
        "telegram:42",
        Path::new("/tmp/captain-release-update-tests"),
        now + 1,
    )
    .unwrap();
    let OperatorDecisionEffect::Launch { attempt, .. } = effect else {
        panic!("host install must launch");
    };
    assert_eq!(attempt.requested_version, "v0.1.0-alpha.9");
    let old_version = state.pending.as_ref().unwrap().decision_version;
    let result = RuntimeUpdateAttemptResult {
        schema_version: RUNTIME_UPDATE_RESULT_SCHEMA_VERSION,
        attempt_id: attempt.attempt_id,
        requested_version: attempt.requested_version,
        status: RuntimeUpdateAttemptStatus::Failed,
        installed_version: None,
        message: "network interrupted".to_string(),
        completed_at: "2026-07-20T08:00:00Z".to_string(),
    };

    assert!(apply_attempt_result(
        &mut state,
        &result,
        "0.1.0-alpha.8",
        now + 2
    ));
    assert!(state.active_attempt.is_none());
    assert!(state.pending.as_ref().unwrap().decision_version > old_version);
    assert!(state
        .outbox
        .iter()
        .any(|item| item.card.notice == RuntimeUpdateNoticeKind::InstallFailed));
}

#[test]
fn orphaned_detached_attempt_recovers_after_bounded_timeout() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let install = callback(&state, RuntimeUpdateTelegramAction::Install);
    let effect = apply_operator_decision(
        &mut state,
        &install,
        "telegram:42",
        Path::new("/tmp/captain-release-update-tests"),
        now + 1,
    )
    .unwrap();
    assert!(matches!(effect, OperatorDecisionEffect::Launch { .. }));

    assert!(!expire_stale_update_attempt(
        &mut state,
        now + UPDATE_ATTEMPT_STALE_MS
    ));
    assert!(state.active_attempt.is_some());
    assert!(expire_stale_update_attempt(
        &mut state,
        now + UPDATE_ATTEMPT_STALE_MS + 1
    ));
    assert!(state.active_attempt.is_none());
    assert!(state
        .outbox
        .iter()
        .any(|item| item.card.notice == RuntimeUpdateNoticeKind::InstallFailed));
}

#[test]
fn successful_attempt_waits_for_restart_then_emits_terminal_card_once() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let install = callback(&state, RuntimeUpdateTelegramAction::Install);
    let effect = apply_operator_decision(
        &mut state,
        &install,
        "telegram:42",
        Path::new("/tmp/captain-release-update-tests"),
        now + 1,
    )
    .unwrap();
    let OperatorDecisionEffect::Launch { attempt, .. } = effect else {
        panic!("host install must launch");
    };
    let result = RuntimeUpdateAttemptResult {
        schema_version: RUNTIME_UPDATE_RESULT_SCHEMA_VERSION,
        attempt_id: attempt.attempt_id,
        requested_version: attempt.requested_version,
        status: RuntimeUpdateAttemptStatus::Succeeded,
        installed_version: Some("0.1.0-alpha.9".to_string()),
        message: "installed".to_string(),
        completed_at: "2026-07-20T08:00:00Z".to_string(),
    };

    assert!(!apply_attempt_result(
        &mut state,
        &result,
        "0.1.0-alpha.8",
        now + 2
    ));
    assert!(state.active_attempt.is_some());
    assert!(apply_attempt_result(
        &mut state,
        &result,
        "0.1.0-alpha.9",
        now + 3
    ));
    assert!(state.pending.is_none());
    assert!(state.active_attempt.is_none());
    assert_eq!(
        state
            .outbox
            .iter()
            .filter(|item| item.card.notice == RuntimeUpdateNoticeKind::Installed)
            .count(),
        1
    );
    assert!(apply_attempt_result(
        &mut state,
        &result,
        "0.1.0-alpha.9",
        now + 4
    ));
    assert_eq!(
        state
            .outbox
            .iter()
            .filter(|item| item.card.notice == RuntimeUpdateNoticeKind::Installed)
            .count(),
        1
    );
}

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
async fn delivery_uses_rich_markdown_and_exact_three_button_keyboard() {
    let now = 1_750_000_000_000;
    let mut state = state_with_alpha9(now);
    let delivery = claim_outbox(&mut state, "worker", now).unwrap();
    let adapter = Arc::new(RecordingAdapter::default());

    let receipt =
        delivery::send_runtime_update_notification(adapter.clone(), "12345", "fr-FR", &delivery)
            .await
            .unwrap();

    assert_eq!(receipt.as_deref(), Some("telegram-message-42"));
    let sent = adapter.deliveries.lock().unwrap();
    let ChannelContent::Text(text) = &sent[0].1 else {
        panic!("expected rich text");
    };
    assert!(text.contains("Mise à jour Captain disponible"));
    let callbacks = sent[0].2["reply_markup"]["inline_keyboard"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row.as_array().unwrap())
        .filter_map(|button| button["callback_data"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(callbacks.len(), 3);
    assert!(callbacks.iter().any(|data| data.contains(":install:")));
    assert!(callbacks.iter().any(|data| data.contains(":defer:")));
    assert!(callbacks.iter().any(|data| data.contains(":refuse:")));
}

#[test]
fn container_detection_is_fail_closed_and_testable() {
    let root = tempfile::tempdir().unwrap();
    let dockerenv = root.path().join(".dockerenv");
    let cgroup = root.path().join("cgroup");
    assert!(!container_marker(&dockerenv, &cgroup));
    std::fs::write(&cgroup, "0::/docker/abc\n").unwrap();
    assert!(container_marker(&dockerenv, &cgroup));
    assert_eq!(
        classify_install_mode(true, true),
        RuntimeUpdateInstallMode::Container
    );
    assert_eq!(
        classify_install_mode(false, false),
        RuntimeUpdateInstallMode::Manual
    );
    assert_eq!(
        classify_install_mode(false, true),
        RuntimeUpdateInstallMode::SelfUpdate
    );
}

#[test]
fn mirror_version_contract_is_small_utf8_and_non_empty() {
    assert_eq!(
        parse_mirror_version(b"v0.1.0-alpha.9\n", "mirror").unwrap(),
        "v0.1.0-alpha.9"
    );
    assert!(parse_mirror_version(&vec![b'a'; 257], "mirror").is_err());
    assert!(parse_mirror_version(&[0xff], "mirror").is_err());
    assert!(parse_mirror_version(b"  \n", "mirror").is_err());
}

#[test]
fn privileged_update_requires_exact_operator_and_exact_chat() {
    let telegram = captain_types::config::TelegramConfig {
        allowed_users: vec!["42".to_string()],
        default_chat_id: Some("-1001".to_string()),
        ..captain_types::config::TelegramConfig::default()
    };
    let context = RuntimeUpdateOperatorContext {
        chat_id: "-1001".to_string(),
        source_message_id: Some("99".to_string()),
    };
    assert!(authorize_runtime_update_identity(&telegram, "telegram:42", &context).is_ok());
    assert!(authorize_runtime_update_identity(&telegram, "telegram:7", &context).is_err());

    let mut wildcard = telegram.clone();
    wildcard.allowed_users = vec!["*".to_string()];
    assert!(
        authorize_runtime_update_identity(&wildcard, "telegram:42", &context)
            .unwrap_err()
            .contains("explicitement autorisé")
    );

    let wrong_chat = RuntimeUpdateOperatorContext {
        chat_id: "-1002".to_string(),
        source_message_id: None,
    };
    assert!(authorize_runtime_update_identity(&telegram, "telegram:42", &wrong_chat).is_err());
}

#[test]
fn malformed_attempt_result_is_quarantined_instead_of_blocking_boot_forever() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join(RUNTIME_UPDATE_RESULT_FILENAME);
    captain_types::durable_fs::atomic_write(&source, b"not-json").unwrap();

    let quarantined = quarantine_update_result(home.path(), &source).unwrap();

    assert!(!source.exists());
    assert_eq!(std::fs::read(quarantined).unwrap(), b"not-json");
}

#[test]
fn future_persisted_state_schema_is_rejected_without_downgrade() {
    let mut value = serde_json::to_value(RuntimeUpdateState::default()).unwrap();
    value["schema_version"] = serde_json::json!(STATE_SCHEMA_VERSION + 1);

    let error = decode_runtime_update_state(value).unwrap_err();

    assert!(error.contains("unsupported schema version"));
}

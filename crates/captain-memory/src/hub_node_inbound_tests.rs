use crate::hub_node_rail::{
    HubNodeInboundOutcome, HubNodeRailError, HubNodeRunApprovalStatus, HubNodeRunStatus,
    NewHubNodeRun,
};
use crate::MemorySubstrate;
use captain_types::approval::{approval_action_digest, ApprovalDecision, RiskLevel};
use captain_wire::hub_protocol::{
    CapabilityDescriptor, DeviceRole, HubNodeEnvelope, HubNodeMessage, LogicalWorkspace,
    NodeTransport, RunApprovalDecision, RunApprovalRequest, RunCompletion, RunEffect, RunRejection,
    RunTerminalStatus, HUB_NODE_PROTOCOL_VERSION,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn connected_memory() -> MemorySubstrate {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let conn = memory.usage_conn();
    conn.lock()
        .unwrap()
        .execute(
            "INSERT INTO captain_devices (
                 device_id, display_name, role, platform, captain_version,
                 protocol_major, protocol_minor, credential_sha256,
                 capabilities_json, grants_json, status, paired_at_ms,
                 last_seen_ms, updated_at_ms
             ) VALUES ('node-1', 'Node', 'node', 'macos', 'alpha.14',
                       1, 0, ?1, '{}', '{}', 'active', 1, 1, 1)",
            ["a".repeat(64)],
        )
        .unwrap();
    let hello = node_envelope(
        "connection-1",
        1,
        None,
        HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: capabilities(),
            resume_after_sequence: 0,
            active_run_ids: Vec::new(),
        },
    );
    memory
        .hub_node_rail()
        .open_connection(&hello, NodeTransport::LongPoll, 15_000, 60_000, 20)
        .unwrap();
    memory
}

fn capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "macos".to_string(),
        transports: vec![NodeTransport::LongPoll, NodeTransport::WebSocket],
        tool_families: vec!["shell".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "workspace-main".to_string(),
            label: "Main workspace".to_string(),
            read_only: false,
        }],
        supports_streaming_output: true,
    }
}

fn node_envelope(
    connection_id: &str,
    sequence: u64,
    ack_sequence: Option<u64>,
    message: HubNodeMessage,
) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-1".to_string(),
        connection_id: connection_id.to_string(),
        sequence,
        ack_sequence,
        sent_at_ms: 100 + sequence as i64,
        message,
    }
}

fn enqueue_and_lease(memory: &MemorySubstrate, effect: RunEffect) {
    memory
        .hub_node_rail()
        .enqueue_run(&NewHubNodeRun {
            run_id: "run-1".to_string(),
            device_id: "node-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            workspace_id: "workspace-main".to_string(),
            tool_name: "shell_exec".to_string(),
            input: json!({"command": "true"}),
            effect,
            created_at_ms: 30,
        })
        .unwrap();
    memory
        .hub_node_rail()
        .lease_next("node-1", "connection-1", 40, 60_000)
        .unwrap()
        .unwrap();
}

fn completion(status: RunTerminalStatus) -> RunCompletion {
    let result_content = "done";
    RunCompletion {
        run_id: "run-1".to_string(),
        attempt: 1,
        status,
        result_content: result_content.to_string(),
        result_sha256: format!("{:x}", Sha256::digest(result_content.as_bytes())),
        total_output_bytes: 4,
        stored_output_bytes: 4,
        capped: false,
        redacted: false,
        path_policy_applied: true,
    }
}

#[test]
fn accepted_progress_and_completion_commit_with_receipts_and_acks() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::LocalMutation);
    let store = memory.hub_node_rail();

    let accepted_envelope = node_envelope(
        "connection-1",
        2,
        Some(2),
        HubNodeMessage::RunAccepted {
            run_id: "run-1".to_string(),
            attempt: 1,
        },
    );
    let accepted = store
        .apply_node_envelope(&accepted_envelope, 60_000, 50)
        .unwrap();
    assert!(matches!(
        accepted.outcome,
        HubNodeInboundOutcome::RunAccepted(ref run)
            if run.status == HubNodeRunStatus::Accepted
    ));
    assert_eq!(accepted.acknowledged_node_sequence, 2);
    assert!(store.pending_outbox("node-1", 0, 10).unwrap().is_empty());

    let progress_envelope = node_envelope(
        "connection-1",
        3,
        Some(2),
        HubNodeMessage::RunProgress {
            run_id: "run-1".to_string(),
            attempt: 1,
            progress_sequence: 1,
            message: "running".to_string(),
            path_policy_applied: true,
        },
    );
    let progress = store
        .apply_node_envelope(&progress_envelope, 60_000, 51)
        .unwrap();
    assert!(matches!(
        progress.outcome,
        HubNodeInboundOutcome::RunProgress(ref run)
            if run.progress_sequence == 1
    ));

    let completed_envelope = node_envelope(
        "connection-1",
        4,
        Some(2),
        HubNodeMessage::RunCompleted(completion(RunTerminalStatus::Succeeded)),
    );
    let completed = store
        .apply_node_envelope(&completed_envelope, 60_000, 52)
        .unwrap();
    assert!(matches!(
        completed.outcome,
        HubNodeInboundOutcome::RunCompleted(ref run)
            if run.status == HubNodeRunStatus::Succeeded
    ));
    let duplicate = store
        .apply_node_envelope(&completed_envelope, 60_000, 53)
        .unwrap();
    assert_eq!(duplicate.outcome, HubNodeInboundOutcome::Duplicate);
}

#[test]
fn rejected_transition_rolls_back_both_receipt_and_ack() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::ReadOnly);
    let store = memory.hub_node_rail();
    let premature_completion = node_envelope(
        "connection-1",
        2,
        Some(2),
        HubNodeMessage::RunCompleted(completion(RunTerminalStatus::Succeeded)),
    );
    assert!(matches!(
        store.apply_node_envelope(&premature_completion, 60_000, 49),
        Err(HubNodeRailError::LeaseConflict)
    ));

    let invalid = node_envelope(
        "connection-1",
        2,
        Some(2),
        HubNodeMessage::RunProgress {
            run_id: "run-1".to_string(),
            attempt: 1,
            progress_sequence: 1,
            message: "too early".to_string(),
            path_policy_applied: true,
        },
    );
    assert!(matches!(
        store.apply_node_envelope(&invalid, 60_000, 50),
        Err(HubNodeRailError::LeaseConflict)
    ));
    assert_eq!(store.pending_outbox("node-1", 0, 10).unwrap().len(), 2);

    let valid = node_envelope(
        "connection-1",
        2,
        Some(2),
        HubNodeMessage::RunAccepted {
            run_id: "run-1".to_string(),
            attempt: 1,
        },
    );
    store.apply_node_envelope(&valid, 60_000, 51).unwrap();
    assert!(store.pending_outbox("node-1", 0, 10).unwrap().is_empty());
}

#[test]
fn heartbeat_renews_only_runs_owned_by_the_active_connection() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::ReadOnly);
    let store = memory.hub_node_rail();
    store
        .apply_node_envelope(
            &node_envelope(
                "connection-1",
                2,
                Some(2),
                HubNodeMessage::RunAccepted {
                    run_id: "run-1".to_string(),
                    attempt: 1,
                },
            ),
            60_000,
            50,
        )
        .unwrap();
    let before = store
        .get_run("run-1")
        .unwrap()
        .unwrap()
        .lease_expires_at_ms
        .unwrap();
    let heartbeat = store
        .apply_node_envelope(
            &node_envelope(
                "connection-1",
                3,
                Some(2),
                HubNodeMessage::Heartbeat {
                    active_run_ids: vec!["run-1".to_string()],
                },
            ),
            60_000,
            60,
        )
        .unwrap();
    assert_eq!(
        heartbeat.outcome,
        HubNodeInboundOutcome::Heartbeat { renewed_runs: 1 }
    );
    assert!(
        store
            .get_run("run-1")
            .unwrap()
            .unwrap()
            .lease_expires_at_ms
            .unwrap()
            > before
    );

    let invalid = node_envelope(
        "connection-1",
        4,
        Some(2),
        HubNodeMessage::Heartbeat {
            active_run_ids: vec!["unknown-run".to_string()],
        },
    );
    assert!(matches!(
        store.apply_node_envelope(&invalid, 60_000, 61),
        Err(HubNodeRailError::LeaseConflict)
    ));
    let retry = node_envelope("connection-1", 4, Some(2), HubNodeMessage::AckOnly);
    assert_eq!(
        store
            .apply_node_envelope(&retry, 60_000, 62)
            .unwrap()
            .acknowledged_node_sequence,
        4
    );
}

#[test]
fn exact_local_approval_blocks_acceptance_until_the_hub_decision_is_durable() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::LocalMutation);
    let store = memory.hub_node_rail();
    let digest = approval_action_digest("shell_exec", br#"{"command":"true"}"#);
    let approval = RunApprovalRequest {
        run_id: "run-1".to_string(),
        attempt: 1,
        approval_id: "approval-1".to_string(),
        action_digest: digest.clone(),
        action_summary: "Run a command in workspace://workspace-main".to_string(),
        risk_level: RiskLevel::High,
        expires_at_ms: 60_000,
        path_policy_applied: true,
    };
    let required = store
        .apply_node_envelope(
            &node_envelope(
                "connection-1",
                2,
                Some(2),
                HubNodeMessage::RunApprovalRequired(approval),
            ),
            60_000,
            50,
        )
        .unwrap();
    assert!(matches!(
        required.outcome,
        HubNodeInboundOutcome::RunApprovalRequired(ref record)
            if record.status == HubNodeRunApprovalStatus::Pending
    ));

    let premature_accept = node_envelope(
        "connection-1",
        3,
        Some(2),
        HubNodeMessage::RunAccepted {
            run_id: "run-1".to_string(),
            attempt: 1,
        },
    );
    assert!(matches!(
        store.apply_node_envelope(&premature_accept, 60_000, 51),
        Err(HubNodeRailError::LeaseConflict)
    ));

    let decision = RunApprovalDecision {
        run_id: "run-1".to_string(),
        attempt: 1,
        approval_id: "approval-1".to_string(),
        action_digest: digest,
        decision: ApprovalDecision::Approved,
        reason: Some("Approved once".to_string()),
        decided_at_ms: 52,
    };
    let decided = store.decide_run_approval(&decision).unwrap();
    assert_eq!(decided.approval.status, HubNodeRunApprovalStatus::Approved);
    let outbound: HubNodeMessage = serde_json::from_str(&decided.outbox.message_json).unwrap();
    assert_eq!(
        outbound,
        HubNodeMessage::RunApprovalDecision(decision.clone())
    );

    let mut retry = decision.clone();
    retry.decided_at_ms = 54;
    let replayed = store.decide_run_approval(&retry).unwrap();
    assert_eq!(replayed.outbox, decided.outbox);
    assert_eq!(replayed.approval.decided_at_ms, Some(52));

    let accepted_envelope = node_envelope(
        "connection-1",
        3,
        Some(3),
        HubNodeMessage::RunAccepted {
            run_id: "run-1".to_string(),
            attempt: 1,
        },
    );
    let accepted = store
        .apply_node_envelope(&accepted_envelope, 60_000, 55)
        .unwrap();
    assert!(matches!(
        accepted.outcome,
        HubNodeInboundOutcome::RunAccepted(ref run)
            if run.status == HubNodeRunStatus::Accepted
    ));
}

#[test]
fn restart_cancels_a_pending_approval_before_any_side_effect() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::ExternalEffect);
    let store = memory.hub_node_rail();
    store
        .apply_node_envelope(
            &node_envelope(
                "connection-1",
                2,
                Some(2),
                HubNodeMessage::RunApprovalRequired(RunApprovalRequest {
                    run_id: "run-1".to_string(),
                    attempt: 1,
                    approval_id: "approval-restart".to_string(),
                    action_digest: approval_action_digest("shell_exec", b"external command"),
                    action_summary: "Run an external command in workspace://workspace-main"
                        .to_string(),
                    risk_level: RiskLevel::Critical,
                    expires_at_ms: 60_000,
                    path_policy_applied: true,
                }),
            ),
            60_000,
            50,
        )
        .unwrap();

    let recovery = store.reconcile_after_restart(55).unwrap();
    assert_eq!(recovery.cancelled_before_effect, 1);
    assert_eq!(recovery.uncertain_side_effects, 0);
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        HubNodeRunStatus::Cancelled
    );
    assert_eq!(
        store.get_run_approval("run-1").unwrap().unwrap().status,
        HubNodeRunApprovalStatus::TimedOut
    );
}

#[test]
fn heartbeat_renews_a_still_pending_local_approval() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::LocalMutation);
    let store = memory.hub_node_rail();
    store
        .apply_node_envelope(
            &node_envelope(
                "connection-1",
                2,
                Some(2),
                HubNodeMessage::RunApprovalRequired(RunApprovalRequest {
                    run_id: "run-1".to_string(),
                    attempt: 1,
                    approval_id: "approval-heartbeat".to_string(),
                    action_digest: approval_action_digest("file_write", b"exact mutation"),
                    action_summary: "Write within workspace://workspace-main".to_string(),
                    risk_level: RiskLevel::Medium,
                    expires_at_ms: 60_000,
                    path_policy_applied: true,
                }),
            ),
            60_000,
            50,
        )
        .unwrap();
    let before = store
        .get_run("run-1")
        .unwrap()
        .unwrap()
        .lease_expires_at_ms
        .unwrap();
    let heartbeat = store
        .apply_node_envelope(
            &node_envelope(
                "connection-1",
                3,
                Some(2),
                HubNodeMessage::Heartbeat {
                    active_run_ids: vec!["run-1".to_string()],
                },
            ),
            60_000,
            60,
        )
        .unwrap();
    assert_eq!(
        heartbeat.outcome,
        HubNodeInboundOutcome::Heartbeat { renewed_runs: 1 }
    );
    assert!(
        store
            .get_run("run-1")
            .unwrap()
            .unwrap()
            .lease_expires_at_ms
            .unwrap()
            > before
    );
}

#[test]
fn denied_approval_is_terminal_before_effect_in_the_decision_transaction() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::ExternalEffect);
    let store = memory.hub_node_rail();
    let digest = approval_action_digest("shell_exec", b"external command");
    store
        .apply_node_envelope(
            &node_envelope(
                "connection-1",
                2,
                Some(2),
                HubNodeMessage::RunApprovalRequired(RunApprovalRequest {
                    run_id: "run-1".to_string(),
                    attempt: 1,
                    approval_id: "approval-denied".to_string(),
                    action_digest: digest.clone(),
                    action_summary: "Run an external command in workspace://workspace-main"
                        .to_string(),
                    risk_level: RiskLevel::Critical,
                    expires_at_ms: 60_000,
                    path_policy_applied: true,
                }),
            ),
            60_000,
            50,
        )
        .unwrap();
    store
        .decide_run_approval(&RunApprovalDecision {
            run_id: "run-1".to_string(),
            attempt: 1,
            approval_id: "approval-denied".to_string(),
            action_digest: digest,
            decision: ApprovalDecision::Denied,
            reason: Some("Not allowed on this device".to_string()),
            decided_at_ms: 51,
        })
        .unwrap();

    let run = store.get_run("run-1").unwrap().unwrap();
    assert_eq!(run.status, HubNodeRunStatus::Cancelled);
    assert_eq!(run.error_code.as_deref(), Some("approval_denied"));

    let recovery = store.reconcile_after_restart(52).unwrap();
    assert_eq!(recovery.cancelled_before_effect, 0);
    assert_eq!(recovery.uncertain_side_effects, 0);
    let run = store.get_run("run-1").unwrap().unwrap();
    assert_eq!(run.status, HubNodeRunStatus::Cancelled);
    assert_eq!(run.error_code.as_deref(), Some("approval_denied"));
}

#[test]
fn local_policy_rejection_is_terminal_correlated_and_replay_safe() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::ExternalEffect);
    let store = memory.hub_node_rail();
    let rejection = RunRejection {
        run_id: "run-1".to_string(),
        attempt: 1,
        code: "workspace_not_granted".to_string(),
        message: "workspace://workspace-main is not granted locally".to_string(),
        retryable: false,
        path_policy_applied: true,
    };
    let envelope = node_envelope(
        "connection-1",
        2,
        Some(2),
        HubNodeMessage::RunRejected(rejection.clone()),
    );
    let applied = store.apply_node_envelope(&envelope, 60_000, 50).unwrap();
    assert!(matches!(
        applied.outcome,
        HubNodeInboundOutcome::RunRejected(ref run)
            if run.status == HubNodeRunStatus::Failed
                && run.rejection.as_ref() == Some(&rejection)
                && run.effect_state == crate::hub_node_rail::HubNodeEffectState::Completed
    ));
    assert_eq!(
        store
            .apply_node_envelope(&envelope, 60_000, 51)
            .unwrap()
            .outcome,
        HubNodeInboundOutcome::Duplicate
    );
}

#[test]
fn rejection_before_effect_remains_authoritative_after_lease_expiry() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::ExternalEffect);
    let store = memory.hub_node_rail();
    let rejection = RunRejection {
        run_id: "run-1".to_string(),
        attempt: 1,
        code: "approval_expired".to_string(),
        message: "The local approval expired before execution".to_string(),
        retryable: true,
        path_policy_applied: true,
    };
    let applied = store
        .apply_node_envelope(
            &node_envelope(
                "connection-1",
                2,
                Some(2),
                HubNodeMessage::RunRejected(rejection),
            ),
            60_000,
            60_041,
        )
        .unwrap();
    assert!(matches!(
        applied.outcome,
        HubNodeInboundOutcome::RunRejected(ref run)
            if run.status == HubNodeRunStatus::Failed
                && run.error_code.as_deref() == Some("approval_expired")
    ));
}

#[test]
fn stale_connection_and_hub_origin_messages_are_rejected_before_receipt() {
    let memory = connected_memory();
    let store = memory.hub_node_rail();
    let reconnect = node_envelope(
        "connection-2",
        2,
        Some(1),
        HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: capabilities(),
            resume_after_sequence: 1,
            active_run_ids: Vec::new(),
        },
    );
    store
        .open_connection(&reconnect, NodeTransport::LongPoll, 15_000, 60_000, 30)
        .unwrap();
    let stale = node_envelope("connection-1", 3, Some(2), HubNodeMessage::AckOnly);
    assert!(matches!(
        store.apply_node_envelope(&stale, 60_000, 31),
        Err(HubNodeRailError::ConnectionConflict)
    ));
    let invalid_direction = node_envelope(
        "connection-2",
        3,
        Some(2),
        HubNodeMessage::CancelRun {
            run_id: "run-1".to_string(),
            attempt: 1,
            reason: "invalid".to_string(),
        },
    );
    assert!(matches!(
        store.apply_node_envelope(&invalid_direction, 60_000, 32),
        Err(HubNodeRailError::InvalidMessageDirection)
    ));
    assert_eq!(
        store
            .apply_node_envelope(
                &node_envelope("connection-2", 3, Some(2), HubNodeMessage::AckOnly),
                60_000,
                33,
            )
            .unwrap()
            .acknowledged_node_sequence,
        3
    );
}

#[test]
fn late_completion_from_a_reconnected_node_reconciles_uncertain_work() {
    let memory = connected_memory();
    enqueue_and_lease(&memory, RunEffect::ExternalEffect);
    let store = memory.hub_node_rail();
    store.reconcile_after_restart(50).unwrap();
    store.reconcile_connections_after_restart(50).unwrap();
    let reconnect = node_envelope(
        "connection-2",
        2,
        Some(2),
        HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: capabilities(),
            resume_after_sequence: 2,
            active_run_ids: Vec::new(),
        },
    );
    store
        .open_connection(&reconnect, NodeTransport::LongPoll, 15_000, 60_000, 51)
        .unwrap();
    let result = store
        .apply_node_envelope(
            &node_envelope(
                "connection-2",
                3,
                Some(3),
                HubNodeMessage::RunCompleted(completion(RunTerminalStatus::Succeeded)),
            ),
            60_000,
            52,
        )
        .unwrap();
    assert!(matches!(
        result.outcome,
        HubNodeInboundOutcome::RunCompleted(ref run)
            if run.status == HubNodeRunStatus::Succeeded
    ));
}

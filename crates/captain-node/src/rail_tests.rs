use super::*;
use captain_types::approval::{approval_action_digest, ApprovalDecision, RiskLevel};
use captain_wire::hub_protocol::{RunApprovalDecision, RunApprovalRequest, RunRejection};
use captain_wire::{
    CapabilityDescriptor, HubNodeDeliveryBatch, HubNodeEnvelope, HubNodeMessage, LogicalWorkspace,
    NodeTransport, RunCompletion, RunEffect, RunLease, RunTerminalStatus,
    HUB_NODE_PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn paired_store(root: &Path, device_id: &str, hub_character: char) -> NodePairingStore {
    let store = NodePairingStore::open(root).unwrap();
    let state = serde_json::json!({
        "schema_version": 1,
        "hub_sha256": hub_character.to_string().repeat(64),
        "phase": {
            "state": "paired",
            "credential": "a".repeat(64),
            "device_id": device_id,
            "protocol_version": {
                "major": HUB_NODE_PROTOCOL_VERSION.major,
                "minor": HUB_NODE_PROTOCOL_VERSION.minor,
            }
        }
    });
    fs::write(
        root.join("pairing.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store.status().unwrap(),
        Some(NodePairingProgress::Paired {
            device_id: device_id.to_string(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
        })
    );
    store
}

fn capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "macos-arm64".to_string(),
        transports: vec![
            NodeTransport::WebSocket,
            NodeTransport::HttpStream,
            NodeTransport::LongPoll,
        ],
        tool_families: vec!["file".to_string(), "shell".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "project-main".to_string(),
            label: "Main Project".to_string(),
            read_only: false,
        }],
        supports_streaming_output: true,
    }
}

fn welcome(
    bootstrap: &HubNodeEnvelope,
    sequence: u64,
    transport: NodeTransport,
) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: bootstrap.device_id.clone(),
        connection_id: bootstrap.connection_id.clone(),
        sequence,
        ack_sequence: None,
        sent_at_ms: 100 + sequence as i64,
        message: HubNodeMessage::Welcome {
            negotiated_version: HUB_NODE_PROTOCOL_VERSION,
            transport,
            heartbeat_interval_ms: 15_000,
            lease_duration_ms: 60_000,
        },
    }
}

fn batch(
    bootstrap: &HubNodeEnvelope,
    acknowledged_node_sequence: u64,
    messages: Vec<HubNodeEnvelope>,
) -> HubNodeDeliveryBatch {
    HubNodeDeliveryBatch {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: bootstrap.device_id.clone(),
        connection_id: bootstrap.connection_id.clone(),
        acknowledged_node_sequence,
        messages,
        retry_after_ms: None,
    }
}

fn run_offer(bootstrap: &HubNodeEnvelope, sequence: u64, lease: RunLease) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: bootstrap.device_id.clone(),
        connection_id: bootstrap.connection_id.clone(),
        sequence,
        ack_sequence: None,
        sent_at_ms: 100 + sequence as i64,
        message: HubNodeMessage::RunOffer(lease),
    }
}

fn cancel_run(bootstrap: &HubNodeEnvelope, sequence: u64, reason: &str) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: bootstrap.device_id.clone(),
        connection_id: bootstrap.connection_id.clone(),
        sequence,
        ack_sequence: None,
        sent_at_ms: 100 + sequence as i64,
        message: HubNodeMessage::CancelRun {
            run_id: "run-local-1".to_string(),
            attempt: 1,
            reason: reason.to_string(),
        },
    }
}

fn lease() -> RunLease {
    RunLease {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        idempotency_key: "idem-local-1".to_string(),
        workspace_id: "project-main".to_string(),
        tool_name: "file_read".to_string(),
        input: serde_json::json!({"path": "src/main.rs"}),
        effect: RunEffect::ReadOnly,
        lease_expires_at_ms: 60_000,
    }
}

fn completion(status: RunTerminalStatus, content: &str) -> RunCompletion {
    RunCompletion {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        status,
        result_content: content.to_string(),
        result_sha256: hex::encode(Sha256::digest(content.as_bytes())),
        total_output_bytes: content.len() as u64,
        stored_output_bytes: content.len() as u64,
        capped: false,
        redacted: false,
        path_policy_applied: true,
    }
}

fn test_now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn local_approval_request(expires_at_ms: i64) -> RunApprovalRequest {
    RunApprovalRequest {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        approval_id: "approval-local-1".to_string(),
        action_digest: approval_action_digest("file_read", br#"{"path":"src/main.rs"}"#),
        action_summary: "Read workspace://project-main/src/main.rs".to_string(),
        risk_level: RiskLevel::Low,
        expires_at_ms,
        path_policy_applied: true,
    }
}

fn approval_decision_envelope(
    bootstrap: &HubNodeEnvelope,
    sequence: u64,
    request: &RunApprovalRequest,
    decision: ApprovalDecision,
    decided_at_ms: i64,
) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: bootstrap.device_id.clone(),
        connection_id: bootstrap.connection_id.clone(),
        sequence,
        ack_sequence: None,
        sent_at_ms: 100 + sequence as i64,
        message: HubNodeMessage::RunApprovalDecision(RunApprovalDecision {
            run_id: request.run_id.clone(),
            attempt: request.attempt,
            approval_id: request.approval_id.clone(),
            action_digest: request.action_digest.clone(),
            decision,
            reason: Some("Exact operator decision".to_string()),
            decided_at_ms,
        }),
    }
}

fn ready_rail_with_offer(root: &Path) -> (NodePairingStore, NodeRailStore, HubNodeEnvelope) {
    ready_rail_with_lease(root, lease())
}

fn ready_rail_with_lease(
    root: &Path,
    lease: RunLease,
) -> (NodePairingStore, NodeRailStore, HubNodeEnvelope) {
    let pairing = paired_store(root, "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    rail.observe_delivery(
        &batch(&hello, 1, vec![welcome(&hello, 1, NodeTransport::LongPoll)]),
        20,
    )
    .unwrap();
    rail.mark_inbound_applied(1, 21).unwrap();
    rail.observe_delivery(&batch(&hello, 2, vec![run_offer(&hello, 2, lease)]), 30)
        .unwrap();
    (pairing, rail, hello)
}

#[test]
fn bootstrap_identity_and_hello_survive_restart_without_consuming_a_sequence() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let pairing = paired_store(&root, "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let first = rail
        .bootstrap_hello(&capabilities(), &["run-before".to_string()], 10)
        .unwrap()
        .envelope;
    let replay = rail
        .bootstrap_hello(&capabilities(), &["run-after".to_string()], 20)
        .unwrap()
        .envelope;
    assert_eq!(replay, first);
    assert_eq!(first.sequence, 1);
    assert_eq!(first.ack_sequence, None);
    assert_eq!(rail.pending_outbound(10).unwrap(), vec![first.clone()]);
    assert_eq!(rail.snapshot().unwrap().last_node_sequence, 1);

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        restarted
            .bootstrap_hello(&capabilities(), &[], 30)
            .unwrap()
            .envelope,
        first
    );
    assert_eq!(
        restarted.snapshot().unwrap().connection_id,
        first.connection_id
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.join("rail.sqlite3"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn one_physical_rail_open_is_shared_by_clone_and_released_with_the_last_handle() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let shared = rail.clone();
    assert!(matches!(
        NodeRailStore::open(&pairing),
        Err(NodeRailError::StateUnavailable)
    ));
    drop(rail);
    assert!(matches!(
        NodeRailStore::open(&pairing),
        Err(NodeRailError::StateUnavailable)
    ));
    drop(shared);
    assert!(NodeRailStore::open(&pairing).is_ok());
}

#[test]
fn delivery_acknowledgement_deduplication_and_pruning_are_transactional() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    let welcome = welcome(&hello, 1, NodeTransport::WebSocket);
    let delivery = batch(&hello, 1, vec![welcome.clone()]);

    let first = rail.observe_delivery(&delivery, 20).unwrap();
    assert_eq!(first.newly_recorded, 1);
    assert!(first.acknowledgement_advanced);
    assert!(first.acknowledgement_enqueued);
    let pending = rail.pending_outbound(10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].sequence, 2);
    assert_eq!(pending[0].ack_sequence, Some(1));
    assert!(matches!(pending[0].message, HubNodeMessage::AckOnly));

    let duplicate = rail.observe_delivery(&delivery, 21).unwrap();
    assert_eq!(duplicate.newly_recorded, 0);
    assert_eq!(duplicate.duplicate_messages, 1);
    assert!(!duplicate.acknowledgement_enqueued);
    assert_eq!(rail.pending_inbound(10).unwrap()[0].envelope, welcome);

    rail.mark_inbound_applied(1, 22).unwrap();
    assert!(rail.pending_inbound(10).unwrap().is_empty());
    let ack_of_ack = batch(&hello, 2, vec![]);
    rail.observe_delivery(&ack_of_ack, 23).unwrap();
    let snapshot = rail.snapshot().unwrap();
    assert_eq!(snapshot.acknowledged_node_sequence, 2);
    assert_eq!(snapshot.confirmed_hub_ack_sequence, 1);
    assert_eq!(snapshot.pending_outbound, 0);
    assert_eq!(snapshot.pending_inbound, 0);
    rail.mark_inbound_applied(1, 24).unwrap();
}

#[test]
fn run_offer_acceptance_and_inbox_application_commit_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let (pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, 31)
        .unwrap();
    assert!(!accepted.replayed);
    assert_eq!(accepted.run.status, NodeRunStatus::Accepted);
    assert_eq!(accepted.run.inbound_sequence, 2);
    assert!(matches!(
        accepted.outbound.as_ref().map(|item| &item.message),
        Some(HubNodeMessage::RunAccepted { run_id, attempt })
            if run_id == "run-local-1" && *attempt == 1
    ));
    assert!(rail.pending_inbound(10).unwrap().is_empty());
    assert_eq!(rail.active_run_ids().unwrap(), vec!["run-local-1"]);

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        restarted.get_run("run-local-1", 1).unwrap().unwrap(),
        accepted.run
    );

    let replay = run_offer(&hello, 3, lease());
    restarted
        .observe_delivery(&batch(&hello, 4, vec![replay]), 40)
        .unwrap();
    let replayed = restarted
        .apply_run_offer(3, &NodeRunDisposition::Accept, 41)
        .unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.outbound, None);
    assert_eq!(replayed.run.inbound_sequence, 3);
    assert!(restarted.pending_inbound(10).unwrap().is_empty());
}

#[test]
fn cancel_before_effect_commits_terminal_completion_with_inbox_application() {
    let temp = tempfile::tempdir().unwrap();
    let (pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, 31)
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            accepted.outbound.unwrap().sequence,
            vec![cancel_run(&hello, 3, "Operator cancelled the run")],
        ),
        40,
    )
    .unwrap();

    let cancelled = rail.apply_cancel_run(3, 41).unwrap();
    assert_eq!(cancelled.run.status, NodeRunStatus::Cancelled);
    assert_eq!(cancelled.run.cancel_inbound_sequence, Some(3));
    assert!(cancelled.run.cancel_sha256.is_some());
    assert!(!cancelled.run.effect_started);
    assert!(!cancelled.signal_runner);
    assert!(!cancelled.replayed);
    assert!(matches!(
        cancelled.outbound.as_ref().map(|envelope| &envelope.message),
        Some(HubNodeMessage::RunCompleted(completion))
            if completion.run_id == "run-local-1"
                && completion.attempt == 1
                && completion.status == RunTerminalStatus::Cancelled
                && completion.result_sha256
                    == hex::encode(Sha256::digest(completion.result_content.as_bytes()))
    ));
    assert_eq!(
        cancelled.run.terminal_outbound_sequence,
        cancelled
            .outbound
            .as_ref()
            .map(|envelope| envelope.sequence)
    );
    assert!(rail.pending_inbound(10).unwrap().is_empty());
    assert!(rail.active_run_ids().unwrap().is_empty());

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        restarted.get_run("run-local-1", 1).unwrap().unwrap(),
        cancelled.run
    );
}

#[test]
fn cancel_supersedes_a_pending_approval_without_deleting_its_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let (pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let request = local_approval_request(50_000);
    let required = rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request), 31)
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            required.outbound.unwrap().sequence,
            vec![cancel_run(&hello, 3, "Project was stopped")],
        ),
        40,
    )
    .unwrap();

    let cancelled = rail.apply_cancel_run(3, 41).unwrap();
    assert_eq!(cancelled.run.status, NodeRunStatus::Cancelled);
    assert!(cancelled.run.approval_decision_inbound_sequence.is_none());
    assert!(cancelled.run.acceptance_outbound_sequence.is_none());
    assert!(cancelled.run.terminal_sha256.is_some());
    drop(rail);
    assert!(NodeRailStore::open(&pairing).is_ok());
}

#[test]
fn cancellation_after_a_local_terminal_result_never_replaces_or_duplicates_it() {
    let temp = tempfile::tempdir().unwrap();
    let (_pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let rejection = NodeRunDisposition::Reject(RunRejection {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        code: "policy_denied".to_string(),
        message: "Local policy denied this action".to_string(),
        retryable: false,
        path_policy_applied: true,
    });
    let rejected = rail.apply_run_offer(2, &rejection, 31).unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            rejected.outbound.unwrap().sequence,
            vec![cancel_run(&hello, 3, "Cancel raced with rejection")],
        ),
        40,
    )
    .unwrap();
    let before = rail.snapshot().unwrap().last_node_sequence;
    let cancelled = rail.apply_cancel_run(3, 41).unwrap();
    assert_eq!(cancelled.run.status, NodeRunStatus::Rejected);
    assert_eq!(cancelled.run.cancel_inbound_sequence, Some(3));
    assert!(cancelled.outbound.is_none());
    assert_eq!(rail.snapshot().unwrap().last_node_sequence, before);

    rail.observe_delivery(
        &batch(
            &hello,
            before,
            vec![cancel_run(&hello, 4, "Cancel raced with rejection")],
        ),
        50,
    )
    .unwrap();
    let before_replay = rail.snapshot().unwrap().last_node_sequence;
    let replay = rail.apply_cancel_run(4, 51).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.run.cancel_inbound_sequence, Some(4));
    assert_eq!(rail.snapshot().unwrap().last_node_sequence, before_replay);
}

#[test]
fn missing_cancellation_or_terminal_evidence_fails_reopen() {
    let inbox_temp = tempfile::tempdir().unwrap();
    let (inbox_pairing, inbox_rail, inbox_hello) = ready_rail_with_offer(inbox_temp.path());
    let accepted = inbox_rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, 31)
        .unwrap();
    inbox_rail
        .observe_delivery(
            &batch(
                &inbox_hello,
                accepted.outbound.unwrap().sequence,
                vec![cancel_run(&inbox_hello, 3, "Operator cancelled the run")],
            ),
            40,
        )
        .unwrap();
    inbox_rail.apply_cancel_run(3, 41).unwrap();
    drop(inbox_rail);

    let database = rusqlite::Connection::open(inbox_temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute("DELETE FROM node_rail_inbox WHERE sequence = 3", [])
        .unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&inbox_pairing),
        Err(NodeRailError::StateCorrupt)
    ));

    let outbox_temp = tempfile::tempdir().unwrap();
    let (outbox_pairing, outbox_rail, outbox_hello) = ready_rail_with_offer(outbox_temp.path());
    let accepted = outbox_rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, 31)
        .unwrap();
    outbox_rail
        .observe_delivery(
            &batch(
                &outbox_hello,
                accepted.outbound.unwrap().sequence,
                vec![cancel_run(&outbox_hello, 3, "Operator cancelled the run")],
            ),
            40,
        )
        .unwrap();
    let cancelled = outbox_rail.apply_cancel_run(3, 41).unwrap();
    let terminal_sequence = cancelled.run.terminal_outbound_sequence.unwrap();
    drop(outbox_rail);

    let database = rusqlite::Connection::open(outbox_temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute(
            "DELETE FROM node_rail_outbox WHERE sequence = ?1",
            [terminal_sequence],
        )
        .unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&outbox_pairing),
        Err(NodeRailError::StateCorrupt)
    ));
}

#[test]
fn execution_claim_requires_hub_ack_and_read_only_restart_uses_a_new_claim() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    assert!(matches!(
        rail.claim_run("run-local-1", 1, now + 1),
        Err(NodeRailError::RunNotReady)
    ));

    let acceptance_sequence = accepted.run.acceptance_outbound_sequence.unwrap();
    rail.observe_delivery(&batch(&hello, acceptance_sequence, vec![]), now + 2)
        .unwrap();
    let first = rail.claim_run("run-local-1", 1, now + 3).unwrap();
    assert_eq!(first.run.status, NodeRunStatus::Running);
    assert!(first.run.effect_started);
    assert_eq!(
        first.run.execution_claim_id.as_deref(),
        Some(first.claim_id.as_str())
    );
    assert_eq!(
        first.run.execution_claim_started_at_ms,
        Some(first.claimed_at_ms)
    );
    assert_eq!(
        uuid::Uuid::parse_str(&first.claim_id)
            .unwrap()
            .hyphenated()
            .to_string(),
        first.claim_id
    );
    assert!(!format!("{first:?}").contains(&first.claim_id));
    let serialized = serde_json::to_string(&first.run).unwrap();
    assert!(!serialized.contains(&first.claim_id));
    assert!(!serialized.contains("execution_claim_id"));
    assert!(!rail.cancellation_requested(&first.claim_id).unwrap());
    assert!(matches!(
        rail.claim_run("run-local-1", 1, now + 4),
        Err(NodeRailError::RunClaimConflict)
    ));

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    let recovered = restarted.get_run("run-local-1", 1).unwrap().unwrap();
    assert_eq!(recovered.status, NodeRunStatus::Accepted);
    assert!(!recovered.effect_started);
    assert!(recovered.execution_claim_id.is_none());
    assert!(recovered.execution_claim_started_at_ms.is_none());
    let second = restarted.claim_run("run-local-1", 1, now + 10).unwrap();
    assert_ne!(second.claim_id, first.claim_id);
}

#[test]
fn claimable_inventory_survives_restart_and_preflight_rejection_is_atomic() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (pairing, rail, _hello) = ready_rail_with_lease(temp.path(), offered);
    rail.apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();

    let claimable = rail.claimable_runs(16).unwrap();
    assert_eq!(claimable.len(), 1);
    assert_eq!(claimable[0].lease.run_id, "run-local-1");
    assert_eq!(rail.claimable_runs(0).unwrap().len(), 1);

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(restarted.claimable_runs(16).unwrap().len(), 1);
    let rejection = RunRejection {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        code: "policy_changed".to_string(),
        message: "Current local policy no longer authorizes this run".to_string(),
        retryable: false,
        path_policy_applied: true,
    };
    let rejected = restarted
        .reject_run_before_effect("run-local-1", 1, &rejection, now + 1)
        .unwrap();
    assert_eq!(rejected.run.status, NodeRunStatus::Rejected);
    assert!(!rejected.run.effect_started);
    assert!(matches!(
        rejected.outbound.message,
        HubNodeMessage::RunRejected(stored) if stored == rejection
    ));
    assert!(restarted.claimable_runs(16).unwrap().is_empty());
    assert!(matches!(
        restarted.claim_run("run-local-1", 1, now + 2),
        Err(NodeRailError::RunClaimConflict)
    ));

    drop(restarted);
    let recovered = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        recovered.get_run("run-local-1", 1).unwrap().unwrap().status,
        NodeRunStatus::Rejected
    );
}

#[test]
fn preflight_rejection_cannot_overtake_a_pending_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (_pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            accepted.run.acceptance_outbound_sequence.unwrap(),
            vec![cancel_run(&hello, 3, "Cancel before policy recheck")],
        ),
        now + 1,
    )
    .unwrap();
    let rejection = RunRejection {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        code: "policy_changed".to_string(),
        message: "Current local policy no longer authorizes this run".to_string(),
        retryable: false,
        path_policy_applied: true,
    };
    assert!(matches!(
        rail.reject_run_before_effect("run-local-1", 1, &rejection, now + 2),
        Err(NodeRailError::RunClaimConflict)
    ));
    assert_eq!(
        rail.apply_cancel_run(3, now + 3).unwrap().run.status,
        NodeRunStatus::Cancelled
    );
}

#[test]
fn acknowledged_acceptance_cannot_overtake_a_pending_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (_pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    let acceptance_sequence = accepted.run.acceptance_outbound_sequence.unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            acceptance_sequence,
            vec![cancel_run(&hello, 3, "Cancel before the runner starts")],
        ),
        now + 1,
    )
    .unwrap();
    assert!(matches!(
        rail.claim_run("run-local-1", 1, now + 2),
        Err(NodeRailError::RunCancellationPending)
    ));
    let cancelled = rail.apply_cancel_run(3, now + 3).unwrap();
    assert_eq!(cancelled.run.status, NodeRunStatus::Cancelled);
    assert!(!cancelled.run.effect_started);
}

#[test]
fn exact_claim_completion_is_atomic_idempotent_and_bound_to_content() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            accepted.run.acceptance_outbound_sequence.unwrap(),
            vec![],
        ),
        now + 1,
    )
    .unwrap();
    let claim = rail.claim_run("run-local-1", 1, now + 2).unwrap();
    let evidence = completion(
        RunTerminalStatus::Succeeded,
        "workspace://project-main/README.md",
    );
    let completed = rail
        .complete_run(&claim.claim_id, &evidence, now + 3)
        .unwrap();
    assert!(!completed.replayed);
    assert_eq!(completed.run.status, NodeRunStatus::Succeeded);
    assert!(completed.run.effect_started);
    assert!(matches!(
        completed.outbound.as_ref().map(|envelope| &envelope.message),
        Some(HubNodeMessage::RunCompleted(stored)) if stored == &evidence
    ));
    let last_node_sequence = rail.snapshot().unwrap().last_node_sequence;

    let replay = rail
        .complete_run(&claim.claim_id, &evidence, now + 4)
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(
        rail.snapshot().unwrap().last_node_sequence,
        last_node_sequence
    );
    let changed = completion(RunTerminalStatus::Succeeded, "different result");
    assert!(matches!(
        rail.complete_run(&claim.claim_id, &changed, now + 5),
        Err(NodeRailError::RunClaimConflict)
    ));
    assert!(matches!(
        rail.cancellation_requested(&claim.claim_id),
        Err(NodeRailError::RunClaimConflict)
    ));

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        restarted.get_run("run-local-1", 1).unwrap().unwrap(),
        completed.run
    );
}

#[test]
fn cancel_during_effect_is_visible_to_the_exact_claim_and_first_terminal_wins() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (_pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    let acceptance_sequence = accepted.run.acceptance_outbound_sequence.unwrap();
    rail.observe_delivery(&batch(&hello, acceptance_sequence, vec![]), now + 1)
        .unwrap();
    let claim = rail.claim_run("run-local-1", 1, now + 2).unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            acceptance_sequence,
            vec![cancel_run(&hello, 3, "Operator cancelled the active run")],
        ),
        now + 3,
    )
    .unwrap();
    let cancellation = rail.apply_cancel_run(3, now + 4).unwrap();
    assert_eq!(cancellation.run.status, NodeRunStatus::CancelRequested);
    assert!(cancellation.signal_runner);
    assert!(cancellation.outbound.is_none());
    assert!(rail.cancellation_requested(&claim.claim_id).unwrap());

    let evidence = completion(
        RunTerminalStatus::Cancelled,
        "Cancelled by the local runner.",
    );
    let completed = rail
        .complete_run(&claim.claim_id, &evidence, now + 5)
        .unwrap();
    assert_eq!(completed.run.status, NodeRunStatus::Cancelled);
    assert!(completed.run.cancel_inbound_sequence.is_some());
    assert!(completed.outbound.is_some());
}

#[test]
fn interrupted_mutation_becomes_uncertain_once_and_is_never_reclaimed() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.effect = RunEffect::LocalMutation;
    offered.tool_name = "file_write".to_string();
    offered.lease_expires_at_ms = now + 60_000;
    let (pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    let acceptance_sequence = accepted.run.acceptance_outbound_sequence.unwrap();
    rail.observe_delivery(&batch(&hello, acceptance_sequence, vec![]), now + 1)
        .unwrap();
    let claim = rail.claim_run("run-local-1", 1, now + 2).unwrap();
    let before_restart_sequence = rail.snapshot().unwrap().last_node_sequence;
    drop(rail);

    let restarted = NodeRailStore::open(&pairing).unwrap();
    let recovered = restarted.get_run("run-local-1", 1).unwrap().unwrap();
    assert_eq!(recovered.status, NodeRunStatus::Uncertain);
    assert!(recovered.effect_started);
    assert_eq!(
        recovered.execution_claim_id.as_deref(),
        Some(claim.claim_id.as_str())
    );
    let pending = restarted.pending_outbound(10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].sequence, before_restart_sequence + 1);
    assert!(matches!(
        &pending[0].message,
        HubNodeMessage::RunCompleted(completion)
            if completion.status == RunTerminalStatus::Uncertain
                && completion.result_sha256
                    == hex::encode(Sha256::digest(completion.result_content.as_bytes()))
    ));
    assert!(matches!(
        restarted.claim_run("run-local-1", 1, now + 10),
        Err(NodeRailError::RunClaimConflict)
    ));
    let recovered_sequence = restarted.snapshot().unwrap().last_node_sequence;
    drop(restarted);

    let reopened = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().last_node_sequence,
        recovered_sequence
    );
    assert_eq!(reopened.pending_outbound(10).unwrap().len(), 1);
}

#[test]
fn approved_work_can_only_be_claimed_after_the_acceptance_is_acknowledged() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (_pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let request = local_approval_request(now + 30_000);
    let required = rail
        .apply_run_offer(
            2,
            &NodeRunDisposition::RequireApproval(request.clone()),
            now,
        )
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            required.outbound.unwrap().sequence,
            vec![approval_decision_envelope(
                &hello,
                3,
                &request,
                ApprovalDecision::Approved,
                now + 1,
            )],
        ),
        now + 2,
    )
    .unwrap();
    let approved = rail.apply_run_approval_decision(3, now + 3).unwrap();
    assert!(matches!(
        rail.claim_run("run-local-1", 1, now + 4),
        Err(NodeRailError::RunNotReady)
    ));
    let acceptance_sequence = approved.run.acceptance_outbound_sequence.unwrap();
    rail.observe_delivery(&batch(&hello, acceptance_sequence, vec![]), now + 5)
        .unwrap();
    let claim = rail.claim_run("run-local-1", 1, now + 6).unwrap();
    assert_eq!(claim.run.status, NodeRunStatus::Running);
    assert_eq!(
        claim.run.approval_decision_inbound_sequence,
        approved.run.approval_decision_inbound_sequence
    );
}

#[test]
fn interrupted_cancelled_read_is_terminalized_once_without_becoming_uncertain() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    let acceptance_sequence = accepted.run.acceptance_outbound_sequence.unwrap();
    rail.observe_delivery(&batch(&hello, acceptance_sequence, vec![]), now + 1)
        .unwrap();
    let claim = rail.claim_run("run-local-1", 1, now + 2).unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            acceptance_sequence,
            vec![cancel_run(&hello, 3, "Stop the active read")],
        ),
        now + 3,
    )
    .unwrap();
    rail.apply_cancel_run(3, now + 4).unwrap();
    drop(rail);

    let restarted = NodeRailStore::open(&pairing).unwrap();
    let recovered = restarted.get_run("run-local-1", 1).unwrap().unwrap();
    assert_eq!(recovered.status, NodeRunStatus::Cancelled);
    assert!(recovered.effect_started);
    assert_eq!(
        recovered.execution_claim_id.as_deref(),
        Some(claim.claim_id.as_str())
    );
    let terminal_count = restarted
        .pending_outbound(10)
        .unwrap()
        .iter()
        .filter(|envelope| {
            matches!(
                &envelope.message,
                HubNodeMessage::RunCompleted(completion)
                    if completion.status == RunTerminalStatus::Cancelled
            )
        })
        .count();
    assert_eq!(terminal_count, 1);
    let last_node_sequence = restarted.snapshot().unwrap().last_node_sequence;
    drop(restarted);
    let reopened = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        reopened.snapshot().unwrap().last_node_sequence,
        last_node_sequence
    );
}

#[test]
fn version_four_rail_migrates_to_claim_history_without_losing_bootstrap_state() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let bootstrap = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    drop(rail);

    let database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute_batch(
            "DROP INDEX node_runs_execution_claim;
             DROP TABLE node_run_claims;
             ALTER TABLE node_runs DROP COLUMN execution_claim_id;
             ALTER TABLE node_runs DROP COLUMN execution_claim_started_at_ms;
             PRAGMA user_version = 4;",
        )
        .unwrap();
    drop(database);

    let migrated = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        migrated
            .bootstrap_hello(&capabilities(), &[], 20)
            .unwrap()
            .envelope,
        bootstrap
    );
}

#[test]
fn missing_execution_claim_evidence_fails_reopen_before_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (pairing, rail, hello) = ready_rail_with_lease(temp.path(), offered);
    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            accepted.run.acceptance_outbound_sequence.unwrap(),
            vec![],
        ),
        now + 1,
    )
    .unwrap();
    rail.claim_run("run-local-1", 1, now + 2).unwrap();
    drop(rail);

    let database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    database.execute("DELETE FROM node_run_claims", []).unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&pairing),
        Err(NodeRailError::StateCorrupt)
    ));
}

#[test]
fn fabricated_completed_claim_history_cannot_authorize_an_unstarted_run() {
    let temp = tempfile::tempdir().unwrap();
    let now = test_now_ms();
    let mut offered = lease();
    offered.lease_expires_at_ms = now + 60_000;
    let (pairing, rail, _hello) = ready_rail_with_lease(temp.path(), offered);
    rail.apply_run_offer(2, &NodeRunDisposition::Accept, now)
        .unwrap();
    drop(rail);

    let database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute(
            "INSERT INTO node_run_claims (
                 claim_id, run_id, attempt, status, started_at_ms, finished_at_ms
             ) VALUES (?1, 'run-local-1', 1, 'completed', ?2, ?3)",
            rusqlite::params![
                uuid::Uuid::new_v4().hyphenated().to_string(),
                now + 1,
                now + 2,
            ],
        )
        .unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&pairing),
        Err(NodeRailError::StateCorrupt)
    ));
}

#[test]
fn invalid_run_disposition_rolls_back_and_a_valid_retry_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let (_pairing, rail, _hello) = ready_rail_with_offer(temp.path());
    let invalid = NodeRunDisposition::Reject(RunRejection {
        run_id: "different-run".to_string(),
        attempt: 1,
        code: "policy_denied".to_string(),
        message: "Local policy denied this action".to_string(),
        retryable: false,
        path_policy_applied: true,
    });
    assert!(matches!(
        rail.apply_run_offer(2, &invalid, 31),
        Err(NodeRailError::RunDecisionConflict)
    ));
    assert_eq!(rail.pending_inbound(10).unwrap().len(), 1);
    assert!(rail.get_run("run-local-1", 1).unwrap().is_none());

    let accepted = rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, 32)
        .unwrap();
    assert_eq!(accepted.run.status, NodeRunStatus::Accepted);
    assert!(rail.pending_inbound(10).unwrap().is_empty());
}

#[test]
fn approval_digest_must_bind_the_exact_offered_tool_input() {
    let temp = tempfile::tempdir().unwrap();
    let (_pairing, rail, _hello) = ready_rail_with_offer(temp.path());
    let before = rail.snapshot().unwrap();
    let request = RunApprovalRequest {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        approval_id: "approval-local-1".to_string(),
        action_digest: approval_action_digest("file_read", br#"{"path":"other.rs"}"#),
        action_summary: "Read another virtualized workspace file".to_string(),
        risk_level: RiskLevel::Low,
        expires_at_ms: 50_000,
        path_policy_applied: true,
    };
    assert!(matches!(
        rail.apply_run_offer(2, &NodeRunDisposition::RequireApproval(request), 31),
        Err(NodeRailError::RunDecisionConflict)
    ));
    assert_eq!(rail.pending_inbound(10).unwrap().len(), 1);
    assert!(rail.get_run("run-local-1", 1).unwrap().is_none());
    assert_eq!(rail.snapshot().unwrap(), before);
}

#[test]
fn concurrent_attempt_and_idempotency_reuse_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let (_pairing, rail, hello) = ready_rail_with_offer(temp.path());
    rail.apply_run_offer(2, &NodeRunDisposition::Accept, 31)
        .unwrap();

    let mut second_attempt = lease();
    second_attempt.attempt = 2;
    rail.observe_delivery(
        &batch(&hello, 4, vec![run_offer(&hello, 3, second_attempt)]),
        40,
    )
    .unwrap();
    assert!(matches!(
        rail.apply_run_offer(3, &NodeRunDisposition::Accept, 41),
        Err(NodeRailError::RunConflict)
    ));
    assert_eq!(rail.pending_inbound(10).unwrap().len(), 1);

    let other_temp = tempfile::tempdir().unwrap();
    let (_pairing, other_rail, other_hello) = ready_rail_with_offer(other_temp.path());
    let rejection = NodeRunDisposition::Reject(RunRejection {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        code: "policy_denied".to_string(),
        message: "Local policy denied this action".to_string(),
        retryable: false,
        path_policy_applied: true,
    });
    let rejected = other_rail.apply_run_offer(2, &rejection, 31).unwrap();
    assert_eq!(rejected.run.status, NodeRunStatus::Rejected);
    assert_eq!(
        rejected.run.terminal_outbound_sequence,
        rejected.run.decision_outbound_sequence
    );
    let mut reused = lease();
    reused.run_id = "different-run".to_string();
    other_rail
        .observe_delivery(
            &batch(&other_hello, 4, vec![run_offer(&other_hello, 3, reused)]),
            40,
        )
        .unwrap();
    assert!(matches!(
        other_rail.apply_run_offer(3, &NodeRunDisposition::Accept, 41),
        Err(NodeRailError::RunConflict)
    ));
}

#[test]
fn approval_request_is_bound_to_the_offer_and_survives_restart() {
    let temp = tempfile::tempdir().unwrap();
    let (pairing, rail, _hello) = ready_rail_with_offer(temp.path());
    let request = local_approval_request(50_000);
    let disposition = NodeRunDisposition::RequireApproval(request.clone());
    assert!(!format!("{disposition:?}").contains("src/main.rs"));
    let pending = rail.apply_run_offer(2, &disposition, 31).unwrap();
    assert_eq!(pending.run.status, NodeRunStatus::ApprovalPending);
    assert!(matches!(
        pending.outbound.as_ref().map(|item| &item.message),
        Some(HubNodeMessage::RunApprovalRequired(stored)) if stored == &request
    ));
    assert!(!format!("{:?}", pending.run).contains("src/main.rs"));

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        restarted.get_run("run-local-1", 1).unwrap().unwrap().status,
        NodeRunStatus::ApprovalPending
    );
    assert_eq!(restarted.active_run_ids().unwrap(), vec!["run-local-1"]);
}

#[test]
fn approved_local_decision_persists_before_acceptance_and_survives_restart() {
    let temp = tempfile::tempdir().unwrap();
    let (pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let request = local_approval_request(50_000);
    let required = rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request.clone()), 31)
        .unwrap();
    let required_sequence = required.outbound.unwrap().sequence;
    rail.observe_delivery(
        &batch(
            &hello,
            required_sequence,
            vec![approval_decision_envelope(
                &hello,
                3,
                &request,
                ApprovalDecision::Approved,
                35,
            )],
        ),
        40,
    )
    .unwrap();

    let approved = rail.apply_run_approval_decision(3, 41).unwrap();
    assert_eq!(approved.run.status, NodeRunStatus::Accepted);
    assert_eq!(approved.run.approval_decision_inbound_sequence, Some(3));
    assert_eq!(
        approved.run.acceptance_outbound_sequence,
        approved.outbound.as_ref().map(|envelope| envelope.sequence)
    );
    assert!(matches!(
        approved.outbound.as_ref().map(|envelope| &envelope.message),
        Some(HubNodeMessage::RunAccepted { run_id, attempt })
            if run_id == "run-local-1" && *attempt == 1
    ));
    assert!(!approved.expired_locally);
    assert!(approved.run.terminal_sha256.is_none());
    assert!(rail.pending_inbound(10).unwrap().is_empty());

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        restarted.get_run("run-local-1", 1).unwrap().unwrap(),
        approved.run
    );
}

#[test]
fn approved_run_can_be_rejected_by_a_new_local_policy_before_effect() {
    let temp = tempfile::tempdir().unwrap();
    let (pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let request = local_approval_request(50_000);
    let required = rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request.clone()), 31)
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            required.outbound.unwrap().sequence,
            vec![approval_decision_envelope(
                &hello,
                3,
                &request,
                ApprovalDecision::Approved,
                35,
            )],
        ),
        40,
    )
    .unwrap();
    let accepted = rail.apply_run_approval_decision(3, 41).unwrap();
    assert_eq!(accepted.run.status, NodeRunStatus::Accepted);

    let rejection = RunRejection {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        code: "policy_changed".to_string(),
        message: "Current local policy no longer authorizes this run".to_string(),
        retryable: false,
        path_policy_applied: true,
    };
    let rejected = rail
        .reject_run_before_effect("run-local-1", 1, &rejection, 42)
        .unwrap();
    assert_eq!(rejected.run.status, NodeRunStatus::Rejected);
    assert!(rejected.run.approval_decision_inbound_sequence.is_some());
    assert!(rejected.run.acceptance_outbound_sequence.is_some());
    assert!(!rejected.run.effect_started);

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        restarted.get_run("run-local-1", 1).unwrap().unwrap(),
        rejected.run
    );
}

#[test]
fn denied_local_decision_closes_before_effect_without_outbound_replay() {
    let temp = tempfile::tempdir().unwrap();
    let (pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let request = local_approval_request(50_000);
    let required = rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request.clone()), 31)
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            required.outbound.unwrap().sequence,
            vec![approval_decision_envelope(
                &hello,
                3,
                &request,
                ApprovalDecision::DeniedAlways,
                35,
            )],
        ),
        40,
    )
    .unwrap();

    let denied = rail.apply_run_approval_decision(3, 41).unwrap();
    assert_eq!(denied.run.status, NodeRunStatus::Cancelled);
    assert_eq!(denied.run.approval_decision_inbound_sequence, Some(3));
    assert!(denied.run.acceptance_outbound_sequence.is_none());
    assert!(denied.run.terminal_outbound_sequence.is_none());
    assert!(denied.run.terminal_sha256.is_some());
    assert!(denied.outbound.is_none());
    assert!(!denied.expired_locally);
    assert!(rail.active_run_ids().unwrap().is_empty());

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        restarted.get_run("run-local-1", 1).unwrap().unwrap(),
        denied.run
    );
}

#[test]
fn locally_expired_approval_never_starts_and_emits_a_correlated_rejection() {
    let temp = tempfile::tempdir().unwrap();
    let (_pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let request = local_approval_request(50);
    let required = rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request.clone()), 31)
        .unwrap();
    rail.observe_delivery(
        &batch(
            &hello,
            required.outbound.unwrap().sequence,
            vec![approval_decision_envelope(
                &hello,
                3,
                &request,
                ApprovalDecision::ApprovedSession,
                40,
            )],
        ),
        50,
    )
    .unwrap();

    let expired = rail.apply_run_approval_decision(3, 51).unwrap();
    assert_eq!(expired.run.status, NodeRunStatus::Rejected);
    assert!(expired.expired_locally);
    assert_eq!(expired.run.approval_decision_inbound_sequence, Some(3));
    assert!(matches!(
        expired.outbound.as_ref().map(|envelope| &envelope.message),
        Some(HubNodeMessage::RunRejected(rejection))
            if rejection.run_id == "run-local-1"
                && rejection.attempt == 1
                && rejection.code == "approval_expired"
    ));
    assert_eq!(
        expired.run.terminal_outbound_sequence,
        expired.outbound.map(|envelope| envelope.sequence)
    );
    assert!(!expired.run.effect_started);
}

#[test]
fn mismatched_local_approval_decision_rolls_back_without_losing_the_inbox() {
    let temp = tempfile::tempdir().unwrap();
    let (_pairing, rail, hello) = ready_rail_with_offer(temp.path());
    let request = local_approval_request(50_000);
    let required = rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request.clone()), 31)
        .unwrap();
    let mut wrong = approval_decision_envelope(&hello, 3, &request, ApprovalDecision::Approved, 35);
    let HubNodeMessage::RunApprovalDecision(decision) = &mut wrong.message else {
        unreachable!();
    };
    decision.action_digest = approval_action_digest("file_read", b"different input");
    rail.observe_delivery(
        &batch(&hello, required.outbound.unwrap().sequence, vec![wrong]),
        40,
    )
    .unwrap();
    let before = rail.get_run("run-local-1", 1).unwrap().unwrap();

    assert!(matches!(
        rail.apply_run_approval_decision(3, 41),
        Err(NodeRailError::RunDecisionConflict)
    ));
    assert_eq!(rail.pending_inbound(10).unwrap().len(), 1);
    assert_eq!(rail.get_run("run-local-1", 1).unwrap().unwrap(), before);
}

#[test]
fn approval_transition_evidence_is_required_on_reopen() {
    let accepted_temp = tempfile::tempdir().unwrap();
    let (accepted_pairing, accepted_rail, accepted_hello) =
        ready_rail_with_offer(accepted_temp.path());
    let request = local_approval_request(50_000);
    let required = accepted_rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request.clone()), 31)
        .unwrap();
    accepted_rail
        .observe_delivery(
            &batch(
                &accepted_hello,
                required.outbound.unwrap().sequence,
                vec![approval_decision_envelope(
                    &accepted_hello,
                    3,
                    &request,
                    ApprovalDecision::Approved,
                    35,
                )],
            ),
            40,
        )
        .unwrap();
    let approved = accepted_rail.apply_run_approval_decision(3, 41).unwrap();
    let acceptance_sequence = approved.run.acceptance_outbound_sequence.unwrap();
    drop(accepted_rail);
    let database = rusqlite::Connection::open(accepted_temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute(
            "DELETE FROM node_rail_outbox WHERE sequence = ?1",
            [acceptance_sequence],
        )
        .unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&accepted_pairing),
        Err(NodeRailError::StateCorrupt)
    ));

    let denied_temp = tempfile::tempdir().unwrap();
    let (denied_pairing, denied_rail, denied_hello) = ready_rail_with_offer(denied_temp.path());
    let request = local_approval_request(50_000);
    let required = denied_rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request.clone()), 31)
        .unwrap();
    denied_rail
        .observe_delivery(
            &batch(
                &denied_hello,
                required.outbound.unwrap().sequence,
                vec![approval_decision_envelope(
                    &denied_hello,
                    3,
                    &request,
                    ApprovalDecision::Denied,
                    35,
                )],
            ),
            40,
        )
        .unwrap();
    denied_rail.apply_run_approval_decision(3, 41).unwrap();
    drop(denied_rail);
    let database = rusqlite::Connection::open(denied_temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute("DELETE FROM node_rail_inbox WHERE sequence = 3", [])
        .unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&denied_pairing),
        Err(NodeRailError::StateCorrupt)
    ));
}

#[test]
fn missing_run_decision_or_approval_evidence_fails_reopen() {
    let accepted_temp = tempfile::tempdir().unwrap();
    let (accepted_pairing, accepted_rail, _hello) = ready_rail_with_offer(accepted_temp.path());
    let accepted = accepted_rail
        .apply_run_offer(2, &NodeRunDisposition::Accept, 31)
        .unwrap();
    let decision_sequence = accepted.run.decision_outbound_sequence.unwrap();
    drop(accepted_rail);
    let database = rusqlite::Connection::open(accepted_temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute(
            "DELETE FROM node_rail_outbox WHERE sequence = ?1",
            [decision_sequence],
        )
        .unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&accepted_pairing),
        Err(NodeRailError::StateCorrupt)
    ));

    let approval_temp = tempfile::tempdir().unwrap();
    let (approval_pairing, approval_rail, _hello) = ready_rail_with_offer(approval_temp.path());
    let request = RunApprovalRequest {
        run_id: "run-local-1".to_string(),
        attempt: 1,
        approval_id: "approval-local-1".to_string(),
        action_digest: approval_action_digest("file_read", br#"{"path":"src/main.rs"}"#),
        action_summary: "Read workspace://project-main/src/main.rs".to_string(),
        risk_level: RiskLevel::Low,
        expires_at_ms: 50_000,
        path_policy_applied: true,
    };
    approval_rail
        .apply_run_offer(2, &NodeRunDisposition::RequireApproval(request), 31)
        .unwrap();
    drop(approval_rail);
    let database = rusqlite::Connection::open(approval_temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute("DELETE FROM node_run_approvals", [])
        .unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&approval_pairing),
        Err(NodeRailError::StateCorrupt)
    ));
}

#[test]
fn version_one_rail_is_migrated_without_losing_bootstrap_state() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let bootstrap = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    drop(rail);

    let database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute_batch(
            "DROP TABLE node_run_claims;
             DROP TABLE node_run_approvals;
             DROP TABLE node_runs;
             PRAGMA user_version = 1;",
        )
        .unwrap();
    drop(database);

    let migrated = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(
        migrated
            .bootstrap_hello(&capabilities(), &[], 20)
            .unwrap()
            .envelope,
        bootstrap
    );
    assert!(migrated.active_run_ids().unwrap().is_empty());
}

#[test]
fn explicit_supersession_is_persisted_in_sequence_and_auto_applied() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    let tombstone = HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: hello.device_id.clone(),
        connection_id: hello.connection_id.clone(),
        sequence: 1,
        ack_sequence: None,
        sent_at_ms: 20,
        message: HubNodeMessage::Superseded {
            original_message_kind: "welcome".to_string(),
            original_message_sha256: "a".repeat(64),
        },
    };
    let replacement = welcome(&hello, 2, NodeTransport::LongPoll);
    let delivery = batch(&hello, 1, vec![tombstone, replacement.clone()]);

    let outcome = rail.observe_delivery(&delivery, 30).unwrap();
    assert_eq!(outcome.newly_recorded, 2);
    assert!(outcome.acknowledgement_enqueued);
    assert_eq!(rail.snapshot().unwrap().last_hub_sequence, 2);
    assert_eq!(
        rail.pending_inbound(10).unwrap(),
        vec![NodeInboundRecord {
            envelope: replacement,
            received_at_ms: 30,
        }]
    );
    let pending = rail.pending_outbound(10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].ack_sequence, Some(2));
    assert!(matches!(pending[0].message, HubNodeMessage::AckOnly));

    drop(rail);
    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(restarted.snapshot().unwrap().last_hub_sequence, 2);
    assert_eq!(restarted.pending_inbound(10).unwrap().len(), 1);
}

#[test]
fn heartbeat_refresh_is_idempotent_for_the_same_active_run_set() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    rail.observe_delivery(
        &batch(&hello, 1, vec![welcome(&hello, 1, NodeTransport::LongPoll)]),
        20,
    )
    .unwrap();

    let first = rail
        .ensure_heartbeat(&["run-b".to_string(), "run-a".to_string()], 30)
        .unwrap();
    let replay = rail
        .ensure_heartbeat(&["run-a".to_string(), "run-b".to_string()], 31)
        .unwrap();
    assert_eq!(replay, first);
    assert!(matches!(
        &first.message,
        HubNodeMessage::Heartbeat { active_run_ids }
            if active_run_ids == &["run-a".to_string(), "run-b".to_string()]
    ));

    let changed = rail.ensure_heartbeat(&["run-c".to_string()], 32).unwrap();
    assert_eq!(changed.sequence, first.sequence + 1);
    assert_eq!(
        rail.pending_outbound(10)
            .unwrap()
            .into_iter()
            .filter(|envelope| matches!(envelope.message, HubNodeMessage::Heartbeat { .. }))
            .count(),
        2
    );
}

#[test]
fn crash_restart_preserves_unapplied_input_and_unacknowledged_output() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    let delivery = batch(&hello, 1, vec![welcome(&hello, 1, NodeTransport::LongPoll)]);
    rail.observe_delivery(&delivery, 20).unwrap();
    let outbound_before = rail.pending_outbound(10).unwrap();
    let inbound_before = rail.pending_inbound(10).unwrap();
    drop(rail);

    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(restarted.pending_outbound(10).unwrap(), outbound_before);
    assert_eq!(restarted.pending_inbound(10).unwrap(), inbound_before);
    assert_eq!(
        restarted
            .bootstrap_hello(&capabilities(), &[], 30)
            .unwrap()
            .envelope,
        hello
    );
}

#[test]
fn gaps_conflicting_replays_and_future_acknowledgements_roll_back() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;

    let gap = batch(
        &hello,
        1,
        vec![welcome(&hello, 2, NodeTransport::WebSocket)],
    );
    assert!(matches!(
        rail.observe_delivery(&gap, 20),
        Err(NodeRailError::SequenceGap)
    ));
    assert_eq!(rail.snapshot().unwrap().last_hub_sequence, 0);
    assert_eq!(rail.snapshot().unwrap().acknowledged_node_sequence, 0);

    let first = batch(
        &hello,
        1,
        vec![welcome(&hello, 1, NodeTransport::WebSocket)],
    );
    rail.observe_delivery(&first, 21).unwrap();
    let conflict = batch(&hello, 1, vec![welcome(&hello, 1, NodeTransport::LongPoll)]);
    assert!(matches!(
        rail.observe_delivery(&conflict, 22),
        Err(NodeRailError::ReplayConflict)
    ));
    assert!(matches!(
        rail.observe_delivery(&batch(&hello, 99, vec![]), 23),
        Err(NodeRailError::InvalidAcknowledgement)
    ));
    let snapshot = rail.snapshot().unwrap();
    assert_eq!(snapshot.last_hub_sequence, 1);
    assert_eq!(snapshot.acknowledged_node_sequence, 1);
    assert_eq!(snapshot.pending_outbound, 1);
}

#[test]
fn inbound_application_cannot_skip_delivery_order() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    let second = HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: hello.device_id.clone(),
        connection_id: hello.connection_id.clone(),
        sequence: 2,
        ack_sequence: None,
        sent_at_ms: 102,
        message: HubNodeMessage::ProtocolError {
            code: "maintenance".to_string(),
            message: "retry later".to_string(),
            retryable: true,
            path_policy_applied: true,
        },
    };
    rail.observe_delivery(
        &batch(
            &hello,
            1,
            vec![welcome(&hello, 1, NodeTransport::WebSocket), second],
        ),
        20,
    )
    .unwrap();
    assert!(matches!(
        rail.mark_inbound_applied(2, 21),
        Err(NodeRailError::ApplyOrderConflict)
    ));
    rail.mark_inbound_applied(1, 22).unwrap();
    rail.mark_inbound_applied(2, 23).unwrap();
}

#[test]
fn rail_identity_cannot_be_reused_after_pairing_state_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let pairing = paired_store(&root, "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    rail.bootstrap_hello(&capabilities(), &[], 10).unwrap();
    drop(rail);
    drop(pairing);

    let changed = paired_store(&root, "node-other", 'c');
    assert!(matches!(
        NodeRailStore::open(&changed),
        Err(NodeRailError::IdentityConflict)
    ));
}

#[test]
fn coordinated_reset_refuses_live_rail_then_removes_all_durable_state() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let pairing = paired_store(&root, "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    rail.bootstrap_hello(&capabilities(), &[], 10).unwrap();
    assert_eq!(pairing.reset(), Err(NodePairingError::StateInUse));
    drop(rail);

    let pairing = NodePairingStore::open(&root).unwrap();
    pairing.reset().unwrap();
    let clean = NodePairingStore::open(&root).unwrap();
    assert_eq!(clean.status().unwrap(), None);
    assert!(matches!(
        NodeRailStore::open(&clean),
        Err(NodeRailError::PairingRequired)
    ));
    assert!(!root.join("rail.sqlite3").exists());
}

#[cfg(unix)]
#[test]
fn rail_rejects_a_symlink_database() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let pairing = paired_store(&root, "node-office", 'b');
    let target = root.join("elsewhere.sqlite3");
    fs::write(&target, b"not-a-database").unwrap();
    symlink(&target, root.join("rail.sqlite3")).unwrap();
    assert!(matches!(
        NodeRailStore::open(&pairing),
        Err(NodeRailError::UnsafeStatePath)
    ));
}

#[test]
fn unsupported_protocol_metadata_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    drop(rail);
    let database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute("UPDATE node_rail_meta SET protocol_major = 99", [])
        .unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&pairing),
        Err(NodeRailError::StateCorrupt)
    ));
}

#[test]
fn rail_snapshot_is_serializable_without_exposing_the_hub_binding() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let snapshot = rail.snapshot().unwrap();
    let json = serde_json::to_string(&snapshot).unwrap();
    assert!(json.contains("node-office"));
    assert!(!json.contains(&"b".repeat(64)));
    assert!(!format!("{rail:?}").contains(temp.path().to_string_lossy().as_ref()));
}

#[test]
fn capability_rotation_waits_for_a_quiescent_rail_then_advances_once() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let original = rail.bootstrap_hello(&capabilities(), &[], 10).unwrap();

    let mut changed = capabilities();
    changed.tool_families.push("git".to_string());
    let deferred = rail.bootstrap_hello(&changed, &[], 11).unwrap();
    assert_eq!(
        deferred.capability_state,
        NodeBootstrapCapabilityState::RotationDeferred
    );
    assert_eq!(deferred.envelope, original.envelope);
    assert!(matches!(
        rail.enqueue(
            HubNodeMessage::Heartbeat {
                active_run_ids: vec![]
            },
            12
        ),
        Err(NodeRailError::ConnectionNotReady)
    ));
    assert_eq!(rail.snapshot().unwrap().last_node_sequence, 1);

    rail.observe_delivery(
        &batch(
            &original.envelope,
            1,
            vec![welcome(&original.envelope, 1, NodeTransport::WebSocket)],
        ),
        20,
    )
    .unwrap();
    rail.mark_inbound_applied(1, 21).unwrap();
    rail.observe_delivery(&batch(&original.envelope, 2, vec![]), 22)
        .unwrap();

    let rotated = rail.bootstrap_hello(&changed, &[], 23).unwrap();
    assert_eq!(
        rotated.capability_state,
        NodeBootstrapCapabilityState::Current
    );
    assert_eq!(rotated.envelope.sequence, 3);
    assert_eq!(rotated.envelope.ack_sequence, Some(1));
    assert_ne!(
        rotated.envelope.connection_id,
        original.envelope.connection_id
    );
    assert_eq!(
        rail.pending_outbound(10).unwrap(),
        vec![rotated.envelope.clone()]
    );

    rail.observe_delivery(
        &batch(
            &rotated.envelope,
            3,
            vec![welcome(&rotated.envelope, 2, NodeTransport::LongPoll)],
        ),
        24,
    )
    .unwrap();
    rail.mark_inbound_applied(2, 25).unwrap();
    rail.observe_delivery(&batch(&rotated.envelope, 4, vec![]), 26)
        .unwrap();
    drop(rail);

    let restarted = NodeRailStore::open(&pairing).unwrap();
    let resumed = restarted.bootstrap_hello(&changed, &[], 27).unwrap();
    assert_eq!(resumed.envelope, rotated.envelope);
    assert_eq!(
        resumed.capability_state,
        NodeBootstrapCapabilityState::Current
    );
}

#[test]
fn terminal_evidence_is_durable_and_debug_redacted_until_hub_acknowledgement() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    rail.observe_delivery(
        &batch(
            &hello,
            1,
            vec![welcome(&hello, 1, NodeTransport::WebSocket)],
        ),
        20,
    )
    .unwrap();
    rail.observe_delivery(&batch(&hello, 2, vec![]), 21)
        .unwrap();

    let secret_output = "terminal-output-that-must-not-appear-in-debug";
    let completion = RunCompletion {
        run_id: "run-1".to_string(),
        attempt: 1,
        status: RunTerminalStatus::Succeeded,
        result_content: secret_output.to_string(),
        result_sha256: hex::encode(Sha256::digest(secret_output.as_bytes())),
        total_output_bytes: secret_output.len() as u64,
        stored_output_bytes: secret_output.len() as u64,
        capped: false,
        redacted: false,
        path_policy_applied: true,
    };
    let durable = rail
        .enqueue(HubNodeMessage::RunCompleted(completion), 22)
        .unwrap();
    assert!(!format!("{durable:?}").contains(secret_output));
    drop(rail);

    let restarted = NodeRailStore::open(&pairing).unwrap();
    assert_eq!(restarted.pending_outbound(10).unwrap(), vec![durable]);
}

#[test]
fn reset_marker_completes_an_interrupted_reset_on_the_next_open() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("node");
    let pairing = paired_store(&root, "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    rail.bootstrap_hello(&capabilities(), &[], 10).unwrap();
    drop(rail);
    fs::write(root.join("reset.pending"), b"captain-node-reset-v1\n").unwrap();
    drop(pairing);

    let recovered = NodePairingStore::open(&root).unwrap();
    assert_eq!(recovered.status().unwrap(), None);
    assert!(!root.join("rail.sqlite3").exists());
    assert!(!root.join("reset.pending").exists());
}

#[test]
fn unsupported_database_schema_version_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    drop(rail);
    let database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    database.pragma_update(None, "user_version", 99).unwrap();
    drop(database);
    assert!(matches!(
        NodeRailStore::open(&pairing),
        Err(NodeRailError::StateVersionUnsupported)
    ));
}

#[test]
fn cumulative_ack_cannot_skip_a_missing_outbox_record() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    rail.observe_delivery(
        &batch(
            &hello,
            1,
            vec![welcome(&hello, 1, NodeTransport::WebSocket)],
        ),
        20,
    )
    .unwrap();
    let database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    database
        .execute("DELETE FROM node_rail_outbox WHERE sequence = 2", [])
        .unwrap();
    drop(database);

    assert!(matches!(
        rail.observe_delivery(&batch(&hello, 2, vec![]), 21),
        Err(NodeRailError::InvalidAcknowledgement)
    ));
    assert_eq!(rail.snapshot().unwrap().acknowledged_node_sequence, 1);
}

#[test]
fn bounded_outbox_fails_without_dropping_existing_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    rail.observe_delivery(
        &batch(
            &hello,
            1,
            vec![welcome(&hello, 1, NodeTransport::WebSocket)],
        ),
        20,
    )
    .unwrap();
    let mut database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    let transaction = database.transaction().unwrap();
    for sequence in 3..=4_096_i64 {
        transaction
            .execute(
                "INSERT INTO node_rail_outbox (
                     sequence, connection_id, message_kind, envelope_json,
                     envelope_sha256, hub_ack_sequence, created_at_ms
                 ) VALUES (?1, ?2, 'ack_only', X'00', ?3, 1, 20)",
                rusqlite::params![sequence, hello.connection_id, "0".repeat(64)],
            )
            .unwrap();
    }
    transaction
        .execute(
            "UPDATE node_rail_meta SET last_node_sequence = 4096 WHERE singleton = 1",
            [],
        )
        .unwrap();
    transaction.commit().unwrap();
    drop(database);
    assert!(matches!(
        rail.enqueue(HubNodeMessage::AckOnly, 21),
        Err(NodeRailError::OutboxFull)
    ));
    assert_eq!(rail.snapshot().unwrap().last_node_sequence, 4_096);
}

#[test]
fn bounded_inbox_rejects_delivery_without_acknowledging_it() {
    let temp = tempfile::tempdir().unwrap();
    let pairing = paired_store(temp.path(), "node-office", 'b');
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    rail.observe_delivery(
        &batch(
            &hello,
            1,
            vec![welcome(&hello, 1, NodeTransport::WebSocket)],
        ),
        20,
    )
    .unwrap();
    let mut database = rusqlite::Connection::open(temp.path().join("rail.sqlite3")).unwrap();
    let transaction = database.transaction().unwrap();
    for sequence in 2..=4_096_i64 {
        transaction
            .execute(
                "INSERT INTO node_rail_inbox (
                     sequence, connection_id, message_kind, envelope_json,
                     envelope_sha256, received_at_ms
                 ) VALUES (?1, ?2, 'protocol_error', X'00', ?3, 20)",
                rusqlite::params![sequence, hello.connection_id, "0".repeat(64)],
            )
            .unwrap();
    }
    transaction
        .execute(
            "UPDATE node_rail_meta SET last_hub_sequence = 4096 WHERE singleton = 1",
            [],
        )
        .unwrap();
    transaction.commit().unwrap();
    drop(database);
    let next = welcome(&hello, 4_097, NodeTransport::WebSocket);
    assert!(matches!(
        rail.observe_delivery(&batch(&hello, 1, vec![next]), 21),
        Err(NodeRailError::InboxFull)
    ));
    let snapshot = rail.snapshot().unwrap();
    assert_eq!(snapshot.last_hub_sequence, 4_096);
    assert_eq!(snapshot.acknowledged_node_sequence, 1);
}

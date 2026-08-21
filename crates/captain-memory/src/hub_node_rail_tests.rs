use crate::hub_node_rail::{HubNodeRailError, HubNodeRunStatus, InboundReceipt, NewHubNodeRun};
use crate::MemorySubstrate;
use captain_wire::hub_protocol::{HubNodeMessage, RunCompletion, RunEffect, RunTerminalStatus};
use serde_json::json;
use sha2::{Digest, Sha256};

fn memory_with_node() -> MemorySubstrate {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    insert_node(&memory, "node-1");
    memory
}

fn insert_node(memory: &MemorySubstrate, device_id: &str) {
    let conn = memory.usage_conn();
    let guard = conn.lock().unwrap();
    guard
        .execute(
            "INSERT INTO captain_devices (
                 device_id, display_name, role, platform, captain_version,
                 protocol_major, protocol_minor, credential_sha256,
                 capabilities_json, grants_json, status, paired_at_ms,
                 last_seen_ms, updated_at_ms
             ) VALUES (?1, 'Workstation', 'node', 'macos', 'alpha.14',
                       1, 0, ?2, '{}', '{}', 'active', 1, 1, 1)",
            rusqlite::params![device_id, "a".repeat(64)],
        )
        .unwrap();
}

fn run(run_id: &str, key: &str, effect: RunEffect, created_at_ms: i64) -> NewHubNodeRun {
    NewHubNodeRun {
        run_id: run_id.to_string(),
        device_id: "node-1".to_string(),
        idempotency_key: key.to_string(),
        workspace_id: "workspace-main".to_string(),
        tool_name: "shell_exec".to_string(),
        input: json!({"command": "printf secret-value"}),
        effect,
        created_at_ms,
    }
}

#[test]
fn enqueue_is_idempotent_and_rejects_changed_work() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    let request = run("run-1", "idem-1", RunEffect::ReadOnly, 10);

    let first = store.enqueue_run(&request).unwrap();
    let replay = store.enqueue_run(&request).unwrap();
    assert_eq!(first, replay);
    assert_eq!(first.status, HubNodeRunStatus::Queued);

    let mut changed = request.clone();
    changed.input = json!({"command": "different"});
    assert!(matches!(
        store.enqueue_run(&changed),
        Err(HubNodeRailError::IdempotencyConflict)
    ));

    let mut same_id = run("run-1", "idem-2", RunEffect::ReadOnly, 11);
    same_id.input = json!({"command": "other"});
    assert!(matches!(
        store.enqueue_run(&same_id),
        Err(HubNodeRailError::RunIdConflict)
    ));
}

#[test]
fn lease_and_offer_are_committed_together_and_ack_is_monotonic() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    store
        .enqueue_run(&run("run-1", "idem-1", RunEffect::ReadOnly, 10))
        .unwrap();

    let leased = store
        .lease_next("node-1", "connection-1", 20, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(leased.run.status, HubNodeRunStatus::Leased);
    assert_eq!(leased.lease.attempt, 1);
    assert_eq!(leased.outbox.sequence, 1);
    let message: HubNodeMessage = serde_json::from_str(&leased.outbox.message_json).unwrap();
    assert!(matches!(message, HubNodeMessage::RunOffer(offer) if offer == leased.lease));

    let pending = store.pending_outbox("node-1", 0, 10).unwrap();
    assert_eq!(pending, vec![leased.outbox]);
    store.acknowledge_hub_sequence("node-1", 1, 30).unwrap();
    store.acknowledge_hub_sequence("node-1", 1, 31).unwrap();
    assert!(store.pending_outbox("node-1", 0, 10).unwrap().is_empty());
    assert!(matches!(
        store.acknowledge_hub_sequence("node-1", 2, 32),
        Err(HubNodeRailError::InvalidAcknowledgement)
    ));
}

#[test]
fn acknowledgement_survives_a_wall_clock_rollback() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    store
        .enqueue_run(&run("run-1", "idem-1", RunEffect::ReadOnly, 10))
        .unwrap();
    store
        .lease_next("node-1", "connection-1", 20, 1_000)
        .unwrap()
        .unwrap();

    store.acknowledge_hub_sequence("node-1", 1, 19).unwrap();

    let conn = memory.usage_conn();
    let guard = conn.lock().unwrap();
    let acked_at_ms: i64 = guard
        .query_row(
            "SELECT acked_at_ms FROM hub_node_outbox
             WHERE device_id = 'node-1' AND sequence = 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let cursor_updated_at_ms: i64 = guard
        .query_row(
            "SELECT updated_at_ms FROM hub_node_cursors WHERE device_id = 'node-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(acked_at_ms, 20);
    assert_eq!(cursor_updated_at_ms, 20);
}

#[test]
fn targeted_lease_offers_exact_run_without_reordering_or_duplication() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    store
        .enqueue_run(&run("run-older", "idem-older", RunEffect::ReadOnly, 10))
        .unwrap();
    store
        .enqueue_run(&run("run-current", "idem-current", RunEffect::ReadOnly, 11))
        .unwrap();

    let leased = store
        .lease_run("node-1", "run-current", "connection-1", 20, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(leased.run.run_id, "run-current");
    assert_eq!(leased.lease.run_id, "run-current");
    assert_eq!(
        store.get_run("run-older").unwrap().unwrap().status,
        HubNodeRunStatus::Queued
    );

    assert!(store
        .lease_run("node-1", "run-current", "connection-1", 21, 1_000)
        .unwrap()
        .is_none());
    assert_eq!(store.pending_outbox("node-1", 0, 10).unwrap().len(), 1);
}

#[test]
fn durable_offer_survives_database_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory.db");
    {
        let memory = MemorySubstrate::open(&db, 0.01).unwrap();
        insert_node(&memory, "node-1");
        memory
            .hub_node_rail()
            .enqueue_run(&run("run-1", "idem-1", RunEffect::ReadOnly, 10))
            .unwrap();
        memory
            .hub_node_rail()
            .lease_next("node-1", "connection-1", 20, 1_000)
            .unwrap()
            .unwrap();
    }

    let reopened = MemorySubstrate::open(&db, 0.01).unwrap();
    let run = reopened.hub_node_rail().get_run("run-1").unwrap().unwrap();
    assert_eq!(run.status, HubNodeRunStatus::Leased);
    assert_eq!(
        reopened
            .hub_node_rail()
            .pending_outbox("node-1", 0, 10)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn inbound_sequences_accept_exact_replay_and_reject_gap_or_conflict() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    let digest = "b".repeat(64);

    assert_eq!(
        store
            .record_inbound_receipt("node-1", "connection-1", 1, "hello", &digest, 10)
            .unwrap(),
        InboundReceipt::Recorded
    );
    assert_eq!(
        store
            .record_inbound_receipt("node-1", "connection-2", 1, "hello", &digest, 11)
            .unwrap(),
        InboundReceipt::Duplicate
    );
    assert!(matches!(
        store.record_inbound_receipt("node-1", "connection-1", 1, "hello", &"c".repeat(64), 12,),
        Err(HubNodeRailError::ReplayConflict)
    ));
    assert!(matches!(
        store.record_inbound_receipt("node-1", "connection-1", 3, "heartbeat", &digest, 13),
        Err(HubNodeRailError::SequenceGap)
    ));
}

#[test]
fn restart_requeues_reads_but_never_blindly_replays_side_effects() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    store
        .enqueue_run(&run("read-run", "idem-read", RunEffect::ReadOnly, 10))
        .unwrap();
    store
        .enqueue_run(&run(
            "mutation-run",
            "idem-mutation",
            RunEffect::LocalMutation,
            11,
        ))
        .unwrap();
    store
        .enqueue_run(&run(
            "cancel-run",
            "idem-cancel",
            RunEffect::ExternalEffect,
            12,
        ))
        .unwrap();

    store
        .lease_next("node-1", "connection-1", 20, 10_000)
        .unwrap();
    store
        .lease_next("node-1", "connection-1", 21, 10_000)
        .unwrap();
    store
        .lease_next("node-1", "connection-1", 22, 10_000)
        .unwrap();
    store
        .request_cancel("cancel-run", "operator_request", 23)
        .unwrap();

    let summary = store.reconcile_after_restart(30).unwrap();
    assert_eq!(summary.requeued_read_only, 1);
    assert_eq!(summary.uncertain_side_effects, 1);
    assert_eq!(summary.cancelled_before_effect, 1);
    assert_eq!(
        store.get_run("read-run").unwrap().unwrap().status,
        HubNodeRunStatus::Queued
    );
    assert_eq!(
        store.get_run("mutation-run").unwrap().unwrap().status,
        HubNodeRunStatus::Uncertain
    );
    assert_eq!(
        store.get_run("cancel-run").unwrap().unwrap().status,
        HubNodeRunStatus::Cancelled
    );
}

#[test]
fn progress_and_terminal_evidence_are_idempotent() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    store
        .enqueue_run(&run("run-1", "idem-1", RunEffect::LocalMutation, 10))
        .unwrap();
    store
        .lease_next("node-1", "connection-1", 20, 10_000)
        .unwrap();
    store
        .mark_accepted("node-1", "run-1", 1, "connection-1", 21)
        .unwrap();
    store
        .record_progress("node-1", "run-1", 1, "connection-1", 1, "started", 22)
        .unwrap();
    store
        .record_progress("node-1", "run-1", 1, "connection-1", 1, "started", 23)
        .unwrap();
    assert!(matches!(
        store.record_progress("node-1", "run-1", 1, "connection-1", 1, "changed", 24),
        Err(HubNodeRailError::ReplayConflict)
    ));

    let result_content = "done";
    let completion = RunCompletion {
        run_id: "run-1".to_string(),
        attempt: 1,
        status: RunTerminalStatus::Succeeded,
        result_content: result_content.to_string(),
        result_sha256: format!("{:x}", Sha256::digest(result_content.as_bytes())),
        total_output_bytes: 4,
        stored_output_bytes: 4,
        capped: false,
        redacted: false,
        path_policy_applied: true,
    };
    let completed = store.complete_run("node-1", &completion, 25).unwrap();
    assert_eq!(completed.status, HubNodeRunStatus::Succeeded);
    assert_eq!(
        store.complete_run("node-1", &completion, 26).unwrap(),
        completed
    );

    let mut conflicting = completion;
    conflicting.status = RunTerminalStatus::Failed;
    assert!(matches!(
        store.complete_run("node-1", &conflicting, 27),
        Err(HubNodeRailError::TerminalConflict)
    ));
}

#[test]
fn late_terminal_evidence_reconciles_an_inferred_uncertain_run() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    store
        .enqueue_run(&run("run-1", "idem-1", RunEffect::ExternalEffect, 10))
        .unwrap();
    store
        .lease_next("node-1", "connection-1", 20, 10_000)
        .unwrap();
    store.reconcile_after_restart(21).unwrap();
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        HubNodeRunStatus::Uncertain
    );

    let result_content = "receipt";
    let completion = RunCompletion {
        run_id: "run-1".to_string(),
        attempt: 1,
        status: RunTerminalStatus::Succeeded,
        result_content: result_content.to_string(),
        result_sha256: format!("{:x}", Sha256::digest(result_content.as_bytes())),
        total_output_bytes: 7,
        stored_output_bytes: 7,
        capped: false,
        redacted: true,
        path_policy_applied: true,
    };
    assert_eq!(
        store
            .complete_run("node-1", &completion, 22)
            .unwrap()
            .status,
        HubNodeRunStatus::Succeeded
    );
}

#[test]
fn accepted_read_only_cancellation_closes_on_restart() {
    let memory = memory_with_node();
    let store = memory.hub_node_rail();
    store
        .enqueue_run(&run("run-1", "idem-1", RunEffect::ReadOnly, 10))
        .unwrap();
    store
        .lease_next("node-1", "connection-1", 20, 10_000)
        .unwrap();
    store
        .mark_accepted("node-1", "run-1", 1, "connection-1", 21)
        .unwrap();
    let cancellation = store
        .request_cancel("run-1", "operator_request", 22)
        .unwrap();
    assert_eq!(
        cancellation.outbox.as_ref().unwrap().message_kind,
        "cancel_run"
    );
    assert_eq!(
        store
            .request_cancel("run-1", "operator_request", 22)
            .unwrap()
            .outbox,
        cancellation.outbox
    );

    let summary = store.reconcile_after_restart(23).unwrap();
    assert_eq!(summary.cancelled_before_effect, 1);
    assert_eq!(
        store.get_run("run-1").unwrap().unwrap().status,
        HubNodeRunStatus::Cancelled
    );
}

#[test]
fn debug_output_redacts_tool_input_and_results() {
    let memory = memory_with_node();
    let record = memory
        .hub_node_rail()
        .enqueue_run(&run("run-1", "idem-1", RunEffect::ReadOnly, 10))
        .unwrap();
    let rendered = format!("{record:?}");
    assert!(!rendered.contains("secret-value"));
    assert!(rendered.contains("[REDACTED]"));
}

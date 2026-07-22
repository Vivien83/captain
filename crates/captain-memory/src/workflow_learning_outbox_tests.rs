use crate::workflow_learning_control::{NewWorkflowProposal, WorkflowLearningStore};
use crate::workflow_learning_outbox::{NewWorkflowOutboxItem, WorkflowOutboxStatus};
use crate::MemorySubstrate;

fn store() -> WorkflowLearningStore {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = WorkflowLearningStore::new(memory.usage_conn());
    store
        .create_observed(&NewWorkflowProposal {
            id: "proposal".to_string(),
            idempotency_key: "observe:proposal".to_string(),
            workflow_signature: "a".repeat(64),
            source_agent_id: "captain".to_string(),
            origin_channel: Some("telegram".to_string()),
            evidence_json: "{}".to_string(),
            created_at_unix_ms: 100,
        })
        .unwrap();
    store
}

fn outbox(id: &str, max_attempts: u32) -> NewWorkflowOutboxItem {
    NewWorkflowOutboxItem {
        id: id.to_string(),
        idempotency_key: format!("notify:{id}"),
        proposal_id: "proposal".to_string(),
        revision_sha256: None,
        topic: "workflow.proposed".to_string(),
        payload_json: r#"{"proposal_id":"proposal"}"#.to_string(),
        max_attempts,
        run_after_unix_ms: 1_000,
        created_at_unix_ms: 100,
    }
}

#[test]
fn outbox_enqueue_and_delivery_are_idempotent() {
    let store = store();
    let first = store.enqueue_outbox(&outbox("message-1", 3)).unwrap();
    let second = store.enqueue_outbox(&outbox("message-1", 3)).unwrap();
    assert_eq!(first, second);

    let claimed = store
        .claim_due_outbox("worker-1", 1_000, 5_000)
        .unwrap()
        .unwrap();
    assert_eq!(claimed.status, WorkflowOutboxStatus::Delivering);
    assert_eq!(claimed.attempt_count, 1);

    let completed = store
        .complete_outbox(
            "message-1",
            "worker-1",
            Some(r#"{"telegram_message_id":42}"#),
            1_200,
        )
        .unwrap();
    assert_eq!(completed.status, WorkflowOutboxStatus::Delivered);
    assert!(store
        .claim_due_outbox("worker-1", 10_000, 5_000)
        .unwrap()
        .is_none());
}

#[test]
fn failed_delivery_retries_then_becomes_dead() {
    let store = store();
    store.enqueue_outbox(&outbox("message-2", 2)).unwrap();
    store.claim_due_outbox("worker-1", 1_000, 5_000).unwrap();
    let retry = store
        .fail_outbox("message-2", "worker-1", "network unavailable", 2_000, 1_100)
        .unwrap();
    assert_eq!(retry.status, WorkflowOutboxStatus::RetryWait);

    store.claim_due_outbox("worker-1", 2_000, 5_000).unwrap();
    let dead = store
        .fail_outbox("message-2", "worker-1", "network unavailable", 3_000, 2_100)
        .unwrap();
    assert_eq!(dead.status, WorkflowOutboxStatus::Dead);
}

#[test]
fn deterministic_delivery_failure_is_dead_lettered_immediately() {
    let store = store();
    store.enqueue_outbox(&outbox("message-corrupt", 8)).unwrap();
    store.claim_due_outbox("worker-1", 1_000, 5_000).unwrap();

    let dead = store
        .dead_letter_outbox(
            "message-corrupt",
            "worker-1",
            "invalid payload schema",
            1_100,
        )
        .unwrap();

    assert_eq!(dead.status, WorkflowOutboxStatus::Dead);
    assert_eq!(dead.attempt_count, 1);
    assert_eq!(dead.last_error.as_deref(), Some("invalid payload schema"));
    assert!(dead.lease_owner.is_none());
    assert!(store
        .claim_due_outbox("worker-1", 10_000, 5_000)
        .unwrap()
        .is_none());
}

#[test]
fn expired_delivery_lease_reuses_the_same_outbox_item() {
    let store = store();
    store.enqueue_outbox(&outbox("message-3", 3)).unwrap();
    store
        .claim_due_outbox("crashed-worker", 1_000, 1_000)
        .unwrap();

    let summary = store.reconcile_expired_outbox(2_001).unwrap();
    assert_eq!(summary.retried, 1);
    let reclaimed = store
        .claim_due_outbox("replacement-worker", 2_001, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.id, "message-3");
    assert_eq!(reclaimed.idempotency_key, "notify:message-3");
    assert_eq!(reclaimed.attempt_count, 2);
}

#[test]
fn restart_immediately_reclaims_an_unexpired_delivery_lease() {
    let store = store();
    store.enqueue_outbox(&outbox("message-restart", 3)).unwrap();
    store
        .claim_due_outbox("crashed-worker", 1_000, 10_000)
        .unwrap();

    let summary = store.reconcile_outbox_after_restart(1_100).unwrap();
    assert_eq!(summary.retried, 1);
    let reclaimed = store
        .claim_due_outbox("replacement-worker", 1_100, 1_000)
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.id, "message-restart");
    assert_eq!(reclaimed.idempotency_key, "notify:message-restart");
    assert_eq!(reclaimed.attempt_count, 2);
}

#[test]
fn reused_outbox_key_with_changed_payload_is_rejected() {
    let store = store();
    store.enqueue_outbox(&outbox("message-4", 3)).unwrap();
    let mut changed = outbox("different-id", 3);
    changed.idempotency_key = "notify:message-4".to_string();
    changed.payload_json = r#"{"different":true}"#.to_string();
    assert!(store.enqueue_outbox(&changed).is_err());
}

#[test]
fn expired_delivery_lease_cannot_be_completed_as_live() {
    let store = store();
    store.enqueue_outbox(&outbox("message-5", 3)).unwrap();
    store.claim_due_outbox("worker", 1_000, 1_000).unwrap();
    assert!(store
        .complete_outbox("message-5", "worker", Some("{}"), 2_001)
        .is_err());
    assert_eq!(store.reconcile_expired_outbox(2_001).unwrap().retried, 1);
}

#[test]
fn topic_scoped_claim_selects_the_oldest_matching_item_only() {
    let store = store();
    let mut unrelated = outbox("unrelated", 3);
    unrelated.topic = "workflow.unrelated".to_string();
    unrelated.run_after_unix_ms = 500;
    unrelated.created_at_unix_ms = 50;
    store.enqueue_outbox(&unrelated).unwrap();

    let mut lifecycle = outbox("lifecycle", 3);
    lifecycle.topic = "workflow.lifecycle".to_string();
    lifecycle.run_after_unix_ms = 600;
    lifecycle.created_at_unix_ms = 60;
    store.enqueue_outbox(&lifecycle).unwrap();

    let mut proposed = outbox("proposed", 3);
    proposed.topic = "workflow.proposed".to_string();
    proposed.run_after_unix_ms = 700;
    proposed.created_at_unix_ms = 70;
    store.enqueue_outbox(&proposed).unwrap();

    let claimed = store
        .claim_due_outbox_for_topics(
            "worker",
            &["workflow.proposed", "workflow.lifecycle"],
            1_000,
            5_000,
        )
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, "lifecycle");
    let unrelated = store.get_outbox("unrelated").unwrap().unwrap();
    assert_eq!(unrelated.status, WorkflowOutboxStatus::Pending);
    assert_eq!(unrelated.attempt_count, 0);
}

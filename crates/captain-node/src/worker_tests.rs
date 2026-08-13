use super::*;
use crate::{
    NodeExecutionPolicyError, NodePairingProgress, NodePairingStore, NodeRunStatus,
    NodeWorkspaceBinding,
};
use captain_types::approval::{approval_action_digest, ApprovalDecision};
use captain_wire::{
    hub_protocol::RunApprovalDecision, CapabilityDescriptor, DeviceGrant, HubNodeDeliveryBatch,
    HubNodeEnvelope, LogicalWorkspace, NodeTransport, HUB_NODE_PROTOCOL_VERSION,
};
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;

struct FakeDriver {
    approval_required: AtomicBool,
    digest_override: Mutex<Option<String>>,
    hold: AtomicBool,
    release: Notify,
    started: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    approved_digests: Mutex<Vec<Option<String>>>,
    output: Mutex<NodeToolExecutionOutput>,
}

impl FakeDriver {
    fn new(approval_required: bool, hold: bool) -> Arc<Self> {
        Arc::new(Self {
            approval_required: AtomicBool::new(approval_required),
            digest_override: Mutex::new(None),
            hold: AtomicBool::new(hold),
            release: Notify::new(),
            started: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            approved_digests: Mutex::new(Vec::new()),
            output: Mutex::new(
                NodeToolExecutionOutput::new(
                    true,
                    "workspace://project-main/result",
                    31,
                    false,
                    false,
                )
                .unwrap(),
            ),
        })
    }

    fn set_digest(&self, digest: Option<String>) {
        *self.digest_override.lock().unwrap() = digest;
    }

    fn release_all(&self) {
        self.hold.store(false, Ordering::SeqCst);
        self.release.notify_waiters();
    }

    fn digest_for(&self, lease: &RunLease) -> String {
        self.digest_override
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| {
                approval_action_digest(&lease.tool_name, &serde_json::to_vec(&lease.input).unwrap())
            })
    }
}

struct ActiveExecution(Arc<FakeDriver>);

impl Drop for ActiveExecution {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl NodeToolDriver for FakeDriver {
    fn review(&self, lease: &RunLease) -> Result<NodeToolReview, RunRejection> {
        let family = if lease.tool_name == "shell_exec" {
            "shell-process"
        } else {
            "file"
        };
        let reviewed = NodeReviewedTool::new(&lease.tool_name, family, lease.effect)
            .map_err(|_| rejection(lease, "invalid_review"))?;
        NodeToolReview::new(
            reviewed,
            self.digest_for(lease),
            self.approval_required.load(Ordering::SeqCst),
            if lease.effect == RunEffect::ExternalEffect {
                RiskLevel::High
            } else {
                RiskLevel::Low
            },
            format!(
                "Run {} in workspace://{}",
                lease.tool_name, lease.workspace_id
            ),
        )
        .map_err(|_| rejection(lease, "invalid_review"))
    }

    fn execute(
        self: Arc<Self>,
        _run: AuthorizedNodeRun,
        approved_action_digest: Option<String>,
        cancellation: NodeRunCancellation,
    ) -> BoxFuture<'static, NodeToolExecutionOutput> {
        Box::pin(async move {
            self.approved_digests
                .lock()
                .unwrap()
                .push(approved_action_digest);
            self.started.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            let _active = ActiveExecution(Arc::clone(&self));
            if self.hold.load(Ordering::SeqCst) {
                tokio::select! {
                    () = self.release.notified() => {},
                    () = cancellation.requested() => {},
                }
            }
            self.output.lock().unwrap().clone()
        })
    }
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

fn paired_store(root: &Path) -> NodePairingStore {
    let store = NodePairingStore::open(root).unwrap();
    let state = serde_json::json!({
        "schema_version": 1,
        "hub_sha256": "b".repeat(64),
        "phase": {
            "state": "paired",
            "credential": "a".repeat(64),
            "device_id": "node-office",
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
            device_id: "node-office".to_string(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
        })
    );
    store
}

fn capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "test".to_string(),
        transports: vec![NodeTransport::LongPoll],
        tool_families: vec!["file".to_string(), "shell-process".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "project-main".to_string(),
            label: "Main Project".to_string(),
            read_only: false,
        }],
        supports_streaming_output: true,
    }
}

fn batch(
    hello: &HubNodeEnvelope,
    acknowledged_node_sequence: u64,
    messages: Vec<HubNodeEnvelope>,
) -> HubNodeDeliveryBatch {
    HubNodeDeliveryBatch {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: hello.device_id.clone(),
        connection_id: hello.connection_id.clone(),
        acknowledged_node_sequence,
        messages,
        retry_after_ms: None,
    }
}

fn envelope(hello: &HubNodeEnvelope, sequence: u64, message: HubNodeMessage) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: hello.device_id.clone(),
        connection_id: hello.connection_id.clone(),
        sequence,
        ack_sequence: None,
        sent_at_ms: now_ms(),
        message,
    }
}

fn lease(index: usize, effect: RunEffect) -> RunLease {
    let tool_name = match effect {
        RunEffect::ReadOnly => "file_read",
        RunEffect::LocalMutation => "file_write",
        RunEffect::ExternalEffect => "shell_exec",
    };
    RunLease {
        run_id: format!("run-local-{index}"),
        attempt: 1,
        idempotency_key: format!("idem-local-{index}"),
        workspace_id: "project-main".to_string(),
        tool_name: tool_name.to_string(),
        input: match effect {
            RunEffect::ReadOnly => serde_json::json!({"path": "README.md"}),
            RunEffect::LocalMutation => {
                serde_json::json!({"path": "output.txt", "content": "ok"})
            }
            RunEffect::ExternalEffect => serde_json::json!({"command": "git status"}),
        },
        effect,
        lease_expires_at_ms: now_ms() + 60_000,
    }
}

fn ready_rail(root: &Path, leases: Vec<RunLease>) -> (NodeRailStore, HubNodeEnvelope) {
    let pairing = paired_store(root);
    let rail = NodeRailStore::open(&pairing).unwrap();
    let hello = rail
        .bootstrap_hello(&capabilities(), &[], 10)
        .unwrap()
        .envelope;
    let welcome = envelope(
        &hello,
        1,
        HubNodeMessage::Welcome {
            negotiated_version: HUB_NODE_PROTOCOL_VERSION,
            transport: NodeTransport::LongPoll,
            heartbeat_interval_ms: 15_000,
            lease_duration_ms: 60_000,
        },
    );
    rail.observe_delivery(&batch(&hello, hello.sequence, vec![welcome]), 20)
        .unwrap();
    rail.mark_inbound_applied(1, 21).unwrap();
    let offers = leases
        .into_iter()
        .enumerate()
        .map(|(index, lease)| {
            envelope(
                &hello,
                u64::try_from(index).unwrap() + 2,
                HubNodeMessage::RunOffer(lease),
            )
        })
        .collect();
    rail.observe_delivery(&batch(&hello, hello.sequence, offers), now_ms())
        .unwrap();
    (rail, hello)
}

fn policy(
    root: &Path,
    grant_workspace: bool,
    allow_mutation: bool,
) -> Result<NodeExecutionPolicy, NodeExecutionPolicyError> {
    NodeExecutionPolicy::new(
        DeviceGrant {
            workspace_ids: grant_workspace
                .then(|| "project-main".to_string())
                .into_iter()
                .collect(),
            tool_families: vec!["file".to_string(), "shell-process".to_string()],
            allow_mutation,
        },
        [NodeWorkspaceBinding::new("project-main", root, false)?],
    )
}

fn rejection(lease: &RunLease, code: &str) -> RunRejection {
    RunRejection {
        run_id: lease.run_id.clone(),
        attempt: lease.attempt,
        code: code.to_string(),
        message: "The fake runtime review failed closed".to_string(),
        retryable: false,
        path_policy_applied: true,
    }
}

fn acceptance_sequence(rail: &NodeRailStore, run_id: &str) -> u64 {
    rail.get_run(run_id, 1)
        .unwrap()
        .unwrap()
        .acceptance_outbound_sequence
        .unwrap()
}

async fn wait_started(driver: &FakeDriver, expected: usize) {
    for _ in 0..100 {
        if driver.started.load(Ordering::SeqCst) >= expected {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("worker did not start {expected} execution(s)");
}

async fn drive_until_terminal(worker: &mut NodeWorker<FakeDriver>, run_id: &str, start_ms: i64) {
    for offset in 0..100 {
        tokio::task::yield_now().await;
        worker.advance(start_ms + offset).await.unwrap();
        if worker
            .rail()
            .get_run(run_id, 1)
            .unwrap()
            .is_some_and(|run| run.status.is_terminal())
        {
            return;
        }
    }
    panic!("run did not reach a terminal state");
}

fn approval_request(rail: &NodeRailStore) -> RunApprovalRequest {
    rail.pending_outbound(256)
        .unwrap()
        .into_iter()
        .find_map(|envelope| match envelope.message {
            HubNodeMessage::RunApprovalRequired(request) => Some(request),
            _ => None,
        })
        .expect("approval request")
}

fn approve(
    rail: &NodeRailStore,
    hello: &HubNodeEnvelope,
    request: &RunApprovalRequest,
    decided_at_ms: i64,
) {
    let decision = RunApprovalDecision {
        run_id: request.run_id.clone(),
        attempt: request.attempt,
        approval_id: request.approval_id.clone(),
        action_digest: request.action_digest.clone(),
        decision: ApprovalDecision::Approved,
        reason: Some("Exact action approved".to_string()),
        decided_at_ms,
    };
    rail.observe_delivery(
        &batch(
            hello,
            rail.pending_outbound(256)
                .unwrap()
                .into_iter()
                .map(|envelope| envelope.sequence)
                .max()
                .unwrap(),
            vec![envelope(
                hello,
                rail.snapshot().unwrap().last_hub_sequence + 1,
                HubNodeMessage::RunApprovalDecision(decision),
            )],
        ),
        decided_at_ms,
    )
    .unwrap();
}

#[tokio::test]
async fn direct_run_waits_for_hub_ack_and_commits_exact_completion() {
    let temp = tempfile::tempdir().unwrap();
    let now = now_ms();
    let (rail, hello) = ready_rail(temp.path(), vec![lease(1, RunEffect::ReadOnly)]);
    let driver = FakeDriver::new(false, false);
    let mut worker = NodeWorker::new(
        rail.clone(),
        policy(temp.path(), true, false).unwrap(),
        Arc::clone(&driver),
    );

    let intake = worker.advance(now).await.unwrap();
    assert_eq!(intake.applied_inbound, 1);
    assert_eq!(intake.launched, 0);
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
    assert_eq!(rail.approved_action_digest("run-local-1", 1).unwrap(), None);

    let accepted = acceptance_sequence(&rail, "run-local-1");
    rail.observe_delivery(&batch(&hello, accepted, vec![]), now + 1)
        .unwrap();
    assert_eq!(worker.advance(now + 2).await.unwrap().launched, 1);
    drive_until_terminal(&mut worker, "run-local-1", now + 3).await;

    let run = rail.get_run("run-local-1", 1).unwrap().unwrap();
    assert_eq!(run.status, NodeRunStatus::Succeeded);
    let completion = rail
        .pending_outbound(256)
        .unwrap()
        .into_iter()
        .find_map(|envelope| match envelope.message {
            HubNodeMessage::RunCompleted(completion) => Some(completion),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        completion.result_sha256,
        hex::encode(Sha256::digest(completion.result_content.as_bytes()))
    );
    assert_eq!(completion.stored_output_bytes, 31);
    assert_eq!(driver.approved_digests.lock().unwrap().as_slice(), &[None]);
}

#[tokio::test]
async fn approval_binds_the_exact_digest_and_drift_fails_before_effect() {
    let temp = tempfile::tempdir().unwrap();
    let now = now_ms();
    let (rail, hello) = ready_rail(temp.path(), vec![lease(1, RunEffect::ExternalEffect)]);
    let driver = FakeDriver::new(true, false);
    let mut worker = NodeWorker::new(
        rail.clone(),
        policy(temp.path(), true, true).unwrap(),
        Arc::clone(&driver),
    );
    worker.advance(now).await.unwrap();
    let request = approval_request(&rail);
    approve(&rail, &hello, &request, now + 1);
    worker.advance(now + 2).await.unwrap();
    assert_eq!(
        rail.approved_action_digest("run-local-1", 1).unwrap(),
        Some(request.action_digest.clone())
    );
    let accepted = acceptance_sequence(&rail, "run-local-1");
    rail.observe_delivery(&batch(&hello, accepted, vec![]), now + 3)
        .unwrap();
    assert_eq!(worker.advance(now + 4).await.unwrap().launched, 1);
    drive_until_terminal(&mut worker, "run-local-1", now + 5).await;
    assert_eq!(
        driver.approved_digests.lock().unwrap().as_slice(),
        &[Some(request.action_digest)]
    );

    let drift_temp = tempfile::tempdir().unwrap();
    let (drift_rail, drift_hello) =
        ready_rail(drift_temp.path(), vec![lease(1, RunEffect::ExternalEffect)]);
    let drift_driver = FakeDriver::new(true, false);
    let mut drift_worker = NodeWorker::new(
        drift_rail.clone(),
        policy(drift_temp.path(), true, true).unwrap(),
        Arc::clone(&drift_driver),
    );
    drift_worker.advance(now).await.unwrap();
    let drift_request = approval_request(&drift_rail);
    approve(&drift_rail, &drift_hello, &drift_request, now + 1);
    drift_worker.advance(now + 2).await.unwrap();
    drift_driver.set_digest(Some("f".repeat(64)));
    let accepted = acceptance_sequence(&drift_rail, "run-local-1");
    drift_rail
        .observe_delivery(&batch(&drift_hello, accepted, vec![]), now + 3)
        .unwrap();
    let cycle = drift_worker.advance(now + 4).await.unwrap();
    assert_eq!(cycle.rejected_before_effect, 1);
    assert_eq!(drift_driver.started.load(Ordering::SeqCst), 0);
    assert_eq!(
        drift_rail
            .get_run("run-local-1", 1)
            .unwrap()
            .unwrap()
            .status,
        NodeRunStatus::Rejected
    );
}

async fn cancellation_case(effect: RunEffect, expected: NodeRunStatus) {
    let temp = tempfile::tempdir().unwrap();
    let now = now_ms();
    let (rail, hello) = ready_rail(temp.path(), vec![lease(1, effect)]);
    let driver = FakeDriver::new(false, true);
    let mut worker = NodeWorker::new(
        rail.clone(),
        policy(temp.path(), true, effect != RunEffect::ReadOnly).unwrap(),
        Arc::clone(&driver),
    );
    worker.advance(now).await.unwrap();
    if effect == RunEffect::ExternalEffect {
        let request = approval_request(&rail);
        approve(&rail, &hello, &request, now + 1);
        worker.advance(now + 2).await.unwrap();
    }
    let accepted = acceptance_sequence(&rail, "run-local-1");
    rail.observe_delivery(&batch(&hello, accepted, vec![]), now + 3)
        .unwrap();
    worker.advance(now + 4).await.unwrap();
    wait_started(&driver, 1).await;
    let cancel = envelope(
        &hello,
        rail.snapshot().unwrap().last_hub_sequence + 1,
        HubNodeMessage::CancelRun {
            run_id: "run-local-1".to_string(),
            attempt: 1,
            reason: "Operator cancelled exact run".to_string(),
        },
    );
    rail.observe_delivery(&batch(&hello, accepted, vec![cancel]), now + 5)
        .unwrap();
    let cycle = worker.advance(now + 6).await.unwrap();
    assert_eq!(cycle.cancelled, 1);
    assert_eq!(
        rail.get_run("run-local-1", 1).unwrap().unwrap().status,
        expected
    );
    assert_eq!(worker.running_len(), 0);
}

#[tokio::test]
async fn cooperative_cancellation_is_cancelled_for_reads_and_uncertain_for_effects() {
    cancellation_case(RunEffect::ReadOnly, NodeRunStatus::Cancelled).await;
    cancellation_case(RunEffect::LocalMutation, NodeRunStatus::Uncertain).await;
    cancellation_case(RunEffect::ExternalEffect, NodeRunStatus::Uncertain).await;
}

#[tokio::test]
async fn queued_cancellation_wins_over_an_uncommitted_finished_result() {
    let temp = tempfile::tempdir().unwrap();
    let now = now_ms();
    let (rail, hello) = ready_rail(temp.path(), vec![lease(1, RunEffect::ReadOnly)]);
    let driver = FakeDriver::new(false, false);
    let mut worker = NodeWorker::new(
        rail.clone(),
        policy(temp.path(), true, false).unwrap(),
        Arc::clone(&driver),
    );
    worker.advance(now).await.unwrap();
    let accepted = acceptance_sequence(&rail, "run-local-1");
    rail.observe_delivery(&batch(&hello, accepted, vec![]), now + 1)
        .unwrap();
    worker.advance(now + 2).await.unwrap();
    wait_started(&driver, 1).await;
    tokio::task::yield_now().await;

    let cancel = envelope(
        &hello,
        rail.snapshot().unwrap().last_hub_sequence + 1,
        HubNodeMessage::CancelRun {
            run_id: "run-local-1".to_string(),
            attempt: 1,
            reason: "Cancellation beats local collection".to_string(),
        },
    );
    rail.observe_delivery(&batch(&hello, accepted, vec![cancel]), now + 3)
        .unwrap();
    worker.advance(now + 4).await.unwrap();
    assert_eq!(
        rail.get_run("run-local-1", 1).unwrap().unwrap().status,
        NodeRunStatus::Cancelled
    );
    assert!(!rail
        .pending_outbound(256)
        .unwrap()
        .iter()
        .any(|envelope| matches!(
            &envelope.message,
            HubNodeMessage::RunCompleted(completion)
                if completion.status == RunTerminalStatus::Succeeded
        )));
}

#[tokio::test]
async fn restart_rechecks_current_policy_before_any_effect() {
    let temp = tempfile::tempdir().unwrap();
    let now = now_ms();
    let (rail, hello) = ready_rail(temp.path(), vec![lease(1, RunEffect::ReadOnly)]);
    let driver = FakeDriver::new(false, false);
    let mut intake_worker = NodeWorker::new(
        rail.clone(),
        policy(temp.path(), true, false).unwrap(),
        Arc::clone(&driver),
    );
    intake_worker.advance(now).await.unwrap();
    drop(intake_worker);
    let accepted = acceptance_sequence(&rail, "run-local-1");
    rail.observe_delivery(&batch(&hello, accepted, vec![]), now + 1)
        .unwrap();

    let mut restarted = NodeWorker::new(
        rail.clone(),
        policy(temp.path(), false, false).unwrap(),
        Arc::clone(&driver),
    );
    let cycle = restarted.advance(now + 2).await.unwrap();
    assert_eq!(cycle.rejected_before_effect, 1);
    assert_eq!(driver.started.load(Ordering::SeqCst), 0);
    let run = rail.get_run("run-local-1", 1).unwrap().unwrap();
    assert_eq!(run.status, NodeRunStatus::Rejected);
    assert!(!run.effect_started);
}

#[tokio::test]
async fn worker_parallelism_is_bounded_and_claim_order_is_deterministic() {
    let temp = tempfile::tempdir().unwrap();
    let now = now_ms();
    let leases = (1..=6)
        .map(|index| lease(index, RunEffect::ReadOnly))
        .collect();
    let (rail, hello) = ready_rail(temp.path(), leases);
    let driver = FakeDriver::new(false, true);
    let mut worker = NodeWorker::new(
        rail.clone(),
        policy(temp.path(), true, false).unwrap(),
        Arc::clone(&driver),
    );
    let intake = worker.advance(now).await.unwrap();
    assert_eq!(intake.applied_inbound, 6);
    let last_acceptance = (1..=6)
        .map(|index| acceptance_sequence(&rail, &format!("run-local-{index}")))
        .max()
        .unwrap();
    rail.observe_delivery(&batch(&hello, last_acceptance, vec![]), now + 1)
        .unwrap();
    assert_eq!(worker.advance(now + 2).await.unwrap().launched, 4);
    wait_started(&driver, 4).await;
    assert_eq!(worker.running_len(), 4);
    assert_eq!(driver.max_active.load(Ordering::SeqCst), 4);

    driver.release_all();
    for offset in 0..100 {
        tokio::task::yield_now().await;
        worker.advance(now + 3 + offset).await.unwrap();
        if (1..=6).all(|index| {
            rail.get_run(&format!("run-local-{index}"), 1)
                .unwrap()
                .unwrap()
                .status
                .is_terminal()
        }) {
            break;
        }
    }
    assert_eq!(driver.started.load(Ordering::SeqCst), 6);
    assert!(driver.max_active.load(Ordering::SeqCst) <= 4);
    assert!((1..=6).all(|index| {
        rail.get_run(&format!("run-local-{index}"), 1)
            .unwrap()
            .unwrap()
            .status
            == NodeRunStatus::Succeeded
    }));
}

#[tokio::test]
async fn dropping_worker_aborts_live_tasks_without_fabricating_terminal_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let now = now_ms();
    let (rail, hello) = ready_rail(temp.path(), vec![lease(1, RunEffect::ReadOnly)]);
    let driver = FakeDriver::new(false, true);
    let mut worker = NodeWorker::new(
        rail.clone(),
        policy(temp.path(), true, false).unwrap(),
        Arc::clone(&driver),
    );
    worker.advance(now).await.unwrap();
    let accepted = acceptance_sequence(&rail, "run-local-1");
    rail.observe_delivery(&batch(&hello, accepted, vec![]), now + 1)
        .unwrap();
    worker.advance(now + 2).await.unwrap();
    wait_started(&driver, 1).await;
    assert_eq!(driver.active.load(Ordering::SeqCst), 1);

    drop(worker);
    tokio::time::timeout(Duration::from_millis(200), async {
        while driver.active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping a worker must abort its detached local effects");

    let run = rail.get_run("run-local-1", 1).unwrap().unwrap();
    assert!(run.effect_started);
    assert!(!run.status.is_terminal());
}

#[test]
fn review_and_output_contracts_reject_raw_paths_and_hide_content() {
    let reviewed = NodeReviewedTool::new("file_read", "file", RunEffect::ReadOnly).unwrap();
    assert!(matches!(
        NodeToolReview::new(
            reviewed.clone(),
            "short",
            false,
            RiskLevel::Low,
            "Read workspace://project-main/README.md",
        ),
        Err(NodeWorkerError::DriverContract)
    ));
    assert!(matches!(
        NodeToolReview::new(
            reviewed.clone(),
            "a".repeat(64),
            false,
            RiskLevel::Low,
            "Read /Users/private/project/README.md",
        ),
        Err(NodeWorkerError::DriverContract)
    ));
    for raw in [
        "path=/srv/private/result",
        r#"{\"path\":\"/tmp/private\"}"#,
        r"C:\\Users\\private\\result",
        r"\\server\\private\\result",
    ] {
        assert!(matches!(
            NodeToolExecutionOutput::new(false, raw, raw.len() as u64, false, true),
            Err(NodeWorkerError::DriverContract)
        ));
    }
    assert!(matches!(
        NodeToolExecutionOutput::new(true, "short", 10, false, false),
        Err(NodeWorkerError::DriverContract)
    ));
    assert!(matches!(
        NodeToolExecutionOutput::new(true, "complete", 8, true, false),
        Err(NodeWorkerError::DriverContract)
    ));

    let review = NodeToolReview::new(
        reviewed,
        "a".repeat(64),
        false,
        RiskLevel::Low,
        "Read workspace://project-main/README.md",
    )
    .unwrap();
    let output =
        NodeToolExecutionOutput::new(true, "workspace://project-main/README.md", 34, false, true)
            .unwrap();
    assert!(!format!("{review:?}").contains("README.md"));
    assert!(!format!("{output:?}").contains("README.md"));
}

#[tokio::test]
async fn cancellation_signal_cannot_be_lost_before_a_driver_waits() {
    let cancellation = NodeRunCancellation::default();
    cancellation.request();
    tokio::time::timeout(Duration::from_millis(50), cancellation.requested())
        .await
        .expect("preexisting cancellation must be observed");
    assert!(cancellation.is_requested());
}

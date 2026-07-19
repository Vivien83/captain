use super::*;
use crate::{CapabilityExecutionAuthority, CapabilityScope, CapabilityStatus};
use async_trait::async_trait;
use rusqlite::Connection;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::oneshot;

struct Fixture {
    _temp: TempDir,
    global: std::path::PathBuf,
    database: std::path::PathBuf,
    key: std::path::PathBuf,
    registry: Arc<CapabilityRegistry>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let global = temp.path().join("capabilities");
        let database = temp.path().join("capabilities.db");
        let key = temp.path().join("capabilities.key");
        let registry = Arc::new(CapabilityRegistry::open(&global, &database).unwrap());
        Self {
            _temp: temp,
            global,
            database,
            key,
            registry,
        }
    }

    fn install(&self, source: &str) -> ResolvedCapability {
        let compiled = crate::compile(source).unwrap();
        std::fs::write(
            self.global.join(format!("{}.captain", compiled.name)),
            source,
        )
        .unwrap();
        self.registry.reload_global().unwrap();
        let view = self
            .registry
            .list()
            .unwrap()
            .into_iter()
            .find(|view| view.name == compiled.name)
            .unwrap();
        if matches!(
            view.status,
            CapabilityStatus::PendingApproval | CapabilityStatus::UpdatePendingApproval
        ) {
            self.registry
                .approve(
                    &CapabilityScope::Global,
                    &compiled.name,
                    view.pending_hash.as_deref().unwrap(),
                    "test-operator",
                )
                .unwrap();
        }
        self.registry
            .resolved_by_tool(&compiled.tool_name, None)
            .unwrap()
            .unwrap()
    }

    fn executor(&self) -> CapabilityExecutor {
        CapabilityExecutor::open(self.registry.clone(), &self.database, &self.key).unwrap()
    }
}

#[derive(Default)]
struct ScriptedInvoker {
    calls: Mutex<Vec<CapabilityInvocation>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
    fail_first: AtomicBool,
    delay_ms: u64,
}

impl ScriptedInvoker {
    fn delayed(delay_ms: u64) -> Self {
        Self {
            delay_ms,
            ..Self::default()
        }
    }

    fn flaky() -> Self {
        Self {
            fail_first: AtomicBool::new(true),
            ..Self::default()
        }
    }
}

#[async_trait]
impl CapabilityToolInvoker for ScriptedInvoker {
    async fn invoke(&self, invocation: CapabilityInvocation) -> CapabilityInvocationResult {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        self.calls.lock().unwrap().push(invocation.clone());
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.fail_first.swap(false, Ordering::SeqCst) {
            CapabilityInvocationResult::error("transient")
        } else {
            CapabilityInvocationResult::success(
                json!({
                    "step": invocation.step_id,
                    "attempt": invocation.attempt,
                    "input": invocation.input,
                })
                .to_string(),
            )
        }
    }
}

struct BlockingInvoker {
    started: Mutex<Option<oneshot::Sender<CapabilityInvocation>>>,
}

#[async_trait]
impl CapabilityToolInvoker for BlockingInvoker {
    async fn invoke(&self, invocation: CapabilityInvocation) -> CapabilityInvocationResult {
        if let Some(started) = self.started.lock().unwrap().take() {
            let _ = started.send(invocation);
        }
        std::future::pending().await
    }
}

struct ProvenIdempotentInvoker {
    inner: ScriptedInvoker,
}

#[async_trait]
impl CapabilityToolInvoker for ProvenIdempotentInvoker {
    async fn invoke(&self, invocation: CapabilityInvocation) -> CapabilityInvocationResult {
        self.inner.invoke(invocation).await
    }

    fn supports_idempotency(&self, _tool_name: &str) -> bool {
        true
    }
}

fn execution_context() -> CapabilityExecutionContext {
    CapabilityExecutionContext {
        caller_agent_id: Some("captain".to_string()),
        workspace: None,
        origin: "test".to_string(),
        authority: None,
    }
}

fn uncertain_expectation(view: &CapabilityRunView, step_id: &str) -> UncertainNodeExpectation {
    let node = view
        .nodes
        .iter()
        .find(|node| node.step_id == step_id)
        .unwrap();
    UncertainNodeExpectation {
        tool_use_id: node.tool_use_id.clone().unwrap(),
        attempt: node.attempts,
    }
}

const PARALLEL_READ: &str = r#"
format = 1
name = "parallel-read"
description = "Read two independent files."
output = "{{steps.second.output}}"

[permissions]
tools = ["file_read"]
read_paths = ["data/**"]

[policy]
timeout_secs = 10
max_parallel = 2

[[steps]]
id = "first"
tool = "file_read"
needs = []
with = { path = "data/first.txt" }

[[steps]]
id = "second"
tool = "file_read"
needs = []
with = { path = "data/second.txt" }
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_reviewed_reads_execute_in_parallel() {
    let fixture = Fixture::new();
    let resolved = fixture.install(PARALLEL_READ);
    let invoker = ScriptedInvoker::delayed(60);
    let result = fixture
        .executor()
        .execute_resolved(resolved, json!({}), execution_context(), &invoker)
        .await
        .unwrap();
    assert_eq!(result.completed_nodes, 2);
    assert_eq!(invoker.max_active.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn dependency_is_a_parallelism_barrier() {
    let fixture = Fixture::new();
    let source = PARALLEL_READ.replace(
        "id = \"second\"\ntool = \"file_read\"\nneeds = []",
        "id = \"second\"\ntool = \"file_read\"\nneeds = [\"first\"]",
    );
    let resolved = fixture.install(&source);
    let invoker = ScriptedInvoker::delayed(20);
    fixture
        .executor()
        .execute_resolved(resolved, json!({}), execution_context(), &invoker)
        .await
        .unwrap();
    assert_eq!(invoker.max_active.load(Ordering::SeqCst), 1);
    let calls = invoker.calls.lock().unwrap();
    assert_eq!(calls[0].step_id, "first");
    assert_eq!(calls[1].step_id, "second");
}

#[tokio::test]
async fn safe_step_retries_with_distinct_durable_attempts() {
    let fixture = Fixture::new();
    let source = PARALLEL_READ
        .replace("max_parallel = 2", "max_parallel = 1")
        .replace(
            "needs = []\nwith = { path = \"data/first.txt\" }",
            "needs = []\nretry = { max_attempts = 2, backoff_ms = 1 }\nwith = { path = \"data/first.txt\" }",
        )
        .replace(
            "\n[[steps]]\nid = \"second\"\ntool = \"file_read\"\nneeds = []\nwith = { path = \"data/second.txt\" }",
            "",
        )
        .replace(
            "output = \"{{steps.second.output}}\"",
            "output = \"{{steps.first.output}}\"",
        );
    let resolved = fixture.install(&source);
    let invoker = ScriptedInvoker::flaky();
    let result = fixture
        .executor()
        .execute_resolved(resolved, json!({}), execution_context(), &invoker)
        .await
        .unwrap();
    assert_eq!(result.output["attempt"], json!(2));
    let calls = invoker.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].tool_use_id, calls[1].tool_use_id);
    assert_eq!(
        fixture.executor().run(&result.run_id).unwrap().nodes[0].attempts,
        2
    );
}

#[tokio::test]
async fn scope_violation_fails_before_tool_invocation() {
    let fixture = Fixture::new();
    let source = r#"
format = 1
name = "scoped-read"
description = "Read one explicitly scoped path."

[inputs.path]
type = "string"

[permissions]
tools = ["file_read"]
read_paths = ["allowed/**"]

[[steps]]
id = "read"
tool = "file_read"
with = { path = "{{input.path}}" }
"#;
    let resolved = fixture.install(source);
    let invoker = ScriptedInvoker::default();
    let error = fixture
        .executor()
        .execute_resolved(
            resolved,
            json!({"path": "../private.txt"}),
            execution_context(),
            &invoker,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ExecutorError::ScopeDenied { .. }));
    assert!(invoker.calls.lock().unwrap().is_empty());
    let run = fixture.executor().list_runs(1).unwrap().remove(0);
    assert_eq!(run.status, CapabilityRunStatus::Failed);
    assert_eq!(run.nodes[0].attempts, 0);
}

#[tokio::test]
async fn project_capability_cannot_execute_in_another_workspace() {
    let fixture = Fixture::new();
    let project = fixture._temp.path().join("project-a");
    let other = fixture._temp.path().join("project-b");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    fixture.registry.register_project(&project).unwrap();
    let source = r#"
format = 1
name = "project-only"
description = "Read one file in its owning project."

[permissions]
tools = ["file_read"]
read_paths = ["**"]

[[steps]]
id = "read"
tool = "file_read"
with = { path = "README.md" }
"#;
    std::fs::write(
        project
            .join(".captain/capabilities")
            .join("project-only.captain"),
        source,
    )
    .unwrap();
    fixture.registry.reload_all().unwrap();
    let resolved = fixture
        .registry
        .resolved_by_tool("cap_project_only", Some(&project))
        .unwrap()
        .unwrap();
    let context = CapabilityExecutionContext {
        caller_agent_id: Some("captain".to_string()),
        workspace: Some(other.to_string_lossy().into_owned()),
        origin: "test".to_string(),
        authority: None,
    };
    let invoker = ScriptedInvoker::default();
    let error = fixture
        .executor()
        .execute_resolved(resolved, json!({}), context, &invoker)
        .await
        .unwrap_err();
    assert!(matches!(error, ExecutorError::WorkspaceMismatch(_)));
    assert!(invoker.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn safe_running_node_recovers_as_resumable_after_abort() {
    let fixture = Fixture::new();
    fixture.install(PARALLEL_READ);
    let executor = fixture.executor();
    let (started_tx, started_rx) = oneshot::channel();
    let invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(started_tx)),
    });
    let task_executor = executor.clone();
    let task_invoker = invoker.clone();
    let task = tokio::spawn(async move {
        task_executor
            .execute_tool(
                "cap_parallel_read",
                json!({}),
                execution_context(),
                task_invoker.as_ref(),
            )
            .await
    });
    let invocation = started_rx.await.unwrap();
    task.abort();
    let _ = task.await;
    let interrupted = executor.run(&invocation.run_id).unwrap();
    assert_eq!(interrupted.status, CapabilityRunStatus::Interrupted);
    assert_eq!(interrupted.nodes[0].status, CapabilityNodeStatus::Pending);
    assert_eq!(interrupted.nodes[0].attempts, 0);
    drop(executor);

    let recovered = fixture.executor();
    let view = recovered.run(&invocation.run_id).unwrap();
    assert_eq!(view.status, CapabilityRunStatus::Interrupted);
    assert_eq!(view.nodes[0].status, CapabilityNodeStatus::Pending);
    let result = recovered
        .resume(&invocation.run_id, &ScriptedInvoker::default())
        .await
        .unwrap();
    assert_eq!(result.completed_nodes, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_run_cannot_be_dispatched_twice_concurrently() {
    let fixture = Fixture::new();
    fixture.install(PARALLEL_READ);
    let executor = fixture.executor();
    let (started_tx, started_rx) = oneshot::channel();
    let invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(started_tx)),
    });
    let task_executor = executor.clone();
    let task_invoker = invoker.clone();
    let task = tokio::spawn(async move {
        task_executor
            .execute_tool(
                "cap_parallel_read",
                json!({}),
                execution_context(),
                task_invoker.as_ref(),
            )
            .await
    });
    let invocation = started_rx.await.unwrap();

    let error = executor
        .resume(&invocation.run_id, invoker.as_ref())
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ExecutorError::RunAlreadyExecuting(run_id) if run_id == invocation.run_id
    ));
    assert_eq!(
        executor.run(&invocation.run_id).unwrap().nodes[0].attempts,
        1
    );

    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn resumed_run_stays_pinned_to_its_original_source_hash() {
    let fixture = Fixture::new();
    fixture.install(PARALLEL_READ);
    let executor = fixture.executor();
    let (started_tx, started_rx) = oneshot::channel();
    let invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(started_tx)),
    });
    let task_executor = executor.clone();
    let task_invoker = invoker.clone();
    let task = tokio::spawn(async move {
        task_executor
            .execute_tool(
                "cap_parallel_read",
                json!({}),
                execution_context(),
                task_invoker.as_ref(),
            )
            .await
    });
    let invocation = started_rx.await.unwrap();
    task.abort();
    let _ = task.await;
    let interrupted = executor.run(&invocation.run_id).unwrap();
    assert_eq!(interrupted.status, CapabilityRunStatus::Interrupted);
    assert_eq!(interrupted.nodes[0].status, CapabilityNodeStatus::Pending);
    drop(executor);

    let updated = PARALLEL_READ.replace("data/first.txt", "data/first-v2.txt");
    std::fs::write(fixture.global.join("parallel-read.captain"), updated).unwrap();
    fixture.registry.reload_global().unwrap();
    let active = fixture
        .registry
        .resolved_by_tool("cap_parallel_read", None)
        .unwrap()
        .unwrap();
    assert_ne!(active.compiled.source_hash, invocation.source_hash);

    let recovered = fixture.executor();
    let resume_invoker = ScriptedInvoker::default();
    let result = recovered
        .resume(&invocation.run_id, &resume_invoker)
        .await
        .unwrap();
    assert_eq!(result.source_hash, invocation.source_hash);
    let calls = resume_invoker.calls.lock().unwrap();
    assert!(calls
        .iter()
        .any(|call| call.input["path"] == json!("data/first.txt")));
    assert!(!calls
        .iter()
        .any(|call| call.input["path"] == json!("data/first-v2.txt")));
}

#[tokio::test]
async fn manual_running_node_waits_for_explicit_resolution_after_abort() {
    let fixture = Fixture::new();
    let source = r#"
format = 1
name = "manual-write"
description = "Write one file with explicit approval."

[permissions]
tools = ["file_write"]
write_paths = ["out/**"]

[[steps]]
id = "write"
tool = "file_write"
with = { path = "out/value.txt", content = "value" }
"#;
    fixture.install(source);
    let executor = fixture.executor();
    let (started_tx, started_rx) = oneshot::channel();
    let invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(started_tx)),
    });
    let task_executor = executor.clone();
    let task_invoker = invoker.clone();
    let task = tokio::spawn(async move {
        task_executor
            .execute_tool(
                "cap_manual_write",
                json!({}),
                execution_context(),
                task_invoker.as_ref(),
            )
            .await
    });
    let invocation = started_rx.await.unwrap();
    task.abort();
    let _ = task.await;
    let interrupted = executor.run(&invocation.run_id).unwrap();
    assert_eq!(interrupted.status, CapabilityRunStatus::WaitingDecision);
    assert_eq!(interrupted.nodes[0].status, CapabilityNodeStatus::Uncertain);
    drop(executor);

    let recovered = fixture.executor();
    let view = recovered.run(&invocation.run_id).unwrap();
    assert_eq!(view.status, CapabilityRunStatus::WaitingDecision);
    assert_eq!(view.nodes[0].status, CapabilityNodeStatus::Uncertain);
    let result = recovered
        .resolve_uncertain(
            &invocation.run_id,
            "write",
            UncertainResolution::ConfirmSucceeded {
                output: json!({"confirmed": true}),
            },
            &ScriptedInvoker::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.output, json!({"confirmed": true}));
}

#[tokio::test]
async fn manual_step_timeout_becomes_uncertain_instead_of_replaying() {
    let fixture = Fixture::new();
    let source = r#"
format = 1
name = "timeout-write"
description = "Exercise uncertain timeout handling."

[permissions]
tools = ["file_write"]
write_paths = ["out/**"]

[policy]
timeout_secs = 5
max_parallel = 1

[[steps]]
id = "write"
tool = "file_write"
timeout_secs = 1
with = { path = "out/value.txt", content = "value" }
"#;
    let resolved = fixture.install(source);
    let (started_tx, _started_rx) = oneshot::channel();
    let invoker = BlockingInvoker {
        started: Mutex::new(Some(started_tx)),
    };
    let executor = fixture.executor();
    let error = executor
        .execute_resolved(resolved, json!({}), execution_context(), &invoker)
        .await
        .unwrap_err();
    let run_id = match error {
        ExecutorError::WaitingDecision { run_id, node_id } => {
            assert_eq!(node_id, "write");
            run_id
        }
        other => panic!("unexpected error: {other}"),
    };
    let view = executor.run(&run_id).unwrap();
    assert_eq!(view.status, CapabilityRunStatus::WaitingDecision);
    assert_eq!(view.nodes[0].status, CapabilityNodeStatus::Uncertain);
    assert_eq!(view.nodes[0].attempts, 1);
}

#[tokio::test]
async fn stale_uncertain_decision_cannot_resolve_a_later_attempt() {
    let fixture = Fixture::new();
    let source = r#"
format = 1
name = "stale-write"
description = "Prove exact uncertain decision identity."

[permissions]
tools = ["file_write"]
write_paths = ["out/**"]

[[steps]]
id = "write"
tool = "file_write"
with = { path = "out/value.txt", content = "value" }
"#;
    fixture.install(source);
    let executor = fixture.executor();

    let (first_tx, first_rx) = oneshot::channel();
    let first_invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(first_tx)),
    });
    let task_executor = executor.clone();
    let task_invoker = first_invoker.clone();
    let first_task = tokio::spawn(async move {
        task_executor
            .execute_tool(
                "cap_stale_write",
                json!({}),
                execution_context(),
                task_invoker.as_ref(),
            )
            .await
    });
    let first_invocation = first_rx.await.unwrap();
    first_task.abort();
    let _ = first_task.await;
    let first_view = executor.run(&first_invocation.run_id).unwrap();
    let first_expectation = uncertain_expectation(&first_view, "write");
    executor
        .apply_uncertain_resolution(
            &first_invocation.run_id,
            "write",
            &first_expectation,
            UncertainResolution::Retry,
        )
        .unwrap();

    let (second_tx, second_rx) = oneshot::channel();
    let second_invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(second_tx)),
    });
    let task_executor = executor.clone();
    let task_invoker = second_invoker.clone();
    let run_id = first_invocation.run_id.clone();
    let mut second_task =
        tokio::spawn(async move { task_executor.resume(&run_id, task_invoker.as_ref()).await });
    let second_invocation = tokio::select! {
        invocation = second_rx => invocation.expect("second invocation channel closed"),
        result = &mut second_task => panic!("resume ended before second dispatch: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(3)) => {
            panic!("resume did not dispatch the second attempt within 3 seconds")
        }
    };
    second_task.abort();
    let _ = second_task.await;
    assert_eq!(second_invocation.attempt, 2);
    assert_ne!(second_invocation.tool_use_id, first_expectation.tool_use_id);

    let stale = executor
        .apply_uncertain_resolution(
            &first_invocation.run_id,
            "write",
            &first_expectation,
            UncertainResolution::ConfirmSucceeded {
                output: json!({"stale": true}),
            },
        )
        .unwrap_err();
    assert!(matches!(
        stale,
        ExecutorError::StaleUncertainDecision { .. }
    ));
    let current = executor.run(&first_invocation.run_id).unwrap();
    assert_eq!(current.status, CapabilityRunStatus::WaitingDecision);
    assert_eq!(current.nodes[0].attempts, 2);
    assert_eq!(
        current.nodes[0].tool_use_id.as_deref(),
        Some(second_invocation.tool_use_id.as_str())
    );
}

#[tokio::test]
async fn duplicate_uncertain_decision_is_accepted_exactly_once() {
    let fixture = Fixture::new();
    let source = r#"
format = 1
name = "once-write"
description = "Prove one operator decision wins."

[permissions]
tools = ["file_write"]
write_paths = ["out/**"]

[[steps]]
id = "write"
tool = "file_write"
with = { path = "out/value.txt", content = "value" }
"#;
    fixture.install(source);
    let executor = fixture.executor();
    let (started_tx, started_rx) = oneshot::channel();
    let invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(started_tx)),
    });
    let task_executor = executor.clone();
    let task_invoker = invoker.clone();
    let task = tokio::spawn(async move {
        task_executor
            .execute_tool(
                "cap_once_write",
                json!({}),
                execution_context(),
                task_invoker.as_ref(),
            )
            .await
    });
    let invocation = started_rx.await.unwrap();
    task.abort();
    let _ = task.await;
    let view = executor.run(&invocation.run_id).unwrap();
    let expectation = uncertain_expectation(&view, "write");
    executor
        .apply_uncertain_resolution(
            &invocation.run_id,
            "write",
            &expectation,
            UncertainResolution::MarkFailed {
                reason: "operator checked side effect".to_string(),
            },
        )
        .unwrap();
    let duplicate = executor
        .apply_uncertain_resolution(
            &invocation.run_id,
            "write",
            &expectation,
            UncertainResolution::Retry,
        )
        .unwrap_err();
    assert!(matches!(
        duplicate,
        ExecutorError::StaleUncertainDecision { .. }
    ));
    assert_eq!(
        executor.run(&invocation.run_id).unwrap().status,
        CapabilityRunStatus::Failed
    );
}

#[tokio::test]
async fn operator_resume_intent_survives_restart_without_enrolling_plain_interrupts() {
    let fixture = Fixture::new();
    fixture.install(
        r#"
format = 1
name = "durable-operator-resume"
description = "Persist an exact operator-authorized retry."

[permissions]
tools = ["file_write"]
write_paths = ["out/**"]

[[steps]]
id = "write"
tool = "file_write"
with = { path = "out/value.txt", content = "value" }
"#,
    );
    let executor = fixture.executor();
    let (started_tx, started_rx) = oneshot::channel();
    let invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(started_tx)),
    });
    let task_executor = executor.clone();
    let task_invoker = Arc::clone(&invoker);
    let task = tokio::spawn(async move {
        task_executor
            .execute_tool(
                "cap_durable_operator_resume",
                json!({}),
                execution_context(),
                task_invoker.as_ref(),
            )
            .await
    });
    let invocation = started_rx.await.unwrap();
    task.abort();
    let _ = task.await;
    let view = executor.run(&invocation.run_id).unwrap();
    executor
        .apply_uncertain_resolution(
            &invocation.run_id,
            "write",
            &uncertain_expectation(&view, "write"),
            UncertainResolution::Retry,
        )
        .unwrap();
    assert_eq!(
        executor.list_operator_resume_run_ids(10).unwrap(),
        vec![invocation.run_id.clone()]
    );
    assert!(executor.claim_operator_resume(&invocation.run_id).unwrap());

    drop(executor);
    let recovered = fixture.executor();
    assert_eq!(
        recovered.list_operator_resume_run_ids(10).unwrap(),
        vec![invocation.run_id.clone()],
        "boot must recover a claim abandoned between the decision and dispatch"
    );
    recovered
        .finish_operator_resume(&invocation.run_id)
        .unwrap();
    assert!(recovered
        .list_operator_resume_run_ids(10)
        .unwrap()
        .is_empty());

    fixture.install(
        r#"
format = 1
name = "plain-interrupted-read"
description = "An ordinary interruption must remain operator-stopped."

[permissions]
tools = ["file_read"]
read_paths = ["input/**"]

[[steps]]
id = "read"
tool = "file_read"
with = { path = "input/value.txt" }
"#,
    );
    let (read_tx, read_rx) = oneshot::channel();
    let read_invoker = Arc::new(BlockingInvoker {
        started: Mutex::new(Some(read_tx)),
    });
    let task_executor = recovered.clone();
    let task_invoker = Arc::clone(&read_invoker);
    let read_task = tokio::spawn(async move {
        task_executor
            .execute_tool(
                "cap_plain_interrupted_read",
                json!({}),
                execution_context(),
                task_invoker.as_ref(),
            )
            .await
    });
    let read_invocation = read_rx.await.unwrap();
    read_task.abort();
    let _ = read_task.await;
    assert_eq!(
        recovered.run(&read_invocation.run_id).unwrap().status,
        CapabilityRunStatus::Interrupted
    );
    assert!(recovered
        .list_operator_resume_run_ids(10)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn keyed_retry_requires_tool_level_idempotency_proof() {
    let fixture = Fixture::new();
    let source = r#"
format = 1
name = "keyed-write"
description = "Exercise keyed retries."

[permissions]
tools = ["file_write"]
write_paths = ["out/**"]

[[steps]]
id = "write"
tool = "file_write"
idempotency = "keyed"
idempotency_key = "{{run.id}}:write"
retry = { max_attempts = 2, backoff_ms = 1 }
with = { path = "out/value.txt", content = "value" }
"#;
    let resolved = fixture.install(source);
    let unsupported = ScriptedInvoker::flaky();
    let error = fixture
        .executor()
        .execute_resolved(
            resolved.clone(),
            json!({}),
            execution_context(),
            &unsupported,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ExecutorError::RunFailed { .. }));
    assert_eq!(unsupported.calls.lock().unwrap().len(), 1);

    let supported = ProvenIdempotentInvoker {
        inner: ScriptedInvoker::flaky(),
    };
    let result = fixture
        .executor()
        .execute_resolved(resolved, json!({}), execution_context(), &supported)
        .await
        .unwrap();
    assert_eq!(result.output["attempt"], json!(2));
    let calls = supported.inner.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].idempotency_key, calls[1].idempotency_key);
}

#[tokio::test]
async fn runtime_inputs_and_outputs_are_not_plaintext_in_sqlite() {
    let fixture = Fixture::new();
    let source = r#"
format = 1
name = "encrypted-state"
description = "Exercise encrypted durable state."
output = "{{steps.read.output}}"

[inputs.path]
type = "string"
sensitive = true

[permissions]
tools = ["file_read"]
read_paths = ["private/**"]

[[steps]]
id = "read"
tool = "file_read"
with = { path = "{{input.path}}" }
"#;
    let resolved = fixture.install(source);
    let secret = "private/never-store-me-in-plaintext.txt";
    let authority_marker = "CAPSPEC_PRIVATE_ENV_MARKER";
    let mut context = execution_context();
    context.authority = Some(CapabilityExecutionAuthority {
        allowed_tools: vec!["file_read".to_string()],
        allowed_env_vars: Some(vec![authority_marker.to_string()]),
        ..CapabilityExecutionAuthority::default()
    });
    let executor = fixture.executor();
    let result = executor
        .execute_resolved(
            resolved,
            json!({"path": secret}),
            context,
            &ScriptedInvoker::default(),
        )
        .await
        .unwrap();
    assert_eq!(result.completed_nodes, 1);
    let restored = executor.resume_context(&result.run_id).unwrap();
    assert_eq!(
        restored
            .execution
            .authority
            .unwrap()
            .allowed_env_vars
            .unwrap(),
        [authority_marker]
    );

    let connection = Connection::open(&fixture.database).unwrap();
    let input_blob: Vec<u8> = connection
        .query_row(
            "SELECT input_blob FROM capspec_runs WHERE run_id = ?1",
            [&result.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!input_blob
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
    let database_bytes = std::fs::read(&fixture.database).unwrap();
    assert!(!database_bytes
        .windows(secret.len())
        .any(|window| window == secret.as_bytes()));
    assert!(!database_bytes
        .windows(authority_marker.len())
        .any(|window| window == authority_marker.as_bytes()));
}

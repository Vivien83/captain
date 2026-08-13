use crate::{
    AuthorizedNodeRun, ClientAccessSession, ClientPairingClient, ClientPairingProfile,
    ClientPairingProgress, ClientPairingStore, NodeExecutionPolicy, NodeNetworkConfig,
    NodePairingClient, NodePairingProfile, NodePairingProgress, NodePairingStore, NodeProxyMode,
    NodeRailLink, NodeRailStore, NodeReviewedTool, NodeRunCancellation, NodeToolDriver,
    NodeToolExecutionOutput, NodeToolReview, NodeWorker, NodeWorkerError, NodeWorkspaceBinding,
};
use captain_kernel::CaptainKernel;
use captain_runtime::{
    execution_routing::RemoteToolExecutionRequest,
    kernel_handle::KernelHandle,
    node_tool_runtime::{
        execute_local_node_tool, review_local_node_tool, LocalNodeToolEffect,
        LocalNodeToolExecution, LocalNodeToolOutput, LocalNodeToolRejection,
    },
};
use captain_types::config::{DefaultModelConfig, ExecPolicy, KernelConfig};
use captain_wire::{
    hub_protocol::RunRejection, CapabilityDescriptor, DeviceGrant, LogicalWorkspace, NodeTransport,
    RunEffect, RunLease,
};
use futures::future::BoxFuture;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const TEST_API_KEY: &str = "alpha14-distributed-api-key-0123456789";
const WORKSPACE_ID: &str = "workspace-main";

struct TestServer {
    address: std::net::SocketAddr,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct RuntimeNodeToolDriver {
    exec_policy: ExecPolicy,
    executions: AtomicUsize,
}

impl RuntimeNodeToolDriver {
    fn new(exec_policy: ExecPolicy) -> Self {
        Self {
            exec_policy,
            executions: AtomicUsize::new(0),
        }
    }

    fn execution_count(&self) -> usize {
        self.executions.load(Ordering::SeqCst)
    }
}

impl NodeToolDriver for RuntimeNodeToolDriver {
    fn review(&self, lease: &RunLease) -> Result<NodeToolReview, RunRejection> {
        let runtime = review_local_node_tool(&lease.tool_name, &lease.input, &self.exec_policy)
            .map_err(|rejection| runtime_rejection(lease, rejection))?;
        let reviewed = NodeReviewedTool::new(
            runtime.tool_name(),
            runtime.family(),
            wire_effect(runtime.effect()),
        )
        .map_err(|_| fixed_rejection(lease, "runtime_review_contract_invalid", false))?;
        NodeToolReview::new(
            reviewed,
            runtime.action_digest(),
            runtime.approval_required(),
            runtime.risk_level(),
            runtime.approval_summary(),
        )
        .map_err(|_| fixed_rejection(lease, "runtime_review_contract_invalid", false))
    }

    fn execute(
        self: Arc<Self>,
        run: AuthorizedNodeRun,
        approved_action_digest: Option<String>,
        cancellation: NodeRunCancellation,
    ) -> BoxFuture<'static, NodeToolExecutionOutput> {
        Box::pin(async move {
            self.executions.fetch_add(1, Ordering::SeqCst);
            let lease = run.lease().clone();
            let workspace_root = run.workspace_root().to_path_buf();
            let tool_use_id = format!("node-{}-{}", lease.run_id, lease.attempt);
            let execution = execute_local_node_tool(LocalNodeToolExecution {
                tool_use_id: &tool_use_id,
                tool_name: &lease.tool_name,
                input: &lease.input,
                workspace_id: &lease.workspace_id,
                workspace_root: &workspace_root,
                exec_policy: &self.exec_policy,
                approved_action_digest: approved_action_digest.as_deref(),
            });
            tokio::pin!(execution);
            tokio::select! {
                result = &mut execution => match result {
                    Ok(output) => node_output(output),
                    Err(rejection) => safe_failure(&format!(
                        "Local Node runtime rejected execution ({}).",
                        rejection.code()
                    )),
                },
                () = cancellation.requested() => {
                    safe_failure("Local Node execution received a cancellation request.")
                }
            }
        })
    }
}

fn wire_effect(effect: LocalNodeToolEffect) -> RunEffect {
    match effect {
        LocalNodeToolEffect::ReadOnly => RunEffect::ReadOnly,
        LocalNodeToolEffect::LocalMutation => RunEffect::LocalMutation,
        LocalNodeToolEffect::ExternalEffect => RunEffect::ExternalEffect,
    }
}

fn runtime_rejection(lease: &RunLease, rejection: LocalNodeToolRejection) -> RunRejection {
    RunRejection {
        run_id: lease.run_id.clone(),
        attempt: lease.attempt,
        code: rejection.code().to_string(),
        message: rejection.message().to_string(),
        retryable: rejection.is_retryable(),
        path_policy_applied: true,
    }
}

fn fixed_rejection(lease: &RunLease, code: &str, retryable: bool) -> RunRejection {
    RunRejection {
        run_id: lease.run_id.clone(),
        attempt: lease.attempt,
        code: code.to_string(),
        message: "The local Runtime review did not satisfy the Node contract".to_string(),
        retryable,
        path_policy_applied: true,
    }
}

fn node_output(output: LocalNodeToolOutput) -> NodeToolExecutionOutput {
    let (succeeded, content, total_output_bytes, capped, redacted) = output.into_parts();
    NodeToolExecutionOutput::new(succeeded, content, total_output_bytes, capped, redacted)
        .unwrap_or_else(|_| safe_failure("Local Node output failed its final wire contract."))
}

fn safe_failure(message: &str) -> NodeToolExecutionOutput {
    NodeToolExecutionOutput::new(false, message, message.len() as u64, false, false).unwrap_or_else(
        |error| match error {
            NodeWorkerError::DriverContract => NodeToolExecutionOutput::new(
                false,
                "Local Node execution failed safely.",
                35,
                false,
                false,
            )
            .expect("fixed Node failure is contract-safe"),
            _ => unreachable!("output construction only returns driver contract errors"),
        },
    )
}

fn now_ms() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should follow the Unix epoch")
            .as_millis(),
    )
    .expect("current timestamp should fit in i64")
}

fn node_capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "test-loopback".to_string(),
        transports: vec![
            NodeTransport::WebSocket,
            NodeTransport::HttpStream,
            NodeTransport::LongPoll,
        ],
        tool_families: vec!["file".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: WORKSPACE_ID.to_string(),
            label: "Smoke workspace".to_string(),
            read_only: false,
        }],
        supports_streaming_output: true,
    }
}

fn node_grant() -> DeviceGrant {
    DeviceGrant {
        workspace_ids: vec![WORKSPACE_ID.to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: true,
    }
}

fn client_capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "test-client".to_string(),
        transports: vec![NodeTransport::HttpStream],
        tool_families: Vec::new(),
        workspaces: Vec::new(),
        supports_streaming_output: true,
    }
}

async fn start_hub(root: &std::path::Path) -> (Arc<CaptainKernel>, TestServer) {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("loopback listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should have an address");
    let mut config = KernelConfig {
        home_dir: root.join("hub"),
        data_dir: root.join("hub-data"),
        api_key: TEST_API_KEY.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    config.pairing.hub_enabled = true;
    let kernel = Arc::new(
        CaptainKernel::boot_with_config(config).expect("test Hub kernel should boot cleanly"),
    );
    kernel.set_self_handle();
    let (router, _state) = captain_api::server::build_router(Arc::clone(&kernel), address).await;
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("test Hub server should remain available");
    });
    (kernel, TestServer { address, task })
}

fn loopback_http(server: &TestServer) -> crate::NodeHttpClient {
    let mut network = NodeNetworkConfig::new(format!("http://{}", server.address));
    network.proxy = NodeProxyMode::Disabled;
    network.connect_timeout_secs = 2;
    network.request_timeout_secs = 3;
    network
        .build_loopback_client()
        .expect("test-only loopback transport should be valid")
}

async fn pair_node(
    kernel: &CaptainKernel,
    http: crate::NodeHttpClient,
    state_root: &std::path::Path,
) -> (String, NodePairingClient) {
    kernel
        .hub_pairing
        .open_enrollment_window(300)
        .expect("Node enrollment window should open");
    let capabilities = node_capabilities();
    let grant = node_grant();
    let client = NodePairingClient::new(
        http,
        NodePairingStore::open(state_root).expect("Node pairing store should open"),
    );
    let progress = client
        .start_or_resume(&NodePairingProfile::new(
            "Distributed smoke Node",
            capabilities.platform.clone(),
            capabilities,
            grant.clone(),
        ))
        .await
        .expect("Node pairing claim should reach the Hub");
    let NodePairingProgress::AwaitingApproval { display_code, .. } = progress else {
        panic!("new Node pairing should await approval")
    };
    kernel
        .hub_pairing
        .approve_display_code(&display_code, &grant)
        .expect("operator should approve the exact Node grant");
    let NodePairingProgress::Paired { device_id, .. } = client
        .poll()
        .await
        .expect("approved Node pairing should be observable")
    else {
        panic!("approved Node pairing should be terminal")
    };
    (device_id, client)
}

async fn pair_client(
    kernel: &CaptainKernel,
    http: crate::NodeHttpClient,
    state_root: &std::path::Path,
) -> ClientPairingClient {
    kernel
        .hub_pairing
        .open_enrollment_window(300)
        .expect("Client enrollment window should open");
    let client = ClientPairingClient::new(
        http,
        ClientPairingStore::open(state_root).expect("Client pairing store should open"),
    );
    let capabilities = client_capabilities();
    let progress = client
        .start_or_resume(&ClientPairingProfile::new(
            "Distributed smoke Client",
            capabilities.platform.clone(),
            capabilities,
        ))
        .await
        .expect("Client pairing claim should reach the Hub");
    let ClientPairingProgress::AwaitingApproval { display_code, .. } = progress else {
        panic!("new Client pairing should await approval")
    };
    kernel
        .hub_pairing
        .approve_display_code(&display_code, &DeviceGrant::default())
        .expect("operator should approve the Client without execution grants");
    assert!(matches!(
        client
            .poll()
            .await
            .expect("approved Client pairing should be observable"),
        ClientPairingProgress::Paired { .. }
    ));
    client
}

fn remote_request(
    session_id: &str,
    agent_id: &str,
    device_id: &str,
    tool_use_id: &str,
    tool_name: &str,
    input: serde_json::Value,
) -> RemoteToolExecutionRequest {
    RemoteToolExecutionRequest {
        scope_id: session_id.to_string(),
        tool_use_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        input,
        caller_agent_id: agent_id.to_string(),
        device_id: device_id.to_string(),
        workspace_id: WORKSPACE_ID.to_string(),
    }
}

async fn launch_and_finish_without_flush(
    link: &mut NodeRailLink,
    worker: &mut NodeWorker<RuntimeNodeToolDriver>,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let cycle = worker
                .advance(now_ms())
                .await
                .expect("Node worker should accept the offered run");
            let active_runs = worker
                .rail()
                .active_run_ids()
                .expect("active Node runs should remain readable");
            link.set_active_runs(&active_runs)
                .await
                .expect("Node acceptance should be durably acknowledged");
            if cycle.launched == 1 {
                break;
            }
            if cycle.applied_inbound == 0 {
                link.synchronize_once()
                    .await
                    .expect("Node transport should remain valid");
            }
        }
    })
    .await
    .expect("Node should receive and launch the offered run");

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let cycle = worker
                .advance(now_ms())
                .await
                .expect("Node worker should collect its terminal output");
            if cycle.completed == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("local Node execution should finish promptly");
}

async fn execute_and_flush(
    link: &mut NodeRailLink,
    worker: &mut NodeWorker<RuntimeNodeToolDriver>,
) {
    launch_and_finish_without_flush(link, worker).await;
    tokio::time::timeout(Duration::from_secs(5), link.flush_pending())
        .await
        .expect("Node completion should flush to the Hub")
        .expect("Node completion should satisfy the rail contract");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a real loopback HTTP/WebSocket server"]
async fn paired_client_routes_real_node_execution_across_crash_without_duplicate() {
    let temp = tempfile::tempdir().expect("distributed smoke root should exist");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir(&workspace).expect("Node workspace should exist");
    let (kernel, server) = start_hub(temp.path()).await;
    let http = loopback_http(&server);

    let node_state = temp.path().join("node-state");
    let (device_id, initial_pairing) = pair_node(&kernel, http.clone(), &node_state).await;
    drop(initial_pairing);
    let pairing_store =
        NodePairingStore::open(&node_state).expect("paired Node state should reopen after restart");
    let rail = NodeRailStore::open(&pairing_store)
        .expect("durable Node rail should bind to paired identity");
    let pairing = NodePairingClient::new(http.clone(), pairing_store);
    let access = pairing
        .issue_access_token()
        .await
        .expect("paired Node should receive a short-lived access token");
    assert_eq!(access.approved_grants(), &node_grant());
    let policy = NodeExecutionPolicy::new(
        access.approved_grants().clone(),
        [NodeWorkspaceBinding::new(WORKSPACE_ID, &workspace, false)
            .expect("logical workspace should bind locally")],
    )
    .expect("local Node grant should form a valid execution policy");
    let driver = Arc::new(RuntimeNodeToolDriver::new(ExecPolicy::default()));
    let mut worker = NodeWorker::new(rail.clone(), policy, Arc::clone(&driver));
    let mut link = tokio::time::timeout(
        Duration::from_secs(5),
        NodeRailLink::connect(
            http.clone(),
            rail.clone(),
            access,
            &node_capabilities(),
            &[],
        ),
    )
    .await
    .expect("Node transport should connect promptly")
    .expect("Node should negotiate a real outbound transport");
    assert_eq!(link.transport(), NodeTransport::WebSocket);

    let captain = kernel
        .registry
        .find_by_name("captain")
        .expect("Captain agent should exist");
    let session = kernel
        .memory
        .create_session(captain.id)
        .expect("shared Hub session should persist");
    let session_id = session.id.to_string();
    let agent_id = captain.id.to_string();

    let client_state = temp.path().join("client-state");
    let client = pair_client(&kernel, http.clone(), &client_state).await;
    let first_client_token = client
        .issue_access_token()
        .await
        .expect("paired Client should receive a scoped token");
    drop(client);
    let second_surface = ClientAccessSession::open(
        http.clone(),
        ClientPairingStore::open(&client_state)
            .expect("paired Client state should reopen on another surface"),
    )
    .expect("another Client surface should reuse the paired identity");
    let second_client_token = second_surface
        .issue_access_token()
        .await
        .expect("another Client surface should rotate its scoped token");
    let api = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("bounded Client HTTP stack should build");
    let target_url = format!(
        "http://{}/api/sessions/{session_id}/execution-target",
        server.address
    );
    let put = api
        .put(&target_url)
        .bearer_auth(first_client_token.as_str())
        .json(&serde_json::json!({
            "target": {
                "kind": "node",
                "device_id": device_id,
                "workspace_id": WORKSPACE_ID,
            }
        }))
        .send()
        .await
        .expect("first Client surface should reach the Hub");
    assert_eq!(put.status(), reqwest::StatusCode::OK);
    let put_payload: serde_json::Value = put
        .json()
        .await
        .expect("execution-target response should be JSON");
    assert_eq!(put_payload["target"]["kind"], "node");
    assert!(!put_payload
        .to_string()
        .contains(&workspace.display().to_string()));

    let get = api
        .get(&target_url)
        .bearer_auth(second_client_token.as_str())
        .send()
        .await
        .expect("second Client surface should reach the same Hub session");
    assert_eq!(get.status(), reqwest::StatusCode::OK);
    let get_payload: serde_json::Value = get
        .json()
        .await
        .expect("shared execution-target response should be JSON");
    assert_eq!(get_payload["target"], put_payload["target"]);
    assert!(!get_payload
        .to_string()
        .contains(&workspace.display().to_string()));

    let write_request = remote_request(
        &session_id,
        &agent_id,
        &device_id,
        "distributed-write-1",
        "file_write",
        serde_json::json!({"path": "result.txt", "content": "durable-alpha14"}),
    );
    let task_kernel = Arc::clone(&kernel);
    let write_task = tokio::spawn(async move {
        KernelHandle::execute_remote_tool(&*task_kernel, write_request).await
    });
    launch_and_finish_without_flush(&mut link, &mut worker).await;
    assert_eq!(driver.execution_count(), 1);
    assert_eq!(
        std::fs::read_to_string(workspace.join("result.txt"))
            .expect("local mutation should be observable before reconnect"),
        "durable-alpha14"
    );
    assert!(
        rail.snapshot()
            .expect("rail snapshot should persist")
            .pending_outbound
            > 0
    );

    drop(worker);
    drop(link);
    let restarted_access = pairing
        .issue_access_token()
        .await
        .expect("restarted Node should rotate its access token");
    let restarted_policy = NodeExecutionPolicy::new(
        restarted_access.approved_grants().clone(),
        [NodeWorkspaceBinding::new(WORKSPACE_ID, &workspace, false)
            .expect("workspace should survive Node restart")],
    )
    .expect("local grant should survive Node restart");
    let mut worker = NodeWorker::new(rail.clone(), restarted_policy, Arc::clone(&driver));
    let active_runs = rail
        .active_run_ids()
        .expect("active runs should survive Node restart");
    let mut link = tokio::time::timeout(
        Duration::from_secs(5),
        NodeRailLink::connect(
            http,
            rail.clone(),
            restarted_access,
            &node_capabilities(),
            &active_runs,
        ),
    )
    .await
    .expect("restarted Node should reconnect promptly")
    .expect("restarted Node should recover its durable transport");
    let write_result = tokio::time::timeout(Duration::from_secs(5), write_task)
        .await
        .expect("interrupted Hub request should reach an explicit terminal state")
        .expect("remote write task should remain joinable")
        .expect("remote write should return a terminal result");
    assert!(write_result.is_error);
    assert!(write_result.content.contains("uncertain"));
    assert_eq!(driver.execution_count(), 1);

    let duplicate = remote_request(
        &session_id,
        &agent_id,
        &device_id,
        "distributed-write-1",
        "file_write",
        serde_json::json!({"path": "result.txt", "content": "durable-alpha14"}),
    );
    let duplicate_result = tokio::time::timeout(
        Duration::from_secs(2),
        KernelHandle::execute_remote_tool(&*kernel, duplicate),
    )
    .await
    .expect("idempotent replay should return existing terminal evidence")
    .expect("idempotent replay should remain successful");
    assert!(!duplicate_result.is_error, "{}", duplicate_result.content);
    assert_eq!(driver.execution_count(), 1);

    let read_request = remote_request(
        &session_id,
        &agent_id,
        &device_id,
        "distributed-read-1",
        "file_read",
        serde_json::json!({"path": "result.txt"}),
    );
    let task_kernel = Arc::clone(&kernel);
    let read_task = tokio::spawn(async move {
        KernelHandle::execute_remote_tool(&*task_kernel, read_request).await
    });
    execute_and_flush(&mut link, &mut worker).await;
    let read_result = tokio::time::timeout(Duration::from_secs(5), read_task)
        .await
        .expect("read completion should reach the Hub")
        .expect("remote read task should remain joinable")
        .expect("remote read should return a terminal result");
    assert!(!read_result.is_error, "{}", read_result.content);
    assert!(read_result.content.contains("durable-alpha14"));
    assert_eq!(driver.execution_count(), 2);
    assert!(!read_result
        .content
        .contains(&workspace.display().to_string()));

    link.close(Some("distributed_smoke_complete"))
        .await
        .expect("clean Node shutdown should reach the Hub");
}

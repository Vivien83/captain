//! Runtime adapter for one durable Hub-to-Node tool execution.

use captain_memory::hub_node_rail::{
    HubNodeRunApprovalStatus, HubNodeRunRecord, HubNodeRunStatus, NewHubNodeRun,
};
use captain_runtime::{
    execution_routing::RemoteToolExecutionRequest,
    node_tool_runtime::{local_node_tool_effect, LocalNodeToolEffect},
    tool_runner::{progress_sink, ToolProgressEvent},
};
use captain_types::{approval::ApprovalDecision, tool::ToolResult};
use captain_wire::{hub_protocol::RunApprovalDecision, RunEffect, RunTerminalStatus};
use sha2::{Digest, Sha256};
use std::time::Duration;

use super::CaptainKernel;

const REMOTE_RUN_ACTIVITY_WAIT: Duration = Duration::from_millis(250);
const REMOTE_RUN_DEFAULT_WAIT_SECS: u64 = 150;
const REMOTE_RUN_MAX_WAIT_SECS: u64 = 24 * 60 * 60;

impl CaptainKernel {
    pub(super) async fn handle_execute_remote_tool(
        &self,
        request: RemoteToolExecutionRequest,
    ) -> Result<ToolResult, String> {
        validate_remote_request(&request)?;
        let effect = remote_run_effect(&request.tool_name, &request.input)?;
        let (run_id, idempotency_key) = remote_run_identity(&request)?;
        self.hub_nodes
            .submit_run(&NewHubNodeRun {
                run_id: run_id.clone(),
                device_id: request.device_id.clone(),
                idempotency_key,
                workspace_id: request.workspace_id.clone(),
                tool_name: request.tool_name.clone(),
                input: request.input.clone(),
                effect,
                created_at_ms: chrono::Utc::now().timestamp_millis(),
            })
            .map_err(|error| remote_service_error(&error))?;

        let mut cancellation = RemoteRunCancellationGuard::new(self, run_id.clone());
        let deadline = tokio::time::Instant::now() + remote_run_wait_limit(&request);
        let mut last_progress_sequence = 0;
        let mut handled_approval_id = None;

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(
                    "the selected Node did not reach a terminal state before the bounded wait expired"
                        .to_string(),
                );
            }

            let run = self
                .hub_nodes
                .get_run(&run_id)
                .map_err(|error| remote_service_error(&error))?
                .ok_or_else(|| "the durable Node run is no longer available".to_string())?;

            if run.progress_sequence > last_progress_sequence {
                last_progress_sequence = run.progress_sequence;
                if let (Some(sender), Some(message)) =
                    (progress_sink(), run.progress_message.clone())
                {
                    let _ = sender.try_send(ToolProgressEvent {
                        tool_use_id: request.tool_use_id.clone(),
                        message,
                        frame_index: None,
                        frames_total: None,
                    });
                }
            }

            if let Some(result) = terminal_tool_result(&request.tool_use_id, &run) {
                cancellation.disarm();
                return Ok(result);
            }

            if let Some(approval) = self
                .hub_nodes
                .get_run_approval(&run_id)
                .map_err(|error| remote_service_error(&error))?
                .filter(|approval| approval.status == HubNodeRunApprovalStatus::Pending)
            {
                if handled_approval_id.as_deref() != Some(approval.approval_id.as_str()) {
                    let approval_id = approval
                        .approval_id
                        .parse::<uuid::Uuid>()
                        .map_err(|_| "remote Node approval identifier is invalid".to_string())?;
                    cancellation.track_approval(approval_id);
                    let outcome = self
                        .handle_remote_node_approval(
                            &request.caller_agent_id,
                            &request.tool_name,
                            &approval,
                        )
                        .await?;
                    cancellation.clear_approval();
                    self.hub_nodes
                        .decide_run_approval(&RunApprovalDecision {
                            run_id: approval.run_id.clone(),
                            attempt: approval.attempt,
                            approval_id: approval.approval_id.clone(),
                            action_digest: approval.action_digest.clone(),
                            decision: one_shot_decision(outcome.decision),
                            reason: outcome.reason,
                            decided_at_ms: chrono::Utc::now().timestamp_millis(),
                        })
                        .map_err(|error| remote_service_error(&error))?;
                    handled_approval_id = Some(approval.approval_id);
                    continue;
                }
            }

            self.hub_nodes
                .wait_for_activity(REMOTE_RUN_ACTIVITY_WAIT)
                .await;
        }
    }
}

struct RemoteRunCancellationGuard<'a> {
    kernel: &'a CaptainKernel,
    run_id: String,
    pending_approval_id: Option<uuid::Uuid>,
    armed: bool,
}

impl<'a> RemoteRunCancellationGuard<'a> {
    fn new(kernel: &'a CaptainKernel, run_id: String) -> Self {
        Self {
            kernel,
            run_id,
            pending_approval_id: None,
            armed: true,
        }
    }

    fn track_approval(&mut self, approval_id: uuid::Uuid) {
        self.pending_approval_id = Some(approval_id);
    }

    fn clear_approval(&mut self) {
        self.pending_approval_id = None;
    }

    fn disarm(&mut self) {
        self.armed = false;
        self.pending_approval_id = None;
    }
}

impl Drop for RemoteRunCancellationGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Some(approval_id) = self.pending_approval_id.take() {
            self.kernel.approval_manager.cancel_pending_request(
                approval_id,
                "The owning remote Node tool run was interrupted.",
            );
        }
        let _ = self
            .kernel
            .hub_nodes
            .request_run_cancel(&self.run_id, "caller_interrupted");
    }
}

fn validate_remote_request(request: &RemoteToolExecutionRequest) -> Result<(), String> {
    if request.scope_id.trim().is_empty()
        || request.tool_use_id.trim().is_empty()
        || request.tool_name.trim().is_empty()
        || request.caller_agent_id.trim().is_empty()
    {
        return Err("remote Node execution request is invalid".to_string());
    }
    Ok(())
}

fn remote_run_identity(request: &RemoteToolExecutionRequest) -> Result<(String, String), String> {
    let input = serde_json::to_vec(&request.input)
        .map_err(|_| "remote Node tool input could not be serialized".to_string())?;
    let mut digest = Sha256::new();
    for segment in [
        request.scope_id.as_bytes(),
        request.caller_agent_id.as_bytes(),
        request.tool_use_id.as_bytes(),
        request.tool_name.as_bytes(),
        request.device_id.as_bytes(),
        request.workspace_id.as_bytes(),
        input.as_slice(),
    ] {
        digest.update(
            u64::try_from(segment.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        digest.update(segment);
    }
    let digest = hex::encode(digest.finalize());
    Ok((format!("tool-{digest}"), format!("tool:{digest}")))
}

fn remote_run_effect(tool_name: &str, input: &serde_json::Value) -> Result<RunEffect, String> {
    match local_node_tool_effect(tool_name, input) {
        Some(LocalNodeToolEffect::ReadOnly) => Ok(RunEffect::ReadOnly),
        Some(LocalNodeToolEffect::LocalMutation) => Ok(RunEffect::LocalMutation),
        Some(LocalNodeToolEffect::ExternalEffect) => Ok(RunEffect::ExternalEffect),
        None => Err("tool is not supported by the selected Node runtime".to_string()),
    }
}

fn remote_run_wait_limit(request: &RemoteToolExecutionRequest) -> Duration {
    let explicit = match request.tool_name.as_str() {
        "shell_exec" => request.input.get("timeout_seconds"),
        _ => request.input.get("timeout_secs"),
    }
    .and_then(serde_json::Value::as_u64)
    .filter(|seconds| *seconds > 0);
    Duration::from_secs(
        explicit
            .unwrap_or(REMOTE_RUN_DEFAULT_WAIT_SECS)
            .saturating_add(30)
            .min(REMOTE_RUN_MAX_WAIT_SECS),
    )
}

fn terminal_tool_result(tool_use_id: &str, run: &HubNodeRunRecord) -> Option<ToolResult> {
    if !run.status.is_terminal() {
        return None;
    }
    if let Some(completion) = run.completion.as_ref() {
        let (content, is_error) = match completion.status {
            RunTerminalStatus::Succeeded => (completion.result_content.clone(), false),
            RunTerminalStatus::Failed => (completion.result_content.clone(), true),
            RunTerminalStatus::Cancelled => (
                format!(
                    "Remote Node execution was cancelled. {}",
                    completion.result_content
                ),
                true,
            ),
            RunTerminalStatus::Uncertain => (
                format!(
                    "Remote Node execution state is uncertain and was not retried. {}",
                    completion.result_content
                ),
                true,
            ),
        };
        return Some(ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content,
            is_error,
            transient_content: Vec::new(),
        });
    }
    if let Some(rejection) = run.rejection.as_ref() {
        return Some(ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: format!(
                "Remote Node refused the operation ({}): {}",
                rejection.code, rejection.message
            ),
            is_error: true,
            transient_content: Vec::new(),
        });
    }
    let content = match run.status {
        HubNodeRunStatus::Cancelled => "Remote Node execution was cancelled.".to_string(),
        HubNodeRunStatus::Uncertain => {
            "Remote Node execution state is uncertain and was not retried.".to_string()
        }
        _ => "Remote Node execution ended without valid terminal evidence.".to_string(),
    };
    Some(ToolResult {
        tool_use_id: tool_use_id.to_string(),
        content,
        is_error: true,
        transient_content: Vec::new(),
    })
}

fn one_shot_decision(decision: ApprovalDecision) -> ApprovalDecision {
    match decision {
        ApprovalDecision::Approved
        | ApprovalDecision::ApprovedSession
        | ApprovalDecision::ApprovedAlways => ApprovalDecision::Approved,
        ApprovalDecision::Denied
        | ApprovalDecision::DeniedSession
        | ApprovalDecision::DeniedAlways => ApprovalDecision::Denied,
        ApprovalDecision::TimedOut => ApprovalDecision::TimedOut,
    }
}

fn remote_service_error(error: &crate::hub_node_service::HubNodeServiceError) -> String {
    use crate::hub_node_service::HubNodeServiceError;
    match error {
        HubNodeServiceError::Disabled => "Hub/Node execution is disabled".to_string(),
        HubNodeServiceError::NodeUnavailable => {
            "the selected Node is unavailable or revoked".to_string()
        }
        HubNodeServiceError::NodeOffline => "the selected Node is offline".to_string(),
        HubNodeServiceError::NodeIncompatible => {
            "the selected Node protocol is incompatible".to_string()
        }
        HubNodeServiceError::WorkspaceNotGranted => {
            "the selected Node workspace is not granted".to_string()
        }
        HubNodeServiceError::ToolFamilyNotGranted => {
            "the selected Node does not grant this tool family".to_string()
        }
        HubNodeServiceError::MutationNotGranted => {
            "the selected Node workspace does not permit this mutation".to_string()
        }
        HubNodeServiceError::ToolNotSupported => {
            "the selected Node runtime does not support this tool".to_string()
        }
        HubNodeServiceError::PathPolicyViolation => {
            "Node tool paths must be relative to the selected logical workspace".to_string()
        }
        HubNodeServiceError::EffectMismatch
        | HubNodeServiceError::DeliveryInvariant
        | HubNodeServiceError::StorageUnavailable
        | HubNodeServiceError::Rail(_) => {
            "the durable Hub/Node execution rail is unavailable".to_string()
        }
        HubNodeServiceError::AuthenticationFailed
        | HubNodeServiceError::NodeRoleRequired
        | HubNodeServiceError::DeviceIdentityMismatch
        | HubNodeServiceError::InvalidTransportRequest
        | HubNodeServiceError::TransportMismatch
        | HubNodeServiceError::TransportBusy => {
            "the Hub/Node execution contract rejected the request".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{DefaultModelConfig, KernelConfig, PairingConfig};
    use captain_wire::{
        hub_protocol::RunApprovalRequest, CapabilityDescriptor, DeviceCredentialExchange,
        DeviceGrant, DevicePairingClaim, DeviceRole, HubNodeEnvelope, HubNodeMessage,
        HubNodePullRequest, LogicalWorkspace, NodeTransport, RunCompletion,
        HUB_NODE_PROTOCOL_VERSION,
    };
    use std::sync::Arc;

    struct TestNode {
        device_id: String,
        access_token: String,
        connection_id: String,
    }

    fn test_kernel(temp: &tempfile::TempDir) -> Arc<CaptainKernel> {
        Arc::new(
            CaptainKernel::boot_with_config(KernelConfig {
                home_dir: temp.path().join("home"),
                data_dir: temp.path().join("data"),
                default_model: DefaultModelConfig {
                    provider: "ollama".to_string(),
                    model: "test-model".to_string(),
                    api_key_env: "OLLAMA_API_KEY".to_string(),
                    base_url: None,
                },
                pairing: PairingConfig {
                    hub_enabled: true,
                    ..PairingConfig::default()
                },
                ..KernelConfig::default()
            })
            .unwrap(),
        )
    }

    fn pair_and_connect_node(kernel: &CaptainKernel, credential_character: char) -> TestNode {
        kernel.hub_pairing.open_enrollment_window(300).unwrap();
        let credential = std::iter::repeat(credential_character)
            .take(64)
            .collect::<String>();
        let capabilities = CapabilityDescriptor {
            captain_version: "0.1.0-alpha.14".to_string(),
            platform: "test-platform".to_string(),
            transports: vec![NodeTransport::WebSocket],
            tool_families: vec!["file".to_string()],
            workspaces: vec![LogicalWorkspace {
                workspace_id: "project-main".to_string(),
                label: "Main Project".to_string(),
                read_only: false,
            }],
            supports_streaming_output: true,
        };
        let grant = DeviceGrant {
            workspace_ids: vec!["project-main".to_string()],
            tool_families: vec!["file".to_string()],
            allow_mutation: true,
        };
        let challenge = kernel
            .hub_pairing
            .create_claim(&DevicePairingClaim {
                display_name: "Test Node".to_string(),
                role: DeviceRole::Node,
                platform: capabilities.platform.clone(),
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                credential_sha256: hex::encode(Sha256::digest(credential.as_bytes())),
                capabilities: capabilities.clone(),
                requested_grants: grant.clone(),
            })
            .unwrap();
        let device = kernel
            .hub_pairing
            .approve_request(&challenge.request_id, &grant)
            .unwrap();
        let access = kernel
            .hub_pairing
            .exchange_device_credential(&DeviceCredentialExchange {
                device_id: device.device_id.clone(),
                credential,
            })
            .unwrap();
        let node = TestNode {
            device_id: device.device_id,
            access_token: access.access_token,
            connection_id: "connection-1".to_string(),
        };
        kernel
            .hub_nodes
            .open_connection(
                &node.access_token,
                &node_envelope(
                    &node,
                    1,
                    None,
                    HubNodeMessage::Hello {
                        role: DeviceRole::Node,
                        capabilities,
                        resume_after_sequence: 0,
                        active_run_ids: Vec::new(),
                    },
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();
        node
    }

    fn request(node: &TestNode, tool_use_id: &str, tool_name: &str) -> RemoteToolExecutionRequest {
        RemoteToolExecutionRequest {
            scope_id: "session-1".to_string(),
            tool_use_id: tool_use_id.to_string(),
            tool_name: tool_name.to_string(),
            input: if tool_name == "file_write" {
                serde_json::json!({"path": "result.txt", "content": "done"})
            } else {
                serde_json::json!({"path": "result.txt"})
            },
            caller_agent_id: "agent-1".to_string(),
            device_id: node.device_id.clone(),
            workspace_id: "project-main".to_string(),
        }
    }

    fn node_envelope(
        node: &TestNode,
        sequence: u64,
        ack_sequence: Option<u64>,
        message: HubNodeMessage,
    ) -> HubNodeEnvelope {
        HubNodeEnvelope {
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            device_id: node.device_id.clone(),
            connection_id: node.connection_id.clone(),
            sequence,
            ack_sequence,
            sent_at_ms: chrono::Utc::now().timestamp_millis(),
            message,
        }
    }

    fn pull(kernel: &CaptainKernel, node: &TestNode) -> captain_wire::HubNodeDeliveryBatch {
        kernel
            .hub_nodes
            .pull(
                &node.access_token,
                &HubNodePullRequest {
                    protocol_version: HUB_NODE_PROTOCOL_VERSION,
                    device_id: node.device_id.clone(),
                    connection_id: node.connection_id.clone(),
                    max_messages: 16,
                    wait_ms: 0,
                },
                NodeTransport::WebSocket,
            )
            .unwrap()
    }

    async fn wait_for_run(
        kernel: &CaptainKernel,
        run_id: &str,
    ) -> captain_memory::hub_node_rail::HubNodeRunRecord {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(run) = kernel.hub_nodes.get_run(run_id).unwrap() {
                    return run;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("run should be durably submitted")
    }

    async fn wait_for_pending_approval(kernel: &CaptainKernel, approval_id: uuid::Uuid) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if kernel
                    .approval_manager
                    .list_pending()
                    .iter()
                    .any(|request| request.id == approval_id)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("approval should reach the shared operator surface");
    }

    fn completion(run_id: &str, status: RunTerminalStatus, content: &str) -> RunCompletion {
        RunCompletion {
            run_id: run_id.to_string(),
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

    #[test]
    fn identity_is_stable_and_never_contains_raw_input() {
        let request = RemoteToolExecutionRequest {
            scope_id: "session-1".to_string(),
            tool_use_id: "call-1".to_string(),
            tool_name: "file_read".to_string(),
            input: serde_json::json!({"path": "private-name.txt"}),
            caller_agent_id: "agent-1".to_string(),
            device_id: "node-1".to_string(),
            workspace_id: "workspace-1".to_string(),
        };
        let first = remote_run_identity(&request).unwrap();
        let second = remote_run_identity(&request).unwrap();

        assert_eq!(first, second);
        assert!(!first.0.contains("private-name"));
        assert!(!first.1.contains("private-name"));
    }

    #[test]
    fn persistent_approval_scope_is_never_forwarded_to_a_node() {
        assert_eq!(
            one_shot_decision(ApprovalDecision::ApprovedAlways),
            ApprovalDecision::Approved
        );
        assert_eq!(
            one_shot_decision(ApprovalDecision::DeniedSession),
            ApprovalDecision::Denied
        );
    }

    #[tokio::test]
    async fn durable_remote_run_relays_progress_and_returns_terminal_output() {
        let temp = tempfile::tempdir().unwrap();
        let kernel = test_kernel(&temp);
        let node = pair_and_connect_node(&kernel, 'a');
        let request = request(&node, "call-progress", "file_read");
        let (run_id, _) = remote_run_identity(&request).unwrap();
        let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(4);
        let task_kernel = Arc::clone(&kernel);
        let task = tokio::spawn(async move {
            captain_runtime::tool_runner::with_progress_sink(
                progress_tx,
                task_kernel.handle_execute_remote_tool(request),
            )
            .await
        });

        let run = wait_for_run(&kernel, &run_id).await;
        assert_eq!(run.status, HubNodeRunStatus::Leased);
        let offer_sequence = pull(&kernel, &node).messages[0].sequence;
        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    2,
                    Some(offer_sequence),
                    HubNodeMessage::RunAccepted {
                        run_id: run_id.clone(),
                        attempt: 1,
                    },
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();
        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    3,
                    Some(offer_sequence),
                    HubNodeMessage::RunProgress {
                        run_id: run_id.clone(),
                        attempt: 1,
                        progress_sequence: 1,
                        message: "Reading selected workspace file".to_string(),
                        path_policy_applied: true,
                    },
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();
        let progress = tokio::time::timeout(Duration::from_secs(2), progress_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(progress.tool_use_id, "call-progress");
        assert_eq!(progress.message, "Reading selected workspace file");
        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    4,
                    Some(offer_sequence),
                    HubNodeMessage::RunCompleted(completion(
                        &run_id,
                        RunTerminalStatus::Succeeded,
                        "workspace://project-main/result.txt",
                    )),
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();

        let result = task.await.unwrap().unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "workspace://project-main/result.txt");
    }

    #[tokio::test]
    async fn node_approval_uses_shared_surface_but_forwards_only_one_shot_decision() {
        let temp = tempfile::tempdir().unwrap();
        let kernel = test_kernel(&temp);
        let node = pair_and_connect_node(&kernel, 'b');
        let request = request(&node, "call-approval", "file_write");
        let (run_id, _) = remote_run_identity(&request).unwrap();
        let task_kernel = Arc::clone(&kernel);
        let task =
            tokio::spawn(async move { task_kernel.handle_execute_remote_tool(request).await });

        wait_for_run(&kernel, &run_id).await;
        let offer_sequence = pull(&kernel, &node).messages[0].sequence;
        let approval_id = uuid::Uuid::new_v4();
        let action_digest =
            captain_types::approval::approval_action_digest("file_write", b"bounded-test-action");
        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    2,
                    Some(offer_sequence),
                    HubNodeMessage::RunApprovalRequired(RunApprovalRequest {
                        run_id: run_id.clone(),
                        attempt: 1,
                        approval_id: approval_id.to_string(),
                        action_digest: action_digest.clone(),
                        action_summary: "Authorize one local mutation on the selected workspace."
                            .to_string(),
                        risk_level: captain_types::approval::RiskLevel::High,
                        expires_at_ms: chrono::Utc::now().timestamp_millis() + 30_000,
                        path_policy_applied: true,
                    }),
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();
        wait_for_pending_approval(&kernel, approval_id).await;
        kernel
            .approval_manager
            .resolve(
                approval_id,
                ApprovalDecision::ApprovedAlways,
                Some("test-operator".to_string()),
            )
            .unwrap();

        let decision_batch = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let batch = pull(&kernel, &node);
                if batch.messages.iter().any(|envelope| {
                    matches!(envelope.message, HubNodeMessage::RunApprovalDecision(_))
                }) {
                    return batch;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("approval decision should reach the Node");
        let decision_envelope = decision_batch
            .messages
            .iter()
            .find(|envelope| matches!(envelope.message, HubNodeMessage::RunApprovalDecision(_)))
            .unwrap();
        let HubNodeMessage::RunApprovalDecision(decision) = &decision_envelope.message else {
            unreachable!();
        };
        assert_eq!(decision.decision, ApprovalDecision::Approved);
        assert_eq!(decision.action_digest, action_digest);

        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    3,
                    Some(decision_envelope.sequence),
                    HubNodeMessage::RunAccepted {
                        run_id: run_id.clone(),
                        attempt: 1,
                    },
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();
        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    4,
                    Some(decision_envelope.sequence),
                    HubNodeMessage::RunCompleted(completion(
                        &run_id,
                        RunTerminalStatus::Succeeded,
                        "written",
                    )),
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();

        let result = task.await.unwrap().unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "written");
    }

    #[tokio::test]
    async fn aborting_the_caller_requests_durable_node_cancellation() {
        let temp = tempfile::tempdir().unwrap();
        let kernel = test_kernel(&temp);
        let node = pair_and_connect_node(&kernel, 'c');
        let request = request(&node, "call-cancel", "file_read");
        let (run_id, _) = remote_run_identity(&request).unwrap();
        let task_kernel = Arc::clone(&kernel);
        let task =
            tokio::spawn(async move { task_kernel.handle_execute_remote_tool(request).await });

        wait_for_run(&kernel, &run_id).await;
        let offer_sequence = pull(&kernel, &node).messages[0].sequence;
        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    2,
                    Some(offer_sequence),
                    HubNodeMessage::RunAccepted {
                        run_id: run_id.clone(),
                        attempt: 1,
                    },
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();
        task.abort();
        let _ = task.await;

        let run = kernel.hub_nodes.get_run(&run_id).unwrap().unwrap();
        assert_eq!(run.status, HubNodeRunStatus::CancelRequested);
        assert!(run.cancel_requested_at_ms.is_some());
    }

    #[tokio::test]
    async fn aborting_during_node_approval_removes_the_shared_prompt() {
        let temp = tempfile::tempdir().unwrap();
        let kernel = test_kernel(&temp);
        let node = pair_and_connect_node(&kernel, 'e');
        let request = request(&node, "call-cancel-approval", "file_write");
        let (run_id, _) = remote_run_identity(&request).unwrap();
        let task_kernel = Arc::clone(&kernel);
        let task =
            tokio::spawn(async move { task_kernel.handle_execute_remote_tool(request).await });

        wait_for_run(&kernel, &run_id).await;
        let offer_sequence = pull(&kernel, &node).messages[0].sequence;
        let approval_id = uuid::Uuid::new_v4();
        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    2,
                    Some(offer_sequence),
                    HubNodeMessage::RunApprovalRequired(RunApprovalRequest {
                        run_id: run_id.clone(),
                        attempt: 1,
                        approval_id: approval_id.to_string(),
                        action_digest: captain_types::approval::approval_action_digest(
                            "file_write",
                            b"cancelled-approval-action",
                        ),
                        action_summary: "Authorize one local mutation.".to_string(),
                        risk_level: captain_types::approval::RiskLevel::High,
                        expires_at_ms: chrono::Utc::now().timestamp_millis() + 30_000,
                        path_policy_applied: true,
                    }),
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();
        wait_for_pending_approval(&kernel, approval_id).await;

        task.abort();
        let _ = task.await;

        assert!(kernel.approval_manager.list_pending().is_empty());
        let run = kernel.hub_nodes.get_run(&run_id).unwrap().unwrap();
        assert_eq!(run.status, HubNodeRunStatus::CancelRequested);
        assert!(run.cancel_requested_at_ms.is_some());
    }

    #[tokio::test]
    async fn lost_connection_returns_uncertain_without_replaying_a_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let kernel = test_kernel(&temp);
        let node = pair_and_connect_node(&kernel, 'd');
        let request = request(&node, "call-uncertain", "file_write");
        let (run_id, _) = remote_run_identity(&request).unwrap();
        let task_kernel = Arc::clone(&kernel);
        let task =
            tokio::spawn(async move { task_kernel.handle_execute_remote_tool(request).await });

        wait_for_run(&kernel, &run_id).await;
        let offer_sequence = pull(&kernel, &node).messages[0].sequence;
        kernel
            .hub_nodes
            .apply_envelope(
                &node.access_token,
                &node_envelope(
                    &node,
                    2,
                    Some(offer_sequence),
                    HubNodeMessage::RunAccepted {
                        run_id: run_id.clone(),
                        attempt: 1,
                    },
                ),
                NodeTransport::WebSocket,
            )
            .unwrap();
        kernel
            .hub_nodes
            .close_connection(
                &node.access_token,
                &node.device_id,
                &node.connection_id,
                Some("network_lost"),
            )
            .unwrap();

        let result = task.await.unwrap().unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("uncertain"));
        assert_eq!(
            kernel.hub_nodes.get_run(&run_id).unwrap().unwrap().status,
            HubNodeRunStatus::Uncertain
        );
    }
}

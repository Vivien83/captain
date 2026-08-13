use super::*;
use captain_memory::hub_node_rail::{
    HubNodeConnectionStatus, HubNodeInboundOutcome, HubNodeRunStatus, NewHubNodeRun,
};
use captain_memory::MemorySubstrate;
use captain_types::config::PairingConfig;
use captain_wire::{
    CapabilityDescriptor, DeviceCredentialExchange, DeviceGrant, DevicePairingClaim,
    LogicalWorkspace, RunEffect, HUB_NODE_PROTOCOL_VERSION,
};
use serde_json::json;

struct PairedTestDevice {
    device_id: String,
    access_token: String,
    capabilities: CapabilityDescriptor,
}

fn enabled_services() -> (MemorySubstrate, Arc<HubPairingService>, HubNodeService) {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let pairing = Arc::new(HubPairingService::new(
        PairingConfig {
            hub_enabled: true,
            ..PairingConfig::default()
        },
        memory.devices().clone(),
    ));
    pairing.open_enrollment_window(300).unwrap();
    let nodes = HubNodeService::new(Arc::clone(&pairing), memory.hub_node_rail().clone());
    (memory, pairing, nodes)
}

fn capabilities(role: DeviceRole) -> CapabilityDescriptor {
    let execution = role == DeviceRole::Node;
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "macos-arm64".to_string(),
        transports: vec![
            NodeTransport::WebSocket,
            NodeTransport::HttpStream,
            NodeTransport::LongPoll,
        ],
        tool_families: execution
            .then(|| vec!["shell-process".to_string()])
            .unwrap_or_default(),
        workspaces: execution
            .then(|| {
                vec![LogicalWorkspace {
                    workspace_id: "project-main".to_string(),
                    label: "Main Project".to_string(),
                    read_only: false,
                }]
            })
            .unwrap_or_default(),
        supports_streaming_output: execution,
    }
}

fn requested_grant(role: DeviceRole) -> DeviceGrant {
    if role == DeviceRole::Node {
        DeviceGrant {
            workspace_ids: vec!["project-main".to_string()],
            tool_families: vec!["shell-process".to_string()],
            allow_mutation: false,
        }
    } else {
        DeviceGrant::default()
    }
}

fn pair_device(
    pairing: &HubPairingService,
    role: DeviceRole,
    credential_character: char,
) -> PairedTestDevice {
    let credential = std::iter::repeat(credential_character)
        .take(64)
        .collect::<String>();
    let capabilities = capabilities(role);
    let grant = requested_grant(role);
    let challenge = pairing
        .create_claim(&DevicePairingClaim {
            display_name: format!("Test {role:?}"),
            role,
            platform: capabilities.platform.clone(),
            protocol_version: HUB_NODE_PROTOCOL_VERSION,
            credential_sha256: sha256_hex(credential.as_bytes()),
            capabilities: capabilities.clone(),
            requested_grants: grant.clone(),
        })
        .unwrap();
    let device = pairing
        .approve_request(&challenge.request_id, &grant)
        .unwrap();
    let access = pairing
        .exchange_device_credential(&DeviceCredentialExchange {
            device_id: device.device_id.clone(),
            credential,
        })
        .unwrap();
    PairedTestDevice {
        device_id: device.device_id,
        access_token: access.access_token,
        capabilities,
    }
}

fn hello(
    device: &PairedTestDevice,
    connection_id: &str,
    sequence: u64,
    ack: u64,
) -> HubNodeEnvelope {
    node_envelope(
        device,
        connection_id,
        sequence,
        (ack > 0).then_some(ack),
        HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: device.capabilities.clone(),
            resume_after_sequence: ack,
            active_run_ids: Vec::new(),
        },
    )
}

fn node_envelope(
    device: &PairedTestDevice,
    connection_id: &str,
    sequence: u64,
    ack_sequence: Option<u64>,
    message: HubNodeMessage,
) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: device.device_id.clone(),
        connection_id: connection_id.to_string(),
        sequence,
        ack_sequence,
        sent_at_ms: chrono::Utc::now().timestamp_millis(),
        message,
    }
}

fn pull(device: &PairedTestDevice, connection_id: &str) -> HubNodePullRequest {
    HubNodePullRequest {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: device.device_id.clone(),
        connection_id: connection_id.to_string(),
        max_messages: 64,
        wait_ms: 0,
    }
}

fn submitted_run(device_id: &str, run_id: &str, effect: RunEffect) -> NewHubNodeRun {
    NewHubNodeRun {
        run_id: run_id.to_string(),
        device_id: device_id.to_string(),
        idempotency_key: format!("idem-{run_id}"),
        workspace_id: "project-main".to_string(),
        tool_name: "shell_exec".to_string(),
        input: json!({"command": "printf ready"}),
        effect,
        created_at_ms: chrono::Utc::now().timestamp_millis(),
    }
}

#[test]
fn paired_node_opens_replays_and_completes_the_delivery_handshake() {
    let (memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'a');
    let hello = hello(&node, "connection-1", 1, 0);

    let opened = service
        .open_connection(&node.access_token, &hello, NodeTransport::WebSocket)
        .unwrap();
    assert_eq!(opened.acknowledged_node_sequence, 1);
    assert_eq!(opened.messages.len(), 1);
    assert!(matches!(
        opened.messages[0].message,
        HubNodeMessage::Welcome {
            transport: NodeTransport::WebSocket,
            ..
        }
    ));
    assert_eq!(
        service
            .open_connection(&node.access_token, &hello, NodeTransport::WebSocket)
            .unwrap(),
        opened
    );

    let now_ms = chrono::Utc::now().timestamp_millis();
    memory
        .hub_node_rail()
        .enqueue_run(&NewHubNodeRun {
            run_id: "run-1".to_string(),
            device_id: node.device_id.clone(),
            idempotency_key: "idem-1".to_string(),
            workspace_id: "project-main".to_string(),
            tool_name: "shell_exec".to_string(),
            input: json!({"command": "true"}),
            effect: RunEffect::ReadOnly,
            created_at_ms: now_ms,
        })
        .unwrap();
    memory
        .hub_node_rail()
        .lease_next(&node.device_id, "connection-1", now_ms + 1, 60_000)
        .unwrap()
        .unwrap();

    let acknowledgement = node_envelope(&node, "connection-1", 2, Some(1), HubNodeMessage::AckOnly);
    assert!(matches!(
        service.apply_envelope(
            &node.access_token,
            &acknowledgement,
            NodeTransport::LongPoll,
        ),
        Err(HubNodeServiceError::TransportMismatch)
    ));
    let (acknowledged, offered) = service
        .apply_envelope(
            &node.access_token,
            &acknowledgement,
            NodeTransport::WebSocket,
        )
        .unwrap();
    assert_eq!(acknowledged.outcome, HubNodeInboundOutcome::Acknowledged);
    assert_eq!(offered.messages.len(), 1);
    assert!(matches!(
        offered.messages[0].message,
        HubNodeMessage::RunOffer(ref lease) if lease.run_id == "run-1"
    ));

    let (accepted, after_accept) = service
        .apply_envelope(
            &node.access_token,
            &node_envelope(
                &node,
                "connection-1",
                3,
                Some(2),
                HubNodeMessage::RunAccepted {
                    run_id: "run-1".to_string(),
                    attempt: 1,
                },
            ),
            NodeTransport::WebSocket,
        )
        .unwrap();
    assert!(matches!(
        accepted.outcome,
        HubNodeInboundOutcome::RunAccepted(ref run) if run.status == HubNodeRunStatus::Accepted
    ));
    assert!(after_accept.messages.is_empty());
    assert_eq!(after_accept.acknowledged_node_sequence, 3);
}

#[tokio::test]
async fn production_submission_validates_grants_offers_exactly_once_and_wakes_waiters() {
    let (memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'b');
    let current = submitted_run(&node.device_id, "run-current", RunEffect::ReadOnly);

    assert!(matches!(
        service.submit_run(&current),
        Err(HubNodeServiceError::NodeOffline)
    ));
    assert!(service.get_run("run-current").unwrap().is_none());

    service
        .open_connection(
            &node.access_token,
            &hello(&node, "connection-submit", 1, 0),
            NodeTransport::WebSocket,
        )
        .unwrap();
    memory
        .hub_node_rail()
        .enqueue_run(&submitted_run(
            &node.device_id,
            "run-older",
            RunEffect::ReadOnly,
        ))
        .unwrap();

    let waiter_service = service.clone();
    let waiter = tokio::spawn(async move {
        waiter_service
            .wait_for_activity(Duration::from_secs(2))
            .await;
    });
    tokio::task::yield_now().await;
    let submitted = service.submit_run(&current).unwrap();
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("submission should wake durable run waiters")
        .unwrap();

    assert_eq!(submitted.status, HubNodeRunStatus::Leased);
    assert_eq!(submitted.run_id, "run-current");
    assert_eq!(
        service.get_run("run-older").unwrap().unwrap().status,
        HubNodeRunStatus::Queued
    );
    let replay = service.submit_run(&current).unwrap();
    assert_eq!(replay.run_id, submitted.run_id);
    assert_eq!(replay.attempt, 1);

    let pending = memory
        .hub_node_rail()
        .pending_outbox(&node.device_id, 0, 64)
        .unwrap();
    assert_eq!(
        pending
            .iter()
            .filter(|record| record.message_kind == "run_offer")
            .count(),
        1
    );
}

#[test]
fn production_submission_rejects_physical_paths_before_persistence() {
    let (_memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'e');
    service
        .open_connection(
            &node.access_token,
            &hello(&node, "connection-path-policy", 1, 0),
            NodeTransport::WebSocket,
        )
        .unwrap();
    let mut run = submitted_run(&node.device_id, "run-physical-path", RunEffect::ReadOnly);
    run.input = json!({"command": "cat /Users/private/notes.txt"});

    assert!(matches!(
        service.submit_run(&run),
        Err(HubNodeServiceError::PathPolicyViolation)
    ));
    assert!(service.get_run(&run.run_id).unwrap().is_none());
}

#[test]
fn production_submission_rejects_ungranted_family_and_mutation() {
    let (_memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'c');
    service
        .open_connection(
            &node.access_token,
            &hello(&node, "connection-grants", 1, 0),
            NodeTransport::WebSocket,
        )
        .unwrap();

    assert!(matches!(
        service.submit_run(&NewHubNodeRun {
            tool_name: "file_read".to_string(),
            input: json!({"path": "README.md"}),
            ..submitted_run(&node.device_id, "run-file", RunEffect::ReadOnly)
        }),
        Err(HubNodeServiceError::ToolFamilyNotGranted)
    ));
    let mutation = NewHubNodeRun {
        input: json!({"command": "touch changed.txt"}),
        ..submitted_run(&node.device_id, "run-mutation", RunEffect::LocalMutation)
    };
    assert!(matches!(
        service.submit_run(&mutation),
        Err(HubNodeServiceError::MutationNotGranted)
    ));
}

#[test]
fn production_submission_derives_family_and_effect_from_exact_work() {
    let (_memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'd');
    service
        .open_connection(
            &node.access_token,
            &hello(&node, "connection-contract", 1, 0),
            NodeTransport::WebSocket,
        )
        .unwrap();

    let mut mismatched = submitted_run(&node.device_id, "run-effect", RunEffect::ReadOnly);
    mismatched.input = json!({"command": "touch changed.txt"});
    assert!(matches!(
        service.submit_run(&mismatched),
        Err(HubNodeServiceError::EffectMismatch)
    ));

    let unsupported = NewHubNodeRun {
        tool_name: "memory_save".to_string(),
        input: json!({"text": "never dispatched"}),
        ..submitted_run(&node.device_id, "run-unsupported", RunEffect::ReadOnly)
    };
    assert!(matches!(
        service.submit_run(&unsupported),
        Err(HubNodeServiceError::ToolNotSupported)
    ));
}

#[test]
fn transport_fallback_delivers_explicit_tombstones_without_sequence_gaps() {
    let (_memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'f');
    let hello = hello(&node, "connection-stable", 1, 0);

    let first = service
        .open_connection(&node.access_token, &hello, NodeTransport::WebSocket)
        .unwrap();
    assert_eq!(first.messages[0].sequence, 1);

    let fallback = service
        .open_connection(&node.access_token, &hello, NodeTransport::LongPoll)
        .unwrap();
    assert_eq!(
        fallback
            .messages
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(matches!(
        &fallback.messages[0].message,
        HubNodeMessage::Superseded {
            original_message_kind,
            original_message_sha256,
        } if original_message_kind == "welcome" && original_message_sha256.len() == 64
    ));
    assert!(matches!(
        fallback.messages[1].message,
        HubNodeMessage::Welcome {
            transport: NodeTransport::LongPoll,
            ..
        }
    ));

    let second_fallback = service
        .open_connection(&node.access_token, &hello, NodeTransport::HttpStream)
        .unwrap();
    assert_eq!(
        second_fallback
            .messages
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(second_fallback.messages[..2]
        .iter()
        .all(|message| matches!(message.message, HubNodeMessage::Superseded { .. })));
    assert!(matches!(
        second_fallback.messages[2].message,
        HubNodeMessage::Welcome {
            transport: NodeTransport::HttpStream,
            ..
        }
    ));
}

#[test]
fn transport_permits_are_unique_per_device_and_transport_and_release_on_drop() {
    let (_memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'd');

    let stream = service
        .acquire_transport_permit(
            &node.access_token,
            &node.device_id,
            NodeTransport::HttpStream,
        )
        .unwrap();
    assert!(matches!(
        service.acquire_transport_permit(
            &node.access_token,
            &node.device_id,
            NodeTransport::HttpStream,
        ),
        Err(HubNodeServiceError::TransportBusy)
    ));
    let poll = service
        .acquire_transport_permit(&node.access_token, &node.device_id, NodeTransport::LongPoll)
        .unwrap();

    drop(stream);
    service
        .acquire_transport_permit(
            &node.access_token,
            &node.device_id,
            NodeTransport::HttpStream,
        )
        .unwrap();
    drop(poll);
}

#[test]
fn transport_permit_closes_its_connection_without_reusing_the_bearer() {
    let (_memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'd');
    let hello = node_envelope(
        &node,
        "permit-connection",
        1,
        None,
        HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities: node.capabilities.clone(),
            resume_after_sequence: 0,
            active_run_ids: Vec::new(),
        },
    );
    service
        .open_connection(&node.access_token, &hello, NodeTransport::WebSocket)
        .unwrap();
    let permit = service
        .acquire_transport_permit(
            &node.access_token,
            &node.device_id,
            NodeTransport::WebSocket,
        )
        .unwrap();

    assert!(matches!(
        service.close_permitted_connection(
            &permit,
            &node.device_id,
            "permit-connection",
            NodeTransport::HttpStream,
            Some("stream_closed"),
        ),
        Err(HubNodeServiceError::DeviceIdentityMismatch)
    ));
    let closed = service
        .close_permitted_connection(
            &permit,
            &node.device_id,
            "permit-connection",
            NodeTransport::WebSocket,
            Some("token_expired"),
        )
        .unwrap();
    assert_eq!(
        closed.status,
        captain_memory::hub_node_rail::HubNodeConnectionStatus::Offline
    );
    assert_eq!(closed.last_error_code.as_deref(), Some("token_expired"));
}

#[test]
fn bearer_identity_and_node_role_are_enforced_before_transport_state() {
    let (_memory, pairing, service) = enabled_services();
    let first = pair_device(&pairing, DeviceRole::Node, 'a');
    let second = pair_device(&pairing, DeviceRole::Node, 'b');
    let client = pair_device(&pairing, DeviceRole::Client, 'c');

    assert!(matches!(
        service.open_connection(
            &"d".repeat(64),
            &hello(&first, "connection-1", 1, 0),
            NodeTransport::LongPoll,
        ),
        Err(HubNodeServiceError::AuthenticationFailed)
    ));
    assert!(matches!(
        service.open_connection(
            &first.access_token,
            &hello(&second, "connection-2", 1, 0),
            NodeTransport::LongPoll,
        ),
        Err(HubNodeServiceError::DeviceIdentityMismatch)
    ));
    assert!(matches!(
        service.open_connection(
            &client.access_token,
            &hello(&client, "connection-client", 1, 0),
            NodeTransport::LongPoll,
        ),
        Err(HubNodeServiceError::NodeRoleRequired)
    ));
}

#[test]
fn revocation_immediately_closes_presence_and_blocks_delivery() {
    let (memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'a');
    service
        .open_connection(
            &node.access_token,
            &hello(&node, "connection-1", 1, 0),
            NodeTransport::LongPoll,
        )
        .unwrap();

    pairing.revoke_device(&node.device_id).unwrap();
    assert!(matches!(
        service.pull(
            &node.access_token,
            &pull(&node, "connection-1"),
            NodeTransport::LongPoll,
        ),
        Err(HubNodeServiceError::AuthenticationFailed)
    ));
    assert_eq!(
        memory
            .hub_node_rail()
            .connection(&node.device_id)
            .unwrap()
            .unwrap()
            .status,
        HubNodeConnectionStatus::Offline
    );
}

#[test]
fn reconnect_invalidates_stale_connection_and_pull_bounds_fail_closed() {
    let (_memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'a');
    service
        .open_connection(
            &node.access_token,
            &hello(&node, "connection-1", 1, 0),
            NodeTransport::LongPoll,
        )
        .unwrap();
    service
        .open_connection(
            &node.access_token,
            &hello(&node, "connection-2", 2, 1),
            NodeTransport::LongPoll,
        )
        .unwrap();

    assert!(matches!(
        service.pull(
            &node.access_token,
            &pull(&node, "connection-1"),
            NodeTransport::LongPoll,
        ),
        Err(HubNodeServiceError::Rail(
            HubNodeRailError::ConnectionConflict
        ))
    ));
    let mut malformed = pull(&node, "connection-2");
    malformed.max_messages = 0;
    assert!(matches!(
        service.pull(&node.access_token, &malformed, NodeTransport::LongPoll,),
        Err(HubNodeServiceError::InvalidTransportRequest)
    ));
    let (_, after_ack) = service
        .apply_envelope(
            &node.access_token,
            &node_envelope(&node, "connection-2", 3, Some(2), HubNodeMessage::AckOnly),
            NodeTransport::LongPoll,
        )
        .unwrap();
    assert!(after_ack.messages.is_empty());
    let empty = service
        .pull(
            &node.access_token,
            &pull(&node, "connection-2"),
            NodeTransport::LongPoll,
        )
        .unwrap();
    assert_eq!(empty.retry_after_ms, Some(HUB_NODE_EMPTY_POLL_RETRY_MS));
    assert!(empty.messages.is_empty());

    let closed = service
        .close_connection(
            &node.access_token,
            &node.device_id,
            "connection-2",
            Some("client_shutdown"),
        )
        .unwrap();
    assert_eq!(closed.status, HubNodeConnectionStatus::Offline);
    assert_eq!(
        service
            .close_connection(
                &node.access_token,
                &node.device_id,
                "connection-2",
                Some("client_shutdown"),
            )
            .unwrap(),
        closed
    );
}

#[test]
fn durable_message_digest_is_verified_before_delivery() {
    let (memory, pairing, service) = enabled_services();
    let node = pair_device(&pairing, DeviceRole::Node, 'a');
    service
        .open_connection(
            &node.access_token,
            &hello(&node, "connection-1", 1, 0),
            NodeTransport::LongPoll,
        )
        .unwrap();
    memory
        .usage_conn()
        .lock()
        .unwrap()
        .execute(
            "UPDATE hub_node_outbox SET message_sha256 = ?2
             WHERE device_id = ?1 AND sequence = 1",
            rusqlite::params![node.device_id, "f".repeat(64)],
        )
        .unwrap();

    assert!(matches!(
        service.pull(
            &node.access_token,
            &pull(&node, "connection-1"),
            NodeTransport::LongPoll,
        ),
        Err(HubNodeServiceError::DeliveryInvariant)
    ));
}

#[test]
fn disabled_pairing_disables_the_transport_boundary() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let pairing = Arc::new(HubPairingService::new(
        PairingConfig {
            hub_enabled: false,
            ..PairingConfig::default()
        },
        memory.devices().clone(),
    ));
    let service = HubNodeService::new(pairing, memory.hub_node_rail().clone());
    let fake = PairedTestDevice {
        device_id: "node-disabled".to_string(),
        access_token: "a".repeat(64),
        capabilities: capabilities(DeviceRole::Node),
    };
    assert!(matches!(
        service.open_connection(
            &fake.access_token,
            &hello(&fake, "connection-1", 1, 0),
            NodeTransport::LongPoll,
        ),
        Err(HubNodeServiceError::Disabled)
    ));
}

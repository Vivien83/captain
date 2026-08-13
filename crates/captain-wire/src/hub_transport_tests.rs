use super::*;
use crate::hub_protocol::{
    DeviceRole, HubNodeMessage, NodeTransport, RunApprovalDecision, RunApprovalRequest,
    RunRejection,
};
use captain_types::approval::{approval_action_digest, ApprovalDecision, RiskLevel};

fn envelope(sequence: u64, message: HubNodeMessage) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-1".to_string(),
        connection_id: "connection-1".to_string(),
        sequence,
        ack_sequence: None,
        sent_at_ms: 100 + sequence as i64,
        message,
    }
}

fn welcome() -> HubNodeMessage {
    HubNodeMessage::Welcome {
        negotiated_version: HUB_NODE_PROTOCOL_VERSION,
        transport: NodeTransport::LongPoll,
        heartbeat_interval_ms: 15_000,
        lease_duration_ms: 60_000,
    }
}

fn hello(transports: Vec<NodeTransport>) -> HubNodeMessage {
    HubNodeMessage::Hello {
        role: DeviceRole::Node,
        capabilities: crate::hub_protocol::CapabilityDescriptor {
            captain_version: "alpha.14".to_string(),
            platform: "linux".to_string(),
            transports,
            tool_families: vec![],
            workspaces: vec![],
            supports_streaming_output: false,
        },
        resume_after_sequence: 0,
        active_run_ids: Vec::new(),
    }
}

fn batch(messages: Vec<HubNodeEnvelope>) -> HubNodeDeliveryBatch {
    HubNodeDeliveryBatch {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-1".to_string(),
        connection_id: "connection-1".to_string(),
        acknowledged_node_sequence: 1,
        messages,
        retry_after_ms: Some(1_000),
    }
}

#[test]
fn pull_requests_are_bounded_and_transport_neutral() {
    let mut request = HubNodePullRequest {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-1".to_string(),
        connection_id: "connection-1".to_string(),
        max_messages: MAX_HUB_NODE_BATCH_MESSAGES as u16,
        wait_ms: MAX_HUB_NODE_LONG_POLL_WAIT_MS,
    };
    request.validate().unwrap();
    request.max_messages = 0;
    assert_eq!(
        request.validate(),
        Err(HubTransportContractError::InvalidBatchLimit)
    );
    request.max_messages = MAX_HUB_NODE_BATCH_MESSAGES as u16;
    request.max_messages += 1;
    assert_eq!(
        request.validate(),
        Err(HubTransportContractError::InvalidBatchLimit)
    );
    request.max_messages = 1;
    request.wait_ms += 1;
    assert_eq!(
        request.validate(),
        Err(HubTransportContractError::InvalidWait)
    );
}

#[test]
fn shared_transport_paths_are_exact_and_versioned_under_the_hub_namespace() {
    assert_eq!(HUB_NODE_CONNECT_PATH, "/api/hub/nodes/connect");
    assert_eq!(HUB_NODE_ENVELOPE_PATH, "/api/hub/nodes/envelope");
    assert_eq!(HUB_NODE_PULL_PATH, "/api/hub/nodes/pull");
    assert_eq!(HUB_NODE_STREAM_PATH, "/api/hub/nodes/stream");
    assert_eq!(HUB_NODE_WEBSOCKET_PATH, "/api/hub/nodes/ws");
    assert_eq!(HUB_NODE_CLOSE_PATH, "/api/hub/nodes/close");
}

#[test]
fn websocket_frames_are_tagged_directional_and_debug_redacted() {
    let node = envelope(
        2,
        HubNodeMessage::ProtocolError {
            code: "node_failure".to_string(),
            message: "node-secret-must-not-appear".to_string(),
            retryable: false,
            path_policy_applied: true,
        },
    );
    let frame = HubNodeWebSocketFrame::NodeEnvelope {
        envelope: node.clone(),
    };
    frame.validate().unwrap();
    let encoded = serde_json::to_value(&frame).unwrap();
    assert_eq!(encoded["type"], "node_envelope");
    assert!(!format!("{frame:?}").contains("node-secret-must-not-appear"));

    let wrong_node_direction = HubNodeWebSocketFrame::NodeEnvelope {
        envelope: HubNodeEnvelope {
            message: welcome(),
            ..node
        },
    };
    assert_eq!(
        wrong_node_direction.validate(),
        Err(HubTransportContractError::InvalidMessageDirection)
    );

    let delivery = batch(vec![envelope(
        4,
        HubNodeMessage::ProtocolError {
            code: "local_failure".to_string(),
            message: "must-not-appear".to_string(),
            retryable: false,
            path_policy_applied: true,
        },
    )]);
    let frame = HubNodeWebSocketFrame::HubDelivery { batch: delivery };
    frame.validate().unwrap();
    assert_eq!(
        serde_json::to_value(&frame).unwrap()["type"],
        "hub_delivery"
    );
    let debug = format!("{frame:?}");
    assert!(!debug.contains("must-not-appear"));
    assert!(debug.contains("message_count"));

    let tombstone = HubNodeWebSocketFrame::HubDelivery {
        batch: batch(vec![envelope(
            4,
            HubNodeMessage::Superseded {
                original_message_kind: "welcome".to_string(),
                original_message_sha256: "a".repeat(64),
            },
        )]),
    };
    tombstone.validate().unwrap();
    let mut invalid_tombstone = tombstone;
    let HubNodeWebSocketFrame::HubDelivery {
        batch: delivery_batch,
    } = &mut invalid_tombstone
    else {
        unreachable!()
    };
    let HubNodeMessage::Superseded {
        original_message_sha256,
        ..
    } = &mut delivery_batch.messages[0].message
    else {
        unreachable!()
    };
    *original_message_sha256 = "not-a-digest".to_string();
    assert!(invalid_tombstone.validate().is_err());

    let wrong_hub_direction = HubNodeWebSocketFrame::HubDelivery {
        batch: batch(vec![envelope(
            4,
            HubNodeMessage::Heartbeat {
                active_run_ids: Vec::new(),
            },
        )]),
    };
    assert_eq!(
        wrong_hub_direction.validate(),
        Err(HubTransportContractError::InvalidMessageDirection)
    );
}

#[test]
fn connect_and_ingress_requests_enforce_direction_and_advertised_transport() {
    let connect = HubNodeConnectRequest {
        transport: NodeTransport::LongPoll,
        hello: envelope(
            1,
            hello(vec![NodeTransport::LongPoll, NodeTransport::HttpStream]),
        ),
    };
    connect.validate().unwrap();
    assert_eq!(
        serde_json::from_str::<HubNodeConnectRequest>(&serde_json::to_string(&connect).unwrap())
            .unwrap(),
        connect
    );

    let mut mismatch = connect.clone();
    mismatch.transport = NodeTransport::WebSocket;
    assert_eq!(
        mismatch.validate(),
        Err(HubTransportContractError::TransportMismatch)
    );
    let mut wrong_direction = connect;
    wrong_direction.hello.message = welcome();
    assert_eq!(
        wrong_direction.validate(),
        Err(HubTransportContractError::InvalidMessageDirection)
    );

    let ingress = HubNodeIngressRequest {
        transport: NodeTransport::LongPoll,
        envelope: envelope(2, HubNodeMessage::AckOnly),
    };
    ingress.validate().unwrap();
    let mut hello_ingress = ingress;
    hello_ingress.envelope.message = hello(vec![NodeTransport::LongPoll]);
    assert_eq!(
        hello_ingress.validate(),
        Err(HubTransportContractError::InvalidMessageDirection)
    );
    hello_ingress.envelope.message = HubNodeMessage::Superseded {
        original_message_kind: "welcome".to_string(),
        original_message_sha256: "a".repeat(64),
    };
    assert_eq!(
        hello_ingress.validate(),
        Err(HubTransportContractError::InvalidMessageDirection)
    );
}

#[test]
fn run_policy_messages_are_accepted_only_in_their_protocol_direction() {
    let digest = approval_action_digest("shell_exec", b"exact action");
    let required = HubNodeMessage::RunApprovalRequired(RunApprovalRequest {
        run_id: "run-1".to_string(),
        attempt: 1,
        approval_id: "approval-1".to_string(),
        action_digest: digest.clone(),
        action_summary: "Run within workspace://workspace-main".to_string(),
        risk_level: RiskLevel::High,
        expires_at_ms: 10_000,
        path_policy_applied: true,
    });
    HubNodeIngressRequest {
        transport: NodeTransport::LongPoll,
        envelope: envelope(2, required.clone()),
    }
    .validate()
    .unwrap();
    assert_eq!(
        batch(vec![envelope(2, required)]).validate(),
        Err(HubTransportContractError::InvalidMessageDirection)
    );

    let rejected = HubNodeMessage::RunRejected(RunRejection {
        run_id: "run-1".to_string(),
        attempt: 1,
        code: "policy_denied".to_string(),
        message: "Local policy denied the exact action".to_string(),
        retryable: false,
        path_policy_applied: true,
    });
    HubNodeIngressRequest {
        transport: NodeTransport::LongPoll,
        envelope: envelope(3, rejected),
    }
    .validate()
    .unwrap();

    let decision = HubNodeMessage::RunApprovalDecision(RunApprovalDecision {
        run_id: "run-1".to_string(),
        attempt: 1,
        approval_id: "approval-1".to_string(),
        action_digest: digest,
        decision: ApprovalDecision::Approved,
        reason: None,
        decided_at_ms: 9_000,
    });
    batch(vec![envelope(4, decision.clone())])
        .validate()
        .unwrap();
    assert_eq!(
        HubNodeIngressRequest {
            transport: NodeTransport::LongPoll,
            envelope: envelope(4, decision),
        }
        .validate(),
        Err(HubTransportContractError::InvalidMessageDirection)
    );
}

#[test]
fn stream_and_close_requests_are_query_safe_and_bounded() {
    let stream = HubNodeStreamRequest {
        protocol_major: HUB_NODE_PROTOCOL_VERSION.major,
        protocol_minor: HUB_NODE_PROTOCOL_VERSION.minor,
        device_id: "node-1".to_string(),
        connection_id: "connection-1".to_string(),
    };
    stream.validate().unwrap();
    assert_eq!(
        stream.pull_request().protocol_version,
        HUB_NODE_PROTOCOL_VERSION
    );
    assert!(!serde_json::to_string(&stream).unwrap().contains("token"));

    let close = HubNodeCloseRequest {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-1".to_string(),
        connection_id: "connection-1".to_string(),
        error_code: Some("client_shutdown".to_string()),
    };
    close.validate().unwrap();
    let mut invalid = close;
    invalid.error_code = Some("secret\nvalue".to_string());
    assert!(matches!(
        invalid.validate(),
        Err(HubTransportContractError::Protocol(_))
    ));
}

#[test]
fn empty_delivery_is_valid_but_ack_size_wait_and_version_are_bounded() {
    batch(vec![]).validate().unwrap();

    let mut invalid_ack = batch(vec![]);
    invalid_ack.acknowledged_node_sequence = 0;
    assert_eq!(
        invalid_ack.validate(),
        Err(HubTransportContractError::InvalidNodeAcknowledgement)
    );

    let mut oversized = batch(
        (1..=(MAX_HUB_NODE_BATCH_MESSAGES as u64 + 1))
            .map(|sequence| envelope(sequence, welcome()))
            .collect(),
    );
    assert_eq!(
        oversized.validate(),
        Err(HubTransportContractError::InvalidBatchLimit)
    );
    oversized.messages.clear();
    oversized.retry_after_ms = Some(0);
    assert_eq!(
        oversized.validate(),
        Err(HubTransportContractError::InvalidWait)
    );

    let mut wrong_version = batch(vec![envelope(4, welcome())]);
    wrong_version.messages[0].protocol_version.minor = 1;
    assert_eq!(
        wrong_version.validate(),
        Err(HubTransportContractError::VersionMismatch)
    );
}

#[test]
fn delivery_batch_keeps_node_ack_outside_contiguous_hub_envelopes() {
    let delivery = batch(vec![
        envelope(4, welcome()),
        envelope(
            5,
            HubNodeMessage::CancelRun {
                run_id: "run-1".to_string(),
                attempt: 1,
                reason: "operator_request".to_string(),
            },
        ),
    ]);
    delivery.validate().unwrap();
    let json = serde_json::to_string(&delivery).unwrap();
    let decoded: HubNodeDeliveryBatch = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, delivery);
}

#[test]
fn delivery_batch_rejects_ack_loops_gaps_and_wrong_direction() {
    let mut with_ack = envelope(4, welcome());
    with_ack.ack_sequence = Some(1);
    assert_eq!(
        batch(vec![with_ack]).validate(),
        Err(HubTransportContractError::AcknowledgementLoop)
    );
    assert_eq!(
        batch(vec![envelope(4, welcome()), envelope(6, welcome())]).validate(),
        Err(HubTransportContractError::SequenceGap)
    );
    assert_eq!(
        batch(vec![envelope(4, hello(vec![NodeTransport::LongPoll]),)]).validate(),
        Err(HubTransportContractError::InvalidMessageDirection)
    );
}

#[test]
fn delivery_batch_rejects_cross_device_or_connection_data() {
    let mut wrong_device = envelope(4, welcome());
    wrong_device.device_id = "node-2".to_string();
    assert_eq!(
        batch(vec![wrong_device]).validate(),
        Err(HubTransportContractError::DeviceMismatch)
    );
    let mut wrong_connection = envelope(4, welcome());
    wrong_connection.connection_id = "connection-2".to_string();
    assert_eq!(
        batch(vec![wrong_connection]).validate(),
        Err(HubTransportContractError::ConnectionMismatch)
    );
}

#[test]
fn delivery_debug_never_renders_payloads() {
    let delivery = batch(vec![envelope(
        4,
        HubNodeMessage::ProtocolError {
            code: "local_failure".to_string(),
            message: "sensitive detail".to_string(),
            retryable: false,
            path_policy_applied: true,
        },
    )]);
    let debug = format!("{delivery:?}");
    assert!(!debug.contains("sensitive detail"));
    assert!(debug.contains("message_count"));
}

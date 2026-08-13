use super::*;
use captain_wire::{
    CapabilityDescriptor, DeviceRole, HubNodeMessage, LogicalWorkspace, ProtocolVersion,
    HUB_NODE_PROTOCOL_VERSION,
};

fn envelope(message: HubNodeMessage) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-ws".to_string(),
        connection_id: "connection-ws".to_string(),
        sequence: 1,
        ack_sequence: None,
        sent_at_ms: 100,
        message,
    }
}

fn node_frame(message: HubNodeMessage) -> String {
    serde_json::to_string(&HubNodeWebSocketFrame::NodeEnvelope {
        envelope: envelope(message),
    })
    .unwrap()
}

fn batch(acknowledged_node_sequence: u64, sequence: Option<u64>) -> HubNodeDeliveryBatch {
    HubNodeDeliveryBatch {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "node-ws".to_string(),
        connection_id: "connection-ws".to_string(),
        acknowledged_node_sequence,
        messages: sequence
            .map(|sequence| HubNodeEnvelope {
                protocol_version: HUB_NODE_PROTOCOL_VERSION,
                device_id: "node-ws".to_string(),
                connection_id: "connection-ws".to_string(),
                sequence,
                ack_sequence: None,
                sent_at_ms: 100 + sequence as i64,
                message: HubNodeMessage::Welcome {
                    negotiated_version: ProtocolVersion { major: 1, minor: 0 },
                    transport: NodeTransport::WebSocket,
                    heartbeat_interval_ms: 15_000,
                    lease_duration_ms: 60_000,
                },
            })
            .into_iter()
            .collect(),
        retry_after_ms: None,
    }
}

#[test]
fn websocket_route_is_exact_get_only() {
    assert!(is_hub_node_websocket_route(
        &Method::GET,
        HUB_NODE_WEBSOCKET_PATH
    ));
    assert!(!is_hub_node_websocket_route(
        &Method::POST,
        HUB_NODE_WEBSOCKET_PATH
    ));
    assert!(!is_hub_node_websocket_route(
        &Method::GET,
        "/api/hub/nodes/ws/extra"
    ));
}

#[test]
fn parser_accepts_node_frames_and_rejects_hub_binary_or_oversized_shapes() {
    let heartbeat = parse_node_frame(&node_frame(HubNodeMessage::Heartbeat {
        active_run_ids: Vec::new(),
    }))
    .unwrap();
    assert!(matches!(
        heartbeat.message,
        HubNodeMessage::Heartbeat { .. }
    ));

    let hub = serde_json::to_string(&HubNodeWebSocketFrame::HubDelivery {
        batch: batch(1, Some(1)),
    })
    .unwrap();
    assert_eq!(
        parse_node_frame(&hub),
        Err(WebSocketFailure::InvalidDirection)
    );
    assert_eq!(
        parse_node_frame("not-json"),
        Err(WebSocketFailure::InvalidFrame)
    );
    assert_eq!(
        parse_node_frame(&"x".repeat(MAX_HUB_NODE_FRAME_BYTES + 1)),
        Err(WebSocketFailure::FrameTooLarge)
    );
}

#[test]
fn websocket_hello_must_advertise_the_primary_transport() {
    let hello = HubNodeMessage::Hello {
        role: DeviceRole::Node,
        capabilities: CapabilityDescriptor {
            captain_version: "0.1.0-alpha.14".to_string(),
            platform: "linux-x86_64".to_string(),
            transports: vec![NodeTransport::LongPoll],
            tool_families: vec!["file".to_string()],
            workspaces: vec![LogicalWorkspace {
                workspace_id: "workspace-main".to_string(),
                label: "Main".to_string(),
                read_only: true,
            }],
            supports_streaming_output: true,
        },
        resume_after_sequence: 0,
        active_run_ids: Vec::new(),
    };
    let envelope = parse_node_frame(&node_frame(hello)).unwrap();
    assert!(HubNodeConnectRequest {
        transport: NodeTransport::WebSocket,
        hello: envelope,
    }
    .validate()
    .is_err());
}

#[test]
fn delivery_cursor_emits_new_ack_or_hub_sequence_only_once() {
    let mut cursor = DeliveryCursor::default();
    let welcome = batch(1, Some(1));
    assert!(cursor.needs_delivery(&welcome));
    cursor.observe(&welcome);
    assert!(!cursor.needs_delivery(&welcome));

    let acknowledged = batch(2, None);
    assert!(cursor.needs_delivery(&acknowledged));
    cursor.observe(&acknowledged);
    assert!(!cursor.needs_delivery(&acknowledged));

    let next = batch(2, Some(2));
    assert!(cursor.needs_delivery(&next));
}

#[test]
fn ip_slots_are_bounded_and_release_with_the_guard() {
    let ip = IpAddr::from([203, 0, 113, 77]);
    let guards = (0..MAX_HUB_NODE_WS_PER_IP)
        .map(|_| try_acquire_ip_slot(ip).unwrap())
        .collect::<Vec<_>>();
    assert!(try_acquire_ip_slot(ip).is_none());
    drop(guards);
    assert!(!websocket_ip_counts().contains_key(&ip));
    let guard = try_acquire_ip_slot(ip).unwrap();
    drop(guard);
    assert!(!websocket_ip_counts().contains_key(&ip));
}

#[test]
fn ingress_rate_window_bounds_bursts_and_resets_after_one_minute() {
    let start = Instant::now();
    let mut window = IngressRateWindow::new(start);
    for _ in 0..MAX_HUB_NODE_WS_MESSAGES_PER_MINUTE {
        assert!(window.allow(start));
    }
    assert!(!window.allow(start));
    assert!(window.allow(start + Duration::from_secs(60)));
}

#[test]
fn close_feedback_is_categorical_and_bounded() {
    for failure in [
        WebSocketFailure::HandshakeTimeout,
        WebSocketFailure::FrameTooLarge,
        WebSocketFailure::InvalidFrame,
        WebSocketFailure::ExpectedHello,
        WebSocketFailure::InvalidDirection,
        WebSocketFailure::UnsupportedFrame,
        WebSocketFailure::HeartbeatTimeout,
        WebSocketFailure::MessageRateExceeded,
    ] {
        assert!(failure.reason().len() < 32);
        assert!((1002..=1009).contains(&failure.code()));
    }
}

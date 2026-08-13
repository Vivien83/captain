use crate::hub_node_rail::{
    HubNodeConnectionStatus, HubNodeRailError, HubNodeRailStore, HubNodeRunStatus, NewHubNodeRun,
};
use crate::MemorySubstrate;
use captain_wire::hub_protocol::{
    CapabilityDescriptor, DeviceGrant, DeviceRole, HubNodeEnvelope, HubNodeMessage,
    LogicalWorkspace, NodeTransport, RunEffect, HUB_NODE_PROTOCOL_VERSION,
};
use serde_json::json;

fn memory_with_nodes() -> MemorySubstrate {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    insert_node(&memory, "node-1", 'a', DeviceGrant::default());
    memory
}

#[test]
fn transport_loss_reconciles_unexpired_runs_without_replaying_side_effects() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    let hello = hello(
        "node-1",
        "connection-loss",
        1,
        0,
        capabilities(vec![NodeTransport::WebSocket]),
    );
    open(store, &hello, NodeTransport::WebSocket, 10);

    for (run_id, idempotency_key, effect) in [
        ("read-run", "read-idem", RunEffect::ReadOnly),
        ("write-run", "write-idem", RunEffect::LocalMutation),
    ] {
        store
            .enqueue_run(&NewHubNodeRun {
                run_id: run_id.to_string(),
                device_id: "node-1".to_string(),
                idempotency_key: idempotency_key.to_string(),
                workspace_id: "workspace-main".to_string(),
                tool_name: "shell_exec".to_string(),
                input: json!({"command": "true"}),
                effect,
                created_at_ms: 11,
            })
            .unwrap();
        store
            .lease_run("node-1", run_id, "connection-loss", 12, 60_000)
            .unwrap();
        store
            .mark_accepted("node-1", run_id, 1, "connection-loss", 13)
            .unwrap();
    }

    store
        .close_connection("node-1", "connection-loss", Some("network_lost"), 14)
        .unwrap();
    let summary = store.reconcile_after_disconnect("node-1", 14).unwrap();

    assert_eq!(summary.requeued_read_only, 1);
    assert_eq!(summary.uncertain_side_effects, 1);
    assert_eq!(
        store.get_run("read-run").unwrap().unwrap().status,
        HubNodeRunStatus::Queued
    );
    assert_eq!(
        store.get_run("write-run").unwrap().unwrap().status,
        HubNodeRunStatus::Uncertain
    );
}

fn insert_node(
    memory: &MemorySubstrate,
    device_id: &str,
    digest_character: char,
    grants: DeviceGrant,
) {
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
                       1, 0, ?2, '{}', ?3, 'active', 1, 1, 1)",
            rusqlite::params![
                device_id,
                digest_character.to_string().repeat(64),
                serde_json::to_string(&grants).unwrap(),
            ],
        )
        .unwrap();
}

fn capabilities(transports: Vec<NodeTransport>) -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "macos".to_string(),
        transports,
        tool_families: vec!["shell".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "workspace-main".to_string(),
            label: "Main workspace".to_string(),
            read_only: false,
        }],
        supports_streaming_output: true,
    }
}

fn hello(
    device_id: &str,
    connection_id: &str,
    sequence: u64,
    acknowledged: u64,
    capabilities: CapabilityDescriptor,
) -> HubNodeEnvelope {
    hello_with_active(
        device_id,
        connection_id,
        sequence,
        acknowledged,
        capabilities,
        Vec::new(),
    )
}

fn hello_with_active(
    device_id: &str,
    connection_id: &str,
    sequence: u64,
    acknowledged: u64,
    capabilities: CapabilityDescriptor,
    active_run_ids: Vec<String>,
) -> HubNodeEnvelope {
    HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: device_id.to_string(),
        connection_id: connection_id.to_string(),
        sequence,
        ack_sequence: (acknowledged > 0).then_some(acknowledged),
        sent_at_ms: 10 + sequence as i64,
        message: HubNodeMessage::Hello {
            role: DeviceRole::Node,
            capabilities,
            resume_after_sequence: acknowledged,
            active_run_ids,
        },
    }
}

fn open(
    store: &HubNodeRailStore,
    hello: &HubNodeEnvelope,
    transport: NodeTransport,
    now_ms: i64,
) -> crate::hub_node_rail::OpenHubNodeConnection {
    store
        .open_connection(hello, transport, 15_000, 60_000, now_ms)
        .unwrap()
}

#[test]
fn hello_is_atomic_idempotent_and_reconnect_advances_cursors() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    let advertised = capabilities(vec![NodeTransport::WebSocket, NodeTransport::LongPoll]);
    let first_hello = hello("node-1", "connection-1", 1, 0, advertised.clone());

    let first = open(store, &first_hello, NodeTransport::WebSocket, 20);
    assert!(!first.replayed);
    assert_eq!(first.last_node_sequence, 1);
    assert_eq!(first.welcome.sequence, 1);
    assert_eq!(first.connection.status, HubNodeConnectionStatus::Active);
    assert_eq!(first.connection.transport, NodeTransport::WebSocket);
    assert!(matches!(
        serde_json::from_str::<HubNodeMessage>(&first.welcome.message_json).unwrap(),
        HubNodeMessage::Welcome {
            transport: NodeTransport::WebSocket,
            ..
        }
    ));

    let replay = open(store, &first_hello, NodeTransport::WebSocket, 21);
    assert!(replay.replayed);
    assert_eq!(replay.welcome, first.welcome);
    assert_eq!(replay.connection.connected_at_ms, 20);

    let second_hello = hello("node-1", "connection-2", 2, 1, advertised);
    let second = open(store, &second_hello, NodeTransport::LongPoll, 30);
    assert_eq!(second.last_node_sequence, 2);
    assert_eq!(second.welcome.sequence, 2);
    assert_eq!(second.connection.connection_id, "connection-2");
    assert_eq!(second.connection.transport, NodeTransport::LongPoll);
    assert_eq!(
        store.pending_outbox("node-1", 1, 10).unwrap(),
        vec![second.welcome]
    );
}

#[test]
fn exact_bootstrap_hello_reactivates_one_connection_across_transport_fallbacks() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    let bootstrap = hello(
        "node-1",
        "connection-stable",
        1,
        0,
        capabilities(vec![
            NodeTransport::WebSocket,
            NodeTransport::HttpStream,
            NodeTransport::LongPoll,
        ]),
    );

    let first = open(store, &bootstrap, NodeTransport::WebSocket, 20);
    assert_eq!(first.last_node_sequence, 1);
    store
        .close_connection("node-1", "connection-stable", Some("transport_closed"), 30)
        .unwrap();

    let fallback = open(store, &bootstrap, NodeTransport::LongPoll, 40);
    assert!(fallback.replayed);
    assert_eq!(fallback.last_node_sequence, 1);
    assert_eq!(fallback.welcome.sequence, 2);
    assert_eq!(fallback.connection.connected_at_ms, 40);
    assert_eq!(fallback.connection.transport, NodeTransport::LongPoll);
    assert!(matches!(
        serde_json::from_str::<HubNodeMessage>(&fallback.welcome.message_json).unwrap(),
        HubNodeMessage::Welcome {
            transport: NodeTransport::LongPoll,
            ..
        }
    ));
    assert_eq!(
        store.pending_outbox("node-1", 0, 10).unwrap(),
        vec![fallback.welcome.clone()]
    );

    let ambiguous_retry = open(store, &bootstrap, NodeTransport::LongPoll, 41);
    assert!(ambiguous_retry.replayed);
    assert_eq!(ambiguous_retry.welcome, fallback.welcome);
    assert_eq!(ambiguous_retry.connection.connected_at_ms, 40);

    let stream_fallback = open(store, &bootstrap, NodeTransport::HttpStream, 50);
    assert!(stream_fallback.replayed);
    assert_eq!(stream_fallback.last_node_sequence, 1);
    assert_eq!(stream_fallback.welcome.sequence, 3);
    assert_eq!(
        stream_fallback.connection.connection_id,
        "connection-stable"
    );
    assert_eq!(
        stream_fallback.connection.transport,
        NodeTransport::HttpStream
    );
    assert_eq!(
        store.pending_outbox("node-1", 0, 10).unwrap(),
        vec![stream_fallback.welcome]
    );
}

#[test]
fn reconnect_resequences_unacked_work_after_the_new_welcome() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    let advertised = capabilities(vec![NodeTransport::LongPoll]);
    open(
        store,
        &hello("node-1", "connection-1", 1, 0, advertised.clone()),
        NodeTransport::LongPoll,
        20,
    );
    store
        .enqueue_run(&NewHubNodeRun {
            run_id: "run-1".to_string(),
            device_id: "node-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            workspace_id: "workspace-main".to_string(),
            tool_name: "shell_exec".to_string(),
            input: json!({"command": "true"}),
            effect: RunEffect::ReadOnly,
            created_at_ms: 21,
        })
        .unwrap();
    assert_eq!(
        store
            .lease_next("node-1", "connection-1", 22, 60_000)
            .unwrap()
            .unwrap()
            .outbox
            .sequence,
        2
    );

    let reconnect = hello("node-1", "connection-2", 2, 1, advertised);
    let opened = open(store, &reconnect, NodeTransport::LongPoll, 30);
    assert_eq!(opened.welcome.sequence, 3);
    let snapshot = store
        .delivery_snapshot("node-1", "connection-2", 10)
        .unwrap();
    assert_eq!(snapshot.acknowledged_node_sequence, 2);
    assert_eq!(
        snapshot
            .messages
            .iter()
            .map(|message| (message.sequence, message.message_kind.as_str()))
            .collect::<Vec<_>>(),
        vec![(2, "run_offer"), (3, "welcome"), (4, "run_offer")]
    );
    assert!(snapshot.messages[0].superseded_at_ms.is_some());
    assert!(snapshot.messages[1].superseded_at_ms.is_none());
    assert!(snapshot.messages[2].superseded_at_ms.is_none());
    let run = store.get_run("run-1").unwrap().unwrap();
    assert_eq!(run.lease_owner.as_deref(), Some("connection-2"));
    assert_eq!(run.lease_expires_at_ms, Some(60_030));
    let requeued_offer: HubNodeMessage =
        serde_json::from_str(&snapshot.messages[2].message_json).unwrap();
    assert!(matches!(
        requeued_offer,
        HubNodeMessage::RunOffer(ref lease) if lease.lease_expires_at_ms == 60_030
    ));
    assert!(matches!(
        store.delivery_snapshot("node-1", "connection-1", 10),
        Err(HubNodeRailError::ConnectionConflict)
    ));

    let old_offer: (Option<i64>, Option<i64>) = memory
        .usage_conn()
        .lock()
        .unwrap()
        .query_row(
            "SELECT acked_at_ms, superseded_at_ms
             FROM hub_node_outbox WHERE device_id = 'node-1' AND sequence = 2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(old_offer.0, None);
    assert_eq!(old_offer.1, Some(30));

    let replay = open(store, &reconnect, NodeTransport::LongPoll, 31);
    assert!(replay.replayed);
    assert_eq!(replay.welcome, opened.welcome);
    assert_eq!(
        store
            .delivery_snapshot("node-1", "connection-2", 10)
            .unwrap()
            .messages,
        snapshot.messages
    );
}

#[test]
fn reconnect_adopts_only_explicitly_reported_active_runs() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    let advertised = capabilities(vec![NodeTransport::LongPoll]);
    open(
        store,
        &hello("node-1", "connection-1", 1, 0, advertised.clone()),
        NodeTransport::LongPoll,
        20,
    );
    store
        .enqueue_run(&NewHubNodeRun {
            run_id: "run-active".to_string(),
            device_id: "node-1".to_string(),
            idempotency_key: "idem-active".to_string(),
            workspace_id: "workspace-main".to_string(),
            tool_name: "shell_exec".to_string(),
            input: json!({"command": "true"}),
            effect: RunEffect::ReadOnly,
            created_at_ms: 21,
        })
        .unwrap();
    store
        .lease_next("node-1", "connection-1", 22, 60_000)
        .unwrap();
    store
        .mark_accepted("node-1", "run-active", 1, "connection-1", 23)
        .unwrap();

    let reconnect = hello_with_active(
        "node-1",
        "connection-2",
        2,
        2,
        advertised,
        vec!["run-active".to_string()],
    );
    open(store, &reconnect, NodeTransport::LongPoll, 30);
    let adopted = store.get_run("run-active").unwrap().unwrap();
    assert_eq!(adopted.lease_owner.as_deref(), Some("connection-2"));
    assert_eq!(adopted.lease_expires_at_ms, Some(60_030));
    store
        .record_progress("node-1", "run-active", 1, "connection-2", 1, "resumed", 31)
        .unwrap();
}

#[test]
fn invalid_active_run_claim_rolls_back_ack_resequence_and_lease_adoption() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    let advertised = capabilities(vec![NodeTransport::LongPoll]);
    open(
        store,
        &hello("node-1", "connection-1", 1, 0, advertised.clone()),
        NodeTransport::LongPoll,
        20,
    );
    store
        .enqueue_run(&NewHubNodeRun {
            run_id: "run-1".to_string(),
            device_id: "node-1".to_string(),
            idempotency_key: "idem-1".to_string(),
            workspace_id: "workspace-main".to_string(),
            tool_name: "shell_exec".to_string(),
            input: json!({"command": "true"}),
            effect: RunEffect::ReadOnly,
            created_at_ms: 21,
        })
        .unwrap();
    store
        .lease_next("node-1", "connection-1", 22, 60_000)
        .unwrap();

    let invalid = hello_with_active(
        "node-1",
        "connection-2",
        2,
        1,
        advertised,
        vec!["unknown-run".to_string()],
    );
    assert!(matches!(
        store.open_connection(&invalid, NodeTransport::LongPoll, 15_000, 60_000, 30),
        Err(HubNodeRailError::LeaseConflict)
    ));
    assert_eq!(
        store.connection("node-1").unwrap().unwrap().connection_id,
        "connection-1"
    );
    assert_eq!(
        store
            .get_run("run-1")
            .unwrap()
            .unwrap()
            .lease_owner
            .as_deref(),
        Some("connection-1")
    );
    assert_eq!(
        store
            .delivery_snapshot("node-1", "connection-1", 10)
            .unwrap()
            .messages
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn revocation_atomically_stops_presence_delivery_and_device_work() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    open(
        store,
        &hello(
            "node-1",
            "connection-1",
            1,
            0,
            capabilities(vec![NodeTransport::LongPoll]),
        ),
        NodeTransport::LongPoll,
        20,
    );

    for (run_id, idempotency_key, effect, created_at_ms) in [
        ("leased-run", "idem-leased", RunEffect::LocalMutation, 21),
        (
            "accepted-run",
            "idem-accepted",
            RunEffect::ExternalEffect,
            23,
        ),
        ("queued-run", "idem-queued", RunEffect::ReadOnly, 26),
    ] {
        store
            .enqueue_run(&NewHubNodeRun {
                run_id: run_id.to_string(),
                device_id: "node-1".to_string(),
                idempotency_key: idempotency_key.to_string(),
                workspace_id: "workspace-main".to_string(),
                tool_name: "shell_exec".to_string(),
                input: json!({"command": "true"}),
                effect,
                created_at_ms,
            })
            .unwrap();
        if run_id == "leased-run" {
            store
                .lease_next("node-1", "connection-1", 22, 60_000)
                .unwrap();
        } else if run_id == "accepted-run" {
            store
                .lease_next("node-1", "connection-1", 24, 60_000)
                .unwrap();
            store
                .mark_accepted("node-1", "accepted-run", 1, "connection-1", 25)
                .unwrap();
        }
    }

    memory.devices().revoke_device("node-1", 30).unwrap();
    memory.devices().revoke_device("node-1", 40).unwrap();
    let device = memory.devices().get_device("node-1").unwrap().unwrap();
    assert_eq!(device.status, "revoked");
    assert_eq!(device.revoked_at_ms, Some(30));
    assert_eq!(device.last_error_code.as_deref(), Some("device_revoked"));
    let connection = store.connection("node-1").unwrap().unwrap();
    assert_eq!(connection.status, HubNodeConnectionStatus::Offline);
    assert_eq!(
        connection.last_error_code.as_deref(),
        Some("device_revoked")
    );
    assert_eq!(
        store.get_run("leased-run").unwrap().unwrap().status,
        HubNodeRunStatus::Cancelled
    );
    assert_eq!(
        store.get_run("queued-run").unwrap().unwrap().status,
        HubNodeRunStatus::Cancelled
    );
    assert_eq!(
        store.get_run("accepted-run").unwrap().unwrap().status,
        HubNodeRunStatus::Uncertain
    );
    assert!(store.pending_outbox("node-1", 0, 10).unwrap().is_empty());
    assert!(matches!(
        store.delivery_snapshot("node-1", "connection-1", 10),
        Err(HubNodeRailError::NodeUnavailable)
    ));
}

#[test]
fn rejected_hello_rolls_back_its_sequence_and_allows_a_correct_retry() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    let envelope = hello(
        "node-1",
        "connection-1",
        1,
        0,
        capabilities(vec![NodeTransport::WebSocket]),
    );

    assert!(matches!(
        store.open_connection(&envelope, NodeTransport::HttpStream, 15_000, 60_000, 20),
        Err(HubNodeRailError::InvalidInput(_))
    ));
    assert!(store.connection("node-1").unwrap().is_none());
    let accepted = open(store, &envelope, NodeTransport::WebSocket, 21);
    assert_eq!(accepted.last_node_sequence, 1);
}

#[test]
fn current_grants_must_remain_a_subset_of_announced_capabilities() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    insert_node(
        &memory,
        "node-1",
        'a',
        DeviceGrant {
            workspace_ids: vec!["workspace-main".to_string()],
            tool_families: vec!["shell".to_string()],
            allow_mutation: true,
        },
    );
    let mut reduced = capabilities(vec![NodeTransport::LongPoll]);
    reduced.workspaces.clear();
    let envelope = hello("node-1", "connection-1", 1, 0, reduced);

    assert!(matches!(
        memory.hub_node_rail().open_connection(
            &envelope,
            NodeTransport::LongPoll,
            15_000,
            60_000,
            20,
        ),
        Err(HubNodeRailError::InvalidInput(_))
    ));
    assert!(memory
        .hub_node_rail()
        .connection("node-1")
        .unwrap()
        .is_none());
}

#[test]
fn a_connection_id_cannot_be_shared_between_devices() {
    let memory = memory_with_nodes();
    insert_node(&memory, "node-2", 'b', DeviceGrant::default());
    let store = memory.hub_node_rail();
    let caps = capabilities(vec![NodeTransport::LongPoll]);
    open(
        store,
        &hello("node-1", "shared-connection", 1, 0, caps.clone()),
        NodeTransport::LongPoll,
        20,
    );

    let conflicting = hello("node-2", "shared-connection", 1, 0, caps.clone());
    assert!(matches!(
        store.open_connection(&conflicting, NodeTransport::LongPoll, 15_000, 60_000, 21),
        Err(HubNodeRailError::ConnectionConflict)
    ));
    assert!(store.connection("node-2").unwrap().is_none());
    open(
        store,
        &hello("node-2", "connection-2", 1, 0, caps),
        NodeTransport::LongPoll,
        22,
    );
}

#[test]
fn close_and_restart_presence_are_fail_closed_and_idempotent() {
    let memory = memory_with_nodes();
    let store = memory.hub_node_rail();
    let envelope = hello(
        "node-1",
        "connection-1",
        1,
        0,
        capabilities(vec![NodeTransport::WebSocket]),
    );
    open(store, &envelope, NodeTransport::WebSocket, 20);

    let closed = store
        .close_connection("node-1", "connection-1", Some("transport_closed"), 30)
        .unwrap();
    assert_eq!(closed.status, HubNodeConnectionStatus::Offline);
    assert_eq!(
        store
            .close_connection("node-1", "connection-1", Some("different"), 31)
            .unwrap(),
        closed
    );

    let reconnect = hello(
        "node-1",
        "connection-2",
        2,
        1,
        capabilities(vec![NodeTransport::LongPoll]),
    );
    open(store, &reconnect, NodeTransport::LongPoll, 40);
    assert!(matches!(
        store.close_connection("node-1", "connection-1", None, 41),
        Err(HubNodeRailError::ConnectionConflict)
    ));
    assert_eq!(store.reconcile_connections_after_restart(42).unwrap(), 1);
    assert_eq!(store.reconcile_connections_after_restart(43).unwrap(), 0);
    let offline = store.connection("node-1").unwrap().unwrap();
    assert_eq!(offline.status, HubNodeConnectionStatus::Offline);
    assert_eq!(
        offline.last_error_code.as_deref(),
        Some("runtime_restarted")
    );
}

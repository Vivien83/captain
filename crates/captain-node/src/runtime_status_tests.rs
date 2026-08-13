use super::*;

fn snapshot() -> NodeRailSnapshot {
    NodeRailSnapshot {
        device_id: "device-1".to_string(),
        connection_id: "connection-1".to_string(),
        last_node_sequence: 4,
        acknowledged_node_sequence: 3,
        last_hub_sequence: 6,
        confirmed_hub_ack_sequence: 5,
        pending_outbound: 1,
        pending_inbound: 2,
    }
}

#[test]
fn runtime_status_round_trips_privately_without_raw_paths() {
    let temp = tempfile::tempdir().unwrap();
    let store = NodeRuntimeStatusStore::open(temp.path()).unwrap();
    let status = NodeRuntimeStatus::connected(
        123,
        NodeTransport::WebSocket,
        NodeBootstrapCapabilityState::Current,
        true,
        snapshot(),
        0,
        None,
    )
    .unwrap();
    store.save(&status).unwrap();
    assert_eq!(store.load().unwrap(), Some(status));
    assert!(!format!("{store:?}").contains(temp.path().to_string_lossy().as_ref()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(temp.path().join(RUNTIME_STATUS_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn runtime_status_rejects_malformed_state_and_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let store = NodeRuntimeStatusStore::open(temp.path()).unwrap();
    fs::write(temp.path().join(RUNTIME_STATUS_FILE), b"not-json").unwrap();
    assert_eq!(store.load(), Err(NodeRuntimeStatusError::StateCorrupt));

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let other = temp.path().join("other.json");
        fs::write(&other, b"{}").unwrap();
        fs::remove_file(temp.path().join(RUNTIME_STATUS_FILE)).unwrap();
        symlink(&other, temp.path().join(RUNTIME_STATUS_FILE)).unwrap();
        assert_eq!(store.load(), Err(NodeRuntimeStatusError::UnsafePath));
    }
}

#[test]
fn runtime_status_rejects_impossible_rail_counters() {
    let mut rail = snapshot();
    rail.acknowledged_node_sequence = rail.last_node_sequence + 1;
    assert_eq!(
        NodeRuntimeStatus::connected(
            123,
            NodeTransport::LongPoll,
            NodeBootstrapCapabilityState::Current,
            false,
            rail,
            0,
            None,
        ),
        Err(NodeRuntimeStatusError::StateCorrupt)
    );
}

#[test]
fn stopped_status_contains_no_previous_connection_identity() {
    let status = NodeRuntimeStatus::stopped(456);
    assert_eq!(status.state(), "stopped");
    assert_eq!(status.device_id(), None);
    assert_eq!(status.rail_snapshot(), None);
    assert_eq!(status.allow_mutation(), None);
    assert_eq!(status.transport(), None);
    assert_eq!(status.fallback_count(), 0);
}

#[test]
fn connected_status_exposes_only_operator_safe_runtime_facts() {
    let status = NodeRuntimeStatus::connected(
        789,
        NodeTransport::HttpStream,
        NodeBootstrapCapabilityState::RotationDeferred,
        false,
        snapshot(),
        2,
        Some("transport_retry"),
    )
    .unwrap();
    assert_eq!(status.state(), "degraded");
    assert_eq!(status.updated_at_ms(), 789);
    assert_eq!(status.transport(), Some(NodeTransport::HttpStream));
    assert_eq!(
        status.capability_state(),
        Some(NodeBootstrapCapabilityState::RotationDeferred)
    );
    assert_eq!(status.allow_mutation(), Some(false));
    assert_eq!(status.fallback_count(), 2);
    assert_eq!(status.last_error_code(), Some("transport_retry"));
}

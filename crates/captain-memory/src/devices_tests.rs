use crate::devices::{DeviceStoreError, NewPairingRequest, PairingPollStatus};
use crate::MemorySubstrate;

fn digest(character: char) -> String {
    std::iter::repeat(character).take(64).collect()
}

fn pairing_request(request_id: &str, credential: char, created_at_ms: i64) -> NewPairingRequest {
    NewPairingRequest {
        request_id: request_id.to_string(),
        display_code_sha256: digest(if credential == 'a' { 'b' } else { 'a' }),
        polling_secret_sha256: digest(if credential == 'c' { 'd' } else { 'c' }),
        credential_sha256: digest(credential),
        display_name: "Office Mac".to_string(),
        role: "node".to_string(),
        platform: "macos-arm64".to_string(),
        captain_version: "0.1.0-alpha.14".to_string(),
        protocol_major: 1,
        protocol_minor: 0,
        capabilities_json: r#"{"tool_families":["file","shell"]}"#.to_string(),
        requested_grants_json: r#"{"workspaces":["project-main"]}"#.to_string(),
        created_at_ms,
        expires_at_ms: created_at_ms + 100,
    }
}

#[test]
fn pairing_survives_reopen_and_approval_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("captain.db");
    let request = pairing_request("00000000-0000-4000-8000-000000000001", 'e', 100);

    {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        memory.devices().create_pairing_request(&request).unwrap();
        assert_eq!(memory.devices().pending_pairings(150).unwrap().len(), 1);
    }

    {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        let device = memory
            .devices()
            .approve_pairing(
                &request.request_id,
                "node-office-mac",
                r#"{"workspaces":["project-main"],"tool_families":["file"]}"#,
                160,
            )
            .unwrap();
        assert_eq!(device.device_id, "node-office-mac");
        assert_eq!(device.status, "active");

        let replay = memory
            .devices()
            .approve_pairing(
                &request.request_id,
                "must-not-be-created",
                r#"{"workspaces":[]}"#,
                170,
            )
            .unwrap();
        assert_eq!(replay.device_id, "node-office-mac");
        assert_eq!(memory.devices().list_devices().unwrap().len(), 1);
    }

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let poll = memory
        .devices()
        .poll_pairing(&request.request_id, &request.polling_secret_sha256, 180)
        .unwrap();
    assert_eq!(poll.status, PairingPollStatus::Approved);
    assert_eq!(poll.device_id.as_deref(), Some("node-office-mac"));
    memory
        .devices()
        .verify_device_credential_digest("node-office-mac", &request.credential_sha256)
        .unwrap();
    assert!(matches!(
        memory
            .devices()
            .verify_device_credential_digest("node-office-mac", &digest('f')),
        Err(DeviceStoreError::InvalidDeviceCredential)
    ));

    let serialized = serde_json::to_string(
        &memory
            .devices()
            .get_device("node-office-mac")
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(!serialized.contains(&request.credential_sha256));
    assert!(!serialized.contains("credential"));
}

#[test]
fn pairing_expiry_denial_and_credentials_fail_closed() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let expired = pairing_request("00000000-0000-4000-8000-000000000002", '1', 100);
    memory.devices().create_pairing_request(&expired).unwrap();

    assert!(matches!(
        memory.devices().approve_pairing(
            &expired.request_id,
            "expired-node",
            r#"{"workspaces":[]}"#,
            expired.expires_at_ms,
        ),
        Err(DeviceStoreError::PairingExpired)
    ));
    let poll = memory
        .devices()
        .poll_pairing(
            &expired.request_id,
            &expired.polling_secret_sha256,
            expired.expires_at_ms,
        )
        .unwrap();
    assert_eq!(poll.status, PairingPollStatus::Expired);

    let mut denied = pairing_request("00000000-0000-4000-8000-000000000003", '2', 300);
    denied.display_code_sha256 = digest('3');
    denied.polling_secret_sha256 = digest('4');
    memory.devices().create_pairing_request(&denied).unwrap();
    assert!(matches!(
        memory
            .devices()
            .poll_pairing(&denied.request_id, &digest('5'), 350),
        Err(DeviceStoreError::InvalidPollingCredential)
    ));
    memory
        .devices()
        .deny_pairing(&denied.request_id, 350)
        .unwrap();
    let poll = memory
        .devices()
        .poll_pairing(&denied.request_id, &denied.polling_secret_sha256, 360)
        .unwrap();
    assert_eq!(poll.status, PairingPollStatus::Denied);

    let mut duplicate = pairing_request("00000000-0000-4000-8000-000000000004", '2', 500);
    duplicate.display_code_sha256 = digest('6');
    duplicate.polling_secret_sha256 = digest('7');
    assert!(matches!(
        memory.devices().create_pairing_request(&duplicate),
        Err(DeviceStoreError::DuplicateCredential)
    ));

    let mut malformed = pairing_request("00000000-0000-4000-8000-000000000006", '9', 700);
    malformed.display_code_sha256 = digest('b');
    malformed.polling_secret_sha256 = digest('d');
    malformed.credential_sha256 = digest('A');
    assert!(matches!(
        memory.devices().create_pairing_request(&malformed),
        Err(DeviceStoreError::Database(_))
    ));

    let debug = format!("{denied:?}");
    assert!(!debug.contains(&denied.display_code_sha256));
    assert!(!debug.contains(&denied.polling_secret_sha256));
    assert!(!debug.contains(&denied.credential_sha256));
}

#[test]
fn pending_pairing_challenge_rotation_invalidates_only_the_old_challenge() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let request = pairing_request("00000000-0000-4000-8000-000000000007", 'e', 100);
    memory.devices().create_pairing_request(&request).unwrap();

    let rotated_display = digest('6');
    let rotated_polling = digest('7');
    memory
        .devices()
        .rotate_pending_pairing_challenge(
            &request.request_id,
            &rotated_display,
            &rotated_polling,
            150,
        )
        .unwrap();

    assert!(matches!(
        memory
            .devices()
            .poll_pairing(&request.request_id, &request.polling_secret_sha256, 160),
        Err(DeviceStoreError::InvalidPollingCredential)
    ));
    let poll = memory
        .devices()
        .poll_pairing(&request.request_id, &rotated_polling, 160)
        .unwrap();
    assert_eq!(poll.status, PairingPollStatus::Pending);
    let recovered = memory
        .devices()
        .pairing_by_credential_digest(&request.credential_sha256, 160)
        .unwrap()
        .unwrap();
    assert_eq!(recovered.request_id, request.request_id);
    assert_eq!(recovered.expires_at_ms, request.expires_at_ms);
}

#[test]
fn client_presence_is_role_scoped_monotonic_and_write_throttled() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut client = pairing_request("00000000-0000-4000-8000-000000000008", 'a', 100);
    client.role = "client".to_string();
    client.requested_grants_json =
        r#"{"workspace_ids":[],"tool_families":[],"allow_mutation":false}"#.to_string();
    memory.devices().create_pairing_request(&client).unwrap();
    memory
        .devices()
        .approve_pairing(
            &client.request_id,
            "client-office",
            &client.requested_grants_json,
            150,
        )
        .unwrap();

    assert!(!memory
        .devices()
        .touch_active_client_presence("client-office", 160, 15)
        .unwrap());
    assert_eq!(
        memory
            .devices()
            .get_device("client-office")
            .unwrap()
            .unwrap()
            .last_seen_ms,
        150
    );
    assert!(memory
        .devices()
        .touch_active_client_presence("client-office", 165, 15)
        .unwrap());
    assert_eq!(
        memory
            .devices()
            .get_device("client-office")
            .unwrap()
            .unwrap()
            .last_seen_ms,
        165
    );

    let node = pairing_request("00000000-0000-4000-8000-000000000009", 'c', 200);
    memory.devices().create_pairing_request(&node).unwrap();
    memory
        .devices()
        .approve_pairing(&node.request_id, "node-office", "{}", 250)
        .unwrap();
    assert!(matches!(
        memory
            .devices()
            .touch_active_client_presence("node-office", 300, 0),
        Err(DeviceStoreError::InvalidDeviceCredential)
    ));

    memory
        .devices()
        .revoke_device("client-office", 300)
        .unwrap();
    assert!(matches!(
        memory
            .devices()
            .touch_active_client_presence("client-office", 315, 15),
        Err(DeviceStoreError::InvalidDeviceCredential)
    ));
}

#[test]
fn revocation_is_immediate_durable_and_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("captain.db");
    let request = pairing_request("00000000-0000-4000-8000-000000000005", '8', 100);

    {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        memory.devices().create_pairing_request(&request).unwrap();
        memory
            .devices()
            .approve_pairing(
                &request.request_id,
                "node-to-revoke",
                r#"{"workspaces":["project-main"]}"#,
                150,
            )
            .unwrap();
        memory
            .devices()
            .touch_device(
                "node-to-revoke",
                "0.1.0-alpha.14",
                1,
                0,
                r#"{"tool_families":["file"]}"#,
                "web_socket",
                None,
                160,
            )
            .unwrap();
        memory
            .devices()
            .revoke_device("node-to-revoke", 170)
            .unwrap();
        memory
            .devices()
            .revoke_device("node-to-revoke", 180)
            .unwrap();
    }

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let device = memory
        .devices()
        .get_device("node-to-revoke")
        .unwrap()
        .unwrap();
    assert_eq!(device.status, "revoked");
    assert_eq!(device.revoked_at_ms, Some(170));
    assert!(matches!(
        memory
            .devices()
            .verify_device_credential_digest("node-to-revoke", &request.credential_sha256),
        Err(DeviceStoreError::InvalidDeviceCredential)
    ));
    assert!(matches!(
        memory
            .devices()
            .set_device_grants("node-to-revoke", r#"{"workspaces":[]}"#, 190),
        Err(DeviceStoreError::DeviceNotActive(status)) if status == "revoked"
    ));
}

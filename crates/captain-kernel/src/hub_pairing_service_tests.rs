use super::*;
use captain_memory::MemorySubstrate;
use captain_wire::{
    CapabilityDescriptor, DeviceGrant, DevicePairingClaim, LogicalWorkspace, NodeTransport,
    PairingPollRequest, PairingState,
};
use std::sync::{Arc, Barrier};

fn enabled_config() -> PairingConfig {
    PairingConfig {
        hub_enabled: true,
        ..PairingConfig::default()
    }
}

fn raw_credential(character: char) -> String {
    std::iter::repeat(character).take(64).collect()
}

fn claim(raw_credential: &str) -> DevicePairingClaim {
    let capabilities = CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "macos-arm64".to_string(),
        transports: vec![NodeTransport::WebSocket, NodeTransport::LongPoll],
        tool_families: vec!["file".to_string(), "shell-process".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "project-main".to_string(),
            label: "Main Project".to_string(),
            read_only: false,
        }],
        supports_streaming_output: true,
    };
    DevicePairingClaim {
        display_name: "Office Mac".to_string(),
        role: DeviceRole::Node,
        platform: capabilities.platform.clone(),
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        credential_sha256: sha256_hex(raw_credential.as_bytes()),
        capabilities,
        requested_grants: DeviceGrant {
            workspace_ids: vec!["project-main".to_string()],
            tool_families: vec!["file".to_string()],
            allow_mutation: false,
        },
    }
}

fn client_claim(raw_credential: &str) -> DevicePairingClaim {
    let mut claim = claim(raw_credential);
    claim.display_name = "Office Client".to_string();
    claim.role = DeviceRole::Client;
    claim.capabilities.tool_families.clear();
    claim.capabilities.workspaces.clear();
    claim.requested_grants = DeviceGrant::default();
    claim
}

fn requested_grant() -> DeviceGrant {
    DeviceGrant {
        workspace_ids: vec!["project-main".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    }
}

fn enabled_service(memory: &MemorySubstrate, now_ms: i64) -> HubPairingService {
    let service = HubPairingService::new(enabled_config(), memory.devices().clone());
    service.open_enrollment_window_at(300, now_ms).unwrap();
    service
}

#[test]
fn disabled_pairing_fails_closed_without_creating_a_claim() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = HubPairingService::new(
        PairingConfig {
            hub_enabled: false,
            ..PairingConfig::default()
        },
        memory.devices().clone(),
    );
    assert_eq!(
        service.create_claim(&claim(&raw_credential('a'))),
        Err(PairingServiceError::Disabled)
    );
    assert!(memory.devices().list_devices().unwrap().is_empty());
}

#[test]
fn claim_approval_exchange_and_revocation_form_one_fail_closed_flow() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = enabled_service(&memory, 1_000);
    let credential = raw_credential('b');
    let challenge = service.create_claim_at(&claim(&credential), 1_000).unwrap();
    challenge.validate(1_000).unwrap();

    let pending = service
        .poll_at(
            &PairingPollRequest {
                request_id: challenge.request_id.clone(),
                polling_secret: challenge.polling_secret.clone(),
            },
            1_500,
        )
        .unwrap();
    assert_eq!(pending.status, PairingState::Pending);

    let device = service
        .approve_request_at(&challenge.request_id, &requested_grant(), 2_000)
        .unwrap();
    assert_eq!(device.role, "node");
    let approved = service
        .poll_at(
            &PairingPollRequest {
                request_id: challenge.request_id,
                polling_secret: challenge.polling_secret,
            },
            2_500,
        )
        .unwrap();
    assert_eq!(approved.status, PairingState::Approved);
    assert_eq!(
        approved.device_id.as_deref(),
        Some(device.device_id.as_str())
    );
    assert_eq!(approved.approved_grants, Some(requested_grant()));

    let access = service
        .exchange_device_credential_at(
            &DeviceCredentialExchange {
                device_id: device.device_id.clone(),
                credential: credential.clone(),
            },
            3_000,
        )
        .unwrap();
    assert_eq!(access.approved_grants, requested_grant());
    assert_eq!(service.active_access_token_count_at(3_500), 1);
    let identity = service
        .authenticate_access_token_at(&access.access_token, 3_500)
        .unwrap();
    assert_eq!(identity.device_id, device.device_id);
    assert_eq!(identity.role, DeviceRole::Node);

    service.revoke_device(&device.device_id).unwrap();
    assert_eq!(service.active_access_token_count_at(4_000), 0);
    assert_eq!(
        service.authenticate_access_token_at(&access.access_token, 4_000),
        Err(PairingServiceError::InvalidDeviceCredential)
    );
    assert_eq!(
        service.exchange_device_credential_at(
            &DeviceCredentialExchange {
                device_id: device.device_id,
                credential,
            },
            4_000,
        ),
        Err(PairingServiceError::InvalidDeviceCredential)
    );
}

#[test]
fn client_authentication_is_role_scoped_revocable_and_refreshes_presence() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = enabled_service(&memory, 1_000);
    let credential = raw_credential('9');
    let challenge = service
        .create_claim_at(&client_claim(&credential), 1_000)
        .unwrap();
    let device = service
        .approve_request_at(&challenge.request_id, &DeviceGrant::default(), 2_000)
        .unwrap();
    let access = service
        .exchange_device_credential_at(
            &DeviceCredentialExchange {
                device_id: device.device_id.clone(),
                credential,
            },
            3_000,
        )
        .unwrap();

    let identity = service
        .authenticate_client_access_token_at(&access.access_token, 3_500)
        .unwrap();
    assert_eq!(identity.role, DeviceRole::Client);
    assert_eq!(
        memory
            .devices()
            .get_device(&device.device_id)
            .unwrap()
            .unwrap()
            .last_seen_ms,
        2_000
    );

    service
        .authenticate_client_access_token_at(&access.access_token, 17_000)
        .unwrap();
    assert_eq!(
        memory
            .devices()
            .get_device(&device.device_id)
            .unwrap()
            .unwrap()
            .last_seen_ms,
        17_000
    );

    service.revoke_device(&device.device_id).unwrap();
    assert_eq!(
        service.authenticate_client_access_token_at(&access.access_token, 18_000),
        Err(PairingServiceError::InvalidDeviceCredential)
    );
}

#[test]
fn one_client_credential_supports_bounded_concurrent_surfaces() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = enabled_service(&memory, 1_000);
    let credential = raw_credential('8');
    let challenge = service
        .create_claim_at(&client_claim(&credential), 1_000)
        .unwrap();
    let device = service
        .approve_request_at(&challenge.request_id, &DeviceGrant::default(), 2_000)
        .unwrap();
    let request = DeviceCredentialExchange {
        device_id: device.device_id,
        credential,
    };
    let tokens = (0..=MAX_ACTIVE_ACCESS_TOKENS_PER_DEVICE)
        .map(|offset| {
            service
                .exchange_device_credential_at(&request, 3_000 + offset as i64)
                .unwrap()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        service.active_access_token_count_at(4_000),
        MAX_ACTIVE_ACCESS_TOKENS_PER_DEVICE
    );
    assert_eq!(
        service.authenticate_client_access_token_at(&tokens[0].access_token, 4_000),
        Err(PairingServiceError::InvalidDeviceCredential)
    );
    for token in tokens.iter().skip(1) {
        assert!(service
            .authenticate_client_access_token_at(&token.access_token, 4_000)
            .is_ok());
    }
}

#[test]
fn short_access_token_survives_hub_and_database_restart_without_plaintext() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("captain.db");
    let credential = raw_credential('7');
    let (device_id, access) = {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        let service = enabled_service(&memory, 1_000);
        let challenge = service
            .create_claim_at(&client_claim(&credential), 1_000)
            .unwrap();
        let device = service
            .approve_request_at(&challenge.request_id, &DeviceGrant::default(), 2_000)
            .unwrap();
        let access = service
            .exchange_device_credential_at(
                &DeviceCredentialExchange {
                    device_id: device.device_id.clone(),
                    credential: credential.clone(),
                },
                3_000,
            )
            .unwrap();
        let connection = memory.usage_conn();
        let connection = connection.lock().unwrap();
        let stored: String = connection
            .query_row(
                "SELECT token_sha256 FROM device_access_tokens WHERE device_id = ?1",
                [&device.device_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored, sha256_hex(access.access_token.as_bytes()));
        assert_ne!(stored, access.access_token);
        (device.device_id, access)
    };

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let restarted = HubPairingService::new(enabled_config(), memory.devices().clone());
    let identity = restarted
        .authenticate_client_access_token_at(&access.access_token, 4_000)
        .unwrap();
    assert_eq!(identity.device_id, device_id);

    restarted.revoke_device(&device_id).unwrap();
    drop(restarted);
    drop(memory);
    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let restarted = HubPairingService::new(enabled_config(), memory.devices().clone());
    assert_eq!(
        restarted.authenticate_client_access_token_at(&access.access_token, 5_000),
        Err(PairingServiceError::InvalidDeviceCredential)
    );
}

#[test]
fn node_access_token_cannot_authenticate_the_client_work_api() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = enabled_service(&memory, 1_000);
    let credential = raw_credential('0');
    let challenge = service.create_claim_at(&claim(&credential), 1_000).unwrap();
    let device = service
        .approve_request_at(&challenge.request_id, &requested_grant(), 2_000)
        .unwrap();
    let access = service
        .exchange_device_credential_at(
            &DeviceCredentialExchange {
                device_id: device.device_id,
                credential,
            },
            3_000,
        )
        .unwrap();

    assert_eq!(
        service.authenticate_client_access_token_at(&access.access_token, 3_500),
        Err(PairingServiceError::InvalidDeviceCredential)
    );
}

#[test]
fn approval_cannot_escalate_beyond_the_requested_grant() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = enabled_service(&memory, 1_000);
    let challenge = service
        .create_claim_at(&claim(&raw_credential('c')), 1_000)
        .unwrap();
    let escalation = DeviceGrant {
        workspace_ids: vec!["project-main".to_string()],
        tool_families: vec!["file".to_string(), "shell-process".to_string()],
        allow_mutation: true,
    };
    assert!(matches!(
        service.approve_request_at(&challenge.request_id, &escalation, 2_000),
        Err(PairingServiceError::InvalidGrant(message))
            if message.contains("exceed")
    ));
    assert_eq!(service.pending_requests_at(2_500).unwrap().len(), 1);
}

#[test]
fn pending_claim_and_approval_survive_a_database_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("captain.db");
    let credential = raw_credential('d');
    let challenge = {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        let service = HubPairingService::new(enabled_config(), memory.devices().clone());
        service.open_enrollment_window(300).unwrap();
        service.create_claim(&claim(&credential)).unwrap()
    };

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let restarted = HubPairingService::new(enabled_config(), memory.devices().clone());
    assert_eq!(restarted.pending_requests().unwrap().len(), 1);
    let device = restarted
        .approve_display_code(&challenge.display_code, &requested_grant())
        .unwrap();
    let access = restarted
        .exchange_device_credential(&DeviceCredentialExchange {
            device_id: device.device_id,
            credential,
        })
        .unwrap();
    assert!(restarted
        .authenticate_access_token(&access.access_token)
        .is_ok());
}

#[test]
fn identical_claim_recovers_after_restart_without_duplicating_the_request() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("captain.db");
    let credential = raw_credential('a');
    let original_claim = claim(&credential);
    let first = {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        let service = enabled_service(&memory, 1_000);
        service.create_claim_at(&original_claim, 1_000).unwrap()
    };

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let restarted = HubPairingService::new(enabled_config(), memory.devices().clone());
    assert_eq!(
        restarted.create_claim_at(&original_claim, 2_000),
        Err(PairingServiceError::EnrollmentClosed)
    );
    restarted.open_enrollment_window_at(300, 2_000).unwrap();
    let recovered = restarted.create_claim_at(&original_claim, 2_000).unwrap();

    assert_eq!(recovered.request_id, first.request_id);
    assert_eq!(recovered.expires_at_ms, first.expires_at_ms);
    assert_ne!(recovered.display_code, first.display_code);
    assert_ne!(recovered.polling_secret, first.polling_secret);
    assert_eq!(restarted.pending_requests_at(2_000).unwrap().len(), 1);
    assert_eq!(
        restarted.poll_at(
            &PairingPollRequest {
                request_id: first.request_id,
                polling_secret: first.polling_secret,
            },
            2_100,
        ),
        Err(PairingServiceError::InvalidPollingCredential)
    );

    let mut conflicting = original_claim;
    conflicting.display_name = "Different Node".to_string();
    assert_eq!(
        restarted.create_claim_at(&conflicting, 2_200),
        Err(PairingServiceError::CredentialAlreadyClaimed)
    );
    assert_eq!(
        memory
            .devices()
            .request_id_for_display_code_digest(
                &sha256_hex(recovered.display_code.as_bytes()),
                2_200,
            )
            .unwrap()
            .as_deref(),
        Some(recovered.request_id.as_str())
    );
    assert!(memory
        .devices()
        .request_id_for_display_code_digest(&sha256_hex(first.display_code.as_bytes()), 2_200)
        .unwrap()
        .is_none());
    let device = restarted
        .approve_request_at(&recovered.request_id, &requested_grant(), 2_250)
        .unwrap();
    let approved = restarted
        .poll_at(
            &PairingPollRequest {
                request_id: recovered.request_id,
                polling_secret: recovered.polling_secret,
            },
            2_300,
        )
        .unwrap();
    assert_eq!(approved.status, PairingState::Approved);
    assert_eq!(
        approved.device_id.as_deref(),
        Some(device.device_id.as_str())
    );
}

#[test]
fn wrong_polling_secret_and_expired_access_token_are_rejected() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = enabled_service(&memory, 1_000);
    let credential = raw_credential('e');
    let challenge = service.create_claim_at(&claim(&credential), 1_000).unwrap();
    assert_eq!(
        service.poll_at(
            &PairingPollRequest {
                request_id: challenge.request_id.clone(),
                polling_secret: raw_credential('f'),
            },
            2_000,
        ),
        Err(PairingServiceError::InvalidPollingCredential)
    );
    let device = service
        .approve_request_at(&challenge.request_id, &requested_grant(), 2_000)
        .unwrap();
    let access = service
        .exchange_device_credential_at(
            &DeviceCredentialExchange {
                device_id: device.device_id,
                credential,
            },
            3_000,
        )
        .unwrap();
    assert_eq!(
        service.authenticate_access_token_at(&access.access_token, access.expires_at_ms),
        Err(PairingServiceError::InvalidDeviceCredential)
    );
    assert_eq!(
        service.active_access_token_count_at(access.expires_at_ms),
        0
    );
}

#[test]
fn pending_limit_and_display_code_normalization_are_bounded() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = enabled_service(&memory, 1_000);
    for character in ['1', '2', '3', '4', '5'] {
        service
            .create_claim_at(&claim(&raw_credential(character)), 1_000)
            .unwrap();
    }
    assert_eq!(
        service.create_claim_at(&claim(&raw_credential('6')), 1_000),
        Err(PairingServiceError::TooManyPending)
    );
    assert_eq!(normalize_display_code("abcd efgh").unwrap(), "ABCD-EFGH");
    assert_eq!(
        normalize_display_code("ABCI-EFGH"),
        Err(PairingServiceError::InvalidDisplayCode)
    );
}

#[test]
fn concurrent_approvals_cannot_exceed_the_device_limit() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let mut config = enabled_config();
    config.max_devices = 1;
    let service = Arc::new(HubPairingService::new(config, memory.devices().clone()));
    service.open_enrollment_window_at(300, 1_000).unwrap();
    let first = service
        .create_claim_at(&claim(&raw_credential('7')), 1_000)
        .unwrap();
    let second = service
        .create_claim_at(&claim(&raw_credential('8')), 1_000)
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));

    let handles = [first.request_id, second.request_id].map(|request_id| {
        let service = Arc::clone(&service);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            service.approve_request_at(&request_id, &requested_grant(), 2_000)
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(
                result,
                Err(PairingServiceError::MaximumDevices { limit: 1 })
            ))
            .count(),
        1
    );
    assert_eq!(
        service
            .list_devices()
            .unwrap()
            .into_iter()
            .filter(|device| device.status == "active")
            .count(),
        1
    );
}

#[test]
fn enrollment_is_closed_by_default_expires_and_resets_on_restart() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let service = HubPairingService::new(enabled_config(), memory.devices().clone());
    assert_eq!(
        service.create_claim_at(&claim(&raw_credential('9')), 1_000),
        Err(PairingServiceError::EnrollmentClosed)
    );

    let opened = service.open_enrollment_window_at(1, 1_000).unwrap();
    assert_eq!(opened.expires_at_ms, Some(61_000));
    assert!(service.enrollment_status_at(60_999).unwrap().open);
    assert!(!service.enrollment_status_at(61_000).unwrap().open);
    assert_eq!(
        service.create_claim_at(&claim(&raw_credential('9')), 61_000),
        Err(PairingServiceError::EnrollmentClosed)
    );

    service.open_enrollment_window_at(300, 70_000).unwrap();
    assert!(service.enrollment_status_at(70_000).unwrap().open);
    let restarted = HubPairingService::new(enabled_config(), memory.devices().clone());
    assert!(!restarted.enrollment_status_at(70_000).unwrap().open);
}

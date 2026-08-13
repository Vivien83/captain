use super::*;

fn challenge() -> PairingChallenge {
    PairingChallenge {
        request_id: "00000000-0000-4000-8000-000000000001".to_string(),
        display_code: "2345-6789".to_string(),
        polling_secret: "a".repeat(64),
        expires_at_ms: 2_000,
        approval_path: "/devices/pair?code=2345-6789".to_string(),
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
    }
}

#[test]
fn challenge_is_relative_validated_and_debug_redacted() {
    let challenge = challenge();
    challenge.validate(1_000).unwrap();
    let debug = format!("{challenge:?}");
    assert!(!debug.contains("2345-6789"));
    assert!(!debug.contains(&"a".repeat(64)));
    assert!(!challenge.approval_path.contains("://"));
}

#[test]
fn challenge_rejects_expired_or_forged_approval_paths() {
    let mut challenge = challenge();
    assert_eq!(
        challenge.validate(challenge.expires_at_ms),
        Err(PairingContractError::InvalidExpiry)
    );
    challenge.expires_at_ms = 3_000;
    challenge.approval_path = "https://attacker.example/devices/pair".to_string();
    assert_eq!(
        challenge.validate(1_000),
        Err(PairingContractError::InvalidApprovalPath)
    );
}

#[test]
fn polling_and_credential_secrets_never_appear_in_debug() {
    let poll = PairingPollRequest {
        request_id: "00000000-0000-4000-8000-000000000002".to_string(),
        polling_secret: "b".repeat(64),
    };
    poll.validate().unwrap();
    assert!(!format!("{poll:?}").contains(&poll.polling_secret));

    let exchange = DeviceCredentialExchange {
        device_id: "node-office".to_string(),
        credential: "c".repeat(64),
    };
    exchange.validate().unwrap();
    assert!(!format!("{exchange:?}").contains(&exchange.credential));
}

#[test]
fn access_token_requires_bearer_and_forward_expiry() {
    let mut token = DeviceAccessToken {
        access_token: "d".repeat(64),
        token_type: "Bearer".to_string(),
        issued_at_ms: 1_000,
        expires_at_ms: 2_000,
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        approved_grants: DeviceGrant::default(),
    };
    token.validate(1_500).unwrap();
    assert!(!format!("{token:?}").contains(&token.access_token));

    token.token_type = "Basic".to_string();
    assert_eq!(
        token.validate(1_500),
        Err(PairingContractError::InvalidTokenType)
    );

    token.token_type = "Bearer".to_string();
    assert_eq!(
        token.validate(2_000),
        Err(PairingContractError::InvalidExpiry)
    );
}

#[test]
fn approved_poll_requires_one_exact_grant_and_terminal_states_leak_none() {
    let approved = PairingPollResponse {
        status: PairingState::Approved,
        device_id: Some("node-office".to_string()),
        approved_grants: Some(DeviceGrant {
            workspace_ids: vec!["project-main".to_string()],
            tool_families: vec!["file".to_string()],
            allow_mutation: false,
        }),
        expires_at_ms: 2_000,
    };
    approved.validate().unwrap();
    let debug = format!("{approved:?}");
    assert!(!debug.contains("project-main"));
    assert!(!debug.contains("file"));

    let mut missing_grant = approved.clone();
    missing_grant.approved_grants = None;
    assert_eq!(
        missing_grant.validate(),
        Err(PairingContractError::InvalidPairingState)
    );

    let mut denied = approved;
    denied.status = PairingState::Denied;
    assert_eq!(
        denied.validate(),
        Err(PairingContractError::InvalidPairingState)
    );
}

#[test]
fn secret_and_display_code_shapes_are_strict() {
    let invalid_poll = PairingPollRequest {
        request_id: "00000000-0000-4000-8000-000000000003".to_string(),
        polling_secret: "A".repeat(64),
    };
    assert_eq!(
        invalid_poll.validate(),
        Err(PairingContractError::InvalidSecret("polling_secret"))
    );

    let mut challenge = challenge();
    challenge.display_code = "0123-I456".to_string();
    assert_eq!(
        challenge.validate(1_000),
        Err(PairingContractError::InvalidDisplayCode)
    );
}

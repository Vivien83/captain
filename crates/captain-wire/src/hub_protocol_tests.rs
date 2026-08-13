use super::*;
use captain_types::approval::{approval_action_digest, ApprovalDecision, RiskLevel};
use serde_json::json;

fn capabilities() -> CapabilityDescriptor {
    CapabilityDescriptor {
        captain_version: "0.1.0-alpha.14".to_string(),
        platform: "macos-arm64".to_string(),
        transports: vec![NodeTransport::WebSocket, NodeTransport::LongPoll],
        tool_families: vec!["file".to_string(), "shell-process".to_string()],
        workspaces: vec![LogicalWorkspace {
            workspace_id: "project-alpha".to_string(),
            label: "Project Alpha".to_string(),
            read_only: false,
        }],
        supports_streaming_output: true,
    }
}

#[test]
fn protocol_negotiates_additive_minor_and_rejects_major_mismatch() {
    let local = ProtocolVersion { major: 1, minor: 4 };
    assert_eq!(
        local.negotiate(ProtocolVersion { major: 1, minor: 2 }),
        Ok(ProtocolVersion { major: 1, minor: 2 })
    );
    assert!(matches!(
        local.negotiate(ProtocolVersion { major: 2, minor: 0 }),
        Err(ProtocolContractError::IncompatibleVersion { .. })
    ));
}

#[test]
fn descriptor_has_no_raw_workspace_path_surface() {
    let encoded = serde_json::to_string(&capabilities()).unwrap();
    assert!(encoded.contains("project-alpha"));
    assert!(!encoded.contains("/Users/"));
    assert!(!encoded.contains("workspace_path"));
    assert!(!encoded.contains("local_path"));
}

#[test]
fn pairing_claim_transmits_only_the_device_credential_digest() {
    let secret = "never-send-this-device-secret";
    let claim = DevicePairingClaim {
        display_name: "Work Mac".to_string(),
        role: DeviceRole::Node,
        platform: "macos-arm64".to_string(),
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        credential_sha256: "a".repeat(64),
        capabilities: capabilities(),
        requested_grants: DeviceGrant {
            workspace_ids: vec!["project-alpha".to_string()],
            tool_families: vec!["file".to_string()],
            allow_mutation: false,
        },
    };
    claim.validate().unwrap();
    let encoded = serde_json::to_string(&claim).unwrap();
    assert!(!encoded.contains(secret));
    assert!(!encoded.contains("device_secret"));
}

#[test]
fn pairing_claim_rejects_conflicting_platform_metadata() {
    let claim = DevicePairingClaim {
        display_name: "Work Mac".to_string(),
        role: DeviceRole::Node,
        platform: "linux-amd64".to_string(),
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        credential_sha256: "a".repeat(64),
        capabilities: capabilities(),
        requested_grants: DeviceGrant::default(),
    };
    assert_eq!(
        claim.validate(),
        Err(ProtocolContractError::DeviceMetadataMismatch("platform"))
    );
}

#[test]
fn pairing_claim_rejects_unadvertised_requested_grants() {
    let claim = DevicePairingClaim {
        display_name: "Work Mac".to_string(),
        role: DeviceRole::Node,
        platform: "macos-arm64".to_string(),
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        credential_sha256: "a".repeat(64),
        capabilities: capabilities(),
        requested_grants: DeviceGrant {
            workspace_ids: vec!["missing-workspace".to_string()],
            ..DeviceGrant::default()
        },
    };
    assert!(matches!(
        claim.validate(),
        Err(ProtocolContractError::CapabilityNotAdvertised(value))
            if value == "missing-workspace"
    ));
}

#[test]
fn grants_cannot_exceed_advertised_capabilities() {
    let invalid = DeviceGrant {
        workspace_ids: vec!["unknown-workspace".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    };
    assert!(matches!(
        invalid.validate_against(&capabilities()),
        Err(ProtocolContractError::CapabilityNotAdvertised(_))
    ));
}

#[test]
fn approved_grants_cannot_escalate_the_original_request() {
    let requested = DeviceGrant {
        workspace_ids: vec!["project-alpha".to_string()],
        tool_families: vec!["file".to_string()],
        allow_mutation: false,
    };
    let escalated = DeviceGrant {
        allow_mutation: true,
        ..requested.clone()
    };
    assert_eq!(
        escalated.validate_subset_of(&requested),
        Err(ProtocolContractError::GrantExceedsRequest)
    );
    requested.validate_subset_of(&requested).unwrap();
}

#[test]
fn run_offer_requires_a_logical_workspace_and_object_input() {
    let mut lease = RunLease {
        run_id: "run:123".to_string(),
        attempt: 1,
        idempotency_key: "run:123:1".to_string(),
        workspace_id: "project-alpha".to_string(),
        tool_name: "file_read".to_string(),
        input: json!({"path": "src/main.rs"}),
        effect: RunEffect::ReadOnly,
        lease_expires_at_ms: 1_800_000_000_000,
    };
    lease.validate().unwrap();
    lease.workspace_id = "/Users/operator/project".to_string();
    assert_eq!(
        lease.validate(),
        Err(ProtocolContractError::InvalidIdentifier("workspace_id"))
    );
}

#[test]
fn completion_requires_path_virtualization_evidence() {
    let content = "effect may have happened";
    let mut completion = RunCompletion {
        run_id: "run:123".to_string(),
        attempt: 1,
        status: RunTerminalStatus::Uncertain,
        result_content: content.to_string(),
        result_sha256: sha256_hex(content.as_bytes()),
        total_output_bytes: 24,
        stored_output_bytes: 24,
        capped: false,
        redacted: true,
        path_policy_applied: true,
    };
    completion.validate().unwrap();
    completion.path_policy_applied = false;
    assert_eq!(
        completion.validate(),
        Err(ProtocolContractError::PathPolicyMissing)
    );
    completion.path_policy_applied = true;
    completion.result_sha256 = "b".repeat(64);
    assert_eq!(
        completion.validate(),
        Err(ProtocolContractError::InvalidDigest("result_sha256"))
    );
}

#[test]
fn progress_and_protocol_errors_require_path_virtualization_evidence() {
    for message in [
        HubNodeMessage::RunProgress {
            run_id: "run:123".to_string(),
            attempt: 1,
            progress_sequence: 1,
            message: "workspace://project-alpha/src/main.rs".to_string(),
            path_policy_applied: false,
        },
        HubNodeMessage::ProtocolError {
            code: "local_failure".to_string(),
            message: "workspace://project-alpha is unavailable".to_string(),
            retryable: true,
            path_policy_applied: false,
        },
    ] {
        assert_eq!(
            message.validate(),
            Err(ProtocolContractError::PathPolicyMissing)
        );
    }
}

#[test]
fn run_rejection_and_approval_contracts_bind_the_exact_attempt_and_action() {
    let action_digest = approval_action_digest("shell_exec", b"exact private command");
    let request = RunApprovalRequest {
        run_id: "run:approval".to_string(),
        attempt: 2,
        approval_id: "approval:123".to_string(),
        action_digest: action_digest.clone(),
        action_summary: "Run a command in workspace://project-alpha".to_string(),
        risk_level: RiskLevel::High,
        expires_at_ms: 1_800_000_000_000,
        path_policy_applied: true,
    };
    request.validate().unwrap();

    let decision = RunApprovalDecision {
        run_id: request.run_id.clone(),
        attempt: request.attempt,
        approval_id: request.approval_id.clone(),
        action_digest: action_digest.clone(),
        decision: ApprovalDecision::Approved,
        reason: Some("Approved for this run".to_string()),
        decided_at_ms: 1_799_999_999_000,
    };
    decision.validate().unwrap();

    let rejection = RunRejection {
        run_id: request.run_id.clone(),
        attempt: request.attempt,
        code: "approval_denied".to_string(),
        message: "The exact local action was not approved".to_string(),
        retryable: false,
        path_policy_applied: true,
    };
    rejection.validate().unwrap();

    let mut mismatched = decision.clone();
    mismatched.action_digest = "not-a-digest".to_string();
    assert_eq!(
        mismatched.validate(),
        Err(ProtocolContractError::InvalidDigest("action digest"))
    );

    let mut unvirtualized = rejection.clone();
    unvirtualized.path_policy_applied = false;
    assert_eq!(
        unvirtualized.validate(),
        Err(ProtocolContractError::PathPolicyMissing)
    );
}

#[test]
fn run_decision_debug_output_redacts_operator_facing_text() {
    let digest = approval_action_digest("file_write", b"private content");
    let request = RunApprovalRequest {
        run_id: "run:redacted".to_string(),
        attempt: 1,
        approval_id: "approval:redacted".to_string(),
        action_digest: digest.clone(),
        action_summary: "contains-private-summary".to_string(),
        risk_level: RiskLevel::Medium,
        expires_at_ms: 1_800_000_000_000,
        path_policy_applied: true,
    };
    let decision = RunApprovalDecision {
        run_id: request.run_id.clone(),
        attempt: request.attempt,
        approval_id: request.approval_id.clone(),
        action_digest: digest,
        decision: ApprovalDecision::Denied,
        reason: Some("contains-private-reason".to_string()),
        decided_at_ms: 1_799_999_999_000,
    };
    let rejection = RunRejection {
        run_id: request.run_id.clone(),
        attempt: request.attempt,
        code: "policy_denied".to_string(),
        message: "contains-private-message".to_string(),
        retryable: false,
        path_policy_applied: true,
    };

    assert!(!format!("{request:?}").contains("contains-private-summary"));
    assert!(!format!("{decision:?}").contains("contains-private-reason"));
    assert!(!format!("{rejection:?}").contains("contains-private-message"));
}

#[test]
fn heartbeat_bounds_the_number_of_active_runs() {
    let message = HubNodeMessage::Heartbeat {
        active_run_ids: (0..257).map(|index| format!("run-{index}")).collect(),
    };
    assert_eq!(
        message.validate(),
        Err(ProtocolContractError::LimitExceeded("active runs"))
    );
}

#[test]
fn hello_active_runs_are_backward_compatible_bounded_and_unique() {
    let legacy = json!({
        "type": "hello",
        "payload": {
            "role": "node",
            "capabilities": capabilities(),
            "resume_after_sequence": 0
        }
    });
    let decoded: HubNodeMessage = serde_json::from_value(legacy).unwrap();
    decoded.validate().unwrap();
    assert!(matches!(
        decoded,
        HubNodeMessage::Hello { active_run_ids, .. } if active_run_ids.is_empty()
    ));

    let duplicate = HubNodeMessage::Hello {
        role: DeviceRole::Node,
        capabilities: capabilities(),
        resume_after_sequence: 0,
        active_run_ids: vec!["run-1".to_string(), "run-1".to_string()],
    };
    assert_eq!(
        duplicate.validate(),
        Err(ProtocolContractError::Duplicate("active run"))
    );
}

#[test]
fn envelope_roundtrip_keeps_sequence_and_uncertain_terminal_state() {
    let content = "confirmation unavailable";
    let envelope = HubNodeEnvelope {
        protocol_version: HUB_NODE_PROTOCOL_VERSION,
        device_id: "device:123".to_string(),
        connection_id: "connection:456".to_string(),
        sequence: 9,
        ack_sequence: Some(8),
        sent_at_ms: 1_800_000_000_000,
        message: HubNodeMessage::RunCompleted(RunCompletion {
            run_id: "run:123".to_string(),
            attempt: 2,
            status: RunTerminalStatus::Uncertain,
            result_content: content.to_string(),
            result_sha256: sha256_hex(content.as_bytes()),
            total_output_bytes: 24,
            stored_output_bytes: 24,
            capped: false,
            redacted: false,
            path_policy_applied: true,
        }),
    };
    envelope.validate().unwrap();
    let encoded = serde_json::to_vec(&envelope).unwrap();
    let decoded: HubNodeEnvelope = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, envelope);
}

#[test]
fn debug_output_redacts_tool_input_and_result_content() {
    let lease = RunLease {
        run_id: "run:private".to_string(),
        attempt: 1,
        idempotency_key: "run:private:1".to_string(),
        workspace_id: "project-alpha".to_string(),
        tool_name: "shell_exec".to_string(),
        input: json!({"command": "contains-sensitive-input"}),
        effect: RunEffect::LocalMutation,
        lease_expires_at_ms: 1_800_000_000_000,
    };
    let result_content = "contains-sensitive-output";
    let completion = RunCompletion {
        run_id: "run:private".to_string(),
        attempt: 1,
        status: RunTerminalStatus::Succeeded,
        result_content: result_content.to_string(),
        result_sha256: sha256_hex(result_content.as_bytes()),
        total_output_bytes: 25,
        stored_output_bytes: 25,
        capped: false,
        redacted: false,
        path_policy_applied: true,
    };
    assert!(!format!("{lease:?}").contains("contains-sensitive-input"));
    assert!(!format!("{completion:?}").contains("contains-sensitive-output"));
}

#[test]
fn additive_unknown_fields_are_ignored() {
    let value = json!({
        "captain_version": "0.1.0-alpha.14",
        "platform": "linux-x86_64",
        "transports": ["long_poll"],
        "tool_families": [],
        "workspaces": [],
        "supports_streaming_output": false,
        "future_capability": {"enabled": true}
    });
    let descriptor: CapabilityDescriptor = serde_json::from_value(value).unwrap();
    descriptor.validate().unwrap();
}

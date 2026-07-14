use super::*;

fn sample_agent_id() -> AgentId {
    "01234567-89ab-cdef-0123-456789abcdef".parse().unwrap()
}

#[test]
fn token_env_is_deterministic_per_agent() {
    assert_eq!(
        agent_api_token_env(&sample_agent_id()),
        "CAPTAIN_AGENT_API_TOKEN_01234567_89AB_CDEF_0123_456789ABCDEF"
    );
}

#[test]
fn ingress_route_match_is_exact_enough() {
    assert!(is_agent_api_ingress_route(
        &Method::POST,
        "/hooks/agents/01234567-89ab-cdef-0123-456789abcdef/ingress"
    ));
    assert!(!is_agent_api_ingress_route(
        &Method::GET,
        "/hooks/agents/01234567-89ab-cdef-0123-456789abcdef/ingress"
    ));
    assert!(!is_agent_api_ingress_route(
        &Method::POST,
        "/hooks/agents/01234567-89ab-cdef-0123-456789abcdef/other"
    ));
}

#[test]
fn descriptor_includes_operator_manifest_url() {
    let descriptor = agent_api_descriptor(&sample_agent_id());

    assert_eq!(
        descriptor.manifest_url,
        "/api/agents/01234567-89ab-cdef-0123-456789abcdef/api/manifest"
    );
}

#[test]
fn token_validation_rejects_missing_and_short_tokens() {
    let agent_id = sample_agent_id();
    std::env::remove_var(agent_api_token_env(&agent_id));
    assert!(!validate_agent_api_token(&HeaderMap::new(), &agent_id));

    std::env::set_var(agent_api_token_env(&agent_id), "short");
    let mut headers = HeaderMap::new();
    headers.insert("authorization", "Bearer short".parse().unwrap());
    assert!(!validate_agent_api_token(&headers, &agent_id));
    std::env::remove_var(agent_api_token_env(&agent_id));
}

#[test]
fn ingress_payload_validation_rejects_empty_and_oversized_messages() {
    let mut req = AgentApiIngressRequest {
        request_id: None,
        message: "hello".to_string(),
        sender_id: None,
        sender_name: None,
        metadata: None,
    };
    assert_eq!(validate_agent_api_ingress_payload(&req), Ok(()));

    req.message = "   ".to_string();
    assert_eq!(
        validate_agent_api_ingress_payload(&req),
        Err(AgentApiIngressValidationError::EmptyMessage)
    );

    req.message = "x".repeat(MAX_AGENT_API_MESSAGE_SIZE + 1);
    assert_eq!(
        validate_agent_api_ingress_payload(&req),
        Err(AgentApiIngressValidationError::MessageTooLarge)
    );
}

#[test]
fn ingress_sender_defaults_are_stable() {
    let agent_id = sample_agent_id();

    assert_eq!(
        agent_api_sender_id(&agent_id, None),
        Some("agent-api:01234567".to_string())
    );
    assert_eq!(
        agent_api_sender_id(&agent_id, Some("external-user".to_string())),
        Some("external-user".to_string())
    );
    assert_eq!(agent_api_sender_name(None), Some("Agent API".to_string()));
    assert_eq!(
        agent_api_sender_name(Some("Webhook".to_string())),
        Some("Webhook".to_string())
    );
}

#[test]
fn ingress_failure_status_keeps_quota_distinct() {
    assert_eq!(
        agent_api_execution_failure_status("daily quota exceeded"),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        agent_api_execution_failure_status("agent loop crashed"),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

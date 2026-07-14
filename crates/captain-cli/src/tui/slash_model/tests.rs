use super::*;
use captain_kernel::model_switch::{ModelSwitchPlan, ModelSwitchRisk, ModelSwitchSessionStrategy};

#[test]
fn model_without_args_opens_picker() {
    assert_eq!(model_for("/model", ""), Some(SlashModel::OpenPicker));
}

#[test]
fn model_with_args_switches_with_parsed_strategy() {
    assert_eq!(
        model_for("/model", "gpt-5 --new-session"),
        Some(SlashModel::Switch {
            model: "gpt-5",
            strategy: Some("new_session"),
        })
    );
    assert_eq!(
        model_for("/model", "codex/gpt-5 --compact"),
        Some(SlashModel::Switch {
            model: "codex/gpt-5",
            strategy: Some("compact_session"),
        })
    );
}

#[test]
fn non_model_commands_stay_in_slash_handler() {
    assert_eq!(model_for("/models", ""), None);
    assert_eq!(model_for("/Model", ""), None);
}

#[test]
fn model_picker_and_preflight_messages_preserve_hermes_text() {
    assert_eq!(no_models_available_message(), "No models available.");
    assert_eq!(
        daemon_preflight_parse_failed_message("bad json"),
        "Model switch preflight parse failed: bad json"
    );
    assert_eq!(
        daemon_preflight_http_failed_message("409 Conflict"),
        "Model switch preflight failed (409 Conflict)"
    );
    assert_eq!(
        daemon_preflight_error_message("offline"),
        "Model switch preflight failed: offline"
    );
    assert_eq!(
        inprocess_preflight_failed_message("missing model"),
        "Switch preflight failed: missing model"
    );
}

#[test]
fn blocked_issue_messages_preserve_hermes_text_and_fallback() {
    let plan = serde_json::json!({
        "blocking_issues": ["active stream", "pending approval"]
    });
    assert_eq!(
        daemon_blocking_issues(&plan),
        "active stream\npending approval"
    );
    assert_eq!(
        model_switch_blocked_message(&daemon_blocking_issues(&plan)),
        "Model switch blocked:\nactive stream\npending approval"
    );
    assert_eq!(
        daemon_blocking_issues(&serde_json::json!({})),
        "Unknown issue"
    );
}

#[test]
fn safe_apply_messages_preserve_hermes_text() {
    assert_eq!(
        safe_switch_http_failed_message("500 Internal Server Error"),
        "Safe model switch failed (500 Internal Server Error)"
    );
    assert_eq!(
        safe_switch_error_message("timeout"),
        "Safe model switch failed: timeout"
    );
    assert_eq!(
        safe_switch_default_success_message(),
        "Model switched safely."
    );
    assert_eq!(
        switch_failed_message("kernel error"),
        "Switch failed: kernel error"
    );
    assert_eq!(no_backend_connected_message(), "No backend connected.");
}

fn inprocess_plan(required: bool) -> ModelSwitchPlan {
    ModelSwitchPlan {
        agent_id: "agent-1".to_string(),
        agent_name: "Captain".to_string(),
        current_provider: "codex".to_string(),
        current_model: "gpt-5".to_string(),
        target_provider: "openai".to_string(),
        target_model: "gpt-5.1".to_string(),
        provider_changed: true,
        model_changed: true,
        active_session_id: "session-1".to_string(),
        active_message_count: 12,
        canonical_summary_present: true,
        canonical_recent_count: 3,
        session_strategy_required: required,
        recommended_session_strategy: ModelSwitchSessionStrategy::CompactSession,
        target_model_known: true,
        target_provider_known: true,
        target_auth_configured: true,
        target_supports_tools: Some(true),
        target_supports_vision: Some(false),
        target_supports_streaming: Some(true),
        driver_ready: true,
        driver_error: None,
        risk: ModelSwitchRisk::High,
        can_apply: true,
        blocking_issues: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn daemon_model_switch_decision_applies_explicit_or_recommended_strategy() {
    let plan = serde_json::json!({
        "session_strategy_required": false,
        "recommended_session_strategy": "compact_session"
    });

    match daemon_model_switch_decision("openai/gpt-5.1", Some("new_session"), &plan) {
        DaemonModelSwitchDecision::Apply(strategy) => assert_eq!(strategy, "new_session"),
        DaemonModelSwitchDecision::RequestChoice(_) => panic!("unexpected prompt"),
    }
    match daemon_model_switch_decision("openai/gpt-5.1", None, &plan) {
        DaemonModelSwitchDecision::Apply(strategy) => assert_eq!(strategy, "compact_session"),
        DaemonModelSwitchDecision::RequestChoice(_) => panic!("unexpected prompt"),
    }
}

#[test]
fn daemon_model_switch_decision_builds_pending_prompt_when_required() {
    let plan = serde_json::json!({
        "session_strategy_required": true,
        "current_provider": "codex",
        "current_model": "gpt-5",
        "target_provider": "openai",
        "target_model": "gpt-5.1",
        "risk": "high",
        "recommended_session_strategy": "compact_session",
        "active_message_count": 9,
        "canonical_summary_present": true
    });

    match daemon_model_switch_decision("openai/gpt-5.1", None, &plan) {
        DaemonModelSwitchDecision::RequestChoice(prompt) => {
            assert_eq!(prompt.model_id, "openai/gpt-5.1");
            assert_eq!(prompt.current_provider, "codex");
            assert_eq!(prompt.target_model, "gpt-5.1");
            assert_eq!(prompt.risk, "high");
            assert_eq!(prompt.recommended_session_strategy, "compact_session");
            assert_eq!(prompt.active_message_count, 9);
            assert!(prompt.canonical_summary_present);
        }
        DaemonModelSwitchDecision::Apply(_) => panic!("expected prompt"),
    }
}

#[test]
fn inprocess_model_switch_decision_preserves_strategy_contract() {
    let optional_plan = inprocess_plan(false);
    let required_plan = inprocess_plan(true);

    match inprocess_model_switch_decision("openai/gpt-5.1", Some("compact_session"), &optional_plan)
    {
        InProcessModelSwitchDecision::Apply(strategy) => {
            assert_eq!(strategy, ModelSwitchSessionStrategy::CompactSession)
        }
        InProcessModelSwitchDecision::RequestChoice(_) => panic!("unexpected prompt"),
    }
    match inprocess_model_switch_decision("openai/gpt-5.1", Some("new_session"), &optional_plan) {
        InProcessModelSwitchDecision::Apply(strategy) => {
            assert_eq!(strategy, ModelSwitchSessionStrategy::NewSession)
        }
        InProcessModelSwitchDecision::RequestChoice(_) => panic!("unexpected prompt"),
    }
    match inprocess_model_switch_decision("openai/gpt-5.1", None, &optional_plan) {
        InProcessModelSwitchDecision::Apply(strategy) => {
            assert_eq!(strategy, ModelSwitchSessionStrategy::CompactSession)
        }
        InProcessModelSwitchDecision::RequestChoice(_) => panic!("unexpected prompt"),
    }
    match inprocess_model_switch_decision("openai/gpt-5.1", None, &required_plan) {
        InProcessModelSwitchDecision::RequestChoice(prompt) => {
            assert_eq!(prompt.current_provider, "codex");
            assert_eq!(prompt.target_provider, "openai");
            assert_eq!(prompt.risk, "high");
            assert_eq!(prompt.recommended_session_strategy, "compact_session");
            assert_eq!(prompt.active_message_count, 12);
            assert!(prompt.canonical_summary_present);
        }
        InProcessModelSwitchDecision::Apply(_) => panic!("expected prompt"),
    }
}

#[test]
fn daemon_apply_success_composes_label_and_fallback_message() {
    let body = serde_json::json!({
        "plan": {
            "target_provider": "openai",
            "target_model": "gpt-5.1"
        },
        "message": "Switched with compacted context."
    });
    let (label, message) = daemon_apply_success(&body, "fallback-model");
    assert_eq!(label.as_deref(), Some("openai/gpt-5.1"));
    assert_eq!(message, "Switched with compacted context.");

    let (label, message) = daemon_apply_success(&serde_json::json!({ "plan": {} }), "gpt-5.2");
    assert_eq!(label.as_deref(), Some("?/gpt-5.2"));
    assert_eq!(message, "Model switched safely.");
}

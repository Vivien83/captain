use super::*;

// V.8f (#184) — ProgressThrottle deduplication.
//
// The Telegram bridge updates the in-flight tool bubble at most once
// per second to stay under the platform rate limit. We unit-test the
// pure decision logic here so the bridge wiring stays a thin shell.

#[test]
fn progress_throttle_first_call_is_ready() {
    let mut t = ProgressThrottle::new(std::time::Duration::from_secs(1));
    let now = std::time::Instant::now();
    assert!(t.ready(now), "first ever call must always be ready");
}

#[test]
fn generic_tool_errors_point_to_captain_docs() {
    let msg = render_error_with_suggestion(
        "ssh_exec",
        "No SSH key named 'server'",
        &crate::retry_transformer::RetryTransform::None,
    );
    assert!(msg.contains("captain_docs"));
    assert!(msg.contains("ssh_exec error recovery"));
}

#[test]
fn meta_tool_errors_do_not_self_loop_to_docs() {
    let msg = render_error_with_suggestion(
        "captain_docs",
        "Missing 'query' parameter",
        &crate::retry_transformer::RetryTransform::None,
    );
    assert!(!msg.contains("Recovery hint"));
    assert!(!msg.contains("captain_docs({"));
}

#[test]
fn specific_retry_suggestion_takes_priority_over_docs_hint() {
    let msg = render_error_with_suggestion(
        "shell_exec",
        "Permission denied",
        &crate::retry_transformer::RetryTransform::SuggestSudo {
            command: "sudo true".to_string(),
            requires_approval: true,
        },
    );
    assert!(msg.contains("Retry suggestion"));
    assert!(!msg.contains("Recovery hint"));
}

#[test]
fn cron_webhook_guard_rejects_localhost() {
    let input = serde_json::json!({
        "delivery": {
            "kind": "webhook",
            "url": "http://127.0.0.1:8080/hook"
        }
    });
    let err = ensure_cron_webhook_url_is_public("cron_create", &input)
        .expect_err("localhost webhook must be rejected");
    assert!(err.contains("SSRF blocked"), "{err}");
}

#[test]
fn cron_webhook_guard_allows_public_https() {
    let input = serde_json::json!({
        "delivery": {
            "kind": "webhook",
            "url": "https://8.8.8.8/hook"
        }
    });
    ensure_cron_webhook_url_is_public("cron_create", &input)
        .expect("public webhook should pass the SSRF guard");
}

#[test]
fn progress_throttle_blocks_within_interval() {
    let mut t = ProgressThrottle::new(std::time::Duration::from_secs(1));
    let t0 = std::time::Instant::now();
    assert!(t.ready(t0));
    // 200ms later — too soon
    assert!(!t.ready(t0 + std::time::Duration::from_millis(200)));
    // 999ms later — still too soon
    assert!(!t.ready(t0 + std::time::Duration::from_millis(999)));
}

#[test]
fn progress_throttle_admits_after_interval() {
    let mut t = ProgressThrottle::new(std::time::Duration::from_secs(1));
    let t0 = std::time::Instant::now();
    assert!(t.ready(t0));
    assert!(t.ready(t0 + std::time::Duration::from_millis(1000)));
    // After the second admit, the timer resets — 500ms later must still block
    assert!(!t.ready(t0 + std::time::Duration::from_millis(1500)));
    // 2000ms after t0 = 1000ms after the last admit -> ready again
    assert!(t.ready(t0 + std::time::Duration::from_millis(2000)));
}

#[test]
fn render_tool_error_includes_structured_recovery_block() {
    let msg = render_error_with_suggestion(
        "cron_update",
        "Missing 'id' parameter",
        &crate::retry_transformer::RetryTransform::None,
    );

    assert!(msg.contains("[tool_error]"));
    assert!(msg.contains("\"code\":\"invalid_tool_input\""));
    assert!(msg.contains("\"tool\":\"cron_update\""));
    assert!(msg.contains("Fix the tool input according to the schema"));
    assert!(msg.contains("captain_docs"));
}

#[test]
fn render_security_error_tells_agent_to_use_secret_rails() {
    let msg = render_error_with_suggestion(
        "file_write",
        "Security blocked: raw secret literal detected in generated file",
        &crate::retry_transformer::RetryTransform::None,
    );

    assert!(msg.contains("\"code\":\"security_blocked\""));
    assert!(msg.contains("secret_write"));
    assert!(msg.contains("env_inject"));
}

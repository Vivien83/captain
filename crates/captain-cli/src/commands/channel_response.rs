//! Shared interpretation of daemon channel action responses.

fn response_message(body: &serde_json::Value, fallback: &str) -> String {
    body.get("message")
        .or_else(|| body.get("error"))
        .or_else(|| body.get("note"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

pub(crate) fn channel_test_outcome(http_success: bool, body: &serde_json::Value) -> (bool, String) {
    let status = body.get("status").and_then(serde_json::Value::as_str);
    let success = http_success && status == Some("ok");
    let fallback = if success {
        "Channel test passed."
    } else if status.is_none() {
        "Channel test returned no usable status."
    } else {
        "Channel test failed."
    };
    (success, response_message(body, fallback))
}

pub(crate) fn channel_configure_outcome(
    http_success: bool,
    body: &serde_json::Value,
) -> (bool, String) {
    let status = body.get("status").and_then(serde_json::Value::as_str);
    let success = http_success && matches!(status, Some("configured" | "configured_reload_failed"));
    let fallback = match status {
        Some("configured") => "Channel configured.",
        Some("configured_reload_failed") => "Channel saved, but the daemon could not activate it.",
        _ => "Channel configuration failed.",
    };
    (success, response_message(body, fallback))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_test_requires_both_http_success_and_ok_status() {
        assert_eq!(
            channel_test_outcome(
                true,
                &serde_json::json!({"status": "ok", "message": "live"})
            ),
            (true, "live".to_string())
        );
        assert_eq!(
            channel_test_outcome(
                true,
                &serde_json::json!({"status": "error", "message": "locked"})
            ),
            (false, "locked".to_string())
        );
        assert!(!channel_test_outcome(false, &serde_json::json!({"status": "ok"})).0);
    }

    #[test]
    fn channel_configuration_preserves_reload_warning_as_successful_persistence() {
        let (success, message) = channel_configure_outcome(
            true,
            &serde_json::json!({
                "status": "configured_reload_failed",
                "note": "restart daemon"
            }),
        );

        assert!(success);
        assert_eq!(message, "restart daemon");
        assert!(
            !channel_configure_outcome(
                true,
                &serde_json::json!({"status": "error", "error": "invalid"})
            )
            .0
        );
    }
}

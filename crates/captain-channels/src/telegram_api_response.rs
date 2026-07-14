//! Telegram Bot API response helpers.

use tracing::warn;

/// Convert a non-2xx Telegram response into an error.
///
/// Previously a failed `sendPhoto` / `sendDocument` / ... was only logged
/// and swallowed, so the agent could claim a delivery that Telegram rejected.
/// Keep this helper shared by media/location endpoints so failures propagate.
pub(crate) async fn ensure_success(
    resp: reqwest::Response,
    endpoint: &'static str,
) -> Result<(), Box<dyn std::error::Error>> {
    if resp.status().is_success() {
        return Ok(());
    }
    let status = resp.status();
    let body_text = resp.text().await.unwrap_or_default();
    warn!("Telegram {endpoint} failed ({status}): {body_text}");
    Err(format!("Telegram {endpoint} failed ({status}): {body_text}").into())
}

pub(crate) fn telegram_message_id_from_response(body: &str) -> Option<i64> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|json| json["result"]["message_id"].as_i64())
}

pub(crate) fn telegram_retry_after_seconds(status: u16, body: &str) -> Option<u64> {
    if status != 429 {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    Some(parsed["parameters"]["retry_after"].as_u64().unwrap_or(5))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_api_response_extracts_message_id() {
        let body = r#"{"ok":true,"result":{"message_id":42}}"#;
        assert_eq!(telegram_message_id_from_response(body), Some(42));
        assert_eq!(
            telegram_message_id_from_response(r#"{"ok":true,"result":{}}"#),
            None
        );
        assert_eq!(telegram_message_id_from_response("not json"), None);
    }

    #[test]
    fn telegram_api_response_retry_after_requires_429() {
        let body = r#"{"ok":false,"error_code":429,"parameters":{"retry_after":11}}"#;

        assert_eq!(telegram_retry_after_seconds(429, body), Some(11));
        assert_eq!(telegram_retry_after_seconds(400, body), None);
    }

    #[test]
    fn telegram_api_response_retry_after_defaults_when_missing_or_invalid() {
        assert_eq!(telegram_retry_after_seconds(429, "{}"), Some(5));
        assert_eq!(telegram_retry_after_seconds(429, "not json"), Some(5));
    }
}

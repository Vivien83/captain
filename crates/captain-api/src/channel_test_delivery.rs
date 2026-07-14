//! Live test-message delivery helpers for active channel setup checks.

use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};

pub(crate) async fn send_channel_test_message(
    channel_name: &str,
    target_id: &str,
    config_values: Option<&serde_json::Value>,
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let test_msg = "Captain test message - your channel is connected.";
    match channel_name {
        "discord" => {
            let token = std::env::var("DISCORD_BOT_TOKEN")
                .map_err(|_| "DISCORD_BOT_TOKEN not set".to_string())?;
            let url = format!("https://discord.com/api/v10/channels/{target_id}/messages");
            let response = client
                .post(&url)
                .header("Authorization", format!("Bot {token}"))
                .json(&serde_json::json!({ "content": test_msg }))
                .send()
                .await
                .map_err(|e| format!("Discord request failed: {e}"))?;
            require_success(response, "Discord").await
        }
        "telegram" => {
            let token = std::env::var("TELEGRAM_BOT_TOKEN")
                .map_err(|_| "TELEGRAM_BOT_TOKEN not set".to_string())?;
            let url = format!("https://api.telegram.org/bot{token}/sendMessage");
            let response = client
                .post(&url)
                .json(&serde_json::json!({ "chat_id": target_id, "text": test_msg }))
                .send()
                .await
                .map_err(|e| format!("Telegram request failed: {e}"))?;
            require_success(response, "Telegram").await
        }
        "signal" => send_signal_test_message(&client, target_id, test_msg, config_values).await,
        "email" => send_email_test_message(target_id, test_msg, config_values).await,
        _ => Err(format!(
            "Live test messaging not supported for {channel_name}."
        )),
    }
}

async fn send_signal_test_message(
    client: &reqwest::Client,
    target_id: &str,
    text: &str,
    config_values: Option<&serde_json::Value>,
) -> Result<(), String> {
    let values = config_values
        .and_then(|value| value.as_object())
        .ok_or_else(|| "Signal config not found".to_string())?;
    let api_url = values
        .get("api_url")
        .and_then(|value| value.as_str())
        .unwrap_or("http://localhost:8080")
        .trim_end_matches('/');
    let phone_number = values
        .get("phone_number")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Signal phone_number not configured".to_string())?;
    let response = client
        .post(format!("{api_url}/v2/send"))
        .json(&serde_json::json!({
            "message": text,
            "number": phone_number,
            "recipients": [target_id],
        }))
        .send()
        .await
        .map_err(|e| format!("Signal request failed: {e}"))?;
    require_success(response, "Signal").await
}

async fn send_email_test_message(
    target_id: &str,
    text: &str,
    config_values: Option<&serde_json::Value>,
) -> Result<(), String> {
    if !target_id.contains('@') || !target_id.contains('.') {
        return Err(format!("Invalid email address: '{target_id}'"));
    }
    let values = config_values
        .and_then(|value| value.as_object())
        .ok_or_else(|| "Email config not found".to_string())?;
    let password_env = string_field(values, "password_env").unwrap_or("EMAIL_PASSWORD");
    let password = std::env::var(password_env).map_err(|_| format!("{password_env} not set"))?;
    let adapter = captain_channels::email::EmailAdapter::new(
        required_string_field(values, "imap_host")?.to_string(),
        u16_field(values, "imap_port", 993)?,
        required_string_field(values, "smtp_host")?.to_string(),
        u16_field(values, "smtp_port", 587)?,
        required_string_field(values, "username")?.to_string(),
        password,
        u64_field(values, "poll_interval_secs", 30)?,
        string_array_field(values, "folders"),
        string_array_field(values, "allowed_senders"),
    );
    let user = ChannelUser {
        platform_id: target_id.to_string(),
        display_name: target_id.to_string(),
        captain_user: None,
    };
    adapter
        .send(
            &user,
            ChannelContent::Text(format!("Subject: Captain channel test\n\n{text}")),
        )
        .await
        .map_err(|e| format!("Email SMTP send failed: {e}"))
}

fn required_string_field<'a>(
    values: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<&'a str, String> {
    string_field(values, key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Email {key} not configured"))
}

fn string_field<'a>(
    values: &'a serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<&'a str> {
    values.get(key).and_then(|value| value.as_str())
}

fn u16_field(
    values: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: u16,
) -> Result<u16, String> {
    match values.get(key) {
        Some(value) if value.is_u64() => value
            .as_u64()
            .and_then(|number| u16::try_from(number).ok())
            .ok_or_else(|| format!("Email {key} is outside u16 range")),
        Some(value) if value.is_string() => value
            .as_str()
            .unwrap_or_default()
            .parse::<u16>()
            .map_err(|_| format!("Email {key} must be a number")),
        Some(_) => Err(format!("Email {key} must be a number")),
        None => Ok(default),
    }
}

fn u64_field(
    values: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: u64,
) -> Result<u64, String> {
    match values.get(key) {
        Some(value) if value.is_u64() => Ok(value.as_u64().unwrap_or(default)),
        Some(value) if value.is_string() => value
            .as_str()
            .unwrap_or_default()
            .parse::<u64>()
            .map_err(|_| format!("Email {key} must be a number")),
        Some(_) => Err(format!("Email {key} must be a number")),
        None => Ok(default),
    }
}

fn string_array_field(
    values: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Vec<String> {
    match values.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect(),
        Some(serde_json::Value::String(text)) => text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

async fn require_success(response: reqwest::Response, label: &str) -> Result<(), String> {
    if response.status().is_success() {
        Ok(())
    } else {
        let body = response.text().await.unwrap_or_default();
        Err(format!("{label} API error: {body}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn email_test_delivery_requires_config_values() {
        let err = send_channel_test_message("email", "user@example.com", None)
            .await
            .expect_err("email test delivery must require config");

        assert!(err.contains("Email config"));
    }

    #[tokio::test]
    async fn email_test_delivery_validates_recipient_before_smtp() {
        let values = serde_json::json!({
            "username": "captain@example.com",
            "password_env": "CAPTAIN_TEST_EMAIL_PASSWORD",
            "imap_host": "imap.example.com",
            "smtp_host": "smtp.example.com",
            "allowed_senders": ["user@example.com"]
        });

        let err = send_channel_test_message("email", "not-an-email", Some(&values))
            .await
            .expect_err("email recipient must be validated locally");

        assert!(err.contains("Invalid email address"));
    }
}

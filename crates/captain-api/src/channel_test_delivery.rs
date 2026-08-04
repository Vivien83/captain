//! Live test-message delivery helpers for active channel setup checks.

use captain_channels::email::{email_address_is_valid, EmailConnectivityReport};
use captain_channels::types::{ChannelAdapter, ChannelContent, ChannelUser};
use captain_types::config::EmailAccountConfig;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct EmailChannelTestOutcome {
    pub(crate) connectivity: EmailConnectivityReport,
    pub(crate) message_sent: bool,
}

pub(crate) async fn send_channel_test_message(
    channel_name: &str,
    target_id: &str,
    config_values: Option<&serde_json::Value>,
    resolve: &(dyn Fn(&str) -> Option<String> + Send + Sync),
) -> Result<(), String> {
    let client = reqwest::Client::new();
    let test_msg = "Captain test message - your channel is connected.";
    match channel_name {
        "discord" => {
            let token = resolve("DISCORD_BOT_TOKEN")
                .ok_or_else(|| "DISCORD_BOT_TOKEN not set".to_string())?;
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
            let token = resolve("TELEGRAM_BOT_TOKEN")
                .ok_or_else(|| "TELEGRAM_BOT_TOKEN not set".to_string())?;
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
        "email" => send_email_test_message(target_id, test_msg, config_values, resolve).await,
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
    resolve: &(dyn Fn(&str) -> Option<String> + Send + Sync),
) -> Result<(), String> {
    if !email_address_is_valid(target_id) {
        return Err(format!("Invalid email address: '{target_id}'"));
    }
    let values = config_values
        .and_then(|value| value.as_object())
        .ok_or_else(|| "Email config not found".to_string())?;
    let password_env = string_field(values, "password_env").unwrap_or("EMAIL_PASSWORD");
    let password = resolve(password_env).ok_or_else(|| format!("{password_env} not set"))?;
    let account = EmailAccountConfig {
        alias: "default".to_string(),
        imap_host: required_string_field(values, "imap_host")?.to_string(),
        imap_port: u16_field(values, "imap_port", 993)?,
        smtp_host: required_string_field(values, "smtp_host")?.to_string(),
        smtp_port: u16_field(values, "smtp_port", 587)?,
        username: required_string_field(values, "username")?.to_string(),
        password_env: password_env.to_string(),
        poll_interval_secs: u64_field(values, "poll_interval_secs", 30)?,
        folders: string_array_field(values, "folders"),
        allowed_senders: string_array_field(values, "allowed_senders"),
        ..EmailAccountConfig::default()
    };
    test_email_account(&account, password, Some(target_id), text)
        .await
        .map(|_| ())
}

pub(crate) async fn test_email_account(
    account: &EmailAccountConfig,
    password: String,
    recipient: Option<&str>,
    text: &str,
) -> Result<EmailChannelTestOutcome, String> {
    if let Some(recipient) = recipient {
        if !email_address_is_valid(recipient) {
            return Err(format!("Invalid email address: '{recipient}'"));
        }
    }
    let adapter = captain_channels::email::EmailAdapter::new_named(
        format!("email:{}", account.alias),
        account.alias.clone(),
        account.imap_host.clone(),
        account.imap_port,
        account.smtp_host.clone(),
        account.smtp_port,
        account.username.clone(),
        password,
        account.poll_interval_secs,
        account.folders.clone(),
        account.allowed_senders.clone(),
    );
    let connectivity = adapter.test_connectivity().await?;
    let Some(recipient) = recipient else {
        return Ok(EmailChannelTestOutcome {
            connectivity,
            message_sent: false,
        });
    };
    let user = ChannelUser {
        platform_id: recipient.to_string(),
        display_name: recipient.to_string(),
        captain_user: None,
    };
    adapter
        .send(
            &user,
            ChannelContent::Text(format!("Subject: Captain channel test\n\n{text}")),
        )
        .await
        .map_err(|e| format!("Email SMTP send failed: {e}"))?;
    Ok(EmailChannelTestOutcome {
        connectivity,
        message_sent: true,
    })
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
        let err = send_channel_test_message("email", "user@example.com", None, &|_| None)
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

        let err = send_channel_test_message("email", "not-an-email", Some(&values), &|_| None)
            .await
            .expect_err("email recipient must be validated locally");

        assert!(err.contains("Invalid email address"));
    }
}

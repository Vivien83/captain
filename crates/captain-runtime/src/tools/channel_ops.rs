use crate::kernel_handle::KernelHandle;
use crate::tools::channel_policy::ensure_active_channel;
use crate::tools::{ensure_no_secret_literal, require_kernel, resolve_file_path_for_caller};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const MAX_CHANNEL_DELIVERIES: usize = 10;

pub(crate) async fn tool_channel_reconfigure(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let channel = input["channel"]
        .as_str()
        .ok_or("Missing 'channel' parameter")?
        .trim()
        .to_lowercase();
    if channel.is_empty() {
        return Err("'channel' cannot be empty".into());
    }
    let channel = channel.as_str();

    let kh = require_kernel(kernel)?;
    let home = kh
        .home_dir()
        .ok_or("kernel home_dir not available — cannot validate channel against config.toml")?;
    let config_path = home.join("config.toml");
    if !config_path.exists() {
        return Err(format!(
            "config.toml not found at {} — write it first before calling channel_reconfigure",
            config_path.display()
        ));
    }
    let raw = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config.toml: {e}"))?;
    let doc: toml_edit::DocumentMut = raw
        .parse()
        .map_err(|e| format!("config.toml is malformed: {e}"))?;
    let configured: Vec<String> = doc
        .get("channels")
        .and_then(|c| c.as_table())
        .map(|t| t.iter().map(|(k, _)| k.to_string()).collect())
        .unwrap_or_default();
    if !configured.iter().any(|n| n == channel) {
        let known = if configured.is_empty() {
            "(none)".to_string()
        } else {
            configured.join(", ")
        };
        return Err(format!(
            "channel '{channel}' is not declared under [channels.*] in {} — known: {known}",
            config_path.display()
        ));
    }
    ensure_active_channel(channel)?;

    kh.publish_integration_configured(channel);
    Ok(serde_json::json!({
        "status": "ok",
        "channel": channel,
        "message": format!("reload requested for '{channel}' — applied within seconds")
    })
    .to_string())
}

pub(crate) async fn tool_channel_delivery_batch(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let deliveries = input["deliveries"]
        .as_array()
        .ok_or("Missing 'deliveries' array parameter")?;
    if deliveries.is_empty() {
        return Err("channel_delivery_batch requires at least one delivery".to_string());
    }
    if deliveries.len() > MAX_CHANNEL_DELIVERIES {
        return Err(format!(
            "channel_delivery_batch accepts at most {MAX_CHANNEL_DELIVERIES} deliveries"
        ));
    }
    let stop_on_error = input["stop_on_error"].as_bool().unwrap_or(true);
    let mut results = Vec::with_capacity(deliveries.len());
    for (idx, delivery) in deliveries.iter().enumerate() {
        let outcome = tool_channel_send(delivery, kernel, workspace_root, caller_agent_id).await;
        match outcome {
            Ok(result) => results.push(serde_json::json!({
                "index": idx,
                "success": true,
                "result": result,
            })),
            Err(error) => {
                results.push(serde_json::json!({
                    "index": idx,
                    "success": false,
                    "error": error,
                }));
                if stop_on_error {
                    break;
                }
            }
        }
    }
    serde_json::to_string_pretty(&serde_json::json!({
        "success": results.iter().all(|r| r["success"].as_bool() == Some(true)),
        "tool": "channel_delivery_batch",
        "deliveries_executed": results.len(),
        "results": results,
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

pub(crate) async fn tool_channel_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let channel = channel_send_channel(input)?;
    ensure_active_channel(&channel)?;

    let kh = require_kernel(kernel)?;
    let recipient = channel_send_recipient(input, kh, &channel).await?;
    let thread_id = input["thread_id"].as_str().filter(|s| !s.is_empty());

    if let Some(result) = send_channel_url_media(input, kh, &channel, &recipient, thread_id).await?
    {
        return Ok(result);
    }
    if let Some(result) = send_channel_file_path(
        input,
        kh,
        kernel,
        workspace_root,
        caller_agent_id,
        &channel,
        &recipient,
        thread_id,
    )
    .await?
    {
        return Ok(result);
    }
    send_channel_text(input, kh, &channel, &recipient, thread_id, caller_agent_id).await
}

fn channel_send_channel(input: &serde_json::Value) -> Result<String, String> {
    input["channel"]
        .as_str()
        .ok_or_else(|| "Missing 'channel' parameter".to_string())
        .map(|channel| channel.trim().to_lowercase())
}

async fn channel_send_recipient(
    input: &serde_json::Value,
    kh: &Arc<dyn KernelHandle>,
    channel: &str,
) -> Result<String, String> {
    let recipient_input = input["recipient"]
        .as_str()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !recipient_input.is_empty() {
        return Ok(recipient_input);
    }
    kh.get_channel_default_recipient(channel)
        .await
        .ok_or_else(|| {
            format!(
                "Missing 'recipient' parameter. Set default_chat_id in [channels.{channel}] config \
                 or pass recipient explicitly."
            )
        })
}

async fn send_channel_url_media(
    input: &serde_json::Value,
    kh: &Arc<dyn KernelHandle>,
    channel: &str,
    recipient: &str,
    thread_id: Option<&str>,
) -> Result<Option<String>, String> {
    if let Some(url) = non_empty_input(input, "image_url") {
        let caption = non_empty_input(input, "message");
        ensure_media_url_is_safe("image_url", url, caption)?;
        return kh
            .send_channel_media(channel, recipient, "image", url, caption, None, thread_id)
            .await
            .map(Some);
    }
    if let Some(url) = non_empty_input(input, "file_url") {
        let caption = non_empty_input(input, "message");
        let filename = input["filename"].as_str();
        ensure_media_url_is_safe("file_url", url, caption)?;
        return kh
            .send_channel_media(
                channel, recipient, "file", url, caption, filename, thread_id,
            )
            .await
            .map(Some);
    }
    Ok(None)
}

fn non_empty_input<'a>(input: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    input[key].as_str().filter(|s| !s.is_empty())
}

fn ensure_media_url_is_safe(field: &str, url: &str, caption: Option<&str>) -> Result<(), String> {
    ensure_no_secret_literal("channel_send", field, url)?;
    if let Some(caption) = caption {
        ensure_no_secret_literal("channel_send", "message", caption)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn send_channel_file_path(
    input: &serde_json::Value,
    kh: &Arc<dyn KernelHandle>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
    channel: &str,
    recipient: &str,
    thread_id: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(raw_path) = non_empty_input(input, "file_path") else {
        return Ok(None);
    };
    let resolved = resolve_file_path_for_caller(raw_path, workspace_root, kernel, caller_agent_id)?;
    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read file '{}': {e}", resolved.display()))?;
    if let Ok(text) = std::str::from_utf8(&data) {
        ensure_no_secret_literal("channel_send", "file_path content", text)?;
    }

    let filename = channel_file_name(input, &resolved);
    let mime_type = channel_file_mime_type(&resolved);
    if mime_type.starts_with("image/") {
        return send_channel_image_data(input, kh, channel, recipient, data, mime_type, thread_id)
            .await
            .map(Some);
    }
    kh.send_channel_file_data(channel, recipient, data, &filename, mime_type, thread_id)
        .await
        .map(Some)
}

fn channel_file_name(input: &serde_json::Value, resolved: &Path) -> String {
    input["filename"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            resolved
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string()
        })
}

fn channel_file_mime_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    mime_type_for_extension(&ext)
}

fn mime_type_for_extension(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "m4a" | "aac" => "audio/mp4",
        "flac" => "audio/flac",
        "mp4" => "video/mp4",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

async fn send_channel_image_data(
    input: &serde_json::Value,
    kh: &Arc<dyn KernelHandle>,
    channel: &str,
    recipient: &str,
    data: Vec<u8>,
    mime_type: &str,
    thread_id: Option<&str>,
) -> Result<String, String> {
    let caption = non_empty_input(input, "message");
    if let Some(caption) = caption {
        ensure_no_secret_literal("channel_send", "message", caption)?;
    }
    kh.send_channel_image_data(channel, recipient, data, mime_type, caption, thread_id)
        .await
}

async fn send_channel_text(
    input: &serde_json::Value,
    kh: &Arc<dyn KernelHandle>,
    channel: &str,
    recipient: &str,
    thread_id: Option<&str>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let message = channel_text_message(input)?;
    let final_message = final_channel_text_message(input, channel, recipient, message)?;
    let caller_name = caller_agent_name(kh, caller_agent_id);
    let topic_id = input["topic_id"].as_str().or(thread_id);
    if let Some(metadata) =
        channel_button_metadata(input, kh, channel, caller_name.as_deref(), topic_id)
    {
        return kh
            .send_channel_rich(channel, recipient, &final_message, &metadata)
            .await;
    }
    kh.send_channel_message_from(
        channel,
        recipient,
        &final_message,
        topic_id,
        caller_name.as_deref(),
    )
    .await
}

fn channel_text_message(input: &serde_json::Value) -> Result<&str, String> {
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter (required for text messages)")?;
    if message.is_empty() {
        return Err("Message cannot be empty".to_string());
    }
    ensure_no_secret_literal("channel_send", "message", message)?;
    Ok(message)
}

fn final_channel_text_message(
    input: &serde_json::Value,
    channel: &str,
    recipient: &str,
    message: &str,
) -> Result<String, String> {
    if channel != "email" {
        return Ok(message.to_string());
    }
    if !recipient.contains('@') || !recipient.contains('.') {
        return Err(format!("Invalid email address: '{recipient}'"));
    }
    Ok(if let Some(subject) = non_empty_input(input, "subject") {
        format!("Subject: {subject}\n\n{message}")
    } else {
        message.to_string()
    })
}

fn caller_agent_name(kh: &Arc<dyn KernelHandle>, caller_agent_id: Option<&str>) -> Option<String> {
    caller_agent_id.and_then(|id| {
        kh.list_agents()
            .iter()
            .find(|agent| agent.id == id)
            .map(|agent| agent.name.clone())
    })
}

fn channel_button_metadata(
    input: &serde_json::Value,
    kh: &Arc<dyn KernelHandle>,
    channel: &str,
    caller_name: Option<&str>,
    topic_id: Option<&str>,
) -> Option<HashMap<String, serde_json::Value>> {
    let buttons = input.get("buttons").and_then(|b| b.as_array())?;
    let mut metadata = HashMap::new();
    let resolved_topic = topic_id.map(String::from).or_else(|| {
        if channel == "telegram" {
            caller_name.and_then(|name| kh.get_telegram_topic(name))
        } else {
            None
        }
    });
    if let Some(tid) = &resolved_topic {
        metadata.insert("thread_id".to_string(), serde_json::json!(tid));
    }
    metadata.insert(
        "reply_markup".to_string(),
        serde_json::json!({"inline_keyboard": inline_keyboard_json(buttons)}),
    );
    Some(metadata)
}

fn inline_keyboard_json(buttons: &[serde_json::Value]) -> Vec<Vec<serde_json::Value>> {
    buttons
        .iter()
        .map(|row| {
            if let Some(arr) = row.as_array() {
                arr.iter().map(channel_button_json).collect()
            } else if let Some(s) = row.as_str() {
                vec![serde_json::json!({"text": s, "callback_data": s})]
            } else {
                vec![]
            }
        })
        .collect()
}

fn channel_button_json(btn: &serde_json::Value) -> serde_json::Value {
    if let Some(s) = btn.as_str() {
        serde_json::json!({"text": s, "callback_data": s})
    } else {
        serde_json::json!({
            "text": btn["text"].as_str().unwrap_or("?"),
            "callback_data": btn["data"].as_str().or(btn["callback_data"].as_str()).unwrap_or("?"),
            "url": btn.get("url").and_then(|u| u.as_str()),
        })
    }
}

pub(crate) async fn tool_telegram_set_topic(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_name = input["agent_name"].as_str().ok_or("Missing 'agent_name'")?;
    let topic_id = input["topic_id"].as_str().ok_or("Missing 'topic_id'")?;
    kh.set_telegram_topic(agent_name, topic_id);
    Ok(format!(
        "Topic {} associé à l'agent '{}'. Tous les messages de cet agent iront automatiquement dans ce topic Telegram.",
        topic_id, agent_name
    ))
}

pub(crate) async fn tool_telegram_get_topic(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_name = input["agent_name"].as_str().ok_or("Missing 'agent_name'")?;
    match kh.get_telegram_topic(agent_name) {
        Some(tid) => Ok(format!("Agent '{}' → topic_id: {}", agent_name, tid)),
        None => Ok(format!("Aucun topic associé à l'agent '{}'", agent_name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn channel_delivery_batch_rejects_empty_and_excessive_batches() {
        let empty =
            tool_channel_delivery_batch(&serde_json::json!({ "deliveries": [] }), None, None, None)
                .await
                .expect_err("empty batches must be rejected before kernel access");
        assert!(empty.contains("at least one delivery"));

        let too_many = serde_json::json!({
            "deliveries": vec![serde_json::json!({"channel": "telegram", "message": "x"}); MAX_CHANNEL_DELIVERIES + 1]
        });
        let err = tool_channel_delivery_batch(&too_many, None, None, None)
            .await
            .expect_err("oversized batches must be rejected before kernel access");
        assert!(err.contains("at most"));
    }

    #[tokio::test]
    async fn channel_send_rejects_non_core_channel_before_kernel_access() {
        let err = tool_channel_send(
            &serde_json::json!({
                "channel": "slack",
                "recipient": "C123",
                "message": "hello"
            }),
            None,
            None,
            None,
        )
        .await
        .expect_err("frozen channels must be rejected before kernel access");

        assert!(err.contains("slack"));
        assert!(err.contains("telegram, discord, signal, email"));
        assert!(err.contains("frozen"));
    }

    #[tokio::test]
    async fn channel_delivery_batch_reports_frozen_channel_item() {
        let result = tool_channel_delivery_batch(
            &serde_json::json!({
                "deliveries": [
                    {"channel": "matrix", "recipient": "!room:example.com", "message": "hello"}
                ],
                "stop_on_error": true
            }),
            None,
            None,
            None,
        )
        .await
        .expect("batch should serialize the per-item failure");

        assert!(result.contains("\"success\": false"));
        assert!(result.contains("matrix"));
        assert!(result.contains("frozen"));
    }

    #[test]
    fn channel_button_json_supports_string_and_rich_buttons() {
        assert_eq!(
            channel_button_json(&serde_json::json!("Oui")),
            serde_json::json!({"text": "Oui", "callback_data": "Oui"})
        );
        assert_eq!(
            channel_button_json(&serde_json::json!({
                "text": "Open",
                "url": "https://example.com",
                "callback_data": "open"
            })),
            serde_json::json!({
                "text": "Open",
                "callback_data": "open",
                "url": "https://example.com"
            })
        );
    }

    #[test]
    fn channel_send_channel_normalizes_input() {
        assert_eq!(
            channel_send_channel(&serde_json::json!({ "channel": " TELEGRAM " })).unwrap(),
            "telegram"
        );
        assert_eq!(
            channel_send_channel(&serde_json::json!({}))
                .expect_err("missing channel must be rejected"),
            "Missing 'channel' parameter"
        );
    }

    #[test]
    fn channel_text_helpers_validate_message_and_email_subject() {
        assert_eq!(
            channel_text_message(&serde_json::json!({ "message": "hello" })).unwrap(),
            "hello"
        );
        assert!(channel_text_message(&serde_json::json!({ "message": "" }))
            .expect_err("empty text message must be rejected")
            .contains("cannot be empty"));
        assert_eq!(
            final_channel_text_message(
                &serde_json::json!({ "subject": "Status" }),
                "email",
                "ops@example.com",
                "ready"
            )
            .unwrap(),
            "Subject: Status\n\nready"
        );
        assert!(
            final_channel_text_message(&serde_json::json!({}), "email", "not-email", "ready")
                .expect_err("invalid email recipient must be rejected")
                .contains("Invalid email address")
        );
    }

    #[test]
    fn channel_file_helpers_resolve_filename_and_mime_type() {
        assert_eq!(
            channel_file_name(
                &serde_json::json!({ "filename": "report.pdf" }),
                Path::new("/tmp/source.bin")
            ),
            "report.pdf"
        );
        assert_eq!(
            channel_file_name(&serde_json::json!({}), Path::new("/tmp/source.csv")),
            "source.csv"
        );
        assert_eq!(
            channel_file_mime_type(Path::new("/tmp/photo.JPG")),
            "image/jpeg"
        );
        assert_eq!(
            channel_file_mime_type(Path::new("/tmp/archive.unknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn inline_keyboard_json_supports_rows_strings_and_ignored_values() {
        assert_eq!(
            inline_keyboard_json(&[
                serde_json::json!("Yes"),
                serde_json::json!([
                    { "text": "Open", "url": "https://example.com", "data": "open" }
                ]),
                serde_json::json!({ "ignored": true }),
            ]),
            vec![
                vec![serde_json::json!({"text": "Yes", "callback_data": "Yes"})],
                vec![serde_json::json!({
                    "text": "Open",
                    "callback_data": "open",
                    "url": "https://example.com"
                })],
                vec![],
            ]
        );
    }
}

//! Telegram update content parsing.

use crate::types::ChannelContent;
use tracing::debug;

pub(crate) async fn parse_telegram_update_content(
    update_id: i64,
    message: &serde_json::Value,
    token: &str,
    client: &reqwest::Client,
    api_base_url: &str,
) -> Option<ChannelContent> {
    if let Some(text) = message["text"].as_str() {
        return Some(parse_telegram_text_content(text, message));
    }

    if let Some(photos) = message["photo"].as_array() {
        let file_id = photos
            .last()
            .and_then(|p| p["file_id"].as_str())
            .unwrap_or("");
        let caption = message["caption"].as_str().map(String::from);
        return Some(
            match telegram_get_file_url(token, client, file_id, api_base_url).await {
                Ok(url) => ChannelContent::Image { url, caption },
                Err(err) => ChannelContent::Text(format!(
                    "[Photo received{}, téléchargement échoué: {err}]",
                    caption
                        .as_deref()
                        .map(|c| format!(": {c}"))
                        .unwrap_or_default()
                )),
            },
        );
    }

    if message.get("document").is_some() {
        return Some(parse_telegram_document_content(message, token, client, api_base_url).await);
    }

    if message.get("voice").is_some() {
        let file_id = message["voice"]["file_id"].as_str().unwrap_or("");
        let duration = message["voice"]["duration"].as_u64().unwrap_or(0) as u32;
        return Some(
            match telegram_get_file_url(token, client, file_id, api_base_url).await {
                Ok(url) => ChannelContent::Voice {
                    url,
                    duration_seconds: duration,
                },
                Err(err) => ChannelContent::Text(format!(
                    "[Voice message, {duration}s, téléchargement échoué: {err}]"
                )),
            },
        );
    }

    if message.get("video").is_some() {
        return Some(
            parse_telegram_video_like_content(
                message,
                "video",
                "Vidéo reçue",
                token,
                client,
                api_base_url,
            )
            .await,
        );
    }

    if message.get("animation").is_some() {
        return Some(
            parse_telegram_video_like_content(
                message,
                "animation",
                "Vidéo (animation) reçue",
                token,
                client,
                api_base_url,
            )
            .await,
        );
    }

    if message.get("location").is_some() {
        let lat = message["location"]["latitude"].as_f64().unwrap_or(0.0);
        let lon = message["location"]["longitude"].as_f64().unwrap_or(0.0);
        return Some(ChannelContent::Location { lat, lon });
    }

    debug!(
        "Telegram: dropping update {update_id} — unsupported message type (no text/photo/document/voice/location)"
    );
    None
}

fn parse_telegram_text_content(text: &str, message: &serde_json::Value) -> ChannelContent {
    if let Some(entities) = message["entities"].as_array() {
        let is_bot_command = entities.iter().any(|entity| {
            entity["type"].as_str() == Some("bot_command") && entity["offset"].as_i64() == Some(0)
        });
        if is_bot_command {
            let parts: Vec<&str> = text.splitn(2, ' ').collect();
            let cmd_name = parts[0].trim_start_matches('/');
            let cmd_name = cmd_name.split('@').next().unwrap_or(cmd_name);
            let args = if parts.len() > 1 {
                parts[1].split_whitespace().map(String::from).collect()
            } else {
                vec![]
            };
            return ChannelContent::Command {
                name: cmd_name.to_string(),
                args,
            };
        }
    }

    ChannelContent::Text(text.to_string())
}

async fn parse_telegram_document_content(
    message: &serde_json::Value,
    token: &str,
    client: &reqwest::Client,
    api_base_url: &str,
) -> ChannelContent {
    let file_id = message["document"]["file_id"].as_str().unwrap_or("");
    let filename = message["document"]["file_name"]
        .as_str()
        .unwrap_or("document")
        .to_string();
    let mime_type = message["document"]["mime_type"].as_str();
    let caption = message["caption"].as_str().map(String::from);

    if is_telegram_video_document(&filename, mime_type) {
        return match telegram_get_file_url(token, client, file_id, api_base_url).await {
            Ok(url) => ChannelContent::Video {
                url,
                duration_seconds: 0,
                caption,
            },
            Err(err) => ChannelContent::Text(format!(
                "[Vidéo reçue comme document ({filename}), téléchargement échoué: {err}]"
            )),
        };
    }

    match telegram_get_file_url(token, client, file_id, api_base_url).await {
        Ok(url) => ChannelContent::File { url, filename },
        Err(err) => ChannelContent::Text(format!(
            "[Document received: {filename}, téléchargement échoué: {err}]"
        )),
    }
}

async fn parse_telegram_video_like_content(
    message: &serde_json::Value,
    key: &str,
    fallback_label: &str,
    token: &str,
    client: &reqwest::Client,
    api_base_url: &str,
) -> ChannelContent {
    let file_id = message[key]["file_id"].as_str().unwrap_or("");
    let duration = message[key]["duration"].as_u64().unwrap_or(0) as u32;
    let caption = message["caption"].as_str().map(String::from);
    match telegram_get_file_url(token, client, file_id, api_base_url).await {
        Ok(url) => ChannelContent::Video {
            url,
            duration_seconds: duration,
            caption,
        },
        Err(err) => {
            ChannelContent::Text(format!("[{fallback_label}, téléchargement échoué: {err}]"))
        }
    }
}

async fn telegram_get_file_url(
    token: &str,
    client: &reqwest::Client,
    file_id: &str,
    api_base_url: &str,
) -> Result<String, String> {
    let url = format!("{api_base_url}/bot{token}/getFile");
    let resp = client
        .post(&url)
        .json(&serde_json::json!({"file_id": file_id}))
        .send()
        .await
        .map_err(|err| format!("requête Telegram getFile échouée: {err}"))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|err| format!("réponse Telegram getFile illisible (HTTP {status}): {err}"))?;
    if body["ok"].as_bool() != Some(true) {
        return Err(telegram_get_file_error(&body, status));
    }
    let file_path = body["result"]["file_path"]
        .as_str()
        .ok_or_else(|| "réponse Telegram getFile sans result.file_path".to_string())?;
    Ok(format!("{api_base_url}/file/bot{token}/{file_path}"))
}

fn telegram_get_file_error(body: &serde_json::Value, status: reqwest::StatusCode) -> String {
    let description = body["description"].as_str().unwrap_or("erreur inconnue");
    match body["error_code"].as_i64() {
        Some(code) => format!("Telegram getFile a échoué ({code}): {description}"),
        None if !status.is_success() => {
            format!("Telegram getFile a échoué (HTTP {status}): {description}")
        }
        None => format!("Telegram getFile a échoué: {description}"),
    }
}

fn is_telegram_video_document(filename: &str, mime_type: Option<&str>) -> bool {
    if mime_type
        .map(|mime| mime.to_ascii_lowercase().starts_with("video/"))
        .unwrap_or(false)
    {
        return true;
    }

    let lower = filename.to_ascii_lowercase();
    [
        ".mp4", ".mov", ".m4v", ".webm", ".mkv", ".avi", ".3gp", ".mpeg", ".mpg",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn telegram_update_content_parses_text() {
        let message = serde_json::json!({ "text": "Hello" });
        let content = parse_telegram_update_content(
            1,
            &message,
            "fake:token",
            &reqwest::Client::new(),
            "https://api.telegram.org",
        )
        .await
        .expect("text content");

        match content {
            ChannelContent::Text(text) => assert_eq!(text, "Hello"),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn telegram_update_content_parses_bot_command() {
        let message = serde_json::json!({
            "text": "/agent@captainbot alpha beta",
            "entities": [{ "type": "bot_command", "offset": 0, "length": 17 }]
        });
        let content = parse_telegram_update_content(
            2,
            &message,
            "fake:token",
            &reqwest::Client::new(),
            "https://api.telegram.org",
        )
        .await
        .expect("command content");

        match content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, vec!["alpha", "beta"]);
            }
            other => panic!("expected command content, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn telegram_update_content_parses_location() {
        let message = serde_json::json!({
            "location": { "latitude": 51.5074, "longitude": -0.1278 }
        });
        let content = parse_telegram_update_content(
            3,
            &message,
            "fake:token",
            &reqwest::Client::new(),
            "https://api.telegram.org",
        )
        .await
        .expect("location content");

        match content {
            ChannelContent::Location { lat, lon } => {
                assert_eq!(lat, 51.5074);
                assert_eq!(lon, -0.1278);
            }
            other => panic!("expected location content, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn telegram_update_content_drops_unsupported_messages() {
        let message = serde_json::json!({
            "sticker": { "file_id": "sticker-id" }
        });

        let content = parse_telegram_update_content(
            4,
            &message,
            "fake:token",
            &reqwest::Client::new(),
            "https://api.telegram.org",
        )
        .await;

        assert!(content.is_none());
    }

    #[test]
    fn telegram_update_content_detects_video_documents() {
        assert!(is_telegram_video_document(
            "clip-original.pdf",
            Some("video/mp4")
        ));
        assert!(is_telegram_video_document("clip-original.MOV", None));
        assert!(!is_telegram_video_document(
            "report.pdf",
            Some("application/pdf")
        ));
    }

    #[test]
    fn telegram_update_content_formats_get_file_errors() {
        let body = serde_json::json!({
            "ok": false,
            "error_code": 400,
            "description": "Bad Request: file is too big"
        });

        let err = telegram_get_file_error(&body, reqwest::StatusCode::OK);

        assert_eq!(
            err,
            "Telegram getFile a échoué (400): Bad Request: file is too big"
        );
    }
}

//! Multimedia pipeline and media understanding runtime handlers.

use std::path::Path;

use crate::media_understanding::MediaEngine;
use crate::tts::TtsEngine;

use super::{
    tool_image_generate, tool_speech_to_text, tool_text_to_speech, tool_video_analyze,
    truncate_owned, validate_path,
};

const MAX_MEDIA_PIPELINE_ITEMS: usize = 12;

pub(crate) async fn tool_media_pipeline(
    input: &serde_json::Value,
    media_engine: Option<&MediaEngine>,
    tts_engine: Option<&TtsEngine>,
    workspace_root: Option<&Path>,
    tool_use_id: &str,
) -> Result<String, String> {
    let items = input["items"]
        .as_array()
        .ok_or("Missing 'items' array parameter")?;
    if items.is_empty() {
        return Err("media_pipeline requires at least one item".to_string());
    }
    if items.len() > MAX_MEDIA_PIPELINE_ITEMS {
        return Err(format!(
            "media_pipeline accepts at most {MAX_MEDIA_PIPELINE_ITEMS} items"
        ));
    }

    let stop_on_error = input["stop_on_error"].as_bool().unwrap_or(false);
    let preview_chars = input["preview_chars"]
        .as_u64()
        .unwrap_or(5000)
        .clamp(500, 20_000) as usize;
    let mut results = Vec::with_capacity(items.len());

    for (idx, item) in items.iter().enumerate() {
        let action = item["action"]
            .as_str()
            .or_else(|| item["type"].as_str())
            .ok_or_else(|| format!("items[{idx}] missing 'action' or 'type'"))?
            .trim()
            .to_ascii_lowercase();
        let outcome = match action.as_str() {
            "describe_image" | "image" | "media_describe" => {
                tool_media_describe(item, media_engine).await
            }
            "transcribe_audio" | "audio" | "media_transcribe" | "speech_to_text" => {
                tool_speech_to_text(item, media_engine, workspace_root).await
            }
            "video" | "video_analyze" => {
                tool_video_analyze(item, media_engine, workspace_root, tool_use_id).await
            }
            "tts" | "text_to_speech" => tool_text_to_speech(item, tts_engine, workspace_root).await,
            "image_generate" | "generate_image" => tool_image_generate(item, workspace_root).await,
            other => Err(format!(
                "Unsupported media_pipeline action '{other}'. Use describe_image, transcribe_audio, video, tts, or image_generate."
            )),
        };
        match outcome {
            Ok(output) => results.push(serde_json::json!({
                "index": idx,
                "action": action,
                "success": true,
                "preview": truncate_owned(&output, preview_chars),
            })),
            Err(error) => {
                results.push(serde_json::json!({
                    "index": idx,
                    "action": action,
                    "success": false,
                    "error": error,
                }));
                if stop_on_error {
                    break;
                }
            }
        }
    }

    let mut out = serde_json::json!({
        "success": results.iter().all(|r| r["success"].as_bool() == Some(true)),
        "tool": "media_pipeline",
        "items_executed": results.len(),
        "results": results,
    });
    if let Some(document) = input.get("document").filter(|v| v.is_object()) {
        out["document"] = create_media_pipeline_document(document, &out, workspace_root).await?;
    }
    serde_json::to_string_pretty(&out).map_err(|e| format!("Serialize error: {e}"))
}

pub(crate) async fn tool_media_describe(
    input: &serde_json::Value,
    media_engine: Option<&MediaEngine>,
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _ = validate_path(path)?;
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read image file: {e}"))?;
    let mime = image_mime_for_path(path)?;

    let attachment = captain_types::media::MediaAttachment {
        media_type: captain_types::media::MediaType::Image,
        mime_type: mime.to_string(),
        source: captain_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
        context_hint: input["prompt"].as_str().map(|s| s.to_string()),
        batch_size_hint: None,
    };
    serde_json::to_string_pretty(&engine.describe_image(&attachment).await?)
        .map_err(|e| format!("Serialize error: {e}"))
}

pub(crate) async fn tool_media_transcribe(
    input: &serde_json::Value,
    media_engine: Option<&MediaEngine>,
) -> Result<String, String> {
    use base64::Engine;
    let engine = media_engine.ok_or("Media engine not available. Check media configuration.")?;
    let path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let _ = validate_path(path)?;
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| format!("Failed to read audio file: {e}"))?;
    let mime = audio_mime_for_path(path)?;

    let attachment = captain_types::media::MediaAttachment {
        media_type: captain_types::media::MediaType::Audio,
        mime_type: mime.to_string(),
        source: captain_types::media::MediaSource::Base64 {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            mime_type: mime.to_string(),
        },
        size_bytes: data.len() as u64,
        context_hint: input["language"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(|s| format!("language:{}", s.trim())),
        batch_size_hint: None,
    };
    serde_json::to_string_pretty(&engine.transcribe_audio(&attachment).await?)
        .map_err(|e| format!("Serialize error: {e}"))
}

async fn create_media_pipeline_document(
    document: &serde_json::Value,
    out: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<serde_json::Value, String> {
    let mut content = String::from("# Media Pipeline Results\n\n");
    if let Some(result_items) = out["results"].as_array() {
        for result in result_items {
            content.push_str(&format!(
                "## Item {} - {}\n\n{}\n\n",
                result["index"].as_u64().unwrap_or(0),
                result["action"].as_str().unwrap_or("media"),
                result["preview"]
                    .as_str()
                    .unwrap_or(result["error"].as_str().unwrap_or(""))
            ));
        }
    }
    let mut doc_input = document.clone();
    if let Some(obj) = doc_input.as_object_mut() {
        obj.entry("title")
            .or_insert_with(|| serde_json::Value::String("Media Pipeline Report".to_string()));
        obj.entry("content")
            .or_insert_with(|| serde_json::Value::String(content));
    }
    let created = crate::document_tools::create_document(&doc_input, workspace_root).await?;
    Ok(serde_json::from_str(&created).unwrap_or_else(|_| serde_json::json!({ "raw": created })))
}

fn image_mime_for_path(path: &str) -> Result<&'static str, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "gif" => Ok("image/gif"),
        "webp" => Ok("image/webp"),
        "bmp" => Ok("image/bmp"),
        "svg" => Ok("image/svg+xml"),
        _ => Err(format!("Unsupported image format: .{ext}")),
    }
}

fn audio_mime_for_path(path: &str) -> Result<&'static str, String> {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match ext.as_str() {
        "mp3" => Ok("audio/mpeg"),
        "wav" => Ok("audio/wav"),
        "ogg" | "oga" => Ok("audio/ogg"),
        "flac" => Ok("audio/flac"),
        "m4a" => Ok("audio/mp4"),
        "webm" => Ok("audio/webm"),
        _ => Err(format!("Unsupported audio format: .{ext}")),
    }
}

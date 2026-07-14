//! File upload and attachment route handlers.

use crate::state::AppState;
use crate::types::AttachmentRef;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use dashmap::DashMap;
use std::sync::{Arc, LazyLock};

#[derive(serde::Serialize)]
struct UploadResponse {
    file_id: String,
    filename: String,
    content_type: String,
    size: usize,
    local_path: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcription: Option<String>,
}

struct UploadMeta {
    #[allow(dead_code)]
    filename: String,
    content_type: String,
}

static UPLOAD_REGISTRY: LazyLock<DashMap<String, UploadMeta>> = LazyLock::new(DashMap::new);

const MAX_UPLOAD_SIZE: usize = 10 * 1024 * 1024;
const ALLOWED_CONTENT_TYPES: &[&str] = &["image/", "text/", "application/pdf", "audio/"];

pub fn register_upload(file_id: String, filename: String, content_type: String) {
    UPLOAD_REGISTRY.insert(
        file_id,
        UploadMeta {
            filename,
            content_type,
        },
    );
}

pub fn resolve_attachments(
    attachments: &[AttachmentRef],
) -> Vec<captain_types::message::ContentBlock> {
    use base64::Engine;

    let upload_dir = std::env::temp_dir().join("captain_uploads");
    let mut blocks = Vec::new();

    for attachment in attachments {
        let content_type = match UPLOAD_REGISTRY.get(&attachment.file_id) {
            Some(meta) => meta.content_type.clone(),
            None if !attachment.content_type.is_empty() => attachment.content_type.clone(),
            None => continue,
        };

        if !content_type.starts_with("image/") {
            continue;
        }

        if uuid::Uuid::parse_str(&attachment.file_id).is_err() {
            continue;
        }

        let file_path = upload_dir.join(&attachment.file_id);
        match std::fs::read(&file_path) {
            Ok(data) => {
                let data = base64::engine::general_purpose::STANDARD.encode(&data);
                blocks.push(captain_types::message::ContentBlock::Image {
                    media_type: content_type,
                    data,
                });
            }
            Err(e) => {
                tracing::warn!(
                    file_id = %attachment.file_id,
                    error = %e,
                    "Failed to read upload for attachment"
                );
            }
        }
    }

    blocks
}

/// POST /api/agents/{id}/upload - Upload a file attachment.
pub async fn upload_file(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if id.parse::<captain_types::agent::AgentId>().is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Invalid agent ID"})),
        );
    }

    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();

    if !is_allowed_content_type(&content_type) {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "Unsupported content type. Allowed: image/*, text/*, audio/*, application/pdf"}),
            ),
        );
    }

    let filename = headers
        .get("X-Filename")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("upload")
        .to_string();

    if body.len() > MAX_UPLOAD_SIZE {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(
                serde_json::json!({"error": format!("File too large (max {} MB)", MAX_UPLOAD_SIZE / (1024 * 1024))}),
            ),
        );
    }

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Empty file body"})),
        );
    }

    let file_id = uuid::Uuid::new_v4().to_string();
    let upload_dir = std::env::temp_dir().join("captain_uploads");
    if let Err(e) = std::fs::create_dir_all(&upload_dir) {
        tracing::warn!("Failed to create upload dir: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to create upload directory"})),
        );
    }

    let file_path = upload_dir.join(&file_id);
    if let Err(e) = std::fs::write(&file_path, &body) {
        tracing::warn!("Failed to write upload: {e}");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "Failed to save file"})),
        );
    }

    let size = body.len();
    register_upload(file_id.clone(), filename.clone(), content_type.clone());
    let transcription = transcribe_audio_upload(&state, &content_type, &file_path, size).await;

    (
        StatusCode::CREATED,
        Json(serde_json::json!(UploadResponse {
            file_id: file_id.clone(),
            filename,
            content_type,
            size,
            local_path: file_path.to_string_lossy().to_string(),
            url: format!("/api/uploads/{file_id}"),
            transcription,
        })),
    )
}

/// GET /api/uploads/{file_id} - Serve an uploaded file.
pub async fn serve_upload(Path(file_id): Path<String>) -> impl IntoResponse {
    if uuid::Uuid::parse_str(&file_id).is_err() {
        return json_bytes(StatusCode::BAD_REQUEST, "{\"error\":\"Invalid file ID\"}");
    }

    let file_path = std::env::temp_dir().join("captain_uploads").join(&file_id);
    let content_type = match UPLOAD_REGISTRY.get(&file_id) {
        Some(meta) => meta.content_type.clone(),
        None if file_path.exists() => "image/png".to_string(),
        None => return json_bytes(StatusCode::NOT_FOUND, "{\"error\":\"File not found\"}"),
    };

    match std::fs::read(&file_path) {
        Ok(data) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, content_type)],
            data,
        ),
        Err(_) => json_bytes(
            StatusCode::NOT_FOUND,
            "{\"error\":\"File not found on disk\"}",
        ),
    }
}

fn is_allowed_content_type(content_type: &str) -> bool {
    ALLOWED_CONTENT_TYPES
        .iter()
        .any(|prefix| content_type.starts_with(prefix))
}

fn json_bytes(
    status: StatusCode,
    body: &str,
) -> (StatusCode, [(axum::http::HeaderName, String); 1], Vec<u8>) {
    (
        status,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json".to_string(),
        )],
        body.as_bytes().to_vec(),
    )
}

async fn transcribe_audio_upload(
    state: &AppState,
    content_type: &str,
    file_path: &std::path::Path,
    size: usize,
) -> Option<String> {
    if !content_type.starts_with("audio/") {
        return None;
    }

    let attachment = captain_types::media::MediaAttachment {
        media_type: captain_types::media::MediaType::Audio,
        mime_type: content_type.to_string(),
        source: captain_types::media::MediaSource::FilePath {
            path: file_path.to_string_lossy().to_string(),
        },
        size_bytes: size as u64,
        context_hint: None,
        batch_size_hint: None,
    };
    match state
        .kernel
        .media_engine
        .transcribe_audio(&attachment)
        .await
    {
        Ok(result) => {
            tracing::info!(
                chars = result.description.len(),
                provider = %result.provider,
                "Audio transcribed"
            );
            Some(result.description)
        }
        Err(e) => {
            tracing::warn!("Audio transcription failed: {e}");
            None
        }
    }
}

//! Authenticated operator routes for immutable Captain artifacts.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, Response, StatusCode};
use captain_types::artifact::{ArtifactPreviewKind, ArtifactVersion};
use uuid::Uuid;

use crate::state::AppState;

const MAX_LIST_ITEMS: usize = 200;
const MAX_TEXT_PREVIEW_BYTES: usize = 2 * 1024 * 1024;
const PASSIVE_PREVIEW_CSP: &str =
    "sandbox; default-src 'none'; img-src data:; media-src data:; font-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'self'";
const DOWNLOAD_CSP: &str =
    "sandbox; default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

#[derive(Debug, serde::Deserialize)]
pub struct ArtifactListQuery {
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ArtifactInspectQuery {
    version: Option<u32>,
}

/// GET /api/artifacts - List latest artifact versions across registered agents.
pub async fn list_artifacts(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ArtifactListQuery>,
) -> Response<Body> {
    let limit = query.limit.unwrap_or(50);
    if !(1..=MAX_LIST_ITEMS).contains(&limit) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_limit",
            "limit must be between 1 and 200",
        );
    }
    match state.kernel.operator_artifact_inventory(limit).await {
        Ok(inventory) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "items": inventory.items,
                "status": inventory.status,
            }),
        ),
        Err(error) => internal_artifact_error("list", error),
    }
}

/// GET /api/artifacts/{id}/versions - List immutable versions newest-first.
pub async fn list_artifact_versions(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response<Body> {
    let artifact_id = match parse_artifact_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    match state.kernel.operator_artifact_versions(artifact_id).await {
        Ok(items) => json_response(
            StatusCode::OK,
            serde_json::json!({
                "artifact_id": artifact_id,
                "count": items.len(),
                "items": items,
            }),
        ),
        Err(error) => unavailable_artifact_error("versions", error),
    }
}

/// GET /api/artifacts/{id}?version=N - Inspect one exact/latest version.
pub async fn inspect_artifact(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ArtifactInspectQuery>,
) -> Response<Body> {
    let artifact_id = match parse_artifact_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    if query.version == Some(0) {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_version",
            "version must be positive",
        );
    }
    match state
        .kernel
        .operator_artifact_inspect(artifact_id, query.version)
        .await
    {
        Ok(artifact) => artifact_metadata_response(&artifact),
        Err(error) => unavailable_artifact_error("inspect", error),
    }
}

/// GET /api/artifacts/{id}/versions/{version}/download - Verified attachment.
pub async fn download_artifact(
    State(state): State<Arc<AppState>>,
    Path((id, version)): Path<(String, u32)>,
) -> Response<Body> {
    let artifact_id = match parse_artifact_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    if version == 0 {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_version",
            "version must be positive",
        );
    }
    match state
        .kernel
        .operator_artifact_read(artifact_id, Some(version))
        .await
    {
        Ok((artifact, data)) => {
            audit_artifact_access(&state, "artifact_download", &artifact);
            payload_response(&artifact, data, false, DOWNLOAD_CSP)
        }
        Err(error) => unavailable_artifact_error("download", error),
    }
}

/// GET /api/artifacts/{id}/versions/{version}/preview - Sandboxed preview.
pub async fn preview_artifact(
    State(state): State<Arc<AppState>>,
    Path((id, version)): Path<(String, u32)>,
) -> Response<Body> {
    let artifact_id = match parse_artifact_id(&id) {
        Ok(id) => id,
        Err(response) => return *response,
    };
    if version == 0 {
        return api_error(
            StatusCode::BAD_REQUEST,
            "invalid_version",
            "version must be positive",
        );
    }
    match state
        .kernel
        .operator_artifact_read(artifact_id, Some(version))
        .await
    {
        Ok((artifact, data)) => {
            audit_artifact_access(&state, "artifact_preview", &artifact);
            preview_response(&artifact, data)
        }
        Err(error) => unavailable_artifact_error("preview", error),
    }
}

fn preview_response(artifact: &ArtifactVersion, data: Vec<u8>) -> Response<Body> {
    match artifact.preview_kind {
        ArtifactPreviewKind::Text | ArtifactPreviewKind::Markdown => {
            escaped_text_preview(artifact, data)
        }
        ArtifactPreviewKind::Html => raw_html_preview(artifact, data),
        ArtifactPreviewKind::Image | ArtifactPreviewKind::Pdf => {
            payload_response(artifact, data, true, PASSIVE_PREVIEW_CSP)
        }
        ArtifactPreviewKind::None => api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "preview_unavailable",
            "this artifact format is download-only",
        ),
    }
}

fn escaped_text_preview(artifact: &ArtifactVersion, data: Vec<u8>) -> Response<Body> {
    if data.len() > MAX_TEXT_PREVIEW_BYTES {
        return preview_too_large();
    }
    let text = match String::from_utf8(data) {
        Ok(text) => text,
        Err(_) => {
            return api_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "preview_encoding_invalid",
                "text preview requires valid UTF-8",
            )
        }
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{}</title><style>html{{color-scheme:dark;background:#0b0d0f;color:#e8e4da;font:14px/1.55 ui-monospace,SFMono-Regular,Consolas,monospace}}body{{margin:0;padding:20px}}pre{{margin:0;white-space:pre-wrap;overflow-wrap:anywhere}}</style></head><body><pre>{}</pre></body></html>",
        escape_html(&artifact.title),
        escape_html(&text)
    );
    html_preview_payload(artifact, body.into_bytes())
}

fn raw_html_preview(artifact: &ArtifactVersion, data: Vec<u8>) -> Response<Body> {
    if data.len() > MAX_TEXT_PREVIEW_BYTES {
        return preview_too_large();
    }
    if std::str::from_utf8(&data).is_err() {
        return api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "preview_encoding_invalid",
            "HTML preview requires valid UTF-8",
        );
    }
    html_preview_payload(artifact, data)
}

fn html_preview_payload(artifact: &ArtifactVersion, data: Vec<u8>) -> Response<Body> {
    response_builder(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition("inline", &artifact.filename),
        )
        .header(header::ETAG, quoted_etag(&artifact.sha256))
        .header(header::CONTENT_SECURITY_POLICY, PASSIVE_PREVIEW_CSP)
        .body(Body::from(data))
        .unwrap_or_else(internal_response_build_error)
}

fn payload_response(
    artifact: &ArtifactVersion,
    data: Vec<u8>,
    inline: bool,
    csp: &'static str,
) -> Response<Body> {
    response_builder(StatusCode::OK)
        .header(header::CONTENT_TYPE, artifact.mime_type.as_str())
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition(
                if inline { "inline" } else { "attachment" },
                &artifact.filename,
            ),
        )
        .header(header::ETAG, quoted_etag(&artifact.sha256))
        .header(header::CONTENT_SECURITY_POLICY, csp)
        .body(Body::from(data))
        .unwrap_or_else(internal_response_build_error)
}

fn artifact_metadata_response(artifact: &ArtifactVersion) -> Response<Body> {
    let id = artifact.artifact_id;
    let version = artifact.version;
    let response = serde_json::json!({
        "artifact": artifact,
        "links": {
            "versions": format!("/api/artifacts/{id}/versions"),
            "download": format!("/api/artifacts/{id}/versions/{version}/download"),
            "preview": if artifact.preview_kind == ArtifactPreviewKind::None {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(format!("/api/artifacts/{id}/versions/{version}/preview"))
            },
        }
    });
    let mut response = json_response(StatusCode::OK, response);
    if let Ok(value) = HeaderValue::from_str(&quoted_etag(&artifact.sha256)) {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

fn response_builder(status: StatusCode) -> axum::http::response::Builder {
    Response::builder()
        .status(status)
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
        .header("Cross-Origin-Resource-Policy", "same-origin")
}

fn json_response(status: StatusCode, value: serde_json::Value) -> Response<Body> {
    let data = serde_json::to_vec(&value)
        .unwrap_or_else(|_| b"{\"error\":\"serialization_failed\"}".to_vec());
    response_builder(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(data))
        .unwrap_or_else(internal_response_build_error)
}

fn api_error(status: StatusCode, code: &str, message: &str) -> Response<Body> {
    json_response(
        status,
        serde_json::json!({"error": code, "message": message}),
    )
}

fn parse_artifact_id(value: &str) -> Result<Uuid, Box<Response<Body>>> {
    Uuid::parse_str(value).map_err(|_| {
        Box::new(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_artifact_id",
            "artifact id must be a UUID",
        ))
    })
}

fn unavailable_artifact_error(operation: &str, error: String) -> Response<Body> {
    tracing::warn!(operation, error = %error, "artifact API operation failed");
    let missing = error.contains("has no versions")
        || error.contains("No such file")
        || error.contains("os error 2");
    api_error(
        if missing {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::CONFLICT
        },
        if missing {
            "artifact_not_found"
        } else {
            "artifact_unavailable"
        },
        if missing {
            "artifact or version not found"
        } else {
            "artifact integrity could not be verified"
        },
    )
}

fn internal_artifact_error(operation: &str, error: String) -> Response<Body> {
    tracing::error!(operation, error = %error, "artifact API store unavailable");
    api_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "artifact_store_unavailable",
        "artifact store is temporarily unavailable",
    )
}

fn preview_too_large() -> Response<Body> {
    api_error(
        StatusCode::PAYLOAD_TOO_LARGE,
        "preview_too_large",
        "text and HTML previews are limited to 2 MiB; download the artifact instead",
    )
}

fn content_disposition(kind: &str, filename: &str) -> String {
    let fallback = filename
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!(
        "{kind}; filename=\"{}\"; filename*=UTF-8''{}",
        if fallback.is_empty() {
            "artifact"
        } else {
            &fallback
        },
        percent_encode(filename.as_bytes())
    )
}

fn percent_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(bytes.len());
    for &byte in bytes {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    output
}

fn quoted_etag(sha256: &str) -> String {
    format!("\"{sha256}\"")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn audit_artifact_access(state: &AppState, action: &str, artifact: &ArtifactVersion) {
    state.kernel.audit_log.record_or_alert(
        "operator",
        captain_runtime::audit::AuditAction::FileAccess,
        format!(
            "{action} id={} version={} sha256={}",
            artifact.artifact_id, artifact.version, artifact.sha256
        ),
        "ok",
    );
}

fn internal_response_build_error(error: axum::http::Error) -> Response<Body> {
    tracing::error!(error = %error, "build artifact API response");
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{\"error\":\"response_build_failed\"}"))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::Router;
    use captain_memory::artifacts::PublishArtifactRequest;
    use captain_runtime::kernel_handle::KernelHandle;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use chrono::Utc;
    use std::time::Instant;
    use tower::ServiceExt;

    fn artifact(kind: ArtifactPreviewKind, mime_type: &str, filename: &str) -> ArtifactVersion {
        ArtifactVersion {
            artifact_id: Uuid::new_v4(),
            version: 1,
            agent_id: "captain".to_string(),
            session_id: Some("session-1".to_string()),
            title: "<unsafe title>".to_string(),
            filename: filename.to_string(),
            mime_type: mime_type.to_string(),
            preview_kind: kind,
            size_bytes: 1,
            sha256: "a".repeat(64),
            created_at: Utc::now(),
            summary: None,
        }
    }

    #[tokio::test]
    async fn text_preview_escapes_active_content_and_sets_sandbox() {
        let response = preview_response(
            &artifact(ArtifactPreviewKind::Text, "text/plain", "unsafe.txt"),
            b"<script>alert(1)</script>".to_vec(),
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .starts_with("sandbox;"));
        let body = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(!body.contains("<script>alert(1)</script>"));
        assert!(body.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[tokio::test]
    async fn raw_html_preview_keeps_bytes_but_denies_script_authority() {
        let response = preview_response(
            &artifact(ArtifactPreviewKind::Html, "text/html", "report.html"),
            b"<script>window.top.location='https://example.com'</script>".to_vec(),
        );
        assert_eq!(response.status(), StatusCode::OK);
        let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap();
        assert!(csp.contains("sandbox"));
        assert!(csp.contains("default-src 'none'"));
        assert!(!csp.contains("script-src"));
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(body.starts_with(b"<script>"));
    }

    #[tokio::test]
    async fn download_is_attachment_nosniff_and_no_store() {
        let artifact = artifact(
            ArtifactPreviewKind::Pdf,
            "application/pdf",
            "résumé 2026.pdf",
        );
        let response = payload_response(&artifact, b"%PDF".to_vec(), false, DOWNLOAD_CSP);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/pdf");
        assert!(response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment;"));
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "private, no-store"
        );
        assert_eq!(response.headers()["X-Content-Type-Options"], "nosniff");
        assert_eq!(
            response.headers()[header::ETAG],
            quoted_etag(&artifact.sha256)
        );
    }

    #[test]
    fn active_unknown_formats_remain_download_only() {
        let response = preview_response(
            &artifact(ArtifactPreviewKind::None, "image/svg+xml", "active.svg"),
            b"<svg/>".to_vec(),
        );
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn mounted_routes_serve_verified_artifacts_without_managed_paths() {
        let temp = tempfile::tempdir().unwrap();
        let kernel = Arc::new(
            captain_kernel::CaptainKernel::boot_with_config(KernelConfig {
                home_dir: temp.path().join("home"),
                data_dir: temp.path().join("data"),
                default_model: DefaultModelConfig {
                    provider: "ollama".to_string(),
                    model: "test-model".to_string(),
                    api_key_env: "OLLAMA_API_KEY".to_string(),
                    base_url: None,
                },
                ..KernelConfig::default()
            })
            .unwrap(),
        );
        let principal = kernel
            .registry
            .list()
            .into_iter()
            .find(|agent| agent.name == "captain")
            .unwrap();
        let source = temp.path().join("report.md");
        std::fs::write(&source, "# Verified API report\n").unwrap();
        let artifact = KernelHandle::artifact_publish(
            kernel.as_ref(),
            &principal.id.to_string(),
            PublishArtifactRequest {
                artifact_id: None,
                agent_id: "forged".to_string(),
                session_id: None,
                title: "Verified API report".to_string(),
                filename: "report.md".to_string(),
                mime_type: "text/markdown".to_string(),
                summary: None,
                source_path: source,
                expected_sha256: None,
            },
        )
        .await
        .unwrap();
        let state = Arc::new(AppState {
            kernel: Arc::clone(&kernel),
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        let app =
            crate::server_artifact_routes::mount_artifact_routes(Router::new()).with_state(state);

        let list = app
            .clone()
            .oneshot(
                Request::get("/api/artifacts?limit=20")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        let list_body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
        let list_json: serde_json::Value = serde_json::from_slice(&list_body).unwrap();
        assert_eq!(list_json["status"]["healthy"], true);
        assert_eq!(
            list_json["items"][0]["artifact_id"],
            artifact.artifact_id.to_string()
        );
        assert!(!String::from_utf8_lossy(&list_body).contains("/data/artifacts"));

        let invalid_limit = app
            .clone()
            .oneshot(
                Request::get("/api/artifacts?limit=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_limit.status(), StatusCode::BAD_REQUEST);

        let inspect = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/artifacts/{}?version={}",
                    artifact.artifact_id, artifact.version
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(inspect.status(), StatusCode::OK);
        assert_eq!(
            inspect.headers()[header::ETAG],
            quoted_etag(&artifact.sha256)
        );
        let inspect_body = to_bytes(inspect.into_body(), usize::MAX).await.unwrap();
        let inspect_json: serde_json::Value = serde_json::from_slice(&inspect_body).unwrap();
        assert_eq!(inspect_json["artifact"]["version"], artifact.version);
        assert!(inspect_json["links"]["preview"].is_string());
        assert!(!String::from_utf8_lossy(&inspect_body).contains("/data/artifacts"));

        let versions = app
            .clone()
            .oneshot(
                Request::get(format!("/api/artifacts/{}/versions", artifact.artifact_id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(versions.status(), StatusCode::OK);

        let preview = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/artifacts/{}/versions/{}/preview",
                    artifact.artifact_id, artifact.version
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preview.status(), StatusCode::OK);
        assert!(preview.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .starts_with("sandbox;"));

        let download = app
            .oneshot(
                Request::get(format!(
                    "/api/artifacts/{}/versions/{}/download",
                    artifact.artifact_id, artifact.version
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(download.into_body(), usize::MAX).await.unwrap(),
            &b"# Verified API report\n"[..]
        );
        kernel.shutdown();
    }
}

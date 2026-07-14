//! Pure helpers for inbound image messages received through channels.

use super::ChannelBridgeHandle;
use crate::types::ChannelContent;
use captain_types::message::ContentBlock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

/// Hard cap for inbound image downloads. Matches the media validation limit so
/// a channel photo saved here can be handed directly to `MediaEngine`.
pub(crate) const MAX_INBOUND_IMAGE_BYTES: u64 = captain_types::media::MAX_IMAGE_BYTES;

#[derive(Debug, Clone)]
pub(crate) struct InboundImageFile {
    pub path: PathBuf,
    pub mime_type: String,
    pub size_bytes: u64,
}

#[derive(Debug, Default)]
pub(super) struct InboundImagePreparation {
    pub(super) file: Option<InboundImageFile>,
    pub(super) description: Option<String>,
    pub(super) processing_error: Option<String>,
}

pub(super) async fn prepare_inbound_image_content(
    handle: &Arc<dyn ChannelBridgeHandle>,
    content: &ChannelContent,
    dest_dir: &Path,
    channel_type: &str,
) -> InboundImagePreparation {
    let ChannelContent::Image { url, caption } = content else {
        return InboundImagePreparation::default();
    };

    match download_image_to_file(url, dest_dir, MAX_INBOUND_IMAGE_BYTES).await {
        Ok(file) => {
            info!(
                path = %file.path.display(),
                mime = %file.mime_type,
                size_bytes = file.size_bytes,
                "Inbound image saved"
            );
            let image_path = file.path.display().to_string();
            let mut preparation = InboundImagePreparation {
                file: Some(file),
                ..InboundImagePreparation::default()
            };
            match handle
                .describe_channel_image(&image_path, caption.as_deref())
                .await
            {
                Ok(Some(description)) if !description.trim().is_empty() => {
                    preparation.description = Some(description.trim().to_string());
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Inbound image description failed for {channel_type}: {e}");
                    preparation.processing_error = Some(e);
                }
            }
            preparation
        }
        Err(e) => {
            warn!("Inbound image download failed for {channel_type}: {e}");
            InboundImagePreparation {
                processing_error: Some(e),
                ..InboundImagePreparation::default()
            }
        }
    }
}

/// Detect image format from the first few magic bytes.
///
/// Returns `Some("image/...")` for JPEG, PNG, GIF, and WebP.
pub(super) fn detect_image_magic(bytes: &[u8]) -> Option<String> {
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return Some("image/jpeg".to_string());
    }
    if bytes.len() >= 4 && bytes[..4] == [0x89, 0x50, 0x4E, 0x47] {
        return Some("image/png".to_string());
    }
    if bytes.len() >= 4 && bytes[..4] == [0x47, 0x49, 0x46, 0x38] {
        return Some("image/gif".to_string());
    }
    if bytes.len() >= 12
        && bytes[..4] == [0x52, 0x49, 0x46, 0x46]
        && bytes[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        return Some("image/webp".to_string());
    }
    None
}

/// Guess image media type from the URL file extension.
pub(super) fn media_type_from_url(url: &str) -> String {
    if url.contains(".png") {
        "image/png".to_string()
    } else if url.contains(".gif") {
        "image/gif".to_string()
    } else if url.contains(".webp") {
        "image/webp".to_string()
    } else {
        // JPEG is the most common image format - safe default.
        "image/jpeg".to_string()
    }
}

pub(super) fn image_extension_for_mime(mime_type: &str) -> &'static str {
    match mime_type.split(';').next().unwrap_or(mime_type).trim() {
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "jpg",
    }
}

pub(super) fn build_inbound_image_prompt(
    channel: &str,
    url: &str,
    caption: Option<&str>,
    image_file: Option<&InboundImageFile>,
    description: Option<&str>,
    processing_error: Option<&str>,
) -> String {
    let cap_line = caption
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(|c| format!("\nLégende: {c}"))
        .unwrap_or_default();
    let user_prompt = caption
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or("Analyse cette image.");
    let error_line = processing_error
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(|e| format!("\nErreur traitement image: {}", truncate_for_prompt(e, 500)))
        .unwrap_or_default();

    match image_file {
        Some(file) => {
            let description_block = description
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(|d| format!("\n\nAnalyse automatique:\n{d}"))
                .unwrap_or_else(|| {
                    "\n\nAnalyse automatique indisponible. Utilise le chemin local avec media_describe si une analyse visuelle plus précise est nécessaire.".to_string()
                });
            format!(
                "[Image reçue depuis {channel}]\n\
                 Chemin local: {path}\n\
                 MIME: {mime}\n\
                 Taille: {size} octets{cap_line}{error_line}{description_block}\n\n\
                 {user_prompt}",
                path = file.path.display(),
                mime = file.mime_type.as_str(),
                size = file.size_bytes,
            )
        }
        None => format!(
            "[Image reçue depuis {channel} — téléchargement local échoué]\n\
             URL: {url}{cap_line}{error_line}\n\n\
             {user_prompt}"
        ),
    }
}

fn truncate_for_prompt(text: &str, max_chars: usize) -> String {
    let mut iter = text.chars();
    let mut out: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        out.push_str("...");
    }
    out
}

/// Download an image URL to a unique file under `dest_dir`, rejecting payloads
/// larger than `max_bytes`. The MIME type is detected from trusted headers,
/// magic bytes, then URL extension, in that order.
pub(crate) async fn download_image_to_file(
    url: &str,
    dest_dir: &std::path::Path,
    max_bytes: u64,
) -> Result<InboundImageFile, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("image download request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("image download HTTP {}", resp.status()));
    }

    if let Some(len) = resp.content_length() {
        if len > max_bytes {
            return Err(format!(
                "image too large: Content-Length {len} bytes exceeds cap {max_bytes}"
            ));
        }
    }

    // Detect media type from Content-Type header - but only trust it if it's
    // actually an image/* type. Many APIs (Telegram, S3 pre-signed URLs) return
    // `application/octet-stream` for all files, which breaks vision.
    let header_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| {
            ct.split(';')
                .next()
                .unwrap_or(ct)
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|ct| ct.starts_with("image/"));

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("image read failed: {e}"))?;

    // Three-tier media type detection:
    // 1. Trusted Content-Type header (only if image/*)
    // 2. Magic byte sniffing (most reliable for binary data)
    // 3. URL extension fallback
    let media_type = header_type
        .unwrap_or_else(|| detect_image_magic(&bytes).unwrap_or_else(|| media_type_from_url(url)));

    if (bytes.len() as u64) > max_bytes {
        return Err(format!(
            "image too large: body {} bytes exceeds cap {max_bytes}",
            bytes.len()
        ));
    }

    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("create dir {} failed: {e}", dest_dir.display()))?;

    let filename = format!(
        "{}.{}",
        uuid::Uuid::new_v4(),
        image_extension_for_mime(&media_type)
    );
    let dest = dest_dir.join(filename);
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| format!("write image {} failed: {e}", dest.display()))?;

    Ok(InboundImageFile {
        path: dest,
        mime_type: media_type,
        size_bytes: bytes.len() as u64,
    })
}

pub(super) async fn image_file_to_blocks(
    file: &InboundImageFile,
    text: &str,
) -> Result<Vec<ContentBlock>, String> {
    use base64::Engine;

    let bytes = tokio::fs::read(&file.path)
        .await
        .map_err(|e| format!("read image {} failed: {e}", file.path.display()))?;
    if (bytes.len() as u64) > MAX_INBOUND_IMAGE_BYTES {
        return Err(format!(
            "image too large for inline block: body {} bytes exceeds cap {}",
            bytes.len(),
            MAX_INBOUND_IMAGE_BYTES
        ));
    }

    Ok(vec![
        ContentBlock::Text {
            text: text.to_string(),
            provider_metadata: None,
        },
        ContentBlock::Image {
            media_type: file.mime_type.clone(),
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use captain_types::agent::AgentId;
    use std::sync::Mutex;

    struct MockImageHandle {
        description: Mutex<Result<Option<String>, String>>,
        described: Mutex<Vec<(String, Option<String>)>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockImageHandle {
        async fn send_message(
            &self,
            _agent_id: AgentId,
            _message: &str,
            _channel_type: Option<&str>,
        ) -> Result<String, String> {
            Ok(String::new())
        }

        async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
            Ok(None)
        }

        async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
            Ok(Vec::new())
        }

        async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
            Err("not available".to_string())
        }

        async fn describe_channel_image(
            &self,
            path: &str,
            prompt: Option<&str>,
        ) -> Result<Option<String>, String> {
            self.described
                .lock()
                .unwrap()
                .push((path.to_string(), prompt.map(str::to_string)));
            self.description.lock().unwrap().clone()
        }
    }

    fn handle(description: Result<Option<String>, String>) -> Arc<MockImageHandle> {
        Arc::new(MockImageHandle {
            description: Mutex::new(description),
            described: Mutex::new(Vec::new()),
        })
    }

    #[test]
    fn detects_common_image_magic_bytes() {
        assert_eq!(
            detect_image_magic(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some("image/jpeg".to_string())
        );
        assert_eq!(
            detect_image_magic(&[0x89, 0x50, 0x4E, 0x47]),
            Some("image/png".to_string())
        );
        assert_eq!(
            detect_image_magic(&[0x47, 0x49, 0x46, 0x38]),
            Some("image/gif".to_string())
        );
        assert_eq!(
            detect_image_magic(&[
                0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50,
            ]),
            Some("image/webp".to_string())
        );
        assert_eq!(detect_image_magic(&[0x00, 0x01, 0x02]), None);
        assert_eq!(detect_image_magic(&[]), None);
    }

    #[test]
    fn guesses_media_type_from_url_or_jpeg_default() {
        assert_eq!(
            media_type_from_url("https://example.com/photo.png"),
            "image/png"
        );
        assert_eq!(
            media_type_from_url("https://example.com/anim.gif"),
            "image/gif"
        );
        assert_eq!(
            media_type_from_url("https://example.com/img.webp"),
            "image/webp"
        );
        assert_eq!(
            media_type_from_url("https://api.telegram.org/file/bot123/photos/file_42"),
            "image/jpeg"
        );
    }

    #[test]
    fn maps_image_mime_to_file_extension() {
        assert_eq!(image_extension_for_mime("image/png"), "png");
        assert_eq!(image_extension_for_mime("image/gif"), "gif");
        assert_eq!(image_extension_for_mime("image/webp"), "webp");
        assert_eq!(image_extension_for_mime("image/jpeg"), "jpg");
        assert_eq!(
            image_extension_for_mime("image/jpeg; charset=binary"),
            "jpg"
        );
    }

    #[test]
    fn prompt_keeps_path_description_and_caption() {
        let file = InboundImageFile {
            path: std::path::PathBuf::from("/tmp/captain/inbound/telegram/photo.jpg"),
            mime_type: "image/jpeg".to_string(),
            size_bytes: 1234,
        };

        let prompt = build_inbound_image_prompt(
            "telegram",
            "https://example.invalid/photo",
            Some("décris le badge"),
            Some(&file),
            Some("On voit un badge NFC sur une table."),
            None,
        );

        assert!(prompt.contains("Chemin local: /tmp/captain/inbound/telegram/photo.jpg"));
        assert!(prompt.contains("MIME: image/jpeg"));
        assert!(prompt.contains("Légende: décris le badge"));
        assert!(prompt.contains("Analyse automatique:"));
        assert!(prompt.contains("On voit un badge NFC"));
        assert!(prompt.ends_with("décris le badge"));
    }

    #[tokio::test]
    async fn download_image_writes_file_under_cap_with_magic_detection() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let payload = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        Mock::given(method("GET"))
            .and(path("/telegram-photo"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/octet-stream")
                    .set_body_bytes(payload.clone()),
            )
            .mount(&server)
            .await;

        let url = format!("{}/telegram-photo", server.uri());
        let dest = std::env::temp_dir().join(format!("captain-test-img-{}", uuid::Uuid::new_v4()));

        let file = download_image_to_file(&url, &dest, 1024 * 1024)
            .await
            .expect("happy path must succeed");
        assert_eq!(file.mime_type, "image/png");
        assert_eq!(file.path.extension().and_then(|e| e.to_str()), Some("png"));
        let bytes = tokio::fs::read(&file.path).await.unwrap();
        assert_eq!(bytes, payload);
        let blocks = image_file_to_blocks(&file, "Analyse robuste")
            .await
            .unwrap();
        assert!(
            matches!(blocks.first(), Some(ContentBlock::Text { text, .. }) if text == "Analyse robuste")
        );
        assert!(
            matches!(blocks.get(1), Some(ContentBlock::Image { media_type, .. }) if media_type == "image/png")
        );
        let _ = tokio::fs::remove_dir_all(&dest).await;
    }

    #[tokio::test]
    async fn download_image_rejects_oversize_payload() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let payload = vec![0xFFu8; 2048];
        Mock::given(method("GET"))
            .and(path("/big.jpg"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "image/jpeg")
                    .set_body_bytes(payload),
            )
            .mount(&server)
            .await;

        let url = format!("{}/big.jpg", server.uri());
        let dest = std::env::temp_dir().join(format!("captain-test-img-{}", uuid::Uuid::new_v4()));

        let res = download_image_to_file(&url, &dest, 1024).await;
        assert!(res.is_err(), "expected size-cap rejection, got {res:?}");
        assert!(res.unwrap_err().contains("too large"));
        let _ = tokio::fs::remove_dir_all(&dest).await;
    }

    #[tokio::test]
    async fn prepare_inbound_image_content_ignores_non_image_content() {
        let handle = handle(Ok(Some("unused".to_string())));
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let dir = std::env::temp_dir().join(format!("captain-image-noop-{}", uuid::Uuid::new_v4()));
        let content = ChannelContent::Text("hello".to_string());

        let prepared =
            prepare_inbound_image_content(&handle_trait, &content, &dir, "telegram").await;

        assert!(prepared.file.is_none());
        assert!(prepared.description.is_none());
        assert!(prepared.processing_error.is_none());
        assert!(handle.described.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn prepare_inbound_image_content_downloads_and_trims_description() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let payload = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        Mock::given(method("GET"))
            .and(path("/photo"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Type", "application/octet-stream")
                    .set_body_bytes(payload.clone()),
            )
            .mount(&server)
            .await;

        let handle = handle(Ok(Some("  Une image nette  ".to_string())));
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let dir =
            std::env::temp_dir().join(format!("captain-image-prepare-{}", uuid::Uuid::new_v4()));
        let content = ChannelContent::Image {
            url: format!("{}/photo", server.uri()),
            caption: Some("décris le badge".to_string()),
        };

        let prepared =
            prepare_inbound_image_content(&handle_trait, &content, &dir, "telegram").await;

        let file = prepared.file.expect("image should download");
        assert!(file.path.starts_with(&dir));
        assert_eq!(file.mime_type, "image/png");
        assert_eq!(tokio::fs::read(&file.path).await.unwrap(), payload);
        assert_eq!(prepared.description.as_deref(), Some("Une image nette"));
        assert!(prepared.processing_error.is_none());
        assert_eq!(
            handle.described.lock().unwrap().as_slice(),
            &[(
                file.path.display().to_string(),
                Some("décris le badge".to_string())
            )]
        );
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

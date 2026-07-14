//! Inbound video download helpers for channel video messages.

use crate::types::ChannelContent;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Hard cap for inbound video downloads. Telegram itself caps bot uploads at
/// 20 MB via `getFile`, but 100 MB leaves headroom for self-hosted bot APIs
/// that lift the limit while still preventing unbounded reads.
pub(crate) const MAX_INBOUND_VIDEO_BYTES: u64 = 100 * 1024 * 1024;

pub(super) async fn prepare_inbound_video_local_path(
    content: &ChannelContent,
    dest_dir: &Path,
    channel_type: &str,
) -> Option<PathBuf> {
    let ChannelContent::Video { url, .. } = content else {
        return None;
    };

    match download_video_to_file(url, dest_dir, MAX_INBOUND_VIDEO_BYTES).await {
        Ok(path) => {
            info!("Inbound video saved to {}", path.display());
            Some(path)
        }
        Err(e) => {
            warn!("Inbound video download failed for {channel_type}: {e}");
            None
        }
    }
}

/// Build the French prompt handed to the agent after an inbound video.
pub(crate) fn build_inbound_video_prompt(
    url: &str,
    duration_seconds: u32,
    caption: Option<&str>,
    local_path: Option<&std::path::Path>,
) -> String {
    let cap_line = caption
        .map(|c| format!("\nLégende: {c}"))
        .unwrap_or_default();
    let user_prompt = caption
        .filter(|c| !c.trim().is_empty())
        .map(String::from)
        .unwrap_or_else(|| "Décris-moi cette vidéo.".to_string());

    match local_path {
        Some(path) => format!(
            "[Vidéo reçue depuis Telegram]\n\
             Chemin local: {p}\n\
             Durée: {duration_seconds}s{cap_line}\n\n\
             {user_prompt}",
            p = path.display(),
        ),
        None => format!(
            "[Vidéo reçue depuis Telegram — téléchargement local échoué]\n\
             URL: {url}\n\
             Durée: {duration_seconds}s{cap_line}\n\n\
             {user_prompt}",
        ),
    }
}

/// Download a video URL to a unique `.mp4` file under `dest_dir`, rejecting
/// payloads larger than `max_bytes`.
pub(crate) async fn download_video_to_file(
    url: &str,
    dest_dir: &std::path::Path,
    max_bytes: u64,
) -> Result<std::path::PathBuf, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("video download request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("video download HTTP {}", resp.status()));
    }

    if let Some(len) = resp.content_length() {
        if len > max_bytes {
            return Err(format!(
                "video too large: Content-Length {len} bytes exceeds cap {max_bytes}"
            ));
        }
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("video read failed: {e}"))?;

    if (bytes.len() as u64) > max_bytes {
        return Err(format!(
            "video too large: body {} bytes exceeds cap {max_bytes}",
            bytes.len()
        ));
    }

    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("create dir {} failed: {e}", dest_dir.display()))?;

    let filename = format!("{}.mp4", uuid::Uuid::new_v4());
    let dest = dest_dir.join(filename);
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| format!("write video {} failed: {e}", dest.display()))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_inbound_video_prompt_uses_local_path_and_caption() {
        let prompt = build_inbound_video_prompt(
            "https://example.test/video.mp4",
            12,
            Some("Analyse le geste."),
            Some(std::path::Path::new("/tmp/captain/video.mp4")),
        );

        assert!(prompt.contains("[Vidéo reçue depuis Telegram]"));
        assert!(prompt.contains("Chemin local: /tmp/captain/video.mp4"));
        assert!(prompt.contains("Durée: 12s"));
        assert!(prompt.contains("Légende: Analyse le geste."));
        assert!(prompt.ends_with("Analyse le geste."));
    }

    #[test]
    fn build_inbound_video_prompt_falls_back_to_url_and_default_prompt() {
        let prompt =
            build_inbound_video_prompt("https://example.test/video.mp4", 7, Some("   "), None);

        assert!(prompt.contains("téléchargement local échoué"));
        assert!(prompt.contains("URL: https://example.test/video.mp4"));
        assert!(prompt.contains("Durée: 7s"));
        assert!(prompt.contains("Légende:    "));
        assert!(prompt.ends_with("Décris-moi cette vidéo."));
    }

    #[tokio::test]
    async fn prepare_inbound_video_local_path_ignores_non_video_content() {
        let dir = std::env::temp_dir().join(format!("captain-video-noop-{}", uuid::Uuid::new_v4()));
        let content = ChannelContent::Text("hello".to_string());

        let local = prepare_inbound_video_local_path(&content, &dir, "telegram").await;

        assert!(local.is_none());
    }

    #[tokio::test]
    async fn prepare_inbound_video_local_path_downloads_video_content() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/clip.mp4"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![1, 2, 3, 4]))
            .mount(&server)
            .await;

        let dir =
            std::env::temp_dir().join(format!("captain-video-download-{}", uuid::Uuid::new_v4()));
        let content = ChannelContent::Video {
            url: format!("{}/clip.mp4", server.uri()),
            duration_seconds: 4,
            caption: None,
        };

        let local = prepare_inbound_video_local_path(&content, &dir, "telegram")
            .await
            .expect("video should download");

        assert!(local.starts_with(&dir));
        assert_eq!(tokio::fs::read(&local).await.unwrap(), vec![1, 2, 3, 4]);
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }

    #[tokio::test]
    async fn download_video_rejects_oversize_payload() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let big_body = vec![0u8; 4 * 1024];
        Mock::given(method("GET"))
            .and(path("/big.mp4"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(big_body))
            .mount(&server)
            .await;

        let url = format!("{}/big.mp4", server.uri());
        let dest = std::env::temp_dir().join(format!("captain-test-vid-{}", uuid::Uuid::new_v4()));

        let res = download_video_to_file(&url, &dest, 1024).await;
        assert!(res.is_err(), "expected size-cap rejection, got {res:?}");
        let err = res.unwrap_err();
        assert!(
            err.contains("too large") || err.contains("exceeds cap"),
            "expected size-cap error message, got: {err}"
        );

        let dest_exists = tokio::fs::metadata(&dest).await.is_ok();
        if dest_exists {
            let mut rd = tokio::fs::read_dir(&dest).await.unwrap();
            assert!(
                rd.next_entry().await.unwrap().is_none(),
                "no file should be persisted when the cap fires"
            );
            let _ = tokio::fs::remove_dir_all(&dest).await;
        }
    }

    #[tokio::test]
    async fn download_video_writes_file_under_cap() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let payload = vec![0xDEu8; 64];
        Mock::given(method("GET"))
            .and(path("/ok.mp4"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Length", "64")
                    .set_body_bytes(payload.clone()),
            )
            .mount(&server)
            .await;

        let url = format!("{}/ok.mp4", server.uri());
        let dest = std::env::temp_dir().join(format!("captain-test-vid-{}", uuid::Uuid::new_v4()));

        let path = download_video_to_file(&url, &dest, 1024 * 1024)
            .await
            .expect("happy path must succeed");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("mp4"));
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert_eq!(bytes, payload);
        let _ = tokio::fs::remove_dir_all(&dest).await;
    }
}

//! Inbound audio download helpers for channel voice messages.

use super::ChannelBridgeHandle;
use crate::types::ChannelContent;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{info, warn};

/// Hard cap for inbound voice/audio downloads. Telegram voice messages are
/// usually tiny; 25 MB matches the common STT provider ceiling and prevents a
/// channel message from turning into an unbounded memory read.
pub(crate) const MAX_INBOUND_AUDIO_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct InboundVoicePreparation {
    pub(super) local_path: Option<PathBuf>,
    pub(super) transcript: Option<String>,
    pub(super) transcription_error: Option<String>,
}

pub(super) async fn prepare_inbound_voice_content(
    handle: &Arc<dyn ChannelBridgeHandle>,
    content: &ChannelContent,
    dest_dir: &Path,
    channel_type: &str,
) -> InboundVoicePreparation {
    let ChannelContent::Voice { url, .. } = content else {
        return InboundVoicePreparation::default();
    };

    match download_audio_to_file(url, dest_dir, MAX_INBOUND_AUDIO_BYTES).await {
        Ok(path) => {
            info!("Inbound audio saved to {}", path.display());
            let audio_path = path.display().to_string();
            let mut preparation = InboundVoicePreparation {
                local_path: Some(path),
                ..InboundVoicePreparation::default()
            };
            match handle.transcribe_channel_audio(&audio_path, None).await {
                Ok(Some(transcript)) if !transcript.trim().is_empty() => {
                    preparation.transcript = Some(transcript.trim().to_string());
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Inbound audio transcription failed for {channel_type}: {e}");
                    preparation.transcription_error = Some(e);
                }
            }
            preparation
        }
        Err(e) => {
            warn!("Inbound audio download failed for {channel_type}: {e}");
            InboundVoicePreparation {
                transcription_error: Some(e),
                ..InboundVoicePreparation::default()
            }
        }
    }
}

/// Build the French prompt handed to the agent after an inbound voice message.
pub(crate) fn build_inbound_voice_prompt(
    url: &str,
    duration_seconds: u32,
    transcript: Option<&str>,
    local_path: Option<&std::path::Path>,
    transcription_error: Option<&str>,
) -> String {
    match (transcript, local_path) {
        (Some(transcript), Some(path)) => format!(
            "[Message vocal reçu depuis Telegram]\n\
             Durée: {duration_seconds}s\n\
             Chemin local: {p}\n\
             Transcription automatique:\n{transcript}",
            p = path.display(),
        ),
        (Some(transcript), None) => format!(
            "[Message vocal reçu depuis Telegram]\n\
             Durée: {duration_seconds}s\n\
             URL: {url}\n\
             Transcription automatique:\n{transcript}"
        ),
        (None, Some(path)) => {
            let error_line = transcription_error
                .map(|e| format!("\nErreur transcription automatique: {e}"))
                .unwrap_or_default();
            format!(
                "[Message vocal reçu depuis Telegram]\n\
                 Durée: {duration_seconds}s\n\
                 Chemin local: {p}{error_line}\n\n\
                 Transcris d'abord ce fichier audio avec media_transcribe ou speech_to_text, puis réponds au contenu du vocal.",
                p = path.display(),
            )
        }
        (None, None) => {
            let error_line = transcription_error
                .map(|e| format!("\nErreur téléchargement/transcription: {e}"))
                .unwrap_or_default();
            format!(
                "[Message vocal reçu depuis Telegram — audio local indisponible]\n\
                 URL: {url}\n\
                 Durée: {duration_seconds}s{error_line}"
            )
        }
    }
}

/// Download an audio URL to a unique `.ogg` file under `dest_dir`, rejecting
/// payloads larger than `max_bytes`.
pub(crate) async fn download_audio_to_file(
    url: &str,
    dest_dir: &std::path::Path,
    max_bytes: u64,
) -> Result<std::path::PathBuf, String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("audio download request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("audio download HTTP {}", resp.status()));
    }

    if let Some(len) = resp.content_length() {
        if len > max_bytes {
            return Err(format!(
                "audio too large: Content-Length {len} bytes exceeds cap {max_bytes}"
            ));
        }
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("audio read failed: {e}"))?;

    if (bytes.len() as u64) > max_bytes {
        return Err(format!(
            "audio too large: body {} bytes exceeds cap {max_bytes}",
            bytes.len()
        ));
    }

    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(|e| format!("create dir {} failed: {e}", dest_dir.display()))?;

    let filename = format!("{}.ogg", uuid::Uuid::new_v4());
    let dest = dest_dir.join(filename);
    tokio::fs::write(&dest, &bytes)
        .await
        .map_err(|e| format!("write audio {} failed: {e}", dest.display()))?;
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use captain_types::agent::AgentId;
    use std::sync::Mutex;

    struct MockAudioHandle {
        transcript: Mutex<Result<Option<String>, String>>,
        transcribed_paths: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl ChannelBridgeHandle for MockAudioHandle {
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

        async fn transcribe_channel_audio(
            &self,
            path: &str,
            _language: Option<&str>,
        ) -> Result<Option<String>, String> {
            self.transcribed_paths
                .lock()
                .unwrap()
                .push(path.to_string());
            self.transcript.lock().unwrap().clone()
        }
    }

    fn handle(transcript: Result<Option<String>, String>) -> Arc<MockAudioHandle> {
        Arc::new(MockAudioHandle {
            transcript: Mutex::new(transcript),
            transcribed_paths: Mutex::new(Vec::new()),
        })
    }

    #[test]
    fn build_inbound_voice_prompt_uses_transcript_and_local_path() {
        let prompt = build_inbound_voice_prompt(
            "https://example.test/voice.ogg",
            9,
            Some("Bonjour Captain"),
            Some(std::path::Path::new("/tmp/captain/voice.ogg")),
            None,
        );

        assert!(prompt.contains("[Message vocal reçu depuis Telegram]"));
        assert!(prompt.contains("Durée: 9s"));
        assert!(prompt.contains("Chemin local: /tmp/captain/voice.ogg"));
        assert!(prompt.ends_with("Transcription automatique:\nBonjour Captain"));
    }

    #[test]
    fn build_inbound_voice_prompt_uses_transcript_and_url_without_local_path() {
        let prompt = build_inbound_voice_prompt(
            "https://example.test/voice.ogg",
            5,
            Some("Texte vocal"),
            None,
            None,
        );

        assert!(prompt.contains("URL: https://example.test/voice.ogg"));
        assert!(prompt.contains("Durée: 5s"));
        assert!(prompt.ends_with("Transcription automatique:\nTexte vocal"));
    }

    #[test]
    fn build_inbound_voice_prompt_requests_transcription_when_local_path_has_no_transcript() {
        let prompt = build_inbound_voice_prompt(
            "https://example.test/voice.ogg",
            14,
            None,
            Some(std::path::Path::new("/tmp/captain/voice.ogg")),
            Some("provider unavailable"),
        );

        assert!(prompt.contains("Chemin local: /tmp/captain/voice.ogg"));
        assert!(prompt.contains("Erreur transcription automatique: provider unavailable"));
        assert!(prompt.contains("Transcris d'abord ce fichier audio"));
    }

    #[test]
    fn build_inbound_voice_prompt_reports_unavailable_audio() {
        let prompt = build_inbound_voice_prompt(
            "https://example.test/voice.ogg",
            3,
            None,
            None,
            Some("download failed"),
        );

        assert!(prompt.contains("audio local indisponible"));
        assert!(prompt.contains("URL: https://example.test/voice.ogg"));
        assert!(prompt.contains("Erreur téléchargement/transcription: download failed"));
    }

    #[tokio::test]
    async fn download_audio_writes_file_under_cap() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let payload = vec![0xA5u8; 48];
        Mock::given(method("GET"))
            .and(path("/voice.ogg"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("Content-Length", "48")
                    .set_body_bytes(payload.clone()),
            )
            .mount(&server)
            .await;

        let url = format!("{}/voice.ogg", server.uri());
        let dest = std::env::temp_dir().join(format!("captain-test-aud-{}", uuid::Uuid::new_v4()));

        let path = download_audio_to_file(&url, &dest, 1024 * 1024)
            .await
            .expect("happy path must succeed");
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("ogg"));
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert_eq!(bytes, payload);
        let _ = tokio::fs::remove_dir_all(&dest).await;
    }

    #[tokio::test]
    async fn prepare_inbound_voice_content_ignores_non_voice_content() {
        let handle = handle(Ok(Some("unused".to_string())));
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let dir = std::env::temp_dir().join(format!("captain-audio-noop-{}", uuid::Uuid::new_v4()));
        let content = ChannelContent::Text("hello".to_string());

        let prepared =
            prepare_inbound_voice_content(&handle_trait, &content, &dir, "telegram").await;

        assert!(prepared.local_path.is_none());
        assert!(prepared.transcript.is_none());
        assert!(prepared.transcription_error.is_none());
        assert!(handle.transcribed_paths.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn prepare_inbound_voice_content_downloads_and_trims_transcript() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/voice.ogg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![9, 8, 7]))
            .mount(&server)
            .await;

        let handle = handle(Ok(Some("  Bonjour Captain  ".to_string())));
        let handle_trait: Arc<dyn ChannelBridgeHandle> = handle.clone();
        let dir =
            std::env::temp_dir().join(format!("captain-audio-prepare-{}", uuid::Uuid::new_v4()));
        let content = ChannelContent::Voice {
            url: format!("{}/voice.ogg", server.uri()),
            duration_seconds: 4,
        };

        let prepared =
            prepare_inbound_voice_content(&handle_trait, &content, &dir, "telegram").await;

        let local_path = prepared.local_path.expect("audio should download");
        assert!(local_path.starts_with(&dir));
        assert_eq!(tokio::fs::read(&local_path).await.unwrap(), vec![9, 8, 7]);
        assert_eq!(prepared.transcript.as_deref(), Some("Bonjour Captain"));
        assert!(prepared.transcription_error.is_none());
        assert_eq!(
            handle.transcribed_paths.lock().unwrap().as_slice(),
            &[local_path.display().to_string()]
        );
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}

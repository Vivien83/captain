//! R.3.3 — Whisper Speech-to-Text auto-install adapter.
//!
//! Supports two providers under the same trait impl:
//! * `groq` — Groq hosted whisper-large-v3-turbo (fast, free tier)
//! * `openai` — OpenAI hosted whisper-1
//!
//! ElevenLabs Scribe STT is exposed through the shared media engine when
//! `ELEVENLABS_API_KEY` is configured (typically by the `tts_elevenlabs`
//! integration), not through this Whisper-specific setup helper.
//!
//! The provider determines which env var the [`crate::media_understanding::
//! MediaEngine`] reads (`GROQ_API_KEY` or `OPENAI_API_KEY`), so the
//! orchestrator exports the right one at install time.

use super::{ConfigPatch, IntegrationSetup};

pub struct SttWhisper;

fn provider_of(creds: &serde_json::Value) -> &str {
    creds
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("groq")
}

#[async_trait::async_trait]
impl IntegrationSetup for SttWhisper {
    fn name(&self) -> &str {
        "stt_whisper"
    }

    fn description(&self) -> &str {
        "Whisper speech-to-text. Provider 'groq' (default, fast + free tier) or 'openai' (whisper-1)."
    }

    fn validate(&self, creds: &serde_json::Value) -> Result<(), String> {
        let provider = provider_of(creds);
        if provider != "groq" && provider != "openai" {
            return Err(format!(
                "provider '{provider}' not supported (use 'groq' or 'openai')"
            ));
        }
        let key = creds
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or("missing field: api_key (string)")?;
        match provider {
            "groq" => {
                if !key.starts_with("gsk_") || key.len() < 20 {
                    return Err("groq api_key must start with 'gsk_' and be ≥ 20 chars".into());
                }
            }
            "openai" => {
                if !key.starts_with("sk-") || key.len() < 20 {
                    return Err("openai api_key must start with 'sk-' and be ≥ 20 chars".into());
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn vault_keys(&self, creds: &serde_json::Value) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(k) = creds.get("api_key").and_then(|v| v.as_str()) {
            out.push((format!("{}_api_key", provider_of(creds)), k.to_string()));
        }
        out
    }

    fn env_exports(&self, creds: &serde_json::Value) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(k) = creds.get("api_key").and_then(|v| v.as_str()) {
            let env_name = match provider_of(creds) {
                "groq" => "GROQ_API_KEY",
                "openai" => "OPENAI_API_KEY",
                _ => return out,
            };
            out.push((env_name.to_string(), k.to_string()));
        }
        out
    }

    fn config_patch(&self, creds: &serde_json::Value) -> Vec<ConfigPatch> {
        let path = vec!["media".to_string()];
        let provider = provider_of(creds);
        vec![
            ConfigPatch {
                path: path.clone(),
                key: "audio_provider".into(),
                value: toml_edit::value(provider),
            },
            ConfigPatch {
                path,
                key: "audio_model".into(),
                value: toml_edit::value(match provider {
                    "groq" => "whisper-large-v3-turbo",
                    "openai" => "whisper-1",
                    _ => "whisper-1",
                }),
            },
        ]
    }

    async fn test(&self, creds: &serde_json::Value) -> Result<String, String> {
        let key = creds
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or("api_key missing for live test")?;
        let provider = provider_of(creds);
        let url = match provider {
            "groq" => "https://api.groq.com/openai/v1/models",
            "openai" => "https://api.openai.com/v1/models",
            _ => return Err(format!("unsupported provider: {provider}")),
        };
        let resp = reqwest::Client::new()
            .get(url)
            .bearer_auth(key)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("models endpoint HTTP error: {e}"))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(format!("{provider} rejected the key (401 Unauthorized)"));
        }
        if !resp.status().is_success() {
            return Err(format!("models endpoint returned status {}", resp.status()));
        }
        Ok(format!("Whisper STT ({provider}) reachable."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_accepts_groq_default() {
        let t = SttWhisper;
        let ok = json!({"api_key": "gsk_FAKE_KEY_LONG_ENOUGH_xxx"});
        assert!(t.validate(&ok).is_ok());
    }

    #[test]
    fn validate_accepts_openai_when_provider_set() {
        let t = SttWhisper;
        let ok = json!({"provider": "openai", "api_key": "sk-FAKE_KEY_LONG_ENOUGH_xxx"});
        assert!(t.validate(&ok).is_ok());
    }

    #[test]
    fn validate_rejects_groq_key_with_openai_provider() {
        let t = SttWhisper;
        let bad = json!({"provider": "openai", "api_key": "gsk_FAKE_KEY_LONG_ENOUGH_xxx"});
        assert!(t.validate(&bad).is_err());
    }

    #[test]
    fn validate_rejects_openai_key_with_default_groq_provider() {
        let t = SttWhisper;
        let bad = json!({"api_key": "sk-FAKE_KEY_LONG_ENOUGH_xxx"});
        assert!(t.validate(&bad).is_err());
    }

    #[test]
    fn validate_rejects_unknown_provider() {
        let t = SttWhisper;
        let bad = json!({"provider": "azure", "api_key": "anything"});
        assert!(t.validate(&bad).is_err());
    }

    #[test]
    fn env_exports_groq_uses_groq_api_key() {
        let t = SttWhisper;
        let creds = json!({"api_key": "gsk_FAKE_KEY_LONG_ENOUGH_xxx"});
        let pairs = t.env_exports(&creds);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "GROQ_API_KEY");
    }

    #[test]
    fn env_exports_openai_uses_openai_api_key() {
        let t = SttWhisper;
        let creds = json!({"provider": "openai", "api_key": "sk-FAKE_KEY_LONG_ENOUGH_xxx"});
        let pairs = t.env_exports(&creds);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "OPENAI_API_KEY");
    }

    #[test]
    fn config_patch_picks_right_model_per_provider() {
        let t = SttWhisper;
        let groq_patches = t.config_patch(&json!({"api_key": "gsk_x"}));
        let groq_model = groq_patches
            .iter()
            .find(|p| p.key == "audio_model")
            .unwrap();
        assert_eq!(groq_model.value.as_str(), Some("whisper-large-v3-turbo"));

        let openai_patches = t.config_patch(&json!({
            "provider": "openai", "api_key": "sk-x"
        }));
        let openai_model = openai_patches
            .iter()
            .find(|p| p.key == "audio_model")
            .unwrap();
        assert_eq!(openai_model.value.as_str(), Some("whisper-1"));
    }
}

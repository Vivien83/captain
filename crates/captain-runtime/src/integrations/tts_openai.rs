//! R.3.3 — OpenAI TTS auto-install adapter.
//!
//! Stores the API key in the vault and exports `OPENAI_API_KEY` so the
//! [`crate::tts::TtsEngine`] picks it up on the next `synthesize()` call
//! without a daemon restart.
//!
//! NB: many OpenAI features (chat, embeddings, STT-Whisper) share the
//! same env var, so installing TTS auto-unlocks them as well. The config
//! patch under `[tts.openai]` only sets TTS-specific defaults (voice,
//! model, format).

use super::{ConfigPatch, IntegrationSetup};

pub struct TtsOpenAi;

#[async_trait::async_trait]
impl IntegrationSetup for TtsOpenAi {
    fn name(&self) -> &str {
        "tts_openai"
    }

    fn description(&self) -> &str {
        "OpenAI Text-to-Speech (tts-1, tts-1-hd). Reuses the OPENAI_API_KEY env var; setting this also unlocks chat / embeddings / Whisper STT for the OpenAI provider."
    }

    fn validate(&self, creds: &serde_json::Value) -> Result<(), String> {
        let key = creds
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or("missing field: api_key (string starting with 'sk-')")?;
        if !key.starts_with("sk-") || key.len() < 20 {
            return Err("api_key must start with 'sk-' and be at least 20 chars".into());
        }
        if let Some(v) = creds.get("voice") {
            let s = v.as_str().ok_or("voice must be a string")?;
            // OpenAI voices as of 2024-2026: alloy, echo, fable, onyx, nova, shimmer.
            const VOICES: &[&str] = &["alloy", "echo", "fable", "onyx", "nova", "shimmer"];
            if !VOICES.contains(&s) {
                return Err(format!(
                    "voice '{s}' is not a known OpenAI voice (alloy/echo/fable/onyx/nova/shimmer)"
                ));
            }
        }
        if let Some(m) = creds.get("model") {
            let s = m.as_str().ok_or("model must be a string")?;
            if !s.starts_with("tts-") {
                return Err("model should be a TTS model (tts-1, tts-1-hd)".into());
            }
        }
        Ok(())
    }

    fn vault_keys(&self, creds: &serde_json::Value) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(k) = creds.get("api_key").and_then(|v| v.as_str()) {
            out.push(("api_key".to_string(), k.to_string()));
        }
        out
    }

    fn env_exports(&self, creds: &serde_json::Value) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(k) = creds.get("api_key").and_then(|v| v.as_str()) {
            out.push(("OPENAI_API_KEY".to_string(), k.to_string()));
        }
        out
    }

    fn config_patch(&self, creds: &serde_json::Value) -> Vec<ConfigPatch> {
        let tts_path = vec!["tts".to_string()];
        let openai_path = vec!["tts".to_string(), "openai".to_string()];
        let voice = creds
            .get("voice")
            .and_then(|v| v.as_str())
            .unwrap_or("nova");
        let model = creds
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("tts-1");
        let format = creds
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("mp3");
        vec![
            ConfigPatch {
                path: tts_path.clone(),
                key: "enabled".into(),
                value: toml_edit::value(true),
            },
            ConfigPatch {
                path: tts_path,
                key: "provider".into(),
                value: toml_edit::value("openai"),
            },
            ConfigPatch {
                path: openai_path.clone(),
                key: "api_key_env".into(),
                value: toml_edit::value("OPENAI_API_KEY"),
            },
            ConfigPatch {
                path: openai_path.clone(),
                key: "voice".into(),
                value: toml_edit::value(voice),
            },
            ConfigPatch {
                path: openai_path.clone(),
                key: "model".into(),
                value: toml_edit::value(model),
            },
            ConfigPatch {
                path: openai_path,
                key: "format".into(),
                value: toml_edit::value(format),
            },
        ]
    }

    async fn test(&self, creds: &serde_json::Value) -> Result<String, String> {
        let key = creds
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or("api_key missing for live test")?;
        // Cheapest auth-checking call: GET /v1/models (no body, no quota
        // burn) returns 200 if the key is valid, 401 otherwise.
        let resp = reqwest::Client::new()
            .get("https://api.openai.com/v1/models")
            .bearer_auth(key)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("/v1/models HTTP error: {e}"))?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err("OpenAI rejected the key (401 Unauthorized)".into());
        }
        if !resp.status().is_success() {
            return Err(format!("/v1/models returned status {}", resp.status()));
        }
        Ok("OpenAI TTS reachable (key validated via /v1/models).".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_accepts_well_formed_creds() {
        let t = TtsOpenAi;
        let ok = json!({"api_key": "sk-FAKE_KEY_LONG_ENOUGH_xxx"});
        assert!(t.validate(&ok).is_ok());
    }

    #[test]
    fn validate_rejects_short_or_wrong_prefix_key() {
        let t = TtsOpenAi;
        assert!(t
            .validate(&json!({"api_key": "no_prefix_long_enough_xxx"}))
            .is_err());
        assert!(t.validate(&json!({"api_key": "sk-short"})).is_err());
        assert!(t.validate(&json!({})).is_err());
    }

    #[test]
    fn validate_rejects_unknown_voice() {
        let t = TtsOpenAi;
        let bad = json!({"api_key": "sk-FAKE_KEY_LONG_ENOUGH_xxx", "voice": "darth_vader"});
        assert!(t.validate(&bad).is_err());
    }

    #[test]
    fn validate_accepts_known_voice() {
        let t = TtsOpenAi;
        let ok = json!({"api_key": "sk-FAKE_KEY_LONG_ENOUGH_xxx", "voice": "nova"});
        assert!(t.validate(&ok).is_ok());
    }

    #[test]
    fn env_exports_returns_openai_api_key() {
        let t = TtsOpenAi;
        let creds = json!({"api_key": "sk-FAKE_KEY_LONG_ENOUGH_xxx"});
        let pairs = t.env_exports(&creds);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "OPENAI_API_KEY");
    }

    #[test]
    fn config_patch_uses_defaults_when_omitted() {
        let t = TtsOpenAi;
        let creds = json!({"api_key": "sk-FAKE_KEY_LONG_ENOUGH_xxx"});
        let patches = t.config_patch(&creds);
        let enabled = patches.iter().find(|p| p.key == "enabled").unwrap();
        assert_eq!(enabled.value.as_bool(), Some(true));
        let provider = patches.iter().find(|p| p.key == "provider").unwrap();
        assert_eq!(provider.value.as_str(), Some("openai"));
        let voice = patches.iter().find(|p| p.key == "voice").unwrap();
        assert_eq!(voice.value.as_str(), Some("nova"));
        let model = patches.iter().find(|p| p.key == "model").unwrap();
        assert_eq!(model.value.as_str(), Some("tts-1"));
    }
}

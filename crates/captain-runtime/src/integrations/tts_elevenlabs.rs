//! R.3.1 — ElevenLabs TTS auto-install adapter.
//!
//! Stores the API key in the vault, references it by env-var in
//! `[tts.elevenlabs]`, and pings `/v1/voices` to confirm the key works.

use super::{ConfigPatch, IntegrationSetup};

pub struct TtsElevenLabs;

#[async_trait::async_trait]
impl IntegrationSetup for TtsElevenLabs {
    fn name(&self) -> &str {
        "tts_elevenlabs"
    }

    fn description(&self) -> &str {
        "ElevenLabs Text-to-Speech engine. Requires API key (free tier available)."
    }

    fn validate(&self, creds: &serde_json::Value) -> Result<(), String> {
        let key = creds
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or("missing field: api_key (string)")?;
        if key.len() < 16 {
            return Err("api_key looks too short (expected ≥ 16 chars)".to_string());
        }
        if let Some(voice) = creds.get("voice_id") {
            let s = voice
                .as_str()
                .ok_or("voice_id must be a string when provided")?;
            if s.is_empty() {
                return Err("voice_id is empty".to_string());
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

    /// R.3.3 — TtsEngine reads `ELEVENLABS_API_KEY` via `std::env::var()`
    /// at every synthesize() call, so exporting it here makes the new key
    /// active without a daemon restart.
    fn env_exports(&self, creds: &serde_json::Value) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(k) = creds.get("api_key").and_then(|v| v.as_str()) {
            out.push(("ELEVENLABS_API_KEY".to_string(), k.to_string()));
        }
        out
    }

    fn config_patch(&self, creds: &serde_json::Value) -> Vec<ConfigPatch> {
        let tts_path = vec!["tts".to_string()];
        let elevenlabs_path = vec!["tts".to_string(), "elevenlabs".to_string()];
        let mut patches = vec![
            ConfigPatch {
                path: tts_path.clone(),
                key: "enabled".into(),
                value: toml_edit::value(true),
            },
            ConfigPatch {
                path: tts_path,
                key: "provider".into(),
                value: toml_edit::value("elevenlabs"),
            },
            ConfigPatch {
                path: elevenlabs_path.clone(),
                key: "api_key_env".into(),
                value: toml_edit::value("ELEVENLABS_API_KEY"),
            },
        ];
        if let Some(voice) = creds.get("voice_id").and_then(|v| v.as_str()) {
            patches.push(ConfigPatch {
                path: elevenlabs_path.clone(),
                key: "voice_id".into(),
                value: toml_edit::value(voice),
            });
        }
        let model = creds
            .get("model_id")
            .and_then(|v| v.as_str())
            .or_else(|| creds.get("model").and_then(|v| v.as_str()))
            .unwrap_or("eleven_turbo_v2_5");
        patches.push(ConfigPatch {
            path: elevenlabs_path,
            key: "model_id".into(),
            value: toml_edit::value(model),
        });
        patches
    }

    async fn test(&self, creds: &serde_json::Value) -> Result<String, String> {
        let key = creds
            .get("api_key")
            .and_then(|v| v.as_str())
            .ok_or("api_key missing for live test")?;
        let resp = reqwest::Client::new()
            .get("https://api.elevenlabs.io/v1/voices")
            .header("xi-api-key", key)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("/v1/voices HTTP error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("/v1/voices returned status {}", resp.status()));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("/v1/voices JSON parse: {e}"))?;
        let count = body
            .get("voices")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        Ok(format!("ElevenLabs reachable ({count} voices visible)."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_accepts_well_formed_creds() {
        let t = TtsElevenLabs;
        let ok = json!({"api_key": "sk_FAKE_KEY_BUT_LONG_ENOUGH_xxx"});
        assert!(t.validate(&ok).is_ok());
    }

    #[test]
    fn validate_rejects_short_key() {
        let t = TtsElevenLabs;
        let bad = json!({"api_key": "short"});
        assert!(t.validate(&bad).is_err());
    }

    #[test]
    fn validate_rejects_missing_key() {
        let t = TtsElevenLabs;
        assert!(t.validate(&json!({})).is_err());
    }

    #[test]
    fn config_patch_default_model_when_omitted() {
        let t = TtsElevenLabs;
        let creds = json!({"api_key": "sk_FAKE_KEY_BUT_LONG_ENOUGH_xxx"});
        let patches = t.config_patch(&creds);
        let enabled = patches.iter().find(|p| p.key == "enabled").unwrap();
        assert_eq!(enabled.value.as_bool(), Some(true));
        let provider = patches.iter().find(|p| p.key == "provider").unwrap();
        assert_eq!(provider.value.as_str(), Some("elevenlabs"));
        let model_p = patches.iter().find(|p| p.key == "model_id").unwrap();
        assert_eq!(model_p.value.as_str(), Some("eleven_turbo_v2_5"),);
    }

    #[test]
    fn vault_keys_returns_api_key() {
        let t = TtsElevenLabs;
        let creds = json!({"api_key": "sk_FAKE_KEY_BUT_LONG_ENOUGH_xxx"});
        let pairs = t.vault_keys(&creds);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "api_key");
    }

    #[test]
    fn env_exports_returns_elevenlabs_api_key() {
        let t = TtsElevenLabs;
        let creds = json!({"api_key": "sk_FAKE_KEY_BUT_LONG_ENOUGH_xxx"});
        let pairs = t.env_exports(&creds);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "ELEVENLABS_API_KEY");
        assert_eq!(pairs[0].1, "sk_FAKE_KEY_BUT_LONG_ENOUGH_xxx");
    }
}

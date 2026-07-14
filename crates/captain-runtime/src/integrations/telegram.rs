//! R.3.1 — Telegram auto-install adapter.
//!
//! Validates a Telegram bot configuration, returns the secret/value pairs
//! the orchestrator must encrypt-and-store, and produces the `toml_edit`
//! patches that land under `[channels.telegram]` in `~/.captain/config.toml`.
//!
//! Live `test()` is intentionally lightweight (calls `getMe`) so it can be
//! invoked from the agent loop without large blast radius. R.3.2 will flesh
//! out the long-poll wiring and hot-reload via the event bus.

use super::{ConfigPatch, IntegrationSetup};

pub struct Telegram;

#[async_trait::async_trait]
impl IntegrationSetup for Telegram {
    fn name(&self) -> &str {
        "telegram"
    }

    fn description(&self) -> &str {
        "Telegram bot channel (long-poll). Requires bot token from @BotFather."
    }

    fn validate(&self, creds: &serde_json::Value) -> Result<(), String> {
        let token = creds
            .get("bot_token")
            .and_then(|v| v.as_str())
            .ok_or("missing field: bot_token (string)")?;
        // Telegram bot tokens are formatted "<numeric_id>:<35+ char secret>".
        let (id, secret) = token
            .split_once(':')
            .ok_or("bot_token must be of the form <id>:<secret>")?;
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
            return Err("bot_token id segment must be numeric".to_string());
        }
        if secret.len() < 20 {
            return Err("bot_token secret segment looks too short".to_string());
        }

        let chat_id = creds
            .get("default_chat_id")
            .and_then(|v| v.as_str())
            .ok_or("missing field: default_chat_id (string)")?;
        if chat_id.is_empty() {
            return Err("default_chat_id is empty".to_string());
        }

        if let Some(users) = creds.get("allowed_users") {
            let arr = users
                .as_array()
                .ok_or("allowed_users must be an array of strings")?;
            for u in arr {
                let s = u.as_str().ok_or("allowed_users entries must be strings")?;
                if s.is_empty() || !s.chars().all(|c| c.is_ascii_digit()) {
                    return Err(format!("allowed_users entry '{s}' must be numeric"));
                }
            }
        }

        Ok(())
    }

    fn vault_keys(&self, creds: &serde_json::Value) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(t) = creds.get("bot_token").and_then(|v| v.as_str()) {
            out.push(("bot_token".to_string(), t.to_string()));
        }
        out
    }

    fn env_exports(&self, creds: &serde_json::Value) -> Vec<(String, String)> {
        let mut out = Vec::new();
        if let Some(t) = creds.get("bot_token").and_then(|v| v.as_str()) {
            out.push(("TELEGRAM_BOT_TOKEN".to_string(), t.to_string()));
        }
        out
    }

    fn config_patch(&self, creds: &serde_json::Value) -> Vec<ConfigPatch> {
        let mut patches = Vec::new();
        let path = vec!["channels".to_string(), "telegram".to_string()];

        // The token itself stays in the vault — config only references it
        // by env-var name to keep secrets off-disk in plaintext.
        patches.push(ConfigPatch {
            path: path.clone(),
            key: "bot_token_env".into(),
            value: toml_edit::value("TELEGRAM_BOT_TOKEN"),
        });

        if let Some(chat_id) = creds.get("default_chat_id").and_then(|v| v.as_str()) {
            patches.push(ConfigPatch {
                path: path.clone(),
                key: "default_chat_id".into(),
                value: toml_edit::value(chat_id),
            });
        }

        if let Some(users) = creds.get("allowed_users").and_then(|v| v.as_array()) {
            let mut arr = toml_edit::Array::new();
            for u in users {
                if let Some(s) = u.as_str() {
                    arr.push(s);
                }
            }
            patches.push(ConfigPatch {
                path: path.clone(),
                key: "allowed_users".into(),
                value: toml_edit::Item::Value(toml_edit::Value::Array(arr)),
            });
        }

        let poll_interval = creds
            .get("poll_interval")
            .and_then(|v| v.as_i64())
            .unwrap_or(30);
        patches.push(ConfigPatch {
            path,
            key: "poll_interval".into(),
            value: toml_edit::value(poll_interval),
        });

        patches
    }

    async fn test(&self, creds: &serde_json::Value) -> Result<String, String> {
        let token = creds
            .get("bot_token")
            .and_then(|v| v.as_str())
            .ok_or("bot_token missing for live test")?;
        let url = format!("https://api.telegram.org/bot{token}/getMe");
        let resp = reqwest::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| format!("getMe HTTP error: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("getMe returned status {}", resp.status()));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("getMe JSON parse: {e}"))?;
        let username = body
            .pointer("/result/username")
            .and_then(|v| v.as_str())
            .unwrap_or("<unknown>");
        Ok(format!("Telegram bot @{username} reachable."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validate_accepts_well_formed_creds() {
        let t = Telegram;
        let ok = json!({
            "bot_token": "1234567890:AABBccDDeeFFggHHiiJJkkLLmmNNooPP",
            "default_chat_id": "999",
            "allowed_users": ["111", "222"],
        });
        assert!(t.validate(&ok).is_ok());
    }

    #[test]
    fn validate_rejects_missing_bot_token() {
        let t = Telegram;
        let bad = json!({"default_chat_id": "1"});
        assert!(t.validate(&bad).is_err());
    }

    #[test]
    fn validate_rejects_malformed_bot_token() {
        let t = Telegram;
        let cases = [
            json!({"bot_token": "no_colon", "default_chat_id": "1"}),
            json!({"bot_token": "abc:secretthatislongenoughxxxxx", "default_chat_id": "1"}),
            json!({"bot_token": "123:short", "default_chat_id": "1"}),
        ];
        for c in &cases {
            assert!(t.validate(c).is_err(), "should reject: {c}");
        }
    }

    #[test]
    fn validate_rejects_non_numeric_allowed_users() {
        let t = Telegram;
        let bad = json!({
            "bot_token": "1:AABBccDDeeFFggHHiiJJkkLLmmNNooPP",
            "default_chat_id": "1",
            "allowed_users": ["alice"],
        });
        assert!(t.validate(&bad).is_err());
    }

    #[test]
    fn vault_keys_returns_only_bot_token() {
        let t = Telegram;
        let creds = json!({
            "bot_token": "1:AABBccDDeeFFggHHiiJJkkLLmmNNooPP",
            "default_chat_id": "1",
        });
        let pairs = t.vault_keys(&creds);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "bot_token");
        assert_eq!(pairs[0].1, "1:AABBccDDeeFFggHHiiJJkkLLmmNNooPP");
    }

    #[test]
    fn env_exports_returns_telegram_bot_token() {
        let t = Telegram;
        let creds = json!({
            "bot_token": "1:AABBccDDeeFFggHHiiJJkkLLmmNNooPP",
            "default_chat_id": "1",
        });
        let pairs = t.env_exports(&creds);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "TELEGRAM_BOT_TOKEN");
    }

    #[test]
    fn config_patch_emits_expected_keys() {
        let t = Telegram;
        let creds = json!({
            "bot_token": "1:AABBccDDeeFFggHHiiJJkkLLmmNNooPP",
            "default_chat_id": "777",
            "allowed_users": ["111"],
        });
        let patches = t.config_patch(&creds);
        let keys: Vec<&str> = patches.iter().map(|p| p.key.as_str()).collect();
        assert!(keys.contains(&"bot_token_env"));
        assert!(keys.contains(&"default_chat_id"));
        assert!(keys.contains(&"allowed_users"));
        assert!(keys.contains(&"poll_interval"));
        for p in &patches {
            assert_eq!(p.path, vec!["channels".to_string(), "telegram".to_string()]);
        }
    }
}

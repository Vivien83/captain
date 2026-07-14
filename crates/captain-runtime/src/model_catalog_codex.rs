use crate::model_catalog_codex_auth::{
    codex_chatgpt_account_id, codex_home, read_codex_credential, refresh_or_rotate_codex_credential,
};
use captain_types::model_catalog::{ModelCatalogEntry, ModelTier, CODEX_BASE_URL};
use std::collections::HashMap;
use std::io::Write;

const CODEX_UA: &str = "codex_cli_rs/0.0.0 (Captain Agent)";
const CODEX_ORIGINATOR: &str = "codex_cli_rs";
// The catalog gates forward-compatible models by client protocol semver.
// Captain implements the v1 Responses request shape; this is intentionally
// independent from Captain's own product version.
const CODEX_CATALOG_CLIENT_VERSION: &str = "1.0.0";

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct CodexModelsCache {
    #[serde(default)]
    models: Vec<CodexCachedModel>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct CodexCachedModel {
    slug: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    supported_in_api: Option<bool>,
    #[serde(default)]
    priority: Option<i64>,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    max_context_window: Option<u64>,
    #[serde(default)]
    input_modalities: Vec<String>,
}

pub(crate) fn apply_codex_models_cache(
    models: &mut Vec<ModelCatalogEntry>,
    aliases: &mut HashMap<String, String>,
) {
    let cached = codex_cached_model_entries();
    if cached.is_empty() {
        return;
    }

    models.retain(|m| m.provider != "codex");
    aliases.retain(|_, target| !target.starts_with("codex/"));
    models.extend(cached);
}

pub fn codex_cached_model_entries() -> Vec<ModelCatalogEntry> {
    let Some(path) = codex_home().map(|p| p.join("models_cache.json")) else {
        return Vec::new();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(mut cache) = serde_json::from_str::<CodexModelsCache>(&raw) else {
        return Vec::new();
    };

    cache.models.sort_by_key(|m| m.priority.unwrap_or(i64::MAX));

    let mut entries = Vec::new();
    for model in cache.models {
        if !codex_cached_model_is_usable(&model) {
            continue;
        }
        let id = format!("codex/{}", model.slug);
        let mut aliases = codex_aliases_for_slug(&model.slug);
        if entries.is_empty() {
            aliases.push("codex".to_string());
        }
        entries.push(ModelCatalogEntry {
            id,
            display_name: format!(
                "{} (Codex)",
                model.display_name.as_deref().unwrap_or(&model.slug)
            ),
            provider: "codex".to_string(),
            tier: infer_codex_model_tier(&model.slug, model.priority),
            context_window: model
                .max_context_window
                .or(model.context_window)
                .unwrap_or(272_000),
            // The ChatGPT/Codex backend currently rejects max_output_tokens.
            // Keep this metadata conservative and do not serialize it in the driver.
            max_output_tokens: 32_768,
            input_cost_per_m: 0.0,
            output_cost_per_m: 0.0,
            supports_tools: true,
            supports_vision: model.input_modalities.iter().any(|m| m == "image"),
            supports_streaming: true,
            aliases,
        });
    }
    entries
}

pub fn codex_cached_model_ids() -> Vec<String> {
    codex_cached_model_entries()
        .into_iter()
        .map(|m| m.id)
        .collect()
}

pub fn codex_model_choices() -> Vec<(String, String)> {
    let cached = codex_cached_model_entries()
        .into_iter()
        .map(|m| {
            let slug = m.id.strip_prefix("codex/").unwrap_or(&m.id).to_string();
            (slug, m.display_name)
        })
        .collect::<Vec<_>>();
    if cached.is_empty() {
        codex_static_model_choices()
    } else {
        cached
    }
}

fn codex_static_model_choices() -> Vec<(String, String)> {
    crate::model_catalog_models_codex::codex_static_model_choices()
}

fn codex_cached_model_is_usable(model: &CodexCachedModel) -> bool {
    !model.slug.trim().is_empty()
        && model.visibility.as_deref().unwrap_or("list") == "list"
        && model.supported_in_api.unwrap_or(true)
}

fn codex_aliases_for_slug(slug: &str) -> Vec<String> {
    let mut aliases = vec![format!("codex-{slug}")];
    if let Some(rest) = slug.strip_prefix("gpt-") {
        aliases.push(format!("codex-{rest}"));
        if let Some(base) = rest.strip_suffix("-codex") {
            aliases.push(format!("codex-{base}"));
        }
        if let Some(base) = rest.strip_suffix("-codex-spark") {
            aliases.push(format!("codex-{base}-spark"));
        }
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

fn infer_codex_model_tier(slug: &str, priority: Option<i64>) -> ModelTier {
    let s = slug.to_lowercase();
    if s.contains("spark") {
        ModelTier::Fast
    } else if s.contains("mini") {
        ModelTier::Smart
    } else if priority.unwrap_or(i64::MAX) <= 2 || s.contains("5.5") || s.contains("5.4") {
        ModelTier::Frontier
    } else {
        ModelTier::Balanced
    }
}

/// Refresh `~/.codex/models_cache.json` from the Codex backend.
///
/// The official Codex CLI uses this cache as the durable source of model
/// availability. Captain keeps the same file fresh after login while still
/// falling back to static entries when the network is unavailable.
pub async fn refresh_codex_models_cache() -> Result<usize, String> {
    let token = match read_codex_credential() {
        Some(token) => token,
        None => refresh_or_rotate_codex_credential("")
            .await
            .ok_or_else(|| {
                "No valid Codex OAuth token available; run `captain login codex`".to_string()
            })?,
    };
    refresh_codex_models_cache_with_token(&token, CODEX_BASE_URL).await
}

pub async fn refresh_codex_models_cache_with_token(
    access_token: &str,
    base_url: &str,
) -> Result<usize, String> {
    let codex_home =
        codex_home().ok_or_else(|| "Unable to resolve CODEX_HOME or ~/.codex".to_string())?;
    let url = codex_models_request_url(base_url)?;
    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("User-Agent", CODEX_UA)
        .header("originator", CODEX_ORIGINATOR)
        .header("Accept", "application/json");
    if let Some(account_id) = codex_chatgpt_account_id(access_token) {
        req = req.header("ChatGPT-Account-ID", account_id);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .and_then(|message| message.as_str())
                    .map(str::to_string)
            })
            .unwrap_or(body);
        let detail = captain_types::truncate_str(detail.trim(), 500);
        return Err(if detail.is_empty() {
            format!("Codex models endpoint returned {status}")
        } else {
            format!("Codex models endpoint returned {status}: {detail}")
        });
    }
    let mut raw: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let parsed = parse_codex_models_cache_value(&raw)?;
    let usable = parsed
        .models
        .iter()
        .filter(|m| codex_cached_model_is_usable(m))
        .count();
    if usable == 0 {
        return Err("Codex models endpoint returned no usable models".to_string());
    }

    if raw.get("models").is_none() {
        raw = serde_json::json!({ "models": parsed.models });
    }
    if let Some(obj) = raw.as_object_mut() {
        obj.insert(
            "fetched_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );
        obj.insert(
            "source".to_string(),
            serde_json::Value::String("captain".to_string()),
        );
    }
    std::fs::create_dir_all(&codex_home).map_err(|e| e.to_string())?;
    write_codex_models_cache(&codex_home, &raw)?;
    Ok(usable)
}

fn codex_models_request_url(base_url: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(&format!("{}/models", base_url.trim_end_matches('/')))
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("client_version", CODEX_CATALOG_CLIENT_VERSION);
    Ok(url)
}

fn write_codex_models_cache(
    codex_home: &std::path::Path,
    raw: &serde_json::Value,
) -> Result<(), String> {
    let serialized = serde_json::to_string_pretty(raw).map_err(|e| e.to_string())?;
    let mut pending = tempfile::Builder::new()
        .prefix(".models_cache.")
        .tempfile_in(codex_home)
        .map_err(|e| e.to_string())?;
    pending
        .write_all(serialized.as_bytes())
        .and_then(|_| pending.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    pending
        .persist(codex_home.join("models_cache.json"))
        .map_err(|e| e.error.to_string())?;
    Ok(())
}

fn parse_codex_models_cache_value(value: &serde_json::Value) -> Result<CodexModelsCache, String> {
    if value.get("models").is_some() {
        serde_json::from_value::<CodexModelsCache>(value.clone()).map_err(|e| e.to_string())
    } else if value.is_array() {
        serde_json::from_value::<Vec<CodexCachedModel>>(value.clone())
            .map(|models| CodexModelsCache { models })
            .map_err(|e| e.to_string())
    } else {
        Err("Codex models response has no `models` array".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_models_cache_parser_accepts_array_or_object() {
        let object = serde_json::json!({
            "models": [{
                "slug": "gpt-5.3-codex",
                "visibility": "list",
                "supported_in_api": true
            }]
        });
        let parsed = parse_codex_models_cache_value(&object).unwrap();
        assert_eq!(parsed.models[0].slug, "gpt-5.3-codex");

        let array = serde_json::json!([{
            "slug": "gpt-5.2",
            "visibility": "list",
            "supported_in_api": true
        }]);
        let parsed = parse_codex_models_cache_value(&array).unwrap();
        assert_eq!(parsed.models[0].slug, "gpt-5.2");
    }

    #[test]
    fn codex_models_request_pins_the_supported_catalog_protocol() {
        let url = codex_models_request_url("https://chatgpt.com/backend-api/codex").unwrap();

        assert_eq!(
            url.as_str(),
            "https://chatgpt.com/backend-api/codex/models?client_version=1.0.0"
        );
    }

    #[test]
    fn codex_models_cache_write_replaces_the_file_atomically() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("models_cache.json"), "stale").unwrap();
        let payload = serde_json::json!({
            "models": [{
                "slug": "gpt-5.6",
                "visibility": "list",
                "supported_in_api": true
            }]
        });

        write_codex_models_cache(home.path(), &payload).unwrap();

        let written: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join("models_cache.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(written["models"][0]["slug"], "gpt-5.6");
        assert_eq!(
            std::fs::read_dir(home.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".models_cache."))
                .count(),
            0
        );
    }
}

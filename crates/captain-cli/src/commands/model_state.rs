use crate::{cli_captain_home, daemon_client, daemon_json, find_daemon};

pub(crate) fn providers_array(body: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    body.as_array()
        .or_else(|| body.get("providers").and_then(|v| v.as_array()))
}

pub(crate) fn current_model_status_json() -> serde_json::Value {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let status = daemon_json(client.get(format!("{base}/api/status")).send());
        let config = read_cli_kernel_config();
        return serde_json::json!({
            "provider": status["default_provider"].as_str().unwrap_or("?"),
            "model": status["default_model"].as_str().unwrap_or("?"),
            "api_key_env": config.as_ref().map(|c| c.default_model.api_key_env.as_str()).unwrap_or(""),
            "source": "daemon",
            "fallbacks": config
                .as_ref()
                .map(fallbacks_json)
                .unwrap_or_default(),
        });
    }

    if let Some(config) = read_cli_kernel_config() {
        return serde_json::json!({
            "provider": config.default_model.provider,
            "model": config.default_model.model,
            "api_key_env": config.default_model.api_key_env,
            "source": "config",
            "fallbacks": fallbacks_json(&config),
        });
    }

    serde_json::json!({
        "provider": "?",
        "model": "?",
        "api_key_env": "",
        "source": "unknown",
        "fallbacks": [],
    })
}

fn read_cli_kernel_config() -> Option<captain_types::config::KernelConfig> {
    let path = cli_captain_home().join("config.toml");
    let content = std::fs::read_to_string(path).ok()?;
    toml::from_str(&content).ok()
}

pub(crate) fn fallbacks_json(
    config: &captain_types::config::KernelConfig,
) -> Vec<serde_json::Value> {
    config
        .fallback_providers
        .iter()
        .map(|fb| {
            serde_json::json!({
                "provider": &fb.provider,
                "model": &fb.model,
                "api_key_env": &fb.api_key_env,
                "base_url": &fb.base_url,
            })
        })
        .collect()
}

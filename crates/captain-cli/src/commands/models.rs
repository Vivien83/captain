use colored::Colorize;
use reqwest::blocking::Client;

use crate::{daemon_client, daemon_json, find_daemon, prompt_input, require_daemon, ui};

use super::model_state::{current_model_status_json, providers_array};

fn print_pretty_json(body: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(body).unwrap_or_default());
}

fn print_model_table_header() {
    println!("{:<40} {:<16} {:<8} CONTEXT", "MODEL", "PROVIDER", "TIER");
    println!("{}", "-".repeat(80));
}

fn print_provider_table_header(include_local: bool) {
    if include_local {
        println!(
            "{:<20} {:<12} {:<10} {:<8} BASE URL",
            "PROVIDER", "AUTH", "MODELS", "LOCAL"
        );
        println!("{}", "-".repeat(88));
    } else {
        println!(
            "{:<20} {:<12} {:<10} BASE URL",
            "PROVIDER", "AUTH", "MODELS"
        );
        println!("{}", "-".repeat(70));
    }
}

fn models_api_url(base: &str, provider_filter: Option<&str>) -> String {
    match provider_filter {
        Some(p) => format!("{base}/api/models?provider={p}"),
        None => format!("{base}/api/models"),
    }
}

pub(crate) fn cmd_models_current(json: bool) {
    let body = current_model_status_json();
    if json {
        print_pretty_json(&body);
        return;
    }

    ui::section("Current Model");
    ui::blank();
    ui::kv("Provider", body["provider"].as_str().unwrap_or("?"));
    ui::kv("Model", body["model"].as_str().unwrap_or("?"));
    ui::kv("Source", body["source"].as_str().unwrap_or("?"));
    ui::kv(
        "API key env",
        body["api_key_env"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("-"),
    );
    ui::kv(
        "Fallbacks",
        &body["fallbacks"]
            .as_array()
            .map(|items| items.len())
            .unwrap_or(0)
            .to_string(),
    );
    if let Some(fallbacks) = body["fallbacks"].as_array() {
        for (idx, fb) in fallbacks.iter().enumerate() {
            println!(
                "    {}. {}/{}",
                idx + 1,
                fb["provider"].as_str().unwrap_or("?"),
                fb["model"].as_str().unwrap_or("?")
            );
        }
    }
}

pub(crate) fn cmd_models_list(provider_filter: Option<&str>, json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let url = models_api_url(&base, provider_filter);
        let body = daemon_json(client.get(&url).send());
        if json {
            print_pretty_json(&body);
            return;
        }
        print_daemon_models(&body);
    } else {
        print_local_models(provider_filter, json);
    }
}

fn print_daemon_models(body: &serde_json::Value) {
    let Some(arr) = body.as_array() else {
        print_pretty_json(body);
        return;
    };
    if arr.is_empty() {
        println!("No models found.");
        return;
    }
    print_model_table_header();
    for m in arr {
        println!(
            "{:<40} {:<16} {:<8} {}",
            m["id"].as_str().unwrap_or("?"),
            m["provider"].as_str().unwrap_or("?"),
            m["tier"].as_str().unwrap_or("?"),
            m["context_window"].as_u64().unwrap_or(0),
        );
    }
}

fn print_local_models(provider_filter: Option<&str>, json: bool) {
    let catalog = captain_runtime::model_catalog::ModelCatalog::new();
    let models = catalog.list_models();
    if json {
        print_pretty_json(&local_models_json(models, provider_filter));
        return;
    }
    if models.is_empty() {
        println!("No models in catalog.");
        return;
    }
    print_model_table_header();
    for m in models {
        if provider_filter.is_some_and(|p| m.provider.as_str() != p) {
            continue;
        }
        println!(
            "{:<40} {:<16} {:<8} {}",
            m.id,
            m.provider,
            format!("{:?}", m.tier),
            m.context_window,
        );
    }
}

fn local_models_json(
    models: &[captain_types::model_catalog::ModelCatalogEntry],
    provider_filter: Option<&str>,
) -> serde_json::Value {
    serde_json::json!(models
        .iter()
        .filter(|m| provider_filter.is_none_or(|p| m.provider.as_str() == p))
        .map(|m| {
            serde_json::json!({
                "id": m.id.as_str(),
                "provider": m.provider.as_str(),
                "tier": format!("{:?}", m.tier),
                "context_window": m.context_window,
            })
        })
        .collect::<Vec<_>>())
}

pub(crate) fn cmd_models_aliases(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/models/aliases")).send());
        if json {
            print_pretty_json(&body);
            return;
        }
        if let Some(obj) = body.as_object() {
            println!("{:<30} RESOLVES TO", "ALIAS");
            println!("{}", "-".repeat(60));
            for (alias, target) in obj {
                println!("{:<30} {}", alias, target.as_str().unwrap_or("?"));
            }
        } else {
            print_pretty_json(&body);
        }
    } else {
        let catalog = captain_runtime::model_catalog::ModelCatalog::new();
        let aliases = catalog.list_aliases();
        if json {
            let obj: serde_json::Map<String, serde_json::Value> = aliases
                .iter()
                .map(|(a, t)| (a.to_string(), serde_json::Value::String(t.to_string())))
                .collect();
            print_pretty_json(&serde_json::Value::Object(obj));
            return;
        }
        println!("{:<30} RESOLVES TO", "ALIAS");
        println!("{}", "-".repeat(60));
        for (alias, target) in aliases {
            println!("{:<30} {}", alias, target);
        }
    }
}

pub(crate) fn cmd_models_providers(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/providers")).send());
        if json {
            print_pretty_json(&body);
            return;
        }
        print_daemon_providers(&body);
    } else {
        print_local_providers(json);
    }
}

fn print_daemon_providers(body: &serde_json::Value) {
    let Some(arr) = providers_array(body) else {
        print_pretty_json(body);
        return;
    };
    print_provider_table_header(true);
    for p in arr {
        println!(
            "{:<20} {:<12} {:<10} {:<8} {}",
            p["id"].as_str().unwrap_or("?"),
            p["auth_status"].as_str().unwrap_or("?"),
            p["model_count"].as_u64().unwrap_or(0),
            if p["is_local"].as_bool().unwrap_or(false) {
                "yes"
            } else {
                "no"
            },
            p["base_url"].as_str().unwrap_or(""),
        );
    }
}

fn print_local_providers(json: bool) {
    let catalog = captain_runtime::model_catalog::ModelCatalog::new();
    let providers = catalog.list_providers();
    if json {
        print_pretty_json(&local_providers_json(providers));
        return;
    }
    print_provider_table_header(false);
    for p in providers {
        println!(
            "{:<20} {:<12} {:<10} {}",
            p.id,
            format!("{:?}", p.auth_status),
            p.model_count,
            p.base_url,
        );
    }
}

fn local_providers_json(
    providers: &[captain_types::model_catalog::ProviderInfo],
) -> serde_json::Value {
    serde_json::json!(providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id.as_str(),
                "auth_status": format!("{:?}", p.auth_status),
                "model_count": p.model_count,
                "base_url": p.base_url.as_str(),
            })
        })
        .collect::<Vec<_>>())
}

pub(crate) fn cmd_models_test(provider: Option<&str>, json: bool) {
    let base = require_daemon("models test");
    let provider = provider.map(str::to_string).unwrap_or_else(|| {
        current_model_status_json()["provider"]
            .as_str()
            .unwrap_or("codex")
            .to_string()
    });
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/providers/{provider}/test"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    let status = body["status"].as_str().unwrap_or("error");
    if status == "ok" {
        ui::success(&format!(
            "{} OK ({} ms)",
            provider,
            body["latency_ms"].as_u64().unwrap_or(0)
        ));
    } else {
        ui::error(&format!(
            "{} failed: {}",
            provider,
            body["error"].as_str().unwrap_or("unknown error")
        ));
        ui::hint(&format!(
            "Run `captain auth status` or `captain auth login {provider}`"
        ));
        std::process::exit(1);
    }
}

pub(crate) fn cmd_models_set(model: Option<String>) {
    let model = match model {
        Some(m) => m,
        None => pick_model(),
    };
    let base = require_daemon("models set");
    let client = daemon_client();

    if let Some(agent_id) = principal_agent_id(&client, &base) {
        set_model_via_agent_switch(&client, &base, &agent_id, &model);
        return;
    }

    set_model_via_config(&client, &base, &model);
}

fn principal_agent_id(client: &Client, base: &str) -> Option<String> {
    let agents = daemon_json(client.get(format!("{base}/api/agents")).send());
    principal_agent_id_from_agents(&agents)
}

fn principal_agent_id_from_agents(agents: &serde_json::Value) -> Option<String> {
    agents.as_array().and_then(|items| {
        items
            .iter()
            .find(|a| a["name"].as_str().is_some_and(|name| name == "captain"))
            .or_else(|| items.first())
            .and_then(|a| a["id"].as_str())
            .map(str::to_string)
    })
}

fn set_model_via_agent_switch(client: &Client, base: &str, agent_id: &str, model: &str) {
    let plan = plan_model_switch(client, base, agent_id, model);
    if model_switch_plan_refused(&plan) {
        return;
    }

    let target = model_switch_target(&plan, model);
    let apply = apply_model_switch(client, base, agent_id, &target);
    if let Some(error) = apply.get("error").and_then(|v| v.as_str()) {
        ui::error(&format!("Failed to apply model switch: {error}"));
    } else {
        ui::success(&format!(
            "Default model set to: {}/{} ({})",
            target.provider, target.model, target.strategy
        ));
    }
}

fn plan_model_switch(
    client: &Client,
    base: &str,
    agent_id: &str,
    model: &str,
) -> serde_json::Value {
    daemon_json(
        client
            .post(format!("{base}/api/agents/{agent_id}/model-switch/plan"))
            .json(&serde_json::json!({"model": model}))
            .send(),
    )
}

fn model_switch_plan_refused(plan: &serde_json::Value) -> bool {
    if let Some(error) = plan.get("error").and_then(|v| v.as_str()) {
        ui::error(&format!("Failed to plan model switch: {error}"));
        return true;
    }
    if plan
        .get("can_apply")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return false;
    }

    ui::error(&format!(
        "Failed to set model: {}",
        model_switch_blocking_issues(plan)
    ));
    true
}

fn model_switch_blocking_issues(plan: &serde_json::Value) -> String {
    plan.get("blocking_issues")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "preflight refused the switch".to_string())
}

struct ModelSwitchTarget {
    provider: String,
    model: String,
    strategy: String,
}

fn model_switch_target(plan: &serde_json::Value, requested_model: &str) -> ModelSwitchTarget {
    ModelSwitchTarget {
        provider: plan["target_provider"].as_str().unwrap_or("").to_string(),
        model: plan["target_model"]
            .as_str()
            .unwrap_or(requested_model)
            .to_string(),
        strategy: model_switch_strategy(plan).to_string(),
    }
}

fn model_switch_strategy(plan: &serde_json::Value) -> &str {
    if plan["session_strategy_required"].as_bool().unwrap_or(false) {
        "new_session"
    } else {
        plan["recommended_session_strategy"]
            .as_str()
            .unwrap_or("new_session")
    }
}

fn apply_model_switch(
    client: &Client,
    base: &str,
    agent_id: &str,
    target: &ModelSwitchTarget,
) -> serde_json::Value {
    daemon_json(
        client
            .post(format!("{base}/api/agents/{agent_id}/model-switch/apply"))
            .json(&serde_json::json!({
                "provider": target.provider.as_str(),
                "model": target.model.as_str(),
                "session_strategy": target.strategy.as_str()
            }))
            .send(),
    )
}

fn set_model_via_config(client: &Client, base: &str, model: &str) {
    let catalog = captain_runtime::model_catalog::ModelCatalog::new();
    let Some(entry) = catalog.find_model(model) else {
        ui::error(&format!("Unknown model: {model}"));
        return;
    };
    let provider_body = daemon_json(
        client
            .post(format!("{base}/api/config/set"))
            .json(&config_set_body(
                "default_model.provider",
                entry.provider.as_str(),
            ))
            .send(),
    );
    let model_body = daemon_json(
        client
            .post(format!("{base}/api/config/set"))
            .json(&config_set_body("default_model.model", entry.id.as_str()))
            .send(),
    );
    if let Some(error) = config_set_error(&provider_body, &model_body) {
        ui::error(&format!("Failed to set model: {error}"));
    } else {
        ui::success(&format!(
            "Default model set to: {}/{}",
            entry.provider, entry.id
        ));
    }
}

fn config_set_body(path: &str, value: &str) -> serde_json::Value {
    serde_json::json!({"path": path, "value": value})
}

fn config_set_error<'a>(
    provider_body: &'a serde_json::Value,
    model_body: &'a serde_json::Value,
) -> Option<&'a str> {
    provider_body["error"]
        .as_str()
        .or_else(|| model_body["error"].as_str())
}

fn pick_model() -> String {
    let catalog = captain_runtime::model_catalog::ModelCatalog::new();
    let models = catalog.list_models();

    if models.is_empty() {
        ui::error("No models in catalog.");
        std::process::exit(1);
    }

    let mut by_provider: std::collections::BTreeMap<
        String,
        Vec<&captain_types::model_catalog::ModelCatalogEntry>,
    > = std::collections::BTreeMap::new();
    for m in models {
        by_provider.entry(m.provider.clone()).or_default().push(m);
    }

    ui::section("Select a model");
    ui::blank();

    let mut numbered: Vec<&str> = Vec::new();
    let mut idx = 1;
    for (provider, provider_models) in &by_provider {
        println!("  {}:", provider.bold());
        for m in provider_models {
            println!("    {idx:>3}. {:<36} {:?}", m.id, m.tier);
            numbered.push(&m.id);
            idx += 1;
        }
    }
    ui::blank();

    loop {
        let input = prompt_input("  Enter number or model ID: ");
        if input.is_empty() {
            continue;
        }
        if let Ok(n) = input.parse::<usize>() {
            if n >= 1 && n <= numbered.len() {
                return numbered[n - 1].to_string();
            }
            ui::error(&format!("Number out of range (1-{})", numbered.len()));
            continue;
        }
        if models.iter().any(|m| m.id == input) {
            return input;
        }
        if catalog.resolve_alias(&input).is_some() {
            return input;
        }
        return input;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn models_api_url_applies_provider_filter() {
        assert_eq!(models_api_url("http://d", None), "http://d/api/models");
        assert_eq!(
            models_api_url("http://d", Some("codex")),
            "http://d/api/models?provider=codex"
        );
    }

    #[test]
    fn principal_agent_id_prefers_captain_then_first_agent() {
        let agents = json!([
            {"id": "worker", "name": "worker"},
            {"id": "captain-id", "name": "captain"}
        ]);
        assert_eq!(
            principal_agent_id_from_agents(&agents),
            Some("captain-id".to_string())
        );

        let fallback = json!([{"id": "first", "name": "worker"}]);
        assert_eq!(
            principal_agent_id_from_agents(&fallback),
            Some("first".to_string())
        );
    }

    #[test]
    fn model_switch_blocking_issues_join_or_default() {
        assert_eq!(
            model_switch_blocking_issues(&json!({"blocking_issues": ["auth", "tools"]})),
            "auth; tools"
        );
        assert_eq!(
            model_switch_blocking_issues(&json!({"blocking_issues": []})),
            "preflight refused the switch"
        );
    }

    #[test]
    fn model_switch_target_preserves_required_strategy_contract() {
        let required = model_switch_target(
            &json!({
                "target_provider": "codex",
                "target_model": "gpt-5.5",
                "session_strategy_required": true,
                "recommended_session_strategy": "compact_session"
            }),
            "fallback",
        );
        assert_eq!(required.provider, "codex");
        assert_eq!(required.model, "gpt-5.5");
        assert_eq!(required.strategy, "new_session");

        let recommended = model_switch_target(
            &json!({
                "target_provider": "openai",
                "recommended_session_strategy": "compact_session"
            }),
            "gpt-4.1",
        );
        assert_eq!(recommended.model, "gpt-4.1");
        assert_eq!(recommended.strategy, "compact_session");
    }

    #[test]
    fn config_set_error_prefers_provider_error() {
        assert_eq!(
            config_set_error(&json!({"error": "provider"}), &json!({"error": "model"})),
            Some("provider")
        );
        assert_eq!(
            config_set_error(&json!({}), &json!({"error": "model"})),
            Some("model")
        );
        assert_eq!(config_set_error(&json!({}), &json!({})), None);
    }
}

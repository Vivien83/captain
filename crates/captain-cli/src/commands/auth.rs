use crate::{daemon_client, daemon_json, find_daemon, ui};

use super::config::cmd_config_set_key;
use super::model_state::{current_model_status_json, providers_array};

pub(crate) use super::auth_codex::cmd_login_codex;

pub(crate) fn cmd_auth_status(json: bool) {
    let body = auth_status_json(None);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }

    ui::section("Auth Status");
    ui::blank();
    ui::kv(
        "Current",
        &format!(
            "{}/{}",
            body["current"]["provider"].as_str().unwrap_or("?"),
            body["current"]["model"].as_str().unwrap_or("?")
        ),
    );
    ui::kv(
        "Auth",
        body["current_provider"]["auth_status"]
            .as_str()
            .unwrap_or("?"),
    );
    ui::kv(
        "API key env",
        body["current_provider"]["api_key_env"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("-"),
    );
    ui::kv(
        "Configured",
        &body["configured_provider_count"]
            .as_u64()
            .unwrap_or(0)
            .to_string(),
    );
    ui::kv(
        "Missing",
        &body["missing_provider_count"]
            .as_u64()
            .unwrap_or(0)
            .to_string(),
    );
    if let Some(err) = body["codex_readiness_error"].as_str() {
        ui::kv_warn("Codex", err);
    } else {
        ui::kv_ok("Codex", "ready");
    }
    if let Some(fallbacks) = body["current"]["fallbacks"].as_array() {
        if !fallbacks.is_empty() {
            ui::blank();
            ui::section("Fallbacks");
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
}

pub(crate) fn cmd_auth_doctor(json: bool, test: bool) {
    let body = auth_status_json(auth_doctor_live_test(test));
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }

    print_auth_doctor_report(&body);
}

fn auth_doctor_live_test(test: bool) -> Option<serde_json::Value> {
    if !test {
        return None;
    }
    let provider = current_model_status_json()["provider"]
        .as_str()
        .unwrap_or("codex")
        .to_string();
    find_daemon().map(|base| {
        let client = daemon_client();
        daemon_json(
            client
                .post(format!("{base}/api/providers/{provider}/test"))
                .send(),
        )
    })
}

fn print_auth_doctor_report(body: &serde_json::Value) {
    ui::step("Captain Auth Doctor");
    println!();
    print_auth_doctor_current_provider(body);
    print_auth_doctor_counts(body);
    print_auth_doctor_live_test(body);
}

fn print_auth_doctor_current_provider(body: &serde_json::Value) {
    let current_provider = body["current"]["provider"].as_str().unwrap_or("?");
    let current_model = body["current"]["model"].as_str().unwrap_or("?");
    let auth_status = body["current_provider"]["auth_status"]
        .as_str()
        .unwrap_or("?");
    if provider_auth_ready(auth_status) {
        ui::check_ok(&format!(
            "Current provider configured: {current_provider}/{current_model}"
        ));
    } else {
        ui::check_fail(&format!(
            "Current provider missing credentials: {current_provider}/{current_model}"
        ));
        ui::hint(&format!("Run `captain auth login {current_provider}`"));
    }

    if let Some(err) = body["codex_readiness_error"].as_str() {
        ui::check_warn(&format!("Codex OAuth: {err}"));
    } else {
        ui::check_ok("Codex OAuth ready");
    }
}

fn print_auth_doctor_counts(body: &serde_json::Value) {
    ui::check_ok(&format!(
        "Configured providers: {}",
        body["configured_provider_count"].as_u64().unwrap_or(0)
    ));
    ui::check_ok(&format!(
        "Fallback providers: {}",
        body["current"]["fallbacks"]
            .as_array()
            .map(|items| items.len())
            .unwrap_or(0)
    ));
}

fn print_auth_doctor_live_test(body: &serde_json::Value) {
    if let Some(test_body) = body.get("live_test") {
        if live_test_ok(test_body) {
            ui::check_ok(&format!(
                "Live provider test OK ({} ms)",
                test_body["latency_ms"].as_u64().unwrap_or(0)
            ));
        } else {
            ui::check_fail(&format!(
                "Live provider test failed: {}",
                test_body["error"].as_str().unwrap_or("unknown error")
            ));
        }
    } else {
        ui::hint("Run `captain auth doctor --test` for a live provider call");
    }
}

fn provider_auth_ready(auth_status: &str) -> bool {
    auth_status == "configured" || auth_status == "not_required"
}

fn live_test_ok(test_body: &serde_json::Value) -> bool {
    test_body["status"].as_str() == Some("ok")
}

pub(crate) fn cmd_auth_login(provider: &str) {
    if provider.eq_ignore_ascii_case("codex") {
        cmd_login_codex(false);
    } else {
        cmd_config_set_key(provider);
    }
}

fn auth_status_json(live_test: Option<serde_json::Value>) -> serde_json::Value {
    let current = current_model_status_json();
    let providers = auth_status_providers();
    let current_provider_id = current["provider"].as_str().unwrap_or("");
    let current_provider = auth_status_current_provider(&providers, current_provider_id);
    let (configured_count, missing_count) = provider_auth_counts(&providers);
    let mut body = serde_json::json!({
        "current": current,
        "current_provider": current_provider,
        "configured_provider_count": configured_count,
        "missing_provider_count": missing_count,
        "providers": providers,
        "codex_readiness_error": captain_runtime::model_catalog::codex_oauth_readiness_error(),
    });
    if let Some(test) = live_test {
        body["live_test"] = test;
    }
    body
}

fn auth_status_providers() -> Vec<serde_json::Value> {
    let providers_body = auth_status_providers_body();
    providers_array(&providers_body)
        .cloned()
        .unwrap_or_default()
}

fn auth_status_providers_body() -> serde_json::Value {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        return daemon_json(client.get(format!("{base}/api/providers")).send());
    }
    let catalog = captain_runtime::model_catalog::ModelCatalog::new();
    let providers: Vec<serde_json::Value> = catalog
        .list_providers()
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": &p.id,
                "display_name": &p.display_name,
                "auth_status": format!("{:?}", p.auth_status).to_ascii_lowercase(),
                "api_key_env": &p.api_key_env,
                "key_required": p.key_required,
                "model_count": p.model_count,
                "base_url": &p.base_url,
            })
        })
        .collect();
    serde_json::json!({ "providers": providers, "total": providers.len() })
}

fn auth_status_current_provider(
    providers: &[serde_json::Value],
    current_provider_id: &str,
) -> serde_json::Value {
    providers
        .iter()
        .find(|p| p["id"].as_str() == Some(current_provider_id))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"id": current_provider_id, "auth_status": "unknown"}))
}

fn provider_auth_counts(providers: &[serde_json::Value]) -> (usize, usize) {
    let configured_count = providers
        .iter()
        .filter(|p| {
            p["auth_status"]
                .as_str()
                .map(provider_auth_ready)
                .unwrap_or(false)
        })
        .count();
    let missing_count = providers
        .iter()
        .filter(|p| p["auth_status"].as_str() == Some("missing"))
        .count();
    (configured_count, missing_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_auth_ready_accepts_configured_or_not_required() {
        assert!(provider_auth_ready("configured"));
        assert!(provider_auth_ready("not_required"));
        assert!(!provider_auth_ready("missing"));
        assert!(!provider_auth_ready("unknown"));
    }

    #[test]
    fn live_test_ok_requires_ok_status() {
        assert!(live_test_ok(&serde_json::json!({"status": "ok"})));
        assert!(!live_test_ok(&serde_json::json!({"status": "error"})));
        assert!(!live_test_ok(&serde_json::json!({})));
    }

    #[test]
    fn provider_counts_and_current_provider_are_stable() {
        let providers = vec![
            serde_json::json!({"id": "codex", "auth_status": "configured"}),
            serde_json::json!({"id": "ollama", "auth_status": "not_required"}),
            serde_json::json!({"id": "openai", "auth_status": "missing"}),
        ];
        assert_eq!(provider_auth_counts(&providers), (2, 1));
        assert_eq!(
            auth_status_current_provider(&providers, "codex")["auth_status"],
            "configured"
        );
        assert_eq!(
            auth_status_current_provider(&providers, "unknown")["auth_status"],
            "unknown"
        );
    }
}

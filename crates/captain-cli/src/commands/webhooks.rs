use crate::{daemon_client, daemon_json, require_daemon, ui};

pub(crate) fn cmd_webhooks_list(json: bool) {
    let base = require_daemon("webhooks list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/triggers")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No webhooks configured.");
            return;
        }
        println!("{:<38} {:<16} URL", "ID", "AGENT");
        println!("{}", "-".repeat(80));
        for w in arr {
            println!(
                "{:<38} {:<16} {}",
                w["id"].as_str().unwrap_or("?"),
                w["agent_id"].as_str().unwrap_or("?"),
                w["url"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_webhooks_create(agent: &str, url: &str) {
    let base = require_daemon("webhooks create");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/triggers"))
            .json(&serde_json::json!({
                "agent_id": agent,
                "pattern": {"webhook": {"url": url}},
                "prompt_template": "Webhook event: {{event}}",
            }))
            .send(),
    );
    if let Some(id) = body["id"].as_str() {
        ui::success(&format!("Webhook created: {id}"));
    } else {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    }
}

pub(crate) fn cmd_webhooks_delete(id: &str) {
    let base = require_daemon("webhooks delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/triggers/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Webhook {id} deleted."));
    }
}

pub(crate) fn cmd_webhooks_test(id: &str) {
    let base = require_daemon("webhooks test");
    let client = daemon_client();
    let body = daemon_json(client.post(format!("{base}/api/triggers/{id}/test")).send());
    if body["success"].as_bool().unwrap_or(false) {
        ui::success(&format!("Webhook {id} test payload sent successfully."));
    } else {
        ui::error(&format!(
            "Webhook test failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    }
}

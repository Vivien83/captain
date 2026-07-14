use crate::{daemon_client, daemon_json, require_daemon, ui};

pub(crate) fn cmd_cron_list(json: bool) {
    let base = require_daemon("cron list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/cron/jobs")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No scheduled jobs.");
            return;
        }
        println!(
            "{:<38} {:<16} {:<20} {:<8} PROMPT",
            "ID", "AGENT", "SCHEDULE", "ENABLED"
        );
        println!("{}", "-".repeat(100));
        for j in arr {
            println!(
                "{:<38} {:<16} {:<20} {:<8} {}",
                j["id"].as_str().unwrap_or("?"),
                j["agent_id"].as_str().unwrap_or("?"),
                j["cron_expr"].as_str().unwrap_or("?"),
                if j["enabled"].as_bool().unwrap_or(false) {
                    "yes"
                } else {
                    "no"
                },
                j["prompt"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(40)
                    .collect::<String>(),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_cron_create(agent: &str, spec: &str, prompt: &str, explicit_name: Option<&str>) {
    let base = require_daemon("cron create");
    let client = daemon_client();
    let name = explicit_name.map(str::to_string).unwrap_or_else(|| {
        let short_prompt: String = prompt
            .split_whitespace()
            .take(4)
            .collect::<Vec<_>>()
            .join("-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(64)
            .collect();
        format!(
            "{}-{}",
            agent,
            if short_prompt.is_empty() {
                "job"
            } else {
                &short_prompt
            }
        )
    });

    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs"))
            .json(&serde_json::json!({
                "agent_id": agent,
                "name": name,
                "schedule": {
                    "kind": "cron",
                    "expr": spec
                },
                "action": {
                    "kind": "agent_turn",
                    "message": prompt
                }
            }))
            .send(),
    );
    if let Some(id) = body["id"].as_str() {
        ui::success(&format!("Cron job created: {id}"));
    } else {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    }
}

pub(crate) fn cmd_cron_delete(id: &str) {
    let base = require_daemon("cron delete");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/cron/jobs/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Cron job {id} deleted."));
    }
}

pub(crate) fn cmd_cron_toggle(id: &str, enable: bool) {
    let base = require_daemon("cron");
    let client = daemon_client();
    let endpoint = if enable { "enable" } else { "disable" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/cron/jobs/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Cron job {id} {endpoint}d."));
    }
}

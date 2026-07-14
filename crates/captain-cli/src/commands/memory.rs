use crate::{daemon_client, daemon_json, require_daemon, ui};

pub(crate) fn cmd_memory_list(agent: &str, json: bool) {
    let base = require_daemon("memory list");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No memory entries for agent '{agent}'.");
            return;
        }
        println!("{:<30} VALUE", "KEY");
        println!("{}", "-".repeat(60));
        for kv in arr {
            println!(
                "{:<30} {}",
                kv["key"].as_str().unwrap_or("?"),
                kv["value"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(50)
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

pub(crate) fn cmd_memory_get(agent: &str, key: &str, json: bool) {
    let base = require_daemon("memory get");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(val) = body["value"].as_str() {
        println!("{val}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_memory_set(agent: &str, key: &str, value: &str) {
    let base = require_daemon("memory set");
    let client = daemon_client();
    let body = daemon_json(
        client
            .put(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .json(&serde_json::json!({"value": value}))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Set {key} for agent '{agent}'."));
    }
}

pub(crate) fn cmd_memory_delete(agent: &str, key: &str) {
    let base = require_daemon("memory delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/memory/agents/{agent}/kv/{key}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Deleted key '{key}' for agent '{agent}'."));
    }
}

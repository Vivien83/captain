use crate::{daemon_client, daemon_json, require_daemon, ui};

pub(crate) fn cmd_hand_install(path: &str) {
    let base = require_daemon("hand install");
    let dir = std::path::Path::new(path);
    let toml_path = dir.join("HAND.toml");
    let skill_path = dir.join("SKILL.md");

    if !toml_path.exists() {
        eprintln!(
            "Error: No HAND.toml found in {}",
            dir.canonicalize()
                .unwrap_or_else(|_| dir.to_path_buf())
                .display()
        );
        std::process::exit(1);
    }

    let toml_content = std::fs::read_to_string(&toml_path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {e}", toml_path.display());
        std::process::exit(1);
    });
    let skill_content = std::fs::read_to_string(&skill_path).unwrap_or_default();

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/install"))
            .json(&serde_json::json!({
                "toml_content": toml_content,
                "skill_content": skill_content,
            }))
            .send(),
    );

    if let Some(err) = body.get("error").and_then(|v| v.as_str()) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }

    println!(
        "Installed hand: {} ({})",
        body["name"].as_str().unwrap_or("?"),
        body["id"].as_str().unwrap_or("?"),
    );
    println!(
        "Use `captain hand activate {}` to start it.",
        body["id"].as_str().unwrap_or("?")
    );
}

pub(crate) fn cmd_hand_list() {
    let base = require_daemon("hand list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands")).send());
    let arr_val = if let Some(arr) = body.get("hands").and_then(|v| v.as_array()) {
        arr.clone()
    } else if let Some(arr) = body.as_array() {
        arr.clone()
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    };

    if arr_val.is_empty() {
        println!("No hands available.");
        return;
    }
    println!("{:<14} {:<20} {:<10} DESCRIPTION", "ID", "NAME", "CATEGORY");
    println!("{}", "-".repeat(72));
    for h in arr_val {
        println!(
            "{:<14} {:<20} {:<10} {}",
            h["id"].as_str().unwrap_or("?"),
            h["name"].as_str().unwrap_or("?"),
            h["category"].as_str().unwrap_or("?"),
            h["description"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(40)
                .collect::<String>(),
        );
    }
    println!("\nUse `captain hand activate <id>` to activate a hand.");
}

pub(crate) fn cmd_hand_active() {
    let base = require_daemon("hand active");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands/active")).send());
    let arr = body
        .get("instances")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default();
    if arr.is_empty() {
        println!("No active hands.");
        return;
    }
    println!("{:<38} {:<14} {:<10} AGENT", "INSTANCE", "HAND", "STATUS");
    println!("{}", "-".repeat(72));
    for i in &arr {
        println!(
            "{:<38} {:<14} {:<10} {}",
            i["instance_id"].as_str().unwrap_or("?"),
            i["hand_id"].as_str().unwrap_or("?"),
            i["status"].as_str().unwrap_or("?"),
            i["agent_name"].as_str().unwrap_or("?"),
        );
    }
}

pub(crate) fn cmd_hand_activate(id: &str) {
    let base = require_daemon("hand activate");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/activate"))
            .header("content-type", "application/json")
            .body("{}")
            .send(),
    );
    if body.get("instance_id").is_some() {
        println!(
            "Hand '{}' activated (instance: {}, agent: {})",
            id,
            body["instance_id"].as_str().unwrap_or("?"),
            body["agent_name"].as_str().unwrap_or("?"),
        );
    } else {
        eprintln!(
            "Failed to activate hand '{}': {}",
            id,
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_hand_deactivate(id: &str) {
    let base = require_daemon("hand deactivate");
    let client = daemon_client();
    let active = daemon_json(client.get(format!("{base}/api/hands/active")).send());
    let arr = active
        .get("instances")
        .and_then(|v| v.as_array())
        .or_else(|| active.as_array())
        .cloned()
        .unwrap_or_default();
    let instance_id = arr.iter().find_map(|i| {
        if i["hand_id"].as_str() == Some(id) {
            i["instance_id"].as_str().map(|s| s.to_string())
        } else {
            None
        }
    });

    match instance_id {
        Some(iid) => {
            let body = daemon_json(
                client
                    .delete(format!("{base}/api/hands/instances/{iid}"))
                    .send(),
            );
            if body.get("status").is_some() {
                println!("Hand '{id}' deactivated.");
            } else {
                eprintln!(
                    "Failed: {}",
                    body["error"].as_str().unwrap_or("Unknown error")
                );
                std::process::exit(1);
            }
        }
        None => {
            eprintln!("No active instance found for hand '{id}'.");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_hand_info(id: &str) {
    let base = require_daemon("hand info");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hands/{id}")).send());
    if body.get("error").is_some() {
        eprintln!("Hand not found: {}", body["error"].as_str().unwrap_or(id));
        std::process::exit(1);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
}

pub(crate) fn cmd_hand_check_deps(id: &str) {
    let base = require_daemon("hand check-deps");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/check-deps"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_hand_install_deps(id: &str) {
    let base = require_daemon("hand install-deps");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/{id}/install-deps"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Dependencies installed for hand '{id}'."));
        if let Some(results) = body.get("results") {
            println!(
                "{}",
                serde_json::to_string_pretty(results).unwrap_or_default()
            );
        }
    }
}

pub(crate) fn cmd_hand_pause(id: &str) {
    let base = require_daemon("hand pause");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/instances/{id}/pause"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Hand instance '{id}' paused."));
    }
}

pub(crate) fn cmd_hand_resume(id: &str) {
    let base = require_daemon("hand resume");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hands/instances/{id}/resume"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Hand instance '{id}' resumed."));
    }
}

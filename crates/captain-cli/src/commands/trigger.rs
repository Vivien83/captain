use crate::{daemon_client, daemon_json, require_daemon};

pub(crate) fn cmd_trigger_list(agent_id: Option<&str>) {
    let base = require_daemon("trigger list");
    let client = daemon_client();

    let url = match agent_id {
        Some(id) => format!("{base}/api/triggers?agent_id={id}"),
        None => format!("{base}/api/triggers"),
    };
    let body = daemon_json(client.get(&url).send());

    match body.as_array() {
        Some(triggers) if triggers.is_empty() => println!("No triggers registered."),
        Some(triggers) => {
            println!(
                "{:<38} {:<38} {:<8} {:<6} PATTERN",
                "TRIGGER ID", "AGENT ID", "ENABLED", "FIRES"
            );
            println!("{}", "-".repeat(110));
            for t in triggers {
                println!(
                    "{:<38} {:<38} {:<8} {:<6} {}",
                    t["id"].as_str().unwrap_or("?"),
                    t["agent_id"].as_str().unwrap_or("?"),
                    t["enabled"].as_bool().unwrap_or(false),
                    t["fire_count"].as_u64().unwrap_or(0),
                    t["pattern"],
                );
            }
        }
        None => println!("No triggers registered."),
    }
}

pub(crate) fn cmd_trigger_create(agent_id: &str, pattern_json: &str, prompt: &str, max_fires: u64) {
    let base = require_daemon("trigger create");
    let pattern: serde_json::Value = serde_json::from_str(pattern_json).unwrap_or_else(|e| {
        eprintln!("Invalid pattern JSON: {e}");
        eprintln!("Examples:");
        eprintln!("  '{{\"lifecycle\":{{}}}}'");
        eprintln!("  '{{\"agent_spawned\":{{\"name_pattern\":\"*\"}}}}'");
        eprintln!("  '{{\"agent_terminated\":{{}}}}'");
        eprintln!("  '{{\"all\":{{}}}}'");
        std::process::exit(1);
    });

    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/triggers"))
            .json(&serde_json::json!({
                "agent_id": agent_id,
                "pattern": pattern,
                "prompt_template": prompt,
                "max_fires": max_fires,
            }))
            .send(),
    );

    if let Some(id) = body["trigger_id"].as_str() {
        println!("Trigger created successfully!");
        println!("  Trigger ID: {id}");
        println!("  Agent ID:   {agent_id}");
    } else {
        eprintln!(
            "Failed to create trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

pub(crate) fn cmd_trigger_delete(trigger_id: &str) {
    let base = require_daemon("trigger delete");
    let client = daemon_client();
    let body = daemon_json(
        client
            .delete(format!("{base}/api/triggers/{trigger_id}"))
            .send(),
    );

    if body.get("status").is_some() {
        println!("Trigger {trigger_id} deleted.");
    } else {
        eprintln!(
            "Failed to delete trigger: {}",
            body["error"].as_str().unwrap_or("Unknown error")
        );
        std::process::exit(1);
    }
}

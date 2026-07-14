use crate::{captain_version, daemon_client, daemon_json, find_daemon, ui};

pub(crate) fn cmd_system_info(json: bool) {
    if let Some(base) = find_daemon() {
        let client = daemon_client();
        let body = daemon_json(client.get(format!("{base}/api/status")).send());
        if json {
            let mut data = body.clone();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("version".to_string(), serde_json::json!(captain_version()));
                obj.insert("api_url".to_string(), serde_json::json!(base));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&data).unwrap_or_default()
            );
            return;
        }
        ui::section("Captain System Info");
        ui::blank();
        let version = captain_version();
        ui::kv("Version", &version);
        ui::kv("Status", body["status"].as_str().unwrap_or("?"));
        ui::kv(
            "Agents",
            &body["agent_count"].as_u64().unwrap_or(0).to_string(),
        );
        ui::kv("Provider", body["default_provider"].as_str().unwrap_or("?"));
        ui::kv("Model", body["default_model"].as_str().unwrap_or("?"));
        ui::kv("API", &base);
        ui::kv("Data dir", body["data_dir"].as_str().unwrap_or("?"));
        ui::kv(
            "Uptime",
            &format!("{}s", body["uptime_seconds"].as_u64().unwrap_or(0)),
        );
    } else {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "version": captain_version(),
                    "daemon": "not_running",
                })
            );
            return;
        }
        ui::section("Captain System Info");
        ui::blank();
        let version = captain_version();
        ui::kv("Version", &version);
        ui::kv_warn("Daemon", "NOT RUNNING");
        ui::hint("Start with: captain start");
    }
}

pub(crate) fn cmd_system_version(json: bool) {
    if json {
        println!("{}", serde_json::json!({"version": captain_version()}));
        return;
    }
    println!("captain {}", captain_version());
}

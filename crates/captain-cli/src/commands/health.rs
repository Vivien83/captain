use crate::{daemon_client, daemon_json, find_daemon, ui};

pub(crate) fn cmd_health(json: bool) {
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            let body = daemon_json(client.get(format!("{base}/api/health")).send());
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&body).unwrap_or_default()
                );
                return;
            }
            ui::success("Daemon is healthy");
            if let Some(status) = body["status"].as_str() {
                ui::kv("Status", status);
            }
            if let Some(uptime) = body.get("uptime_secs").and_then(|v| v.as_u64()) {
                let hours = uptime / 3600;
                let mins = (uptime % 3600) / 60;
                ui::kv("Uptime", &format!("{hours}h {mins}m"));
            }
        }
        None => {
            if json {
                println!("{}", serde_json::json!({"error": "daemon not running"}));
                std::process::exit(1);
            }
            ui::error("Daemon is not running.");
            ui::hint("Start it with: captain start");
            std::process::exit(1);
        }
    }
}

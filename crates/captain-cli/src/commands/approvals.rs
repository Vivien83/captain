use crate::{daemon_client, daemon_json, require_daemon, ui};

pub(crate) fn cmd_approvals_list(json: bool) {
    let base = require_daemon("approvals list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/approvals")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No pending approvals.");
            return;
        }
        println!("{:<38} {:<16} {:<12} REQUEST", "ID", "AGENT", "TYPE");
        println!("{}", "-".repeat(80));
        for a in arr {
            println!(
                "{:<38} {:<16} {:<12} {}",
                a["id"].as_str().unwrap_or("?"),
                a["agent_name"].as_str().unwrap_or("?"),
                a["approval_type"].as_str().unwrap_or("?"),
                a["description"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_approvals_respond(id: &str, approve: bool) {
    let base = require_daemon("approvals");
    let client = daemon_client();
    let endpoint = if approve { "approve" } else { "reject" };
    let body = daemon_json(
        client
            .post(format!("{base}/api/approvals/{id}/{endpoint}"))
            .send(),
    );
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Approval {id} {endpoint}d."));
    }
}

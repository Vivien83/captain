use crate::{daemon_client, daemon_json, require_daemon};

pub(crate) fn cmd_message(agent: &str, text: &str, json: bool) {
    let base = require_daemon("message");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/agents/{agent}/message"))
            .json(&serde_json::json!({"message": text}))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    } else if let Some(reply) = body["reply"].as_str() {
        println!("{reply}");
    } else if let Some(reply) = body["response"].as_str() {
        println!("{reply}");
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

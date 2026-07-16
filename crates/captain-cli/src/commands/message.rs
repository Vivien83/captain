use super::agent_caps::resolve_daemon_agent_id;
use crate::{daemon_client, daemon_json, require_daemon};

pub(crate) fn cmd_message(agent: &str, text: &str, json: bool) {
    let base = require_daemon("message");
    let client = daemon_client();
    let agent_id = resolve_daemon_agent_id(&base, &client, agent);
    let body = daemon_json(
        client
            .post(format!("{base}/api/agents/{agent_id}/message"))
            .json(&message_payload(text))
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

fn message_payload(text: &str) -> serde_json::Value {
    serde_json::json!({
        "message": text,
        "channel_type": "cli"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_message_identifies_cli_origin() {
        let payload = message_payload("hello");
        assert_eq!(payload["message"], "hello");
        assert_eq!(payload["channel_type"], "cli");
    }
}

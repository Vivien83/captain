use serde_json::Value;

use crate::{daemon_client, daemon_json, find_daemon, ui};

use super::agent_caps::resolve_daemon_agent_id;

pub(crate) fn cmd_agent_api(agent_ref: &str, json: bool, manifest: bool, rotate_token: bool) {
    let Some(base) = find_daemon() else {
        ui::error_with_fix(
            "No running daemon found",
            "Start Captain first with: captain start",
        );
        std::process::exit(1);
    };

    let client = daemon_client();
    let agent_id = resolve_daemon_agent_id(&base, &client, agent_ref);

    if rotate_token {
        if manifest {
            ui::error_with_fix(
                "`--manifest` cannot be combined with `--rotate-token`",
                "Run `captain agent api <agent> --manifest` after rotating if you need the contract.",
            );
            std::process::exit(1);
        }
        let body = daemon_json(
            client
                .post(format!("{base}/api/agents/{agent_id}/api/token/rotate"))
                .send(),
        );
        exit_on_api_error(&body);
        if json {
            print_json(&body);
        } else {
            print_rotation_report(&body);
        }
        return;
    }

    let url = if manifest {
        format!("{base}/api/agents/{agent_id}/api/manifest")
    } else {
        format!("{base}/api/agents/{agent_id}/api")
    };
    let body = daemon_json(client.get(url).send());
    exit_on_api_error(&body);

    if json || manifest {
        print_json(&body);
    } else {
        print_agent_api_report(&body);
    }
}

fn exit_on_api_error(body: &Value) {
    if let Some(error) = api_error(body) {
        ui::error_with_fix(
            &format!("Agent API unavailable: {error}"),
            "Run `captain agent list` to copy a valid agent ID, or start Captain again.",
        );
        std::process::exit(1);
    }
}

fn print_json(body: &Value) {
    println!("{}", serde_json::to_string_pretty(body).unwrap_or_default());
}

fn print_agent_api_report(body: &Value) {
    ui::section("Agent API");
    ui::blank();
    for line in agent_api_report_lines(body) {
        println!("{line}");
    }
}

fn print_rotation_report(body: &Value) {
    ui::section("Agent API Token Rotated");
    ui::blank();
    for line in agent_api_rotation_lines(body) {
        println!("{line}");
    }
    ui::blank();
    ui::hint(
        "Token is shown once. Store it in the external service; status output stays redacted.",
    );
}

fn agent_api_report_lines(body: &Value) -> Vec<String> {
    let api = &body["api"];
    let status = &body["config_status"];
    let ingress = &status["ingress"];
    let egress = &status["egress"];
    let queue = &status["queue"];

    let mut lines = vec![
        format!(
            "  Agent: {} ({})",
            body["agent_name"].as_str().unwrap_or("?"),
            body["agent_id"].as_str().unwrap_or("?")
        ),
        format!("  State: {}", status["state"].as_str().unwrap_or("?")),
        format!(
            "  Can receive: {}",
            yes_no(status["can_receive"].as_bool().unwrap_or(false))
        ),
        format!(
            "  Can send callbacks: {}",
            yes_no(status["can_send_callbacks"].as_bool().unwrap_or(false))
        ),
        String::new(),
        "Ingress".to_string(),
        format!("  URL: {}", api["ingress_url"].as_str().unwrap_or("?")),
        format!(
            "  Auth: {}",
            api["auth_scheme"]
                .as_str()
                .unwrap_or("Authorization: Bearer $TOKEN")
        ),
        format!(
            "  Token env: {} ({})",
            api["token_env"].as_str().unwrap_or("?"),
            configured_label(api["token_configured"].as_bool().unwrap_or(false))
        ),
        format!(
            "  Rotate token: {}",
            api["token_rotate_url"].as_str().unwrap_or("?")
        ),
        format!(
            "  Manifest: {}",
            api["manifest_url"].as_str().unwrap_or("?")
        ),
        String::new(),
        "Egress".to_string(),
        format!("  State: {}", egress["state"].as_str().unwrap_or("?")),
        format!(
            "  Configure: {}",
            egress["configure_url"].as_str().unwrap_or("?")
        ),
        format!("  Test: {}", egress["test_url"].as_str().unwrap_or("?")),
        format!(
            "  Events: {}",
            api["audit_events_url"].as_str().unwrap_or("?")
        ),
        format!(
            "  Queue: {} pending, {} dead letters ({})",
            queue["pending"].as_u64().unwrap_or(0),
            queue["dead_letters"].as_u64().unwrap_or(0),
            queue["status_url"].as_str().unwrap_or("?")
        ),
    ];

    let actions = status["operator_actions"]
        .as_array()
        .map(|actions| {
            actions
                .iter()
                .filter_map(Value::as_str)
                .filter(|action| !action.trim().is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !actions.is_empty() {
        lines.push(String::new());
        lines.push("Operator actions".to_string());
        lines.extend(actions.into_iter().map(|action| format!("  - {action}")));
    }

    if ingress["state"].as_str() == Some("missing_token") {
        lines.push(String::new());
        lines.push(format!(
            "  Next: captain agent api {} --rotate-token",
            body["agent_id"].as_str().unwrap_or("<agent>")
        ));
    }

    lines
}

fn agent_api_rotation_lines(body: &Value) -> Vec<String> {
    let rotation = &body["rotation"];
    vec![
        format!("  Status: {}", rotation["status"].as_str().unwrap_or("?")),
        format!("  Agent: {}", rotation["agent_id"].as_str().unwrap_or("?")),
        format!(
            "  Token env: {}",
            rotation["token_env"].as_str().unwrap_or("?")
        ),
        format!(
            "  Stored in: {}",
            rotation["stored_in"].as_str().unwrap_or("secrets.env")
        ),
        format!("  Token: {}", rotation["token"].as_str().unwrap_or("?")),
        "  Use: Authorization: Bearer <token>".to_string(),
    ]
}

fn api_error(value: &Value) -> Option<&str> {
    value["error"]
        .as_str()
        .filter(|error| !error.trim().is_empty())
}

fn configured_label(configured: bool) -> &'static str {
    if configured {
        "configured"
    } else {
        "missing"
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_api_report_includes_ingress_egress_and_next_action() {
        let body = serde_json::json!({
            "agent_id": "agent-1",
            "agent_name": "veille-technologique",
            "api": {
                "ingress_url": "/hooks/agents/agent-1/ingress",
                "auth_scheme": "Authorization: Bearer $TOKEN",
                "token_env": "CAPTAIN_AGENT_API_TOKEN_AGENT_1",
                "token_configured": false,
                "token_rotate_url": "/api/agents/agent-1/api/token/rotate",
                "manifest_url": "/api/agents/agent-1/api/manifest",
                "audit_events_url": "/api/agents/agent-1/api/events"
            },
            "config_status": {
                "state": "not_ready",
                "can_receive": false,
                "can_send_callbacks": false,
                "operator_actions": [
                    "Set CAPTAIN_AGENT_API_TOKEN_AGENT_1 to a bearer token with at least 32 characters."
                ],
                "ingress": {"state": "missing_token"},
                "egress": {
                    "state": "disabled",
                    "configure_url": "/api/agents/agent-1/api/egress/configure",
                    "test_url": "/api/agents/agent-1/api/egress/test"
                },
                "queue": {
                    "pending": 0,
                    "dead_letters": 0,
                    "status_url": "/api/agents/agent-1/api/egress"
                }
            }
        });

        let report = agent_api_report_lines(&body).join("\n");

        assert!(report.contains("veille-technologique"));
        assert!(report.contains("/hooks/agents/agent-1/ingress"));
        assert!(report.contains("/api/agents/agent-1/api/manifest"));
        assert!(report.contains("/api/agents/agent-1/api/egress/configure"));
        assert!(report.contains("captain agent api agent-1 --rotate-token"));
    }

    #[test]
    fn rotation_report_prints_token_only_for_explicit_rotation() {
        let body = serde_json::json!({
            "rotation": {
                "status": "rotated",
                "agent_id": "agent-1",
                "token_env": "CAPTAIN_AGENT_API_TOKEN_AGENT_1",
                "token": "returned-token-value",
                "stored_in": "secrets.env"
            }
        });

        let report = agent_api_rotation_lines(&body).join("\n");

        assert!(report.contains("returned-token-value"));
        assert!(report.contains("Authorization: Bearer <token>"));
    }
}

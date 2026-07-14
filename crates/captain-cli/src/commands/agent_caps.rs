use serde_json::Value;

use crate::{daemon_client, daemon_json, find_daemon, ui};

pub(crate) fn cmd_agent_caps(agent_ref: &str, json: bool) {
    let Some(base) = find_daemon() else {
        ui::error_with_fix(
            "No running daemon found",
            "Start Captain first with: captain start",
        );
        std::process::exit(1);
    };

    let client = daemon_client();
    let agent_id = resolve_daemon_agent_id(&base, &client, agent_ref);
    let agent = daemon_json(client.get(format!("{base}/api/agents/{agent_id}")).send());
    if let Some(error) = api_error(&agent) {
        ui::error_with_fix(
            &format!("Agent not found or unavailable: {error}"),
            "Run `captain agent list` to copy a valid agent ID.",
        );
        std::process::exit(1);
    }
    let budget = daemon_json(
        client
            .get(format!("{base}/api/budget/agents/{agent_id}"))
            .send(),
    );

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "agent": agent,
                "budget": budget,
            }))
            .unwrap_or_default()
        );
        return;
    }

    print_agent_caps_report(&agent, &budget);
}

pub(crate) fn resolve_daemon_agent_id(
    base: &str,
    client: &reqwest::blocking::Client,
    agent_ref: &str,
) -> String {
    let body = daemon_json(client.get(format!("{base}/api/agents")).send());
    let Some(agents) = body.as_array() else {
        return agent_ref.to_string();
    };
    let needle = agent_ref.trim();
    let mut matches = agents
        .iter()
        .filter_map(|agent| {
            let id = agent["id"].as_str()?;
            let name = agent["name"].as_str().unwrap_or("");
            (id == needle || id.starts_with(needle) || name.eq_ignore_ascii_case(needle))
                .then_some((id, name))
        })
        .collect::<Vec<_>>();
    matches.dedup_by(|a, b| a.0 == b.0);
    match matches.as_slice() {
        [(id, _)] => (*id).to_string(),
        [] => needle.to_string(),
        _ => {
            ui::error(&format!("Ambiguous agent reference: {needle}"));
            for (id, name) in matches {
                println!("  {id}  {name}");
            }
            std::process::exit(1);
        }
    }
}

fn print_agent_caps_report(agent: &Value, budget: &Value) {
    ui::section("Agent Capabilities");
    ui::blank();
    for line in agent_caps_report_lines(agent, budget) {
        println!("{line}");
    }
    if api_error(budget).is_some() {
        ui::blank();
        ui::hint("Budget details are unavailable; check `captain status --verbose`.");
    }
}

fn agent_caps_report_lines(agent: &Value, budget: &Value) -> Vec<String> {
    let caps = if agent["capabilities_effective"].is_object() {
        &agent["capabilities_effective"]
    } else {
        &agent["capabilities"]
    };
    let resources = &agent["resources"];
    vec![
        format!(
            "  Agent: {} ({})",
            agent["name"].as_str().unwrap_or("?"),
            agent["id"].as_str().unwrap_or("?")
        ),
        format!("  State: {}", agent["state"].as_str().unwrap_or("?")),
        format!(
            "  Model: {}/{}",
            agent["model"]["provider"].as_str().unwrap_or("?"),
            agent["model"]["model"].as_str().unwrap_or("?")
        ),
        format!(
            "  Profile: {}",
            agent["profile"].as_str().unwrap_or("custom")
        ),
        String::new(),
        "Capabilities".to_string(),
        format!("  Tools: {}", format_scope_list(&caps["tools"])),
        format!("  Network: {}", format_scope_list(&caps["network"])),
        format!("  Shell: {}", format_scope_list(&caps["shell"])),
        format!("  Memory read: {}", format_scope_list(&caps["memory_read"])),
        format!(
            "  Memory write: {}",
            format_scope_list(&caps["memory_write"])
        ),
        format!(
            "  Agent spawn: {}",
            allowed_label(caps["agent_spawn"].as_bool().unwrap_or(false))
        ),
        format!(
            "  Agent messages: {}",
            format_scope_list(&caps["agent_message"])
        ),
        format!(
            "  OFP discover: {}",
            allowed_label(caps["ofp_discover"].as_bool().unwrap_or(false))
        ),
        format!("  OFP connect: {}", format_scope_list(&caps["ofp_connect"])),
        String::new(),
        "Budget".to_string(),
        format!("  LLM tokens/hour: {}", budget_tokens_line(budget)),
        format!("  Cost/hour: {}", budget_money_line(&budget["hourly"])),
        format!("  Cost/day: {}", budget_money_line(&budget["daily"])),
        format!("  Cost/month: {}", budget_money_line(&budget["monthly"])),
        String::new(),
        "Resource limits".to_string(),
        format!(
            "  Tool calls/min: {}",
            resources["max_tool_calls_per_minute"]
                .as_u64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_string())
        ),
        format!(
            "  Memory: {}",
            resources["max_memory_bytes"]
                .as_u64()
                .map(format_bytes)
                .unwrap_or_else(|| "unavailable".to_string())
        ),
        format!(
            "  CPU: {}",
            resources["max_cpu_time_ms"]
                .as_u64()
                .map(format_duration_ms)
                .unwrap_or_else(|| "unavailable".to_string())
        ),
        format!(
            "  Network/hour: {}",
            resources["max_network_bytes_per_hour"]
                .as_u64()
                .map(format_bytes)
                .unwrap_or_else(|| "unavailable".to_string())
        ),
    ]
}

fn api_error(value: &Value) -> Option<&str> {
    value["error"]
        .as_str()
        .filter(|error| !error.trim().is_empty())
}

fn format_scope_list(value: &Value) -> String {
    let items = value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .filter(|item| !item.trim().is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if items.is_empty() {
        "none".to_string()
    } else if items.iter().any(|item| item == "*") {
        "all (*)".to_string()
    } else {
        items.join(", ")
    }
}

fn allowed_label(allowed: bool) -> &'static str {
    if allowed {
        "allowed"
    } else {
        "blocked"
    }
}

fn budget_tokens_line(budget: &Value) -> String {
    let tokens = &budget["tokens"];
    if tokens.is_null() {
        return "unavailable".to_string();
    }
    let used = tokens["used"].as_u64().unwrap_or(0);
    let limit = tokens["limit"].as_u64().unwrap_or(0);
    let pct = tokens["pct"].as_f64().unwrap_or(0.0);
    if limit > 0 {
        format!("{used} / {limit} ({:.1}%)", pct * 100.0)
    } else {
        format!("{used} / unlimited")
    }
}

fn budget_money_line(window: &Value) -> String {
    if window.is_null() {
        return "unavailable".to_string();
    }
    let spend = window["spend"].as_f64().unwrap_or(0.0);
    let limit = window["limit"].as_f64().unwrap_or(0.0);
    let pct = window["pct"].as_f64().unwrap_or(0.0);
    if limit > 0.0 {
        format!("${spend:.4} / ${limit:.2} ({:.1}%)", pct * 100.0)
    } else {
        format!("${spend:.4} / unlimited")
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= GIB {
        format!("{:.1} GiB", bytes_f / GIB)
    } else if bytes_f >= MIB {
        format!("{:.1} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.1} KiB", bytes_f / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration_ms(ms: u64) -> String {
    if ms >= 1000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{ms}ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_scope_list_reports_empty_all_and_named_scopes() {
        assert_eq!(format_scope_list(&serde_json::json!([])), "none");
        assert_eq!(format_scope_list(&serde_json::json!(["*"])), "all (*)");
        assert_eq!(
            format_scope_list(&serde_json::json!(["file_read", "shell_exec"])),
            "file_read, shell_exec"
        );
    }

    #[test]
    fn budget_lines_report_limited_and_unlimited_usage() {
        assert_eq!(
            budget_tokens_line(&serde_json::json!({
                "tokens": {"used": 250, "limit": 1000, "pct": 0.25}
            })),
            "250 / 1000 (25.0%)"
        );
        assert_eq!(
            budget_money_line(&serde_json::json!({
                "spend": 0.25,
                "limit": 0.0,
                "pct": 0.0
            })),
            "$0.2500 / unlimited"
        );
    }

    #[test]
    fn agent_caps_report_includes_effective_capabilities_and_live_budget() {
        let agent = serde_json::json!({
            "id": "agent-1",
            "name": "worker",
            "state": "Running",
            "profile": "coding",
            "model": {"provider": "codex", "model": "gpt-5.5"},
            "capabilities": {"tools": [], "network": []},
            "capabilities_effective": {
                "tools": ["file_read", "shell_exec"],
                "network": ["api.example.com:443"],
                "shell": ["*"],
                "memory_read": ["project.*"],
                "memory_write": [],
                "agent_spawn": true,
                "agent_message": ["worker-*"],
                "ofp_discover": false,
                "ofp_connect": []
            },
            "resources": {
                "max_tool_calls_per_minute": 60,
                "max_memory_bytes": 268435456,
                "max_cpu_time_ms": 30000,
                "max_network_bytes_per_hour": 104857600
            }
        });
        let budget = serde_json::json!({
            "tokens": {"used": 250, "limit": 1000, "pct": 0.25},
            "hourly": {"spend": 0.01, "limit": 1.0, "pct": 0.01},
            "daily": {"spend": 0.02, "limit": 5.0, "pct": 0.004},
            "monthly": {"spend": 0.03, "limit": 50.0, "pct": 0.0006}
        });

        let report = agent_caps_report_lines(&agent, &budget).join("\n");

        assert!(report.contains("  Tools: file_read, shell_exec"));
        assert!(report.contains("  Shell: all (*)"));
        assert!(report.contains("  Agent spawn: allowed"));
        assert!(report.contains("  LLM tokens/hour: 250 / 1000 (25.0%)"));
        assert!(report.contains("  Memory: 256.0 MiB"));
    }

    #[test]
    fn agent_caps_report_marks_missing_resource_fields_unavailable() {
        let agent = serde_json::json!({
            "id": "agent-1",
            "name": "worker",
            "state": "Running",
            "model": {"provider": "codex", "model": "gpt-5.5"},
            "capabilities": {"tools": [], "network": []}
        });
        let report = agent_caps_report_lines(&agent, &serde_json::json!({})).join("\n");

        assert!(report.contains("  Memory: unavailable"));
        assert!(report.contains("  CPU: unavailable"));
        assert!(report.contains("  Network/hour: unavailable"));
    }
}

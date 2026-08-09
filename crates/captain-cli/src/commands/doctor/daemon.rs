use crate::{daemon_client, find_daemon, ui};

use super::DoctorReport;

pub(super) fn check_daemon_details(report: &mut DoctorReport) {
    let Some(base) = find_daemon() else {
        return;
    };

    if report.full {
        check_operational_inventory(report, &base);
    }

    if !report.json {
        println!("\n  Daemon Health:");
    }
    let client = daemon_client();
    match client.get(format!("{base}/api/health/detail")).send() {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                print_health_detail(report, &body);
            }
        }
        Ok(resp) => {
            if !report.json {
                ui::check_warn(&format!("Health detail returned {}", resp.status()));
            }
            report.push(serde_json::json!({"check": "daemon_health", "status": "warn"}));
        }
        Err(e) => {
            if !report.json {
                ui::check_warn(&format!("Failed to query daemon health: {e}"));
            }
            report.push(serde_json::json!({"check": "daemon_health", "status": "warn", "error": e.to_string()}));
        }
    }

    check_daemon_skills(report, &base);
    check_daemon_mcp(report, &base);
    check_integration_health(report, &base);
}

fn check_operational_inventory(report: &mut DoctorReport, base: &str) {
    if !report.json {
        println!("\n  Operational Inventory:");
    }
    let client = daemon_client();
    match client.get(format!("{base}/api/status")).send() {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                if !report.json {
                    print_operational_inventory(&body);
                }
                record_deployment_readiness(report, &body);
                report.push(serde_json::json!({
                    "check": "operational_inventory",
                    "status": "ok",
                    "daemon": body,
                }));
            }
        }
        Ok(resp) => {
            if !report.json {
                ui::check_warn(&format!("Status endpoint returned {}", resp.status()));
            }
            report.push(serde_json::json!({"check": "operational_inventory", "status": "warn"}));
        }
        Err(e) => {
            if !report.json {
                ui::check_warn(&format!("Failed to query status endpoint: {e}"));
            }
            report.push(serde_json::json!({"check": "operational_inventory", "status": "warn", "error": e.to_string()}));
        }
    }
}

fn print_operational_inventory(body: &serde_json::Value) {
    ui::check_ok(&format!(
        "Daemon: {} {}",
        body["status"].as_str().unwrap_or("?"),
        body["version"].as_str().unwrap_or("?")
    ));
    ui::check_ok(&format!(
        "Model: {}/{}",
        body["default_provider"].as_str().unwrap_or("?"),
        body["default_model"].as_str().unwrap_or("?")
    ));
    ui::check_ok(&format!(
        "Paths: data={}, config={}",
        body["data_dir"].as_str().unwrap_or("?"),
        body["config_path"].as_str().unwrap_or("?")
    ));
    ui::check_ok(&format!(
        "Channels: {}/{} configured ({})",
        body["channel_configured_count"].as_u64().unwrap_or(0),
        body["channel_total"].as_u64().unwrap_or(0),
        body["configured_channels"]
            .as_array()
            .map(|arr| arr
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default()
    ));
    ui::check_ok(&format!(
        "TTS: {} / {}",
        body["tts"]["provider"].as_str().unwrap_or("?"),
        body["tts"]["voice"].as_str().unwrap_or("?")
    ));
    ui::check_ok(&format!(
        "Media: image={}, audio={}, video={}",
        body["media"]["image_description"]
            .as_bool()
            .unwrap_or(false),
        body["media"]["audio_transcription"]
            .as_bool()
            .unwrap_or(false),
        body["media"]["video_description"]
            .as_bool()
            .unwrap_or(false)
    ));
    print_deployment_readiness(body);
}

fn readiness_value(body: &serde_json::Value) -> Option<&serde_json::Value> {
    body.get("deployment")?.get("readiness")
}

fn deployment_report_status(body: &serde_json::Value) -> Option<&str> {
    match readiness_value(body)?.get("state")?.as_str()? {
        "ready" | "not_configured" => Some("ok"),
        "pending" | "degraded" => Some("warn"),
        "failed" => Some("fail"),
        _ => Some("warn"),
    }
}

fn record_deployment_readiness(report: &mut DoctorReport, body: &serde_json::Value) {
    let Some(readiness) = readiness_value(body) else {
        return;
    };
    let status = deployment_report_status(body).unwrap_or("warn");
    report.push(serde_json::json!({
        "check": "deployment_readiness",
        "status": status,
        "readiness": readiness,
    }));
    if status == "fail" {
        report.fail();
    }
}

fn print_deployment_readiness(body: &serde_json::Value) {
    let Some(readiness) = readiness_value(body) else {
        ui::check_warn("Deployment readiness: unavailable");
        return;
    };
    let state = readiness["state"].as_str().unwrap_or("unknown");
    match state {
        "ready" => ui::check_ok("Deployment readiness: ready"),
        "not_configured" => ui::check_ok("Deployment readiness: no public domain configured"),
        "failed" => ui::check_fail("Deployment readiness: failed"),
        "pending" => ui::check_warn("Deployment readiness: initial check pending"),
        "degraded" => ui::check_warn("Deployment readiness: degraded"),
        _ => ui::check_warn("Deployment readiness: unknown"),
    }
    if let Some(checked_at) = readiness["checked_at"].as_str() {
        ui::hint(&format!("Last deployment check: {checked_at}"));
    }
    if let Some(actions) = readiness["operator_actions"].as_array() {
        for action in actions.iter().filter_map(|value| value.as_str()).take(3) {
            ui::hint(action);
        }
    }
}

fn print_health_detail(report: &mut DoctorReport, body: &serde_json::Value) {
    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
        if !report.json {
            ui::check_ok(&format!("Running agents: {agents}"));
        }
        report.push(serde_json::json!({"check": "daemon_agents", "status": "ok", "count": agents}));
    }
    if let Some(uptime) = body
        .get("uptime_seconds")
        .or_else(|| body.get("uptime_secs"))
        .and_then(|v| v.as_u64())
    {
        let hours = uptime / 3600;
        let mins = (uptime % 3600) / 60;
        if !report.json {
            ui::check_ok(&format!("Daemon uptime: {hours}h {mins}m"));
        }
        report.push(serde_json::json!({"check": "daemon_uptime", "status": "ok", "secs": uptime}));
    }
    if let Some(db_status) = body.get("database").and_then(|v| v.as_str()) {
        if db_status == "connected" || db_status == "ok" {
            if !report.json {
                ui::check_ok("Database connectivity: OK");
            }
        } else {
            if !report.json {
                ui::check_fail(&format!("Database status: {db_status}"));
            }
            report.fail();
        }
        report.push(serde_json::json!({"check": "daemon_db", "status": db_status}));
    }
    if let Some(audit) = body.get("audit") {
        let valid = audit["valid"].as_bool().unwrap_or(false);
        let status = audit["status"].as_str().unwrap_or("unknown");
        let active_epoch = audit["active_epoch"].as_u64().unwrap_or(0);
        if !report.json {
            if valid {
                ui::check_ok(&format!("Audit integrity: {status} (epoch {active_epoch})"));
            } else {
                ui::check_fail(&format!(
                    "Audit integrity: {status} (active epoch {active_epoch})"
                ));
                if let Some(error) = audit["last_error"].as_str() {
                    ui::hint(error);
                }
            }
        }
        if !valid {
            report.fail();
        }
        report.push(serde_json::json!({
            "check": "audit_integrity",
            "status": if valid { "ok" } else { "fail" },
            "audit": audit,
        }));
    }
}

fn check_daemon_skills(report: &mut DoctorReport, base: &str) {
    let client = daemon_client();
    if let Ok(resp) = client.get(format!("{base}/api/skills")).send() {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                if let Some(arr) = body.as_array() {
                    if !report.json {
                        ui::check_ok(&format!("Skills loaded in daemon: {}", arr.len()));
                    }
                    report.push(serde_json::json!({"check": "daemon_skills", "status": "ok", "count": arr.len()}));
                }
            }
        }
    }
}

fn check_daemon_mcp(report: &mut DoctorReport, base: &str) {
    let client = daemon_client();
    if let Ok(resp) = client.get(format!("{base}/api/mcp/servers")).send() {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                if let Some(arr) = body.as_array() {
                    let connected = arr
                        .iter()
                        .filter(|s| {
                            s.get("connected")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                        })
                        .count();
                    if !report.json {
                        ui::check_ok(&format!(
                            "MCP servers: {} configured, {} connected",
                            arr.len(),
                            connected
                        ));
                    }
                    report.push(serde_json::json!({"check": "daemon_mcp", "status": "ok", "configured": arr.len(), "connected": connected}));
                }
            }
        }
    }
}

fn check_integration_health(report: &mut DoctorReport, base: &str) {
    let client = daemon_client();
    if let Ok(resp) = client.get(format!("{base}/api/integrations/health")).send() {
        if resp.status().is_success() {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                let entries = body.get("health").and_then(|h| h.as_array());
                if let Some(arr) = entries {
                    let healthy = arr
                        .iter()
                        .filter(|v| {
                            v.get("status")
                                .and_then(|s| s.as_str())
                                .map(|s| s.eq_ignore_ascii_case("ready"))
                                .unwrap_or(false)
                        })
                        .count();
                    let total = arr.len();
                    if healthy == total {
                        if !report.json {
                            ui::check_ok(&format!("Integration health: {healthy}/{total} healthy"));
                        }
                    } else if !report.json {
                        ui::check_warn(&format!("Integration health: {healthy}/{total} healthy"));
                    }
                    report.push(serde_json::json!({"check": "integration_health", "status": if healthy == total { "ok" } else { "warn" }, "healthy": healthy, "total": total}));
                }
            }
        }
    }
}

pub(super) fn check_runtime_tools(report: &mut DoctorReport) {
    if !report.json {
        println!();
    }
    check_command_version(report, "rust", "Rust", "rustc", &["--version"], false);
    check_python(report);
    check_command_version(report, "node", "Node.js", "node", &["--version"], true);
}

fn check_python(report: &mut DoctorReport) {
    if command_version("python3", &["--version"]).is_none() {
        match command_version("python", &["--version"]) {
            Some(version) => push_runtime_ok(report, "python", "Python", &version),
            None => {
                if !report.json {
                    ui::check_warn("Python not found (needed for Python skill runtime)");
                }
                report.push(serde_json::json!({"check": "python", "status": "warn"}));
            }
        }
    } else if let Some(version) = command_version("python3", &["--version"]) {
        push_runtime_ok(report, "python", "Python", &version);
    }
}

fn check_command_version(
    report: &mut DoctorReport,
    check: &str,
    label: &str,
    command: &str,
    args: &[&str],
    warn_only: bool,
) {
    match command_version(command, args) {
        Some(version) => push_runtime_ok(report, check, label, &version),
        None if warn_only => {
            if !report.json {
                ui::check_warn(&format!(
                    "{label} not found (needed for Node skill runtime)"
                ));
            }
            report.push(serde_json::json!({"check": check, "status": "warn"}));
        }
        None => {
            if !report.json {
                ui::check_fail(&format!("{label} toolchain not found"));
            }
            report.push(serde_json::json!({"check": check, "status": "fail"}));
            report.fail();
        }
    }
}

fn command_version(command: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(command)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn push_runtime_ok(report: &mut DoctorReport, check: &str, label: &str, version: &str) {
    if !report.json {
        ui::check_ok(&format!("{label}: {version}"));
    }
    report.push(serde_json::json!({"check": check, "status": "ok", "version": version}));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_with_readiness(state: &str) -> serde_json::Value {
        serde_json::json!({
            "deployment": {
                "readiness": {
                    "state": state,
                    "checks": [],
                    "operator_actions": [],
                }
            }
        })
    }

    #[test]
    fn deployment_readiness_maps_operator_states_without_false_failure() {
        assert_eq!(
            deployment_report_status(&body_with_readiness("ready")),
            Some("ok")
        );
        assert_eq!(
            deployment_report_status(&body_with_readiness("not_configured")),
            Some("ok")
        );
        assert_eq!(
            deployment_report_status(&body_with_readiness("pending")),
            Some("warn")
        );
        assert_eq!(
            deployment_report_status(&body_with_readiness("degraded")),
            Some("warn")
        );
        assert_eq!(
            deployment_report_status(&body_with_readiness("failed")),
            Some("fail")
        );
    }

    #[test]
    fn failed_deployment_readiness_marks_full_doctor_failed() {
        let mut report = DoctorReport::new(true, false, true);
        let body = body_with_readiness("failed");

        record_deployment_readiness(&mut report, &body);

        assert!(!report.all_ok);
        assert_eq!(report.checks[0]["check"], "deployment_readiness");
        assert_eq!(report.checks[0]["status"], "fail");
    }
}

use crate::{daemon_client, daemon_json, require_daemon, ui};

pub(crate) fn cmd_security_status(json: bool) {
    let base = require_daemon("security status");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/health/detail")).send());
    let audit = body
        .get("audit")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"status": "unknown", "valid": false}));
    if json {
        let data = serde_json::json!({
            "audit_trail": {
                "algorithm": "versioned_sha256_hash_chain",
                "integrity": audit,
            },
            "content_guards": "heuristic_shell_and_url_patterns",
            "host_execution": body.get("execution").cloned().unwrap_or_default(),
            "wasm_sandbox": "dual_metering_fuel_epoch",
            "wire_protocol": "ofp_hmac_sha256_mutual_auth",
            "api_keys": "zeroizing_auto_wipe",
            "manifests": "ed25519_signed",
            "agent_count": body.get("agent_count").and_then(|v| v.as_u64()),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&data).unwrap_or_default()
        );
        return;
    }
    ui::section("Security Status");
    ui::blank();
    ui::kv("Audit trail", "Versioned SHA-256 hash chain");
    let audit_valid = audit["valid"].as_bool().unwrap_or(false);
    let audit_status = audit["status"].as_str().unwrap_or("unknown");
    let active_epoch = audit["active_epoch"].as_u64().unwrap_or(0);
    let invalid_epochs = audit["invalid_epochs"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    if audit_valid {
        ui::check_ok(&format!(
            "Audit integrity: {audit_status} (epoch {active_epoch})"
        ));
    } else {
        ui::check_fail(&format!(
            "Audit integrity: {audit_status} (active epoch {active_epoch}, {invalid_epochs} invalid)"
        ));
        if let Some(error) = audit["last_error"].as_str() {
            ui::hint(error);
        }
    }
    ui::kv(
        "Content guards",
        "Heuristic shell/URL patterns (no provenance tracking)",
    );
    print_host_execution_status(&body["execution"]);
    ui::kv("WASM sandbox", "Dual metering (fuel + epoch)");
    ui::kv("Wire protocol", "OFP HMAC-SHA256 mutual auth");
    ui::kv("API keys", "Zeroizing<String> (auto-wipe on drop)");
    ui::kv("Manifests", "Ed25519 signed");
    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
        ui::kv("Active agents", &agents.to_string());
    }
}

fn print_host_execution_status(execution: &serde_json::Value) {
    let profile = execution["profile"].as_str().unwrap_or("unknown");
    let backend = execution["backend"].as_str().unwrap_or("unknown");
    let isolation = execution["isolation_level"].as_str().unwrap_or("unknown");
    let configured = execution["configured_policy_mode"]
        .as_str()
        .unwrap_or_else(|| execution["policy_mode"].as_str().unwrap_or("unknown"));
    let effective = execution["policy_mode"].as_str().unwrap_or("unknown");
    let critical = execution["critical_mode"].as_str().unwrap_or("unknown");
    let host_allowed = execution["host_execution_allowed"]
        .as_bool()
        .unwrap_or(effective != "deny");
    ui::kv("Execution profile", profile);
    ui::kv(
        "Host execution",
        &format!(
            "{backend}; {isolation}; configured {configured}; effective {effective}/{critical}; {}",
            if host_allowed { "allowed" } else { "blocked" }
        ),
    );
    if execution["os_isolation"].as_bool().unwrap_or(false) {
        ui::kv_ok("Host OS isolation", "active");
    } else {
        ui::kv_warn("Host OS isolation", "none for shell_exec");
        ui::hint("Use explicit docker_exec or WASM execution for an isolated backend.");
    }
    let docker = &execution["docker"];
    if !docker.is_null() {
        let enabled = docker["enabled"].as_bool().unwrap_or(false);
        let ready = docker["untrusted_profile_ready"].as_bool().unwrap_or(false);
        let availability = docker["runtime_availability"].as_str().unwrap_or("unknown");
        ui::kv(
            "Docker rail",
            &format!(
                "{}; explicit only; runtime {availability}; untrusted profile {}",
                if enabled { "enabled" } else { "disabled" },
                if ready { "ready" } else { "not ready" }
            ),
        );
        if let Some(violations) = docker["violations"].as_array() {
            for violation in violations.iter().filter_map(|value| value.as_str()) {
                ui::kv_warn("Docker policy", violation);
            }
        }
    }
}

pub(crate) fn cmd_security_audit(limit: usize, json: bool) {
    let base = require_daemon("security audit");
    let client = daemon_client();
    let body = daemon_json(
        client
            .get(format!("{base}/api/audit/recent?limit={limit}"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = audit_entries_from_response(&body) {
        if arr.is_empty() {
            println!("No audit entries.");
            return;
        }
        println!(
            "{:<24} {:<6} {:<16} {:<18} EVENT",
            "TIMESTAMP", "EPOCH", "AGENT", "ACTION"
        );
        println!("{}", "-".repeat(92));
        for entry in arr {
            println!("{}", format_audit_row(entry));
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

fn audit_entries_from_response(body: &serde_json::Value) -> Option<&[serde_json::Value]> {
    body.get("entries")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .or_else(|| body.as_array().map(Vec::as_slice))
}

fn format_audit_row(entry: &serde_json::Value) -> String {
    let epoch = entry["epoch"]
        .as_u64()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!(
        "{:<24} {:<6} {:<16} {:<18} {}",
        entry["timestamp"].as_str().unwrap_or("?"),
        epoch,
        entry["agent_id"].as_str().unwrap_or("?"),
        entry["action"].as_str().unwrap_or("?"),
        entry["detail"].as_str().unwrap_or(""),
    )
}

pub(crate) fn cmd_security_verify() {
    let base = require_daemon("security verify");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/audit/verify")).send());
    if body["valid"].as_bool().unwrap_or(false) {
        ui::success("Audit trail integrity verified (SHA-256 hash chain valid).");
    } else {
        ui::error("Audit trail integrity check FAILED.");
        if let Some(msg) = body["error"].as_str() {
            ui::hint(msg);
        }
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{audit_entries_from_response, format_audit_row};

    #[test]
    fn audit_table_reads_the_versioned_api_envelope() {
        let body = serde_json::json!({
            "entries": [{
                "timestamp": "2026-07-29T20:00:00Z",
                "epoch": 3,
                "agent_id": "captain",
                "action": "FutureAction",
                "detail": "preserved",
            }],
            "integrity": {
                "valid": false,
                "active_epoch": 3,
            }
        });

        let entries = audit_entries_from_response(&body).expect("entries envelope");
        assert_eq!(entries.len(), 1);
        let row = format_audit_row(&entries[0]);
        assert!(row.contains("3"));
        assert!(row.contains("captain"));
        assert!(row.contains("FutureAction"));
        assert!(row.contains("preserved"));
    }
}

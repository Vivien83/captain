use crate::{daemon_client, daemon_json, require_daemon, ui};

pub(crate) fn cmd_security_status(json: bool) {
    let base = require_daemon("security status");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/health/detail")).send());
    if json {
        let data = serde_json::json!({
            "audit_trail": "merkle_hash_chain_sha256",
            "taint_tracking": "information_flow_labels",
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
    ui::kv("Audit trail", "Merkle hash chain (SHA-256)");
    ui::kv("Taint tracking", "Information flow labels");
    ui::kv("WASM sandbox", "Dual metering (fuel + epoch)");
    ui::kv("Wire protocol", "OFP HMAC-SHA256 mutual auth");
    ui::kv("API keys", "Zeroizing<String> (auto-wipe on drop)");
    ui::kv("Manifests", "Ed25519 signed");
    if let Some(agents) = body.get("agent_count").and_then(|v| v.as_u64()) {
        ui::kv("Active agents", &agents.to_string());
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
    if let Some(arr) = body.as_array() {
        if arr.is_empty() {
            println!("No audit entries.");
            return;
        }
        println!("{:<24} {:<16} {:<12} EVENT", "TIMESTAMP", "AGENT", "TYPE");
        println!("{}", "-".repeat(80));
        for entry in arr {
            println!(
                "{:<24} {:<16} {:<12} {}",
                entry["timestamp"].as_str().unwrap_or("?"),
                entry["agent_name"].as_str().unwrap_or("?"),
                entry["event_type"].as_str().unwrap_or("?"),
                entry["description"].as_str().unwrap_or(""),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_security_verify() {
    let base = require_daemon("security verify");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/audit/verify")).send());
    if body["valid"].as_bool().unwrap_or(false) {
        ui::success("Audit trail integrity verified (Merkle chain valid).");
    } else {
        ui::error("Audit trail integrity check FAILED.");
        if let Some(msg) = body["error"].as_str() {
            ui::hint(msg);
        }
        std::process::exit(1);
    }
}

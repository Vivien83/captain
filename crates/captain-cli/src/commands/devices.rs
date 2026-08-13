use crate::{daemon_client, daemon_json, require_daemon, ui};

pub(crate) fn cmd_devices_list(json: bool) {
    let base = require_daemon("devices list");
    let client = daemon_client();
    let body = daemon_json(client.get(format!("{base}/api/hub/devices")).send());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    if let Some(arr) = body.get("devices").and_then(serde_json::Value::as_array) {
        if arr.is_empty() {
            println!("No paired devices.");
            return;
        }
        println!(
            "{:<38} {:<20} {:<8} {:<13} {:<22} LAST SEEN",
            "ID", "NAME", "ROLE", "STATE", "VERSION"
        );
        println!("{}", "-".repeat(126));
        for d in arr {
            println!(
                "{:<38} {:<20} {:<8} {:<13} {:<22} {}",
                d["device_id"].as_str().unwrap_or("?"),
                d["display_name"].as_str().unwrap_or("?"),
                d["role"].as_str().unwrap_or("?"),
                d["status"].as_str().unwrap_or("?"),
                d["captain_version"].as_str().unwrap_or("?"),
                format_last_seen(d["last_seen_ms"].as_i64()),
            );
        }
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_devices_pair() {
    let base = require_daemon("devices pair");
    let client = daemon_client();
    let body = daemon_json(
        client
            .post(format!("{base}/api/hub/pairing/enrollment"))
            .json(&serde_json::json!({"duration_secs": 600}))
            .send(),
    );
    if body["open"].as_bool() == Some(true) {
        ui::section("Add device");
        ui::kv("Enrollment", "open for 10 minutes");
        if let Some(expires) = body["expires_at_ms"].as_i64() {
            ui::kv("Expires", &format_last_seen(Some(expires)));
        }
        ui::blank();
        println!("  On the device, start its Captain Client or run:");
        println!("  captain node pair --hub https://your-hub.example --workspace <DIR>");
        ui::blank();
        ui::hint(
            "Approve the displayed code from Captain Web or `captain devices approve <CODE>`.",
        );
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
    }
}

pub(crate) fn cmd_devices_pending(json: bool) {
    let base = require_daemon("devices pending");
    let body = daemon_json(
        daemon_client()
            .get(format!("{base}/api/hub/pairing/requests"))
            .send(),
    );
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&body).unwrap_or_default()
        );
        return;
    }
    let requests = body
        .get("requests")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if requests.is_empty() {
        println!("No pending pairing requests.");
        return;
    }
    println!(
        "{:<38} {:<20} {:<8} {:<20} EXPIRES",
        "REQUEST ID", "NAME", "ROLE", "PLATFORM"
    );
    println!("{}", "-".repeat(112));
    for request in requests {
        println!(
            "{:<38} {:<20} {:<8} {:<20} {}",
            request["request_id"].as_str().unwrap_or("?"),
            request["display_name"].as_str().unwrap_or("?"),
            request["role"].as_str().unwrap_or("?"),
            request["platform"].as_str().unwrap_or("?"),
            format_last_seen(request["expires_at_ms"].as_i64()),
        );
    }
}

pub(crate) fn cmd_devices_approve(code: &str, allow_mutation: bool) {
    let base = require_daemon("devices approve");
    let client = daemon_client();
    let review = daemon_json(
        client
            .post(format!("{base}/api/hub/pairing/review"))
            .json(&serde_json::json!({"display_code": code}))
            .send(),
    );
    let Some(mut grant) = review.get("requested_grants").cloned() else {
        print_api_error(&review, "Pairing code could not be reviewed");
        return;
    };
    let mutation_requested = grant["allow_mutation"].as_bool().unwrap_or(false);
    grant["allow_mutation"] = serde_json::Value::Bool(allow_mutation && mutation_requested);
    let body = daemon_json(
        client
            .post(format!("{base}/api/hub/pairing/approve"))
            .json(&serde_json::json!({"display_code": code, "grant": grant}))
            .send(),
    );
    if let Some(device_id) = body["device_id"].as_str() {
        ui::success(&format!("Device {} approved.", device_id));
        ui::kv(
            "Mutation authority",
            if allow_mutation && mutation_requested {
                "approved"
            } else {
                "read-only"
            },
        );
    } else {
        print_api_error(&body, "Device approval failed");
    }
}

pub(crate) fn cmd_devices_deny(request_id: &str) {
    let Some(request_id) = canonical_request_id(request_id) else {
        ui::error("Invalid pairing request ID.");
        return;
    };
    let base = require_daemon("devices deny");
    let body = daemon_json(
        daemon_client()
            .post(format!("{base}/api/hub/pairing/requests/{request_id}/deny"))
            .send(),
    );
    if body["ok"].as_bool() == Some(true) {
        ui::success(&format!("Pairing request {request_id} denied."));
    } else {
        print_api_error(&body, "Pairing denial failed");
    }
}

pub(crate) fn cmd_devices_remove(id: &str) {
    let base = require_daemon("devices remove");
    let client = daemon_client();
    let body = daemon_json(client.delete(format!("{base}/api/hub/devices/{id}")).send());
    if body.get("error").is_some() {
        ui::error(&format!(
            "Failed: {}",
            body["error"].as_str().unwrap_or("?")
        ));
    } else {
        ui::success(&format!("Device {id} removed."));
    }
}

fn format_last_seen(timestamp_ms: Option<i64>) -> String {
    timestamp_ms
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|timestamp| timestamp.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_else(|| "?".to_string())
}

fn canonical_request_id(value: &str) -> Option<String> {
    uuid::Uuid::parse_str(value)
        .ok()
        .map(|request_id| request_id.to_string())
}

fn print_api_error(body: &serde_json::Value, fallback: &str) {
    let message = body
        .pointer("/error/message")
        .or_else(|| body.get("error"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(fallback);
    ui::error(message);
}

#[cfg(test)]
mod tests {
    use super::{canonical_request_id, format_last_seen};

    #[test]
    fn device_timestamp_rendering_is_stable_and_safe() {
        assert_eq!(format_last_seen(Some(0)), "1970-01-01T00:00:00Z");
        assert_eq!(format_last_seen(None), "?");
        assert_eq!(format_last_seen(Some(i64::MAX)), "?");
    }

    #[test]
    fn pairing_request_ids_are_canonicalized_before_entering_a_url() {
        assert_eq!(
            canonical_request_id("01234567-89AB-CDEF-0123-456789ABCDEF").as_deref(),
            Some("01234567-89ab-cdef-0123-456789abcdef")
        );
        assert_eq!(canonical_request_id("../../status"), None);
    }
}

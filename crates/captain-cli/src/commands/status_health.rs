use crate::{truncate_display, ui};

pub(super) fn print_runtime_health_summary(body: &serde_json::Value) {
    let health = &body["runtime_health"];
    let Some(line) = runtime_health_summary_line(health) else {
        return;
    };

    if health["state"].as_str().unwrap_or("ok") == "ok" {
        ui::kv_ok("Health", &line);
    } else {
        ui::kv_warn("Health", &line);
        if let Some(action) = first_action(health) {
            ui::hint(&action);
        }
    }
}

pub(super) fn print_disk_summary(body: &serde_json::Value) {
    let Some(line) = disk_summary_line(&body["disk"]) else {
        return;
    };
    if body["disk"]["cleanup_recommended"]
        .as_bool()
        .unwrap_or(false)
    {
        ui::kv_warn("Disk", &line);
        ui::hint("Clean build/debug artifacts before starting long compile or install work.");
    } else {
        ui::kv_ok("Disk", &line);
    }
}

pub(super) fn print_verbose_runtime_health(body: &serde_json::Value) {
    let health = &body["runtime_health"];
    if health.is_null() || health["issue_count"].as_u64().unwrap_or(0) == 0 {
        return;
    }

    ui::blank();
    ui::section("Runtime Health");
    ui::kv_warn(
        "State",
        &runtime_health_summary_line(health).unwrap_or_else(|| "unknown".to_string()),
    );
    if let Some(issues) = health["issues"].as_array() {
        for issue in issues.iter().take(6) {
            let severity = issue["severity"].as_str().unwrap_or("warn");
            let kind = issue["kind"].as_str().unwrap_or("runtime");
            let summary = truncate_display(issue["summary"].as_str().unwrap_or(""), 120);
            println!("    {severity} -- {kind} -- {summary}");
        }
    }
    if let Some(actions) = health["operator_actions"].as_array() {
        for action in actions.iter().filter_map(|item| item.as_str()).take(3) {
            ui::hint(action);
        }
    }
}

fn runtime_health_summary_line(health: &serde_json::Value) -> Option<String> {
    let state = health["state"].as_str()?;
    let issue_count = health["issue_count"].as_u64().unwrap_or(0);
    if issue_count == 0 {
        Some(state.to_string())
    } else {
        Some(format!("{state} ({issue_count} issue(s))"))
    }
}

fn first_action(health: &serde_json::Value) -> Option<String> {
    health["operator_actions"]
        .as_array()?
        .iter()
        .find_map(|item| item.as_str().map(String::from))
}

fn disk_summary_line(disk: &serde_json::Value) -> Option<String> {
    if disk.is_null() {
        return None;
    }
    if disk["readable"].as_bool() == Some(false) {
        return Some("unavailable".to_string());
    }
    let available = disk["available_gib"].as_f64()?;
    let threshold = disk["cleanup_threshold_gib"].as_f64().unwrap_or(15.0);
    Some(format!(
        "{available:.1} GiB free (cleanup at <= {threshold:.1} GiB)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_health_summary_handles_ok_state() {
        let health = serde_json::json!({"state": "ok", "issue_count": 0});
        assert_eq!(runtime_health_summary_line(&health).unwrap(), "ok");
    }

    #[test]
    fn runtime_health_summary_reports_issue_count() {
        let health = serde_json::json!({"state": "warn", "issue_count": 2});
        assert_eq!(
            runtime_health_summary_line(&health).unwrap(),
            "warn (2 issue(s))"
        );
    }

    #[test]
    fn disk_summary_reports_free_space_and_threshold() {
        let disk = serde_json::json!({
            "readable": true,
            "available_gib": 43.2,
            "cleanup_threshold_gib": 15.0
        });
        assert_eq!(
            disk_summary_line(&disk).unwrap(),
            "43.2 GiB free (cleanup at <= 15.0 GiB)"
        );
    }
}

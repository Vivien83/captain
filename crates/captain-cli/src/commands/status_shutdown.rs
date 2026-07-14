use crate::ui;

pub(super) fn print_shutdown_drain_summary(body: &serde_json::Value) {
    let shutdown = &body["shutdown"];
    let Some(line) = shutdown_drain_summary_line(shutdown) else {
        return;
    };

    ui::kv_warn("Shutdown", &line);
    if let Some(action) = first_action(shutdown) {
        ui::hint(&action);
    }
}

fn shutdown_drain_summary_line(shutdown: &serde_json::Value) -> Option<String> {
    if shutdown["status"].as_str()? != "draining" {
        return None;
    }
    let trigger = shutdown["trigger"].as_str().unwrap_or("control");
    let active = shutdown["active_work_count"]
        .as_u64()
        .unwrap_or_else(|| shutdown["active_run_count"].as_u64().unwrap_or(0));
    let active_processes = shutdown["active_process_count"].as_u64().unwrap_or(0);
    let initial = shutdown["initial_active_work_count"]
        .as_u64()
        .unwrap_or(active);
    let age = shutdown["age_seconds"].as_u64().unwrap_or(0);
    let process_note = if active_processes > 0 {
        format!(", {active_processes} process(es)")
    } else {
        String::new()
    };
    Some(format!("draining via {trigger}: {active} active work item(s){process_note}, started with {initial}, age {age}s"))
}

fn first_action(shutdown: &serde_json::Value) -> Option<String> {
    shutdown["operator_actions"]
        .as_array()?
        .iter()
        .find_map(|item| item.as_str().map(String::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_drain_summary_hides_idle_state() {
        let shutdown = serde_json::json!({"status": "idle", "active_run_count": 1});

        assert!(shutdown_drain_summary_line(&shutdown).is_none());
    }

    #[test]
    fn shutdown_drain_summary_reports_trigger_and_counts() {
        let shutdown = serde_json::json!({
            "status": "draining",
            "trigger": "SIGTERM",
            "active_work_count": 2,
            "active_run_count": 1,
            "active_process_count": 1,
            "initial_active_work_count": 3,
            "age_seconds": 42,
            "operator_actions": ["Run captain status to inspect active work."]
        });

        assert_eq!(
            shutdown_drain_summary_line(&shutdown).unwrap(),
            "draining via SIGTERM: 2 active work item(s), 1 process(es), started with 3, age 42s"
        );
        assert_eq!(
            first_action(&shutdown).unwrap(),
            "Run captain status to inspect active work."
        );
    }
}

pub(super) fn in_process_runtime_health(
    llm_driver_ready: bool,
    workload: &serde_json::Value,
    disk: &serde_json::Value,
    budget: &serde_json::Value,
) -> serde_json::Value {
    let channels = serde_json::json!({"locked": []});
    let agent_api = serde_json::json!({
        "egress_queue": {
            "readable": true,
            "pending": 0,
            "due": 0,
            "dead_letters": 0
        }
    });
    let consciousness =
        serde_json::json!({"state": "steady", "signals": [], "operator_actions": []});
    let shutdown = serde_json::json!({
        "status": "idle",
        "active_work_count": 0,
        "active_run_count": 0,
        "active_process_count": 0
    });
    captain_api::status_runtime_health::build_runtime_health_status(
        llm_driver_ready,
        &channels,
        workload,
        &agent_api,
        &consciousness,
        disk,
        &shutdown,
        budget,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_process_health_keeps_disk_cleanup_visible() {
        let workload = serde_json::json!({
            "projects": {"attention_count": 0},
            "automation": {
                "delivery": {
                    "failed_jobs": 0,
                    "redelivery_queued": 0,
                    "redelivery_due": 0,
                    "dead_letters": 0
                }
            }
        });
        let disk = serde_json::json!({
            "available_gib": 14.5,
            "cleanup_threshold_gib": 15.0,
            "cleanup_recommended": true
        });
        let budget = serde_json::json!({
            "provider_subscriptions": {"state": "unavailable"},
            "operator_actions": []
        });
        let health = in_process_runtime_health(true, &workload, &disk, &budget);

        assert_eq!(health["state"], "warn");
        assert_eq!(health["issues"][0]["kind"], "disk_space");
    }
}

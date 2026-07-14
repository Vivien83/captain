use std::path::PathBuf;

use crate::cli_captain_home;

pub(super) fn ensure_status_observability(body: &mut serde_json::Value) {
    if body["shutdown"].is_null() {
        let active_runs = body["active_run_count"].as_u64().unwrap_or(0);
        body["shutdown"] = serde_json::json!({
            "status": "idle",
            "active_work_count": active_runs,
            "active_run_count": active_runs,
            "active_process_count": 0,
            "operator_actions": [],
        });
    }

    if body["disk"].is_null() {
        let home_dir = body["home_dir"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(cli_captain_home);
        body["disk"] = captain_api::status_disk::build_disk_status(&home_dir);
    }

    if body["runtime_health"].is_null() {
        let channels = body
            .get("channels")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"locked": []}));
        let workload = body
            .get("workload")
            .cloned()
            .unwrap_or_else(clean_workload_status);
        let agent_api = body
            .get("agent_api")
            .cloned()
            .unwrap_or_else(clean_agent_api_status);
        let consciousness = body
            .get("consciousness")
            .cloned()
            .unwrap_or_else(clean_consciousness_status);
        let llm_ready = body["llm_driver_ready"].as_bool().unwrap_or(true);
        body["runtime_health"] = captain_api::status_runtime_health::build_runtime_health_status(
            llm_ready,
            &channels,
            &workload,
            &agent_api,
            &consciousness,
            &body["disk"],
            &body["shutdown"],
        );
    }
}

fn clean_workload_status() -> serde_json::Value {
    serde_json::json!({
        "projects": {"attention_count": 0},
        "automation": {
            "delivery": {
                "failed_jobs": 0,
                "redelivery_queued": 0,
                "redelivery_due": 0,
                "dead_letters": 0
            }
        }
    })
}

fn clean_agent_api_status() -> serde_json::Value {
    serde_json::json!({
        "egress_queue": {
            "readable": true,
            "pending": 0,
            "due": 0,
            "dead_letters": 0
        }
    })
}

fn clean_consciousness_status() -> serde_json::Value {
    serde_json::json!({"state": "steady", "signals": [], "operator_actions": []})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_daemon_status_gets_observability_fields() {
        let mut body = serde_json::json!({
            "status": "running",
            "home_dir": std::env::temp_dir(),
            "llm_driver_ready": true
        });

        ensure_status_observability(&mut body);

        assert!(body["disk"]["available_gib"].as_f64().is_some());
        if body["disk"]["cleanup_recommended"]
            .as_bool()
            .unwrap_or(false)
        {
            assert_eq!(body["runtime_health"]["state"], "warn");
            assert_eq!(body["runtime_health"]["issues"][0]["kind"], "disk_space");
        } else {
            assert_eq!(body["runtime_health"]["state"], "ok");
            assert_eq!(body["runtime_health"]["issue_count"], 0);
        }
        assert_eq!(body["shutdown"]["status"], "idle");
    }
}

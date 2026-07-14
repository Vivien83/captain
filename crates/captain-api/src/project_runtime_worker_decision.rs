use crate::project_runtime_resume::runtime_should_resume_stale_run;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeWorkerExistingDecision {
    Launch,
    SkipDone,
    Blocked { status: String },
    RecoverRunning,
    AlreadyRunning,
}

pub(crate) fn runtime_worker_existing_decision(
    runtime: &serde_json::Value,
    status: Option<&str>,
) -> RuntimeWorkerExistingDecision {
    match status {
        Some("done") => RuntimeWorkerExistingDecision::SkipDone,
        Some(status) if status == "blocked" || status == "failed" => {
            RuntimeWorkerExistingDecision::Blocked {
                status: status.to_string(),
            }
        }
        Some("running") if runtime_should_resume_stale_run(false, runtime) => {
            RuntimeWorkerExistingDecision::RecoverRunning
        }
        Some("running") => RuntimeWorkerExistingDecision::AlreadyRunning,
        _ => RuntimeWorkerExistingDecision::Launch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stale_runtime_with_running_worker() -> serde_json::Value {
        serde_json::json!({
            "status": "running",
            "orchestrator": {"active": true},
            "workers": [{"phase": "build", "status": "running"}],
            "worker_results": {"observe": {"status": "done"}}
        })
    }

    #[test]
    fn done_worker_is_skipped() {
        assert_eq!(
            runtime_worker_existing_decision(&stale_runtime_with_running_worker(), Some("done")),
            RuntimeWorkerExistingDecision::SkipDone
        );
    }

    #[test]
    fn blocked_or_failed_worker_stops_for_operator_action() {
        assert_eq!(
            runtime_worker_existing_decision(&stale_runtime_with_running_worker(), Some("blocked")),
            RuntimeWorkerExistingDecision::Blocked {
                status: "blocked".to_string()
            }
        );
        assert_eq!(
            runtime_worker_existing_decision(&stale_runtime_with_running_worker(), Some("failed")),
            RuntimeWorkerExistingDecision::Blocked {
                status: "failed".to_string()
            }
        );
    }

    #[test]
    fn stale_running_worker_is_recovered() {
        assert_eq!(
            runtime_worker_existing_decision(&stale_runtime_with_running_worker(), Some("running")),
            RuntimeWorkerExistingDecision::RecoverRunning
        );
    }

    #[test]
    fn non_stale_running_worker_is_rejected_as_already_running() {
        let runtime = serde_json::json!({
            "status": "ready",
            "orchestrator": {"active": false},
            "workers": [{"phase": "build", "status": "running"}],
            "worker_results": {}
        });

        assert_eq!(
            runtime_worker_existing_decision(&runtime, Some("running")),
            RuntimeWorkerExistingDecision::AlreadyRunning
        );
    }

    #[test]
    fn missing_or_unknown_status_launches_worker() {
        assert_eq!(
            runtime_worker_existing_decision(&stale_runtime_with_running_worker(), None),
            RuntimeWorkerExistingDecision::Launch
        );
        assert_eq!(
            runtime_worker_existing_decision(&stale_runtime_with_running_worker(), Some("planned")),
            RuntimeWorkerExistingDecision::Launch
        );
    }
}

pub(crate) fn runtime_should_resume_stale_run(
    process_running: bool,
    runtime: &serde_json::Value,
) -> bool {
    runtime_declares_active(runtime)
        && !process_running
        && runtime_has_recoverable_progress(runtime)
}

pub(crate) fn runtime_declares_active(runtime: &serde_json::Value) -> bool {
    runtime
        .get("status")
        .and_then(|v| v.as_str())
        .map(|status| status == "running")
        .unwrap_or(false)
        || runtime
            .pointer("/orchestrator/active")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

fn runtime_has_recoverable_progress(runtime: &serde_json::Value) -> bool {
    let worker_has_progress = runtime
        .get("workers")
        .and_then(|v| v.as_array())
        .map(|workers| {
            workers.iter().any(|worker| {
                worker
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(|status| status != "ready")
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    let result_has_progress = runtime
        .get("worker_results")
        .and_then(|v| v.as_object())
        .map(|results| !results.is_empty())
        .unwrap_or(false);
    worker_has_progress || result_has_progress
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_resume_requires_active_progress_without_process() {
        let runtime = serde_json::json!({
            "status": "running",
            "orchestrator": {"active": true},
            "workers": [
                {"phase": "observe", "status": "done"},
                {"phase": "build", "status": "running"}
            ],
            "worker_results": {"observe": {"status": "done"}}
        });
        assert!(runtime_should_resume_stale_run(false, &runtime));
        assert!(!runtime_should_resume_stale_run(true, &runtime));

        let empty_runtime = serde_json::json!({
            "status": "running",
            "orchestrator": {"active": true},
            "workers": [{"phase": "observe", "status": "ready"}],
            "worker_results": {}
        });
        assert!(!runtime_should_resume_stale_run(false, &empty_runtime));
    }
}

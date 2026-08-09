//! Background adapters for the shared Live Runs operator contract.

use super::event::{AppEvent, BackendRef};
use captain_kernel::ToolRunOperatorSurface;
use captain_runtime::{
    tool_run_operator::{operator_tail, OperatorToolRun, OperatorToolRunTail},
    tool_runs::{global_registry, MAX_RUNS},
};
use serde::Deserialize;
use std::sync::mpsc;
use std::time::Duration;

const TAIL_LINES: usize = 200;

#[derive(Deserialize)]
struct RunListResponse {
    items: Vec<OperatorToolRun>,
}

#[derive(Deserialize)]
struct RunTailResponse {
    tail: OperatorToolRunTail,
}

#[derive(Deserialize)]
struct RunCancelResponse {
    run: OperatorToolRun,
}

pub fn spawn_fetch_runs(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let result = match backend {
            BackendRef::Daemon(base_url) => fetch_daemon_runs(&base_url),
            BackendRef::InProcess(_) => Ok(global_registry()
                .list(None, MAX_RUNS)
                .into_iter()
                .map(OperatorToolRun::from)
                .collect()),
        };
        let _ = tx.send(AppEvent::ToolRunsLoaded(result));
    });
}

pub fn spawn_fetch_tail(backend: BackendRef, run_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let result = match backend {
            BackendRef::Daemon(base_url) => fetch_daemon_tail(&base_url, &run_id),
            BackendRef::InProcess(_) => fetch_inprocess_tail(&run_id),
        };
        let _ = tx.send(AppEvent::ToolRunTailLoaded { run_id, result });
    });
}

pub fn spawn_cancel_run(backend: BackendRef, run_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let result = match backend {
            BackendRef::Daemon(base_url) => cancel_daemon_run(&base_url, &run_id),
            BackendRef::InProcess(kernel) => kernel
                .operator_cancel_tool_run(ToolRunOperatorSurface::Tui, &run_id)
                .map(OperatorToolRun::from)
                .map_err(|error| format!("Tool run cancellation refused: {error}")),
        };
        let _ = tx.send(AppEvent::ToolRunCancelled { run_id, result });
    });
}

fn fetch_daemon_runs(base_url: &str) -> Result<Vec<OperatorToolRun>, String> {
    let response = daemon_client()
        .get(format!("{base_url}/api/tool-runs"))
        .query(&[("limit", MAX_RUNS)])
        .send()
        .map_err(|error| format!("Live Runs unavailable: {error}"))?;
    decode_response::<RunListResponse>(response, "Live Runs").map(|body| body.items)
}

fn fetch_daemon_tail(base_url: &str, run_id: &str) -> Result<OperatorToolRunTail, String> {
    let response = daemon_client()
        .get(format!("{base_url}/api/tool-runs/{run_id}/tail"))
        .query(&[("max_lines", TAIL_LINES)])
        .send()
        .map_err(|error| format!("Tool run tail unavailable: {error}"))?;
    decode_response::<RunTailResponse>(response, "Tool run tail").map(|body| body.tail)
}

fn cancel_daemon_run(base_url: &str, run_id: &str) -> Result<OperatorToolRun, String> {
    let response = daemon_client()
        .post(format!("{base_url}/api/tool-runs/{run_id}/cancel"))
        .send()
        .map_err(|error| format!("Tool run cancellation unavailable: {error}"))?;
    decode_response::<RunCancelResponse>(response, "Tool run cancellation").map(|body| body.run)
}

fn fetch_inprocess_tail(run_id: &str) -> Result<OperatorToolRunTail, String> {
    let registry = global_registry();
    let snapshot = registry
        .snapshot(run_id)
        .ok_or_else(|| "Tool run not found".to_string())?;
    let page = registry.tail_output(run_id, TAIL_LINES).map_err(|_| {
        "Retained tool output is unavailable or failed integrity verification".to_string()
    })?;
    Ok(operator_tail(run_id, snapshot.status, page))
}

fn daemon_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .default_headers(crate::daemon_auth_headers())
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

fn decode_response<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<T, String> {
    if !response.status().is_success() {
        return Err(format!("{label} failed: HTTP {}", response.status()));
    }
    response
        .json::<T>()
        .map_err(|error| format!("{label} response invalid: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_list_envelope_decodes_only_the_shared_projection() {
        let payload = serde_json::json!({
            "count": 1,
            "items": [{
                "run_id": "toolrun-test",
                "tool_name": "shell_exec",
                "status": "running",
                "detached": true,
                "cancellable": true,
                "started_at_unix_ms": 1,
                "finished_at_unix_ms": null,
                "elapsed_ms": 2,
                "caller_agent_id": "captain",
                "origin_tool_use_id": null,
                "input_sha256": null,
                "retry_of_run_id": null,
                "retry_attempt": 0,
                "is_error": null,
                "result_available": false,
                "result_truncated": false,
                "output_available": true,
                "output_stored_bytes": 12,
                "output_total_bytes": 12,
                "output_sha256": null,
                "output_capped": false,
                "output_redacted": true
            }]
        });

        let decoded: RunListResponse = serde_json::from_value(payload).unwrap();
        assert_eq!(decoded.items.len(), 1);
        assert_eq!(decoded.items[0].run_id, "toolrun-test");
    }
}

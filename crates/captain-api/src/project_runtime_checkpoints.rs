use crate::routes::AppState;
use captain_memory::{project, project_checkpoint};

pub(crate) const PROJECT_RUNTIME_PROTOCOL: &str = "captain.project_runtime.v2";

pub(crate) fn latest_checkpoint_runtime(
    state: &AppState,
    project: &project::Project,
) -> Option<serde_json::Value> {
    state
        .kernel
        .memory
        .checkpoint_latest(&project.id)
        .ok()
        .flatten()
        .and_then(|checkpoint| runtime_from_checkpoint_state(&checkpoint.state))
}

pub(crate) fn append_runtime_checkpoint(
    state: &AppState,
    project: &project::Project,
    runtime: &serde_json::Value,
    run_id: &str,
    session_id: String,
) {
    let summaries = runtime_worker_completion_lines(runtime);
    let _ = state
        .kernel
        .memory
        .checkpoint_append(project_checkpoint::NewCheckpoint {
            project_id: project.id.clone(),
            session_id: Some(session_id),
            summary: format!(
                "Autonomous project run completed for {}.\n{}",
                project.name, summaries
            ),
            state: serde_json::json!({
                "runtime_protocol": PROJECT_RUNTIME_PROTOCOL,
                "checkpoint_kind": "runtime_final",
                "run_id": run_id,
                "status": "done",
                "phase": "learn",
                "worker_count": runtime.get("workers").and_then(|v| v.as_array()).map(|w| w.len()).unwrap_or(0),
                "runtime": runtime,
            }),
        });
}

pub(crate) fn append_runtime_phase_checkpoint(
    state: &AppState,
    project: &project::Project,
    runtime: &serde_json::Value,
    run_id: &str,
    phase: &str,
    status: &str,
    session_id: String,
) {
    let summaries = runtime_worker_checkpoint_lines(runtime);
    let phase_detail = runtime
        .get("workers")
        .and_then(|v| v.as_array())
        .and_then(|workers| {
            workers
                .iter()
                .find(|worker| worker.get("phase").and_then(|v| v.as_str()) == Some(phase))
        })
        .and_then(|worker| {
            worker
                .get("summary")
                .or_else(|| worker.get("error"))
                .and_then(|v| v.as_str())
        })
        .map(|detail| trim_runtime_text(detail, 700));

    let _ = state
        .kernel
        .memory
        .checkpoint_append(project_checkpoint::NewCheckpoint {
            project_id: project.id.clone(),
            session_id: Some(session_id),
            summary: format!(
                "Autonomous project checkpoint for {}: phase {phase} {status}.\n{}\n{}",
                project.name,
                summaries,
                phase_detail
                    .as_deref()
                    .map(|detail| format!("Latest detail: {detail}"))
                    .unwrap_or_else(|| "Latest detail: none recorded.".to_string())
            ),
            state: serde_json::json!({
                "runtime_protocol": PROJECT_RUNTIME_PROTOCOL,
                "checkpoint_kind": "runtime_phase",
                "run_id": run_id,
                "status": status,
                "phase": phase,
                "worker_count": runtime.get("workers").and_then(|v| v.as_array()).map(|w| w.len()).unwrap_or(0),
                "runtime": runtime,
            }),
        });
}

pub(crate) fn runtime_from_checkpoint_state(
    state: &serde_json::Value,
) -> Option<serde_json::Value> {
    let protocol = state.get("runtime_protocol").and_then(|v| v.as_str());
    if protocol != Some(PROJECT_RUNTIME_PROTOCOL) {
        return None;
    }
    state
        .get("runtime")
        .cloned()
        .filter(|runtime| runtime.is_object())
}

pub(crate) fn trim_runtime_text(text: &str, max_chars: usize) -> String {
    let cleaned = text
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect::<String>();
    if cleaned.chars().count() <= max_chars {
        return cleaned;
    }
    let mut out = cleaned.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn runtime_worker_completion_lines(runtime: &serde_json::Value) -> String {
    runtime
        .get("workers")
        .and_then(|v| v.as_array())
        .map(|workers| {
            workers
                .iter()
                .filter_map(|worker| {
                    let phase = worker.get("phase").and_then(|v| v.as_str())?;
                    let summary = worker.get("summary").and_then(|v| v.as_str())?;
                    Some(format!("- {phase}: {}", trim_runtime_text(summary, 260)))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "- No worker summaries were recorded.".to_string())
}

fn runtime_worker_checkpoint_lines(runtime: &serde_json::Value) -> String {
    runtime
        .get("workers")
        .and_then(|v| v.as_array())
        .map(|workers| {
            workers
                .iter()
                .filter_map(|worker| {
                    let phase = worker.get("phase").and_then(|v| v.as_str())?;
                    let status = worker.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    let detail = worker
                        .get("summary")
                        .or_else(|| worker.get("error"))
                        .and_then(|v| v.as_str())
                        .map(|value| trim_runtime_text(value, 220))
                        .unwrap_or_else(|| "no summary yet".to_string());
                    Some(format!("- {phase} [{status}]: {detail}"))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "- No worker state was recorded.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_worker_checkpoint_lines_include_status_and_detail() {
        let runtime = serde_json::json!({
            "workers": [
                {"phase": "observe", "status": "done", "summary": "Repository inspected."},
                {"phase": "build", "status": "failed", "error": "Compile error."}
            ]
        });
        let lines = runtime_worker_checkpoint_lines(&runtime);
        assert!(lines.contains("- observe [done]: Repository inspected."));
        assert!(lines.contains("- build [failed]: Compile error."));
    }

    #[test]
    fn runtime_from_checkpoint_state_requires_protocol_and_runtime_object() {
        let state = serde_json::json!({
            "runtime_protocol": PROJECT_RUNTIME_PROTOCOL,
            "runtime": {"status": "running", "current_phase": "build"}
        });
        let runtime = runtime_from_checkpoint_state(&state).unwrap();
        assert_eq!(runtime["current_phase"], "build");

        assert!(runtime_from_checkpoint_state(&serde_json::json!({
            "runtime_protocol": "old",
            "runtime": {"status": "running"}
        }))
        .is_none());
        assert!(runtime_from_checkpoint_state(&serde_json::json!({
            "runtime_protocol": PROJECT_RUNTIME_PROTOCOL,
            "runtime": null
        }))
        .is_none());
    }
}

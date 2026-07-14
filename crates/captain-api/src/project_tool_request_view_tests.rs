use super::*;
use crate::project_runtime_status::project_runtime_operator_status;

fn project_with_metadata(metadata: serde_json::Value) -> captain_memory::project::Project {
    captain_memory::project::Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Ship".to_string(),
        status: captain_memory::project::ProjectStatus::Active,
        deadline: None,
        created_at: 1,
        updated_at: 2,
        metadata,
    }
}

#[test]
fn tool_request_success_view_omits_raw_project_runtime_and_bounds_tools() {
    let project = project_with_metadata(serde_json::json!({
        "runtime": {
            "status": "ready",
            "current_phase": "build",
            "resume_pending": {
                "reason": "tool_request_approved",
                "phase": "build",
                "operator_reason": "operator reason secret"
            },
            "workers": [{
                "phase": "build",
                "prompt": "raw worker prompt secret",
                "tool_request": {
                    "tools": ["shell_exec"],
                    "decision_reason": "decision secret"
                }
            }],
            "timeline": [{
                "id": "event-1",
                "detail": "event detail secret",
                "data": {"secret": "event data secret"}
            }]
        },
        "workspace": {"path": "/Users/example/private"},
        "secret": "metadata secret"
    }));
    let operator_status =
        project_runtime_operator_status(&project, &project.metadata["runtime"], false);
    let long_tool = format!("{}TOOL_SECRET_TAIL", "x".repeat(120));

    let view = project_tool_request_success_view(
        &project,
        operator_status,
        "build",
        "approve",
        &["shell_exec".to_string(), long_tool],
        true,
    );

    assert!(view.get("runtime").is_none());
    assert!(view["project"].get("metadata").is_none());
    assert_eq!(view["phase"], "build");
    assert_eq!(view["decision"], "approve");
    assert_eq!(view["runtime_resume_pending"], true);
    assert_eq!(view["tools"][1].as_str().unwrap().chars().count(), 80);

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "operator reason secret",
        "raw worker prompt secret",
        "decision secret",
        "event detail secret",
        "event data secret",
        "/Users/example/private",
        "metadata secret",
        "TOOL_SECRET_TAIL",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn tool_request_error_views_omit_raw_storage_and_input_details() {
    let storage = safe_project_tool_request_storage_error(
        "sqlite failed at /Users/example/db with ghp_secret",
    );
    let runtime = safe_project_tool_request_runtime_error(
        "no pending tool request found for phase verify-secret",
    );
    let invalid_phase = safe_project_tool_request_phase("verify-secret");

    assert_eq!(
        storage,
        "Project tool request state could not be saved; verify project storage availability"
    );
    assert_eq!(runtime, "No pending project tool request found");
    assert_eq!(invalid_phase, "unknown");

    let encoded = serde_json::json!({
        "storage": storage,
        "runtime": runtime,
        "invalid_phase": invalid_phase,
    })
    .to_string();
    for forbidden in [
        "/Users/example",
        "ghp_secret",
        "sqlite failed",
        "verify-secret",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

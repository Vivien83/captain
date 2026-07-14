use super::*;

#[test]
fn approve_reopens_blocked_phase_for_resume() {
    let mut runtime = serde_json::json!({
        "status": "blocked",
        "current_phase": "build",
        "workers": [{
            "phase": "build",
            "status": "blocked",
            "error": "missing tool",
            "tool_request": {
                "tools": ["shell_exec"],
                "status": "pending_captain_decision"
            }
        }],
        "worker_results": {
            "build": {
                "status": "blocked",
                "blocked": true,
                "tool_request": {
                    "tools": ["shell_exec"],
                    "status": "pending_captain_decision"
                }
            }
        }
    });

    apply_project_tool_request_decision(
        &mut runtime,
        "build",
        ToolRequestDecision::Approve,
        &["shell_exec".to_string()],
        Some("Compile locally."),
    )
    .unwrap();

    assert_eq!(runtime["status"], "ready");
    assert_eq!(runtime["resume_pending"]["reason"], "tool_request_approved");
    assert_eq!(runtime["workers"][0]["status"], "ready");
    assert_eq!(runtime["workers"][0]["approved_tools"][0], "shell_exec");
    assert_eq!(runtime["workers"][0]["tool_request"]["status"], "approved");
    assert_eq!(runtime["worker_results"]["build"]["status"], "ready");
    assert_eq!(runtime["worker_results"]["build"]["blocked"], false);
}

#[test]
fn deny_keeps_phase_blocked_without_resume() {
    let mut runtime = serde_json::json!({
        "status": "blocked",
        "current_phase": "build",
        "resume_pending": {"reason": "tool_request_approved", "phase": "build"},
        "workers": [{
            "phase": "build",
            "status": "blocked",
            "tool_request": {
                "tools": ["shell_exec"],
                "status": "pending_captain_decision"
            }
        }]
    });

    apply_project_tool_request_decision(
        &mut runtime,
        "build",
        ToolRequestDecision::Deny,
        &["shell_exec".to_string()],
        None,
    )
    .unwrap();

    assert_eq!(runtime["status"], "blocked");
    assert!(runtime.get("resume_pending").is_none());
    assert_eq!(runtime["workers"][0]["tool_request"]["status"], "denied");
}

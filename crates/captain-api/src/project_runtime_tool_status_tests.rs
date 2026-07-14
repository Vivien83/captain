use super::*;

#[test]
fn approved_tools_for_phase_collects_request_and_worker_tools() {
    let runtime = json!({
        "worker_results": {
            "build": {
                "tool_request": {
                    "status": "approved",
                    "tools": ["shell_exec"]
                }
            }
        },
        "workers": [{
            "phase": "build",
            "approved_tools": ["document_extract", "shell_exec"]
        }]
    });
    let tools = approved_tools_for_phase(&runtime, "build");
    assert_eq!(tools, vec!["document_extract", "shell_exec"]);
}

#[test]
fn tool_decisions_context_warns_about_denied_request() {
    let runtime = json!({
        "worker_results": {
            "build": {
                "tool_request": {
                    "status": "denied",
                    "tools": ["shell_exec"],
                    "decision_reason": "Too risky for this repo."
                }
            }
        }
    });
    let context = tool_decisions_context(&runtime, "build");
    assert!(context.contains("Denied tools for this phase: shell_exec"));
    assert!(context.contains("Too risky for this repo."));
    assert!(context.contains("Do NOT request those tools again"));
}

#[test]
fn prepare_denied_tool_request_retry_reopens_phase_but_keeps_decision() {
    let mut runtime = json!({
        "worker_results": {
            "build": {
                "status": "blocked",
                "blocked": true,
                "tool_request": {
                    "status": "denied",
                    "tools": ["shell_exec"],
                    "decision_reason": "Use a safer path."
                }
            }
        },
        "workers": [{
            "phase": "build",
            "status": "blocked",
            "error": "denied"
        }]
    });
    assert!(prepare_denied_tool_request_retry(&mut runtime, "build"));
    assert_eq!(runtime["worker_results"]["build"]["status"], "ready");
    assert_eq!(runtime["worker_results"]["build"]["blocked"], false);
    assert_eq!(
        runtime["worker_results"]["build"]["tool_request"]["status"],
        "denied"
    );
    assert_eq!(runtime["workers"][0]["status"], "ready");
    assert_eq!(
        runtime["workers"][0]["retry_after_denied_tool_request"],
        true
    );
    assert!(runtime["workers"][0].get("error").is_none());
}

#[test]
fn repeated_denied_tool_request_keeps_request_denied() {
    let runtime = json!({
        "worker_results": {
            "build": {
                "tool_request": {
                    "status": "denied",
                    "tools": ["shell_exec"],
                    "decision_reason": "Use a safer path."
                }
            }
        }
    });
    let request = json!({
        "status": "pending_captain_decision",
        "tools": ["shell_exec", "browser_open"],
        "reason": "Need shell and browser."
    });
    let repeated = repeated_denied_tool_request(&runtime, "build", request);
    assert_eq!(repeated["status"], "denied");
    assert_eq!(repeated["repeat_of_denied_tool_request"], true);
    assert_eq!(repeated["repeated_denied_tools"], json!(["shell_exec"]));
    assert_eq!(repeated["previous_decision_reason"], "Use a safer path.");
    assert!(repeated["decision_reason"]
        .as_str()
        .unwrap()
        .contains("already denied"));
}

#[test]
fn response_declares_blocked_only_from_initial_status_lines() {
    assert!(response_declares_blocked(
        "STATUS: blocked\nSUMMARY: missing approval"
    ));
    assert!(!response_declares_blocked(
        "SUMMARY: ok\nDETAIL: status: blocked appears too late"
    ));
}

#[test]
fn runtime_tool_request_parser_extracts_tools_and_reason() {
    let request = extract_runtime_tool_request(
        "STATUS: blocked\nTOOL_REQUEST: browser_batch, `document_extract`\nREASON: Need to inspect rendered docs.",
    )
    .expect("tool request should parse");
    assert_eq!(request["status"], "pending_captain_decision");
    assert_eq!(
        request["tools"],
        json!(["browser_batch", "document_extract"])
    );
    assert_eq!(request["reason"], "Need to inspect rendered docs.");
}

#[test]
fn runtime_tool_request_parser_returns_none_without_tools() {
    assert!(extract_runtime_tool_request("STATUS: blocked\nREASON: no explicit tools").is_none());
}

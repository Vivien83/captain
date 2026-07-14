use super::*;
use crate::commands::status_project_attention_render::{
    project_attention_action_body_hint, project_attention_action_hint,
    project_attention_action_reason_hint, project_attention_hidden_count,
    project_attention_last_event_hint, project_attention_question_hint,
    project_attention_tool_request_hint, project_attention_worker_hint,
};

fn project_with_runtime(runtime: serde_json::Value) -> captain_memory::project::Project {
    captain_memory::project::Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Ship".to_string(),
        status: captain_memory::project::ProjectStatus::Active,
        deadline: None,
        created_at: 1,
        updated_at: 2,
        metadata: serde_json::json!({ "runtime": runtime }),
    }
}

#[test]
fn project_attention_from_metadata_detects_pending_question() {
    let project = project_with_runtime(serde_json::json!({
        "status": "running",
        "current_phase": "build",
        "progress": 42,
        "user_questions": [{"ask_id": "ask-1", "status": "pending"}],
        "workers": [{"status": "blocked"}, {"status": "ready"}]
    }));
    let attention = project_attention_from_metadata(&project).unwrap();
    assert_eq!(attention["state"], "waiting_for_user");
    assert_eq!(attention["pending_questions"], 1);
    assert_eq!(attention["progress"], 42);
    assert_eq!(attention["workers"]["total"], 2);
    assert_eq!(attention["workers"]["by_status"]["blocked"], 1);
    assert_eq!(attention["workers"]["by_status"]["ready"], 1);
    assert_eq!(attention["project_slug"], "demo");
    assert_eq!(attention["actions"][0]["label"], "answer_question");
    assert_eq!(attention["actions"][0]["body_hint"]["ask_id"], "ask-1");
}

#[test]
fn project_attention_from_metadata_sanitizes_raw_runtime_metadata() {
    let long_ask_id = format!("{}ASK_ID_SECRET_TAIL", "a".repeat(140));
    let long_event_title = format!("{}EVENT_TITLE_SECRET_TAIL", "e".repeat(520));
    let project = project_with_runtime(serde_json::json!({
        "status": "runtime-status-secret",
        "current_phase": "phase-secret",
        "resume_pending": {"reason": "resume-secret-reason"},
        "user_questions": [{
            "ask_id": long_ask_id,
            "phase": "question-phase-secret",
            "status": "pending",
            "question": "Need an answer.",
            "worker_id": "worker-secret-question",
            "agent_id": "agent-secret-question",
            "run_id": "run-secret-question",
            "answer": "stored-answer-secret",
            "metadata": {"raw": "metadata-secret-question"}
        }],
        "worker_results": {
            "tool-phase-secret": {
                "tool_request": {
                    "tools": ["shell_exec", {"raw": "tool-object-secret"}],
                    "reason": "tool-reason-secret",
                    "status": "pending_captain_decision"
                }
            }
        },
        "workers": [
            {"status": "worker-status-secret"},
            {"status": "running"}
        ],
        "timeline": [{
            "id": "event-1",
            "kind": "worker.done",
            "title": long_event_title,
            "phase": "event-phase-secret",
            "status": "event-status-secret",
            "ts": "2026-05-23T10:00:00Z",
            "data": {"secret": "timeline-data-secret"}
        }]
    }));

    let attention = project_attention_from_metadata(&project).unwrap();
    let ask_hint = attention["actions"][0]["body_hint"]["ask_id"]
        .as_str()
        .unwrap();
    let event_title = attention["last_event"]["title"].as_str().unwrap();

    assert_eq!(attention["state"], "waiting_for_user");
    assert_eq!(attention["status"], "ready");
    assert_eq!(attention["phase"], "unknown");
    assert!(attention["resume_pending_reason"].is_null());
    assert_eq!(attention["first_pending_question"]["phase"], "unknown");
    assert!(attention["first_pending_question"]
        .get("worker_id")
        .is_none());
    assert!(attention["first_pending_question"].get("answer").is_none());
    assert_eq!(attention["pending_tool_request"]["phase"], "unknown");
    assert_eq!(
        attention["pending_tool_request"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(attention["workers"]["by_status"]["other"], 1);
    assert_eq!(attention["workers"]["by_status"]["running"], 1);
    assert_eq!(attention["last_event"]["phase"], "unknown");
    assert_eq!(attention["last_event"]["status"], "other");
    assert!(ask_hint.chars().count() <= 120);
    assert!(event_title.chars().count() <= 500);

    let encoded = serde_json::to_string(&attention).unwrap();
    for forbidden in [
        "runtime-status-secret",
        "phase-secret",
        "resume-secret-reason",
        "question-phase-secret",
        "worker-secret-question",
        "agent-secret-question",
        "run-secret-question",
        "stored-answer-secret",
        "metadata-secret-question",
        "tool-phase-secret",
        "tool-object-secret",
        "worker-status-secret",
        "event-phase-secret",
        "event-status-secret",
        "timeline-data-secret",
        "ASK_ID_SECRET_TAIL",
        "EVENT_TITLE_SECRET_TAIL",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn project_attention_from_metadata_detects_pending_tool_request() {
    let project = project_with_runtime(serde_json::json!({
        "status": "blocked",
        "current_phase": "build",
        "worker_results": {
            "build": {
                "tool_request": {
                    "tools": ["shell_exec"],
                    "status": "pending_captain_decision"
                }
            }
        }
    }));
    let attention = project_attention_from_metadata(&project).unwrap();
    assert_eq!(attention["state"], "tool_request_pending");
    assert_eq!(attention["pending_tool_request"]["tools"][0], "shell_exec");
    assert_eq!(attention["actions"][0]["label"], "respond_tool_request");
    assert_eq!(
        attention["actions"][0]["path"],
        "/api/projects/project-1/runtime/tool-request"
    );
}

#[test]
fn project_attention_from_metadata_explains_approved_tool_resume() {
    let project = project_with_runtime(serde_json::json!({
        "status": "ready",
        "current_phase": "build",
        "resume_pending": {
            "reason": "tool_request_approved",
            "phase": "build"
        }
    }));
    let attention = project_attention_from_metadata(&project).unwrap();
    assert_eq!(attention["state"], "resume_ready");
    assert_eq!(attention["resume_pending_reason"], "tool_request_approved");
    assert_eq!(
        attention["summary"],
        "An approved tool request is ready to resume build."
    );
    assert_eq!(attention["actions"][0]["label"], "resume_runtime");
    assert_eq!(
        attention["actions"][0]["reason"],
        "Continue after an approved project tool request."
    );
}

#[test]
fn project_attention_from_metadata_explains_denied_tool_request() {
    let project = project_with_runtime(serde_json::json!({
        "status": "blocked",
        "current_phase": "build",
        "worker_results": {
            "build": {
                "tool_request": {
                    "tools": ["shell_exec"],
                    "status": "denied",
                    "decision_reason": "Too risky."
                }
            }
        }
    }));
    let attention = project_attention_from_metadata(&project).unwrap();
    assert_eq!(attention["state"], "tool_request_denied");
    assert_eq!(attention["denied_tool_request"]["tools"][0], "shell_exec");
    assert_eq!(
        attention["denied_tool_request"]["decision_reason"],
        "Too risky."
    );
    assert_eq!(
        attention["summary"],
        "Operator denied shell_exec; build needs another path."
    );
    assert_eq!(attention["actions"][0]["label"], "resume_runtime");
    assert_eq!(
        attention["actions"][0]["reason"],
        "Continue only after reviewing the denied project tool request."
    );
}

#[test]
fn project_attention_from_metadata_explains_repeated_denied_tool_request() {
    let project = project_with_runtime(serde_json::json!({
        "status": "blocked",
        "current_phase": "build",
        "worker_results": {
            "build": {
                "tool_request": {
                    "tools": ["shell_exec", "browser_open"],
                    "status": "denied",
                    "repeat_of_denied_tool_request": true,
                    "repeated_denied_tools": ["shell_exec"],
                    "decision_reason": "Repeated request."
                }
            }
        }
    }));
    let attention = project_attention_from_metadata(&project).unwrap();
    assert_eq!(attention["state"], "tool_request_denied");
    assert_eq!(
        attention["denied_tool_request"]["repeat_of_denied_tool_request"],
        true
    );
    assert_eq!(
        attention["summary"],
        "Worker repeated denied shell_exec; build still needs another path."
    );
    assert_eq!(attention["actions"][0]["label"], "resume_runtime");
    assert_eq!(
        attention["actions"][0]["reason"],
        "Review the repeated denied tool request; do not approve the same tools without new evidence."
    );
}

#[test]
fn project_attention_from_metadata_marks_failed_project_startable() {
    let project = project_with_runtime(serde_json::json!({
        "status": "failed",
        "current_phase": "verify",
        "timeline": [{
            "id": "event-1",
            "kind": "worker.failed",
            "title": "Verification failed",
            "phase": "verify",
            "status": "failed",
            "ts": "2026-05-22T20:00:00Z"
        }]
    }));

    let attention = project_attention_from_metadata(&project).unwrap();

    assert_eq!(attention["state"], "failed");
    assert_eq!(attention["last_event"]["title"], "Verification failed");
    assert_eq!(attention["actions"][0]["label"], "start_runtime");
    assert_eq!(
        attention["actions"][0]["path"],
        "/api/projects/project-1/runtime/start"
    );
}

#[test]
fn project_attention_action_hint_formats_first_api_action() {
    let item = serde_json::json!({
        "actions": [{
            "label": "resume_runtime",
            "method": "POST",
            "path": "/api/projects/project-1/runtime/start"
        }]
    });

    assert_eq!(
        project_attention_action_hint(&item).unwrap(),
        "resume_runtime POST /api/projects/project-1/runtime/start"
    );
}

#[test]
fn project_attention_action_hint_ignores_missing_action_path() {
    let item = serde_json::json!({
        "actions": [{"label": "resume_runtime", "method": "POST"}]
    });

    assert!(project_attention_action_hint(&item).is_none());
}

#[test]
fn project_attention_action_body_hint_formats_first_action_body() {
    let item = serde_json::json!({
        "actions": [{
            "label": "answer_question",
            "method": "POST",
            "path": "/api/projects/project-1/runtime/answer",
            "body_hint": {"ask_id": "ask-1", "answer": "..."}
        }]
    });

    assert_eq!(
        project_attention_action_body_hint(&item).unwrap(),
        r#"{"answer":"...","ask_id":"ask-1"}"#
    );
}

#[test]
fn project_attention_action_reason_hint_formats_first_action_reason() {
    let item = serde_json::json!({
        "actions": [{
            "label": "resume_runtime",
            "method": "POST",
            "path": "/api/projects/project-1/runtime/resume",
            "reason": "Continue after an approved project tool request."
        }]
    });

    assert_eq!(
        project_attention_action_reason_hint(&item).unwrap(),
        "Continue after an approved project tool request."
    );
}

#[test]
fn project_attention_hidden_count_handles_bounded_and_legacy_lists() {
    assert_eq!(project_attention_hidden_count(8, Some(11)), 3);
    assert_eq!(project_attention_hidden_count(11, None), 3);
    assert_eq!(project_attention_hidden_count(3, Some(3)), 0);
}

#[test]
fn project_attention_question_hint_formats_pending_question_options() {
    let item = serde_json::json!({
        "first_pending_question": {
            "ask_id": "ask-abcdef",
            "question": "Which build path should the worker use?",
            "options": ["Use local compile", "Skip compile"]
        }
    });

    assert_eq!(
        project_attention_question_hint(&item).unwrap(),
        "[ask-a...] Which build path should the worker use? | options: 1. Use local compile / 2. Skip compile"
    );
}

#[test]
fn project_attention_tool_request_hint_formats_pending_request_reason() {
    let item = serde_json::json!({
        "pending_tool_request": {
            "phase": "build",
            "tools": ["shell_exec", "file_write"],
            "reason": "Need to compile and patch generated code.",
            "status": "pending_captain_decision"
        }
    });

    assert_eq!(
        project_attention_tool_request_hint(&item).unwrap(),
        "pending_captain_decision -- phase build -- shell_exec, file_write -- Need to compile and patch generated code."
    );
}

#[test]
fn project_attention_tool_request_hint_formats_repeated_denial() {
    let item = serde_json::json!({
        "denied_tool_request": {
            "phase": "verify",
            "tools": ["shell_exec"],
            "decision_reason": "Use the existing test output instead.",
            "status": "denied",
            "repeat_of_denied_tool_request": true
        }
    });

    assert_eq!(
        project_attention_tool_request_hint(&item).unwrap(),
        "denied -- phase verify -- shell_exec -- repeated denied request -- Use the existing test output instead."
    );
}

#[test]
fn project_attention_worker_hint_formats_progress_and_status_counts() {
    let item = serde_json::json!({
        "progress": 42,
        "workers": {
            "total": 3,
            "by_status": {
                "blocked": 1,
                "ready": 2
            }
        }
    });

    assert_eq!(
        project_attention_worker_hint(&item).unwrap(),
        "progress 42% -- workers 3 (blocked 1, ready 2)"
    );
}

#[test]
fn project_attention_last_event_hint_formats_runtime_event() {
    let item = serde_json::json!({
        "last_event": {
            "kind": "worker.ask_user",
            "title": "Builder needs user direction",
            "phase": "build",
            "status": "waiting_user"
        }
    });

    assert_eq!(
        project_attention_last_event_hint(&item).unwrap(),
        "Builder needs user direction -- phase build -- waiting_user"
    );
}

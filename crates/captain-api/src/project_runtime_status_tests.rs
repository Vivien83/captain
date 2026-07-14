use super::*;

fn project() -> project::Project {
    project::Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Ship".to_string(),
        status: project::ProjectStatus::Active,
        deadline: None,
        created_at: 1,
        updated_at: 1,
        metadata: json!({}),
    }
}

#[test]
fn status_prioritizes_pending_user_question() {
    let runtime = json!({
        "status": "running",
        "current_phase": "build",
        "user_questions": [{
            "ask_id": "ask-1",
            "status": "pending",
            "question": "Which path?"
        }],
        "workers": [{"status": "blocked"}],
    });
    let status = project_runtime_operator_status(&project(), &runtime, true);
    assert_eq!(status["state"], "waiting_for_user");
    assert_eq!(status["pending_questions"], 1);
    assert_eq!(status["updated_at"], 1);
    assert_eq!(status["actions"][0]["label"], "answer_question");
}

#[test]
fn status_sanitizes_pending_user_question_payload() {
    let runtime = json!({
        "status": "running",
        "current_phase": "build",
        "user_questions": [{
            "ask_id": "ask-1",
            "phase": "build",
            "worker_role": "planner",
            "status": "pending",
            "delivery": "web",
            "question": "Which path?",
            "options": ["Fast", "Safe"],
            "worker_id": "worker-secret-question",
            "agent_id": "agent-secret-question",
            "run_id": "run-secret-question",
            "answer": "stored-answer-secret",
            "metadata": {"raw": "metadata-secret-question"},
        }],
    });

    let status = project_runtime_operator_status(&project(), &runtime, true);
    let question = &status["first_pending_question"];

    assert_eq!(status["state"], "waiting_for_user");
    assert_eq!(question["ask_id"], "ask-1");
    assert_eq!(question["role"], "planner");
    assert_eq!(question["question"], "Which path?");
    assert_eq!(question["options"][0], "Fast");
    assert!(question.get("worker_id").is_none());
    assert!(question.get("agent_id").is_none());
    assert!(question.get("run_id").is_none());
    assert!(question.get("answer").is_none());
    assert!(question.get("metadata").is_none());

    let encoded = serde_json::to_string(&status).unwrap();
    for forbidden in [
        "worker-secret-question",
        "agent-secret-question",
        "run-secret-question",
        "stored-answer-secret",
        "metadata-secret-question",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn status_sanitizes_top_level_actions_counts_and_last_event() {
    let long_ask_id = format!("{}ASK_ID_SECRET_TAIL", "a".repeat(140));
    let long_event_title = format!("{}EVENT_TITLE_SECRET_TAIL", "e".repeat(520));
    let runtime = json!({
        "status": "runtime-status-secret",
        "current_phase": "phase-secret",
        "resume_pending": {"reason": "resume-secret-reason"},
        "user_questions": [{
            "ask_id": long_ask_id,
            "phase": "question-phase-secret",
            "status": "pending",
            "question": "Need an answer."
        }],
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
    });

    let status = project_runtime_operator_status(&project(), &runtime, false);
    let ask_hint = status["actions"][0]["body_hint"]["ask_id"]
        .as_str()
        .unwrap();
    let event_title = status["last_event"]["title"].as_str().unwrap();

    assert_eq!(status["state"], "waiting_for_user");
    assert_eq!(status["status"], "ready");
    assert_eq!(status["phase"], "unknown");
    assert!(status["resume_pending_reason"].is_null());
    assert_eq!(status["first_pending_question"]["phase"], "unknown");
    assert_eq!(status["workers"]["by_status"]["other"], 1);
    assert_eq!(status["workers"]["by_status"]["running"], 1);
    assert_eq!(status["last_event"]["phase"], "unknown");
    assert_eq!(status["last_event"]["status"], "other");
    assert!(ask_hint.chars().count() <= 120);
    assert!(event_title.chars().count() <= 500);

    let encoded = serde_json::to_string(&status).unwrap();
    for forbidden in [
        "runtime-status-secret",
        "phase-secret",
        "resume-secret-reason",
        "question-phase-secret",
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
fn status_marks_stale_active_runtime_without_process() {
    let runtime = json!({
        "status": "running",
        "current_phase": "verify",
        "orchestrator": {"active": true},
        "workers": [{"status": "done"}],
    });
    let status = project_runtime_operator_status(&project(), &runtime, false);
    assert_eq!(status["state"], "stale_active");
    assert_eq!(status["running_in_process"], false);
    assert_eq!(status["actions"][0]["label"], "resume_runtime");
}

#[test]
fn status_marks_answered_question_ready_to_resume() {
    let runtime = json!({
        "status": "ready",
        "current_phase": "execute",
        "resume_pending": {"reason": "project_ask_answered"},
        "user_questions": [{"ask_id": "ask-1", "status": "answered"}],
    });
    let status = project_runtime_operator_status(&project(), &runtime, false);
    assert_eq!(status["state"], "resume_ready");
    assert_eq!(status["resume_pending"], true);
    assert_eq!(status["resume_pending_reason"], "project_ask_answered");
    assert_eq!(
        status["summary"],
        "A user answer is stored; phase execute is ready to resume."
    );
    assert_eq!(status["actions"][0]["label"], "resume_runtime");
    assert_eq!(
        status["actions"][0]["reason"],
        "Continue after a persisted project answer."
    );
}

#[test]
fn status_marks_approved_tool_request_ready_to_resume() {
    let runtime = json!({
        "status": "ready",
        "current_phase": "build",
        "resume_pending": {"reason": "tool_request_approved", "phase": "build"},
        "worker_results": {
            "build": {
                "status": "blocked",
                "tool_request": {
                    "tools": ["shell_exec"],
                    "status": "approved"
                }
            }
        },
    });
    let status = project_runtime_operator_status(&project(), &runtime, false);
    assert_eq!(status["state"], "resume_ready");
    assert_eq!(status["resume_pending_reason"], "tool_request_approved");
    assert_eq!(
        status["summary"],
        "An approved tool request is stored; phase build is ready to resume."
    );
    assert_eq!(
        status["actions"][0]["reason"],
        "Continue after an approved project tool request."
    );
}

#[test]
fn status_marks_pending_tool_request_actionable() {
    let runtime = json!({
        "status": "blocked",
        "current_phase": "build",
        "worker_results": {
            "build": {
                "id": "worker-secret-tool",
                "worker_id": "worker-secret-tool-alt",
                "agent_id": "agent-secret-tool",
                "status": "blocked",
                "tool_request": {
                    "tools": ["shell_exec", "file_write", {"raw": "tool-object-secret"}],
                    "reason": "Need to compile the generated code.",
                    "status": "pending_captain_decision"
                }
            }
        },
        "workers": [{"phase": "build", "status": "blocked"}],
    });
    let status = project_runtime_operator_status(&project(), &runtime, false);
    assert_eq!(status["state"], "tool_request_pending");
    assert_eq!(status["pending_tool_request"]["phase"], "build");
    assert_eq!(status["pending_tool_request"]["tools"][0], "shell_exec");
    assert_eq!(
        status["pending_tool_request"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(status["pending_tool_request"].get("worker_id").is_none());
    assert!(status["pending_tool_request"].get("agent_id").is_none());
    assert_eq!(status["actions"][0]["label"], "respond_tool_request");
    assert_eq!(
        status["actions"][0]["body_hint"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(status["actions"].as_array().unwrap().len(), 1);
    assert!(project_runtime_needs_operator_attention(&status));
    let encoded = serde_json::to_string(&status).unwrap();
    for forbidden in [
        "worker-secret-tool",
        "worker-secret-tool-alt",
        "agent-secret-tool",
        "tool-object-secret",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn status_marks_denied_tool_request_actionable() {
    let runtime = json!({
        "status": "blocked",
        "current_phase": "build",
        "worker_results": {
            "build": {
                "id": "worker-secret-denied",
                "agent_id": "agent-secret-denied",
                "status": "blocked",
                "tool_request": {
                    "tools": ["shell_exec"],
                    "reason": "Need to compile the generated code.",
                    "decision_reason": "Too risky for this project.",
                    "status": "denied",
                    "decided_by": "operator",
                    "decided_at": "2026-05-21T19:30:00Z",
                    "previous_denied_tool_request": {
                        "agent_id": "previous-agent-secret",
                        "reason": "previous-raw-secret"
                    }
                }
            }
        },
    });
    let status = project_runtime_operator_status(&project(), &runtime, false);
    assert_eq!(status["state"], "tool_request_denied");
    assert!(status["pending_tool_request"].is_null());
    assert_eq!(status["denied_tool_request"]["phase"], "build");
    assert_eq!(status["denied_tool_request"]["tools"][0], "shell_exec");
    assert_eq!(
        status["denied_tool_request"]["decision_reason"],
        "Too risky for this project."
    );
    assert!(status["denied_tool_request"].get("worker_id").is_none());
    assert!(status["denied_tool_request"].get("agent_id").is_none());
    assert!(status["denied_tool_request"]
        .get("previous_denied_tool_request")
        .is_none());
    assert_eq!(
        status["summary"],
        "The operator denied shell_exec for phase build; the project needs a different path or manual review."
    );
    assert_eq!(status["actions"][0]["label"], "resume_runtime");
    assert_eq!(
        status["actions"][0]["reason"],
        "Continue only after reviewing the denied project tool request."
    );
    assert!(project_runtime_needs_operator_attention(&status));
    let encoded = serde_json::to_string(&status).unwrap();
    for forbidden in [
        "worker-secret-denied",
        "agent-secret-denied",
        "previous-agent-secret",
        "previous-raw-secret",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn status_marks_repeated_denied_tool_request_without_new_approval() {
    let runtime = json!({
        "status": "blocked",
        "current_phase": "build",
        "worker_results": {
            "build": {
                "status": "blocked",
                "tool_request": {
                    "tools": ["shell_exec", "browser_open"],
                    "reason": "Still need shell.",
                    "decision_reason": "Repeated request for tools already denied.",
                    "status": "denied",
                    "repeat_of_denied_tool_request": true,
                    "repeated_denied_tools": ["shell_exec"],
                    "previous_decision_reason": "Use a safer path."
                }
            }
        },
    });
    let status = project_runtime_operator_status(&project(), &runtime, false);
    assert_eq!(status["state"], "tool_request_denied");
    assert_eq!(
        status["denied_tool_request"]["repeat_of_denied_tool_request"],
        true
    );
    assert_eq!(
        status["denied_tool_request"]["repeated_denied_tools"][0],
        "shell_exec"
    );
    assert_eq!(
        status["summary"],
        "A worker repeated a denied request for shell_exec in phase build; Captain kept it denied instead of asking again."
    );
    assert_eq!(
        status["actions"][0]["reason"],
        "Review the repeated denied tool request; do not approve the same tools without new evidence."
    );
}

#[test]
fn attention_filter_keeps_only_operator_action_states() {
    let runtime = json!({"status": "ready", "current_phase": "observe"});
    let ready = project_runtime_operator_status(&project(), &runtime, false);
    assert!(!project_runtime_needs_operator_attention(&ready));

    let runtime = json!({"status": "failed", "current_phase": "verify"});
    let failed = project_runtime_operator_status(&project(), &runtime, false);
    assert!(project_runtime_needs_operator_attention(&failed));
}

#[test]
fn attention_priority_keeps_direct_waits_before_terminal_failures() {
    let waiting = json!({"state": "waiting_for_user"});
    let tool_request = json!({"state": "tool_request_pending"});
    let resume = json!({"state": "resume_ready"});
    let failed = json!({"state": "failed"});
    let blocked = json!({"state": "blocked"});

    assert!(
        project_runtime_attention_priority(&waiting)
            < project_runtime_attention_priority(&tool_request)
    );
    assert!(
        project_runtime_attention_priority(&tool_request)
            < project_runtime_attention_priority(&resume)
    );
    assert!(
        project_runtime_attention_priority(&resume) < project_runtime_attention_priority(&failed)
    );
    assert!(
        project_runtime_attention_priority(&failed) < project_runtime_attention_priority(&blocked)
    );
}

#[test]
fn limit_project_runtime_attention_keeps_total_before_truncation() {
    let mut items = (0..10)
        .map(|idx| {
            json!({
                "state": "blocked",
                "updated_at": idx,
                "project_slug": format!("blocked-{idx}")
            })
        })
        .collect::<Vec<_>>();

    let total = limit_project_runtime_attention_items(&mut items);

    assert_eq!(total, 10);
    assert_eq!(items.len(), 8);
    assert_eq!(items[0]["project_slug"], "blocked-9");
}

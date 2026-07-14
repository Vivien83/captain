use super::*;

#[test]
fn replay_view_limits_and_omits_raw_payloads() {
    let body = json!({
        "project": {"id": "project-1", "slug": "demo"},
        "operator_status": {
            "project_id": "project-1",
            "project_slug": "demo",
            "state": "waiting_for_user",
            "phase": "build",
            "progress": 60,
            "workers": {"total": 1},
            "summary": "Waiting on a bounded user answer.",
            "actions": [{
                "label": "answer_question",
                "body_hint": {"ask_id": "ask-1", "answer": "..."}
            }]
        },
        "runtime": {
            "workers": [{
                "id": "demo-build",
                "role": "Builder",
                "phase": "build",
                "status": "blocked",
                "task": "raw phase task do-not-print",
                "prompt": "raw prompt do-not-print",
                "agent_id": "agent-secret",
                "run_id": "run-secret",
                "tool_request": {
                    "status": "pending",
                    "tools": ["shell_exec"],
                    "reason": "raw reason do-not-print"
                },
                "cost_usd": 0.25,
                "tool_decisions": [{
                    "tool": "shell_exec",
                    "reason": "Run the build command.",
                    "status": "ok",
                    "duration_ms": 7,
                    "input_summary": "raw input do-not-print",
                    "output_summary": "raw output do-not-print"
                }]
            }],
            "worker_results": {
                "build": {
                    "summary": "STATUS: blocked\nSUMMARY: waiting on user",
                    "answer": "worker answer do-not-print"
                }
            },
            "user_questions": [{
                "ask_id": "ask-1",
                "phase": "build",
                "worker_role": "Builder",
                "status": "pending",
                "question": "Which path?",
                "options": ["Simple", "Complex"],
                "answer": "private answer do-not-print",
                "agent_id": "agent-secret",
                "run_id": "run-secret"
            }],
            "timeline": [{
                "id": "fallback",
                "ts": "2026-05-23T10:00:00Z",
                "kind": "runtime.only",
                "title": "Fallback",
                "detail": "old",
                "phase": "observe",
                "status": "ready",
                "data": {"secret": "do-not-print"}
            }]
        },
        "transcript": {
            "session_id": "session-1",
            "count": 2,
            "stored_count": 2,
            "truncated": false,
            "events": [
                {"id": "event-1", "ts": "2026-05-23T10:01:00Z", "kind": "one", "title": "One", "detail": "first", "phase": "observe", "status": "done", "data": {"secret": "do-not-print"}},
                {"id": "event-2", "ts": "2026-05-23T10:02:00Z", "kind": "two", "title": "Two", "detail": "second", "phase": "build", "status": "blocked", "actor": "agent-secret"}
            ]
        }
    });

    let replay = project_replay_view(&body, "demo", 1, 1);
    let rendered = serde_json::to_string(&replay).unwrap();

    assert_eq!(replay["events"]["count"], 1);
    assert_eq!(replay["events"]["items"][0]["id"], "event-2");
    assert_eq!(replay["pending_questions"]["total"], 1);
    assert_eq!(
        replay["workers"]["items"][0]["summary"],
        "STATUS: blocked\nSUMMARY: waiting on user"
    );
    assert_eq!(
        replay["workers"]["items"][0]["tool_request"]["tools"][0],
        "shell_exec"
    );
    assert_eq!(replay["workers"]["items"][0]["cost_usd"], 0.25);
    assert_eq!(
        replay["workers"]["items"][0]["tool_decisions"][0]["tool"],
        "shell_exec"
    );
    assert_eq!(
        replay["workers"]["items"][0]["tool_decisions"][0]["reason"],
        "Run the build command."
    );
    assert_eq!(
        replay["pending_questions"]["items"][0]["next_action"],
        "captain project answer demo --ask-id ask-1 --answer \"...\""
    );
    assert_eq!(
        replay["next_actions"][0],
        "captain project answer demo --ask-id ask-1 --answer \"...\""
    );
    assert!(!rendered.contains("raw phase task"));
    assert!(!rendered.contains("raw prompt"));
    assert!(!rendered.contains("private answer"));
    assert!(!rendered.contains("worker answer"));
    assert!(!rendered.contains("raw input"));
    assert!(!rendered.contains("raw output"));
    assert!(!rendered.contains("agent-secret"));
    assert!(!rendered.contains("run-secret"));
    assert!(!rendered.contains("do-not-print"));
    assert!(!rendered.contains("\"data\""));
    assert!(!rendered.contains("\"actor\""));
}

#[test]
fn replay_lines_are_compact() {
    let worker = json!({
        "phase": "verify",
        "status": "done",
        "role": "Verifier",
        "tool_calls": 4,
        "cost_usd": 0.25,
        "tool_decisions": [{
            "tool": "shell_exec",
            "reason": "Run verification checks.",
            "status": "ok",
            "duration_ms": 7
        }],
        "tool_request": {"tools": ["shell_exec"]},
        "summary": "STATUS: complete"
    });
    let question = json!({
        "ask_id": "ask-1",
        "phase": "build",
        "worker_role": "Builder",
        "question": "Which path?"
    });
    let event = json!({
        "ts": "2026-05-23T10:00:00Z",
        "phase": "verify",
        "status": "done",
        "kind": "worker.completed",
        "title": "Verifier completed",
        "detail": "All checks passed"
    });

    assert_eq!(
        replay_worker_line(&worker),
        "      verify -- done -- Verifier -- tool_calls 4 -- cost $0.2500 -- decisions shell_exec [ok, 7ms]: Run verification checks. -- needs shell_exec -- STATUS: complete"
    );
    assert_eq!(
        replay_question_line(&question),
        "      ask-1 -- build -- Builder: Which path?"
    );
    assert!(replay_event_line(&event).contains("worker.completed"));
}

#[test]
fn replay_runtime_url_sends_clamped_event_limit() {
    assert_eq!(
        project_replay_runtime_url("http://127.0.0.1:50051", "demo", 0),
        "http://127.0.0.1:50051/api/projects/demo/runtime?events=1"
    );
    assert_eq!(
        project_replay_runtime_url("http://127.0.0.1:50051", "demo", 999),
        "http://127.0.0.1:50051/api/projects/demo/runtime?events=80"
    );
}

#[test]
fn replay_workers_limit_keeps_actionable_and_recent_tail() {
    let mut workers = vec![json!({
        "id": "worker-running-old",
        "role": "Builder",
        "phase": "build",
        "status": "running"
    })];
    workers.extend((0..12).map(|idx| {
        json!({
            "id": format!("worker-done-{idx}"),
            "role": "Worker",
            "phase": "verify",
            "status": "done"
        })
    }));
    let body = json!({
        "project": {"id": "project-1", "slug": "demo"},
        "runtime": {"workers": workers}
    });

    let replay = project_replay_view(&body, "demo", 20, 4);
    let ids = replay["workers"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|worker| worker["id"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(replay["workers"]["count"], 4);
    assert_eq!(replay["workers"]["total"], 13);
    assert_eq!(replay["workers"]["truncated"], true);
    assert!(ids.contains(&"worker-running-old"));
    assert!(!ids.contains(&"worker-done-4"));
    assert!(ids.contains(&"worker-done-11"));
}

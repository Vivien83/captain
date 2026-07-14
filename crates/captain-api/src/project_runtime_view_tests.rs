use super::*;

fn project() -> project::Project {
    project::Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Ship without leaking metadata".to_string(),
        status: project::ProjectStatus::Active,
        deadline: None,
        created_at: 1,
        updated_at: 2,
        metadata: json!({
            "runtime": {"secret": "metadata-runtime-secret"},
            "workspace": {"path": "/private/secret-path"}
        }),
    }
}

#[test]
fn project_runtime_project_view_omits_metadata() {
    let view = project_runtime_project_view(&project());

    assert_eq!(view["id"], "project-1");
    assert_eq!(view["slug"], "demo");
    assert!(view.get("metadata").is_none());

    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("metadata-runtime-secret"));
    assert!(!encoded.contains("secret-path"));
}

#[test]
fn project_source_view_omits_local_paths_and_clone_urls() {
    let view = project_source_view(&json!({
        "type": "github",
        "full_name": "owner/repo",
        "branch": "main",
        "path": "/private/source",
        "local_path": "/private/local",
        "clone_url": "https://token-secret@example.test/owner/repo.git"
    }));

    assert_eq!(view["type"], "github");
    assert_eq!(view["full_name"], "owner/repo");
    assert!(view.get("path").is_none());
    assert!(view.get("local_path").is_none());
    assert!(view.get("clone_url").is_none());

    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("/private/source"));
    assert!(!encoded.contains("token-secret"));
}

#[test]
fn project_workspace_view_omits_rules_file_path() {
    let view = project_workspace_view(&json!({
        "path": "/private/workspace",
        "default_root": "/private/default-root",
        "authorized": true,
        "rules_file": "/private/workspace/AGENTS.md"
    }));

    assert_eq!(view["authorized"], true);
    assert!(view.get("path").is_none());
    assert!(view.get("default_root").is_none());
    assert!(view.get("rules_file").is_none());
    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("/private/workspace"));
    assert!(!encoded.contains("/private/default-root"));
    assert!(!encoded.contains("AGENTS.md"));
}

#[test]
fn project_runtime_view_omits_raw_worker_question_and_event_payloads() {
    let long_event_title = format!("{}EVENT_TITLE_SECRET_TAIL", "e".repeat(520));
    let runtime = json!({
        "protocol": "captain.project_runtime.v2",
        "generation": 2,
        "status": "runtime-status-secret",
        "current_phase": "phase-secret",
        "progress": 143,
        "orchestrator": {"active": true, "run_id": "run-secret"},
        "manager_agent": {
            "id": "manager-agent-secret",
            "name": "captain",
            "model": "codex"
        },
        "parallelism": {"running": 1, "max_parallel_agents": 2},
        "resume_pending": {"reason": "resume-secret-reason", "phase": "resume-phase-secret"},
        "user_questions": [{
            "ask_id": "ask-1",
            "phase": "question-phase-secret",
            "worker_role": "planner",
            "status": "pending",
            "question": "Choose path",
            "options": ["Safe", {"raw": "option-object-secret"}],
            "answer": "answer-secret",
            "worker_id": "worker-question-secret",
            "agent_id": "agent-question-secret",
            "run_id": "run-question-secret",
            "metadata": {"raw": "question-metadata-secret"}
        }],
        "workers": [{
            "id": "worker-1",
            "role": "builder",
            "phase": "worker-phase-secret",
            "status": "worker-status-secret",
            "mode": "parallel",
            "agent_id": "agent-1",
            "task": "task-body-secret",
            "prompt": "prompt-secret",
            "dependencies": ["dependency-secret"],
            "authorized_tools": ["shell_exec", {"raw": "tool-object-secret"}],
            "summary": "Build summary",
            "tool_request": {
                "tools": ["shell_exec", {"raw": "tool-request-object-secret"}],
                "reason": "Need shell",
                "status": "pending"
            }
        }],
        "worker_results": {
            "build": {
                "status": "done",
                "summary": "Result summary",
                "output": {"secret": "result-output-secret"}
            },
            "result-phase-secret": {
                "status": "result-status-secret",
                "summary": "Unknown result secret",
                "tool_request": {
                    "tools": ["browser_open"],
                    "previous_denied_tool_request": {"secret": "previous-request-secret"}
                }
            }
        },
        "timeline": [{
            "id": "event-1",
            "kind": "worker.done",
            "title": long_event_title,
            "detail": "Event detail",
            "phase": "event-phase-secret",
            "status": "event-status-secret",
            "data": {"secret": "event-data-secret"}
        }]
    });

    let view = project_runtime_view(&runtime);

    assert_eq!(view["status"], "ready");
    assert_eq!(view["current_phase"], "unknown");
    assert_eq!(view["progress"], 100);
    assert_eq!(view["orchestrator"]["active"], true);
    assert!(view["orchestrator"].get("run_id").is_none());
    assert_eq!(view["manager_agent"]["name"], "captain");
    assert!(view["manager_agent"].get("id").is_none());
    assert!(view["resume_pending"]["reason"].is_null());
    assert_eq!(view["user_questions"][0]["phase"], "unknown");
    assert!(view["user_questions"][0].get("answer").is_none());
    assert_eq!(view["workers"][0]["phase"], "unknown");
    assert_eq!(view["workers"][0]["status"], "other");
    assert!(view["workers"][0].get("task").is_none());
    assert_eq!(
        view["workers"][0]["authorized_tools"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(view["worker_results"].get("build").is_some());
    assert!(view["worker_results"].get("unknown").is_none());
    assert_eq!(view["timeline"][0]["phase"], "unknown");
    assert_eq!(view["timeline"][0]["status"], "other");
    assert!(view["timeline"][0].get("data").is_none());
    assert!(
        view["timeline"][0]["title"]
            .as_str()
            .unwrap()
            .chars()
            .count()
            <= 500
    );

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "runtime-status-secret",
        "phase-secret",
        "run-secret",
        "manager-agent-secret",
        "resume-secret-reason",
        "resume-phase-secret",
        "question-phase-secret",
        "option-object-secret",
        "answer-secret",
        "worker-question-secret",
        "agent-question-secret",
        "run-question-secret",
        "question-metadata-secret",
        "worker-phase-secret",
        "worker-status-secret",
        "task-body-secret",
        "prompt-secret",
        "dependency-secret",
        "tool-object-secret",
        "tool-request-object-secret",
        "result-phase-secret",
        "result-status-secret",
        "Unknown result secret",
        "result-output-secret",
        "previous-request-secret",
        "event-phase-secret",
        "event-status-secret",
        "event-data-secret",
        "EVENT_TITLE_SECRET_TAIL",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn safe_runtime_events_strip_data_payloads() {
    let events = vec![json!({
        "id": "event-1",
        "title": "Visible",
        "detail": "Small detail",
        "data": {"secret": "event-data-secret"}
    })];

    let safe = safe_runtime_events(events);

    assert_eq!(safe[0]["title"], "Visible");
    assert!(safe[0].get("data").is_none());
    let encoded = serde_json::to_string(&safe).unwrap();
    assert!(!encoded.contains("event-data-secret"));
}

#[test]
fn project_runtime_view_limits_timeline_to_recent_tail() {
    let timeline = (0..105)
        .map(|idx| {
            json!({
                "id": format!("event-{idx}"),
                "ts": format!("2026-05-24T10:{idx:02}:00Z"),
                "title": format!("Event {idx}")
            })
        })
        .collect::<Vec<_>>();
    let runtime = json!({ "timeline": timeline });

    let view = project_runtime_view(&runtime);
    let events = view["timeline"].as_array().unwrap();

    assert_eq!(events.len(), RUNTIME_TIMELINE_VIEW_LIMIT);
    assert_eq!(events[0]["id"], "event-5");
    assert_eq!(events[99]["id"], "event-104");
}

#[test]
fn project_runtime_view_limits_questions_but_keeps_pending() {
    let mut questions = vec![json!({
        "ask_id": "ask-pending-old",
        "status": "pending",
        "question": "Still needs an answer"
    })];
    questions.extend((0..25).map(|idx| {
        json!({
            "ask_id": format!("ask-answered-{idx}"),
            "status": "answered",
            "question": format!("Answered {idx}")
        })
    }));
    let runtime = json!({ "user_questions": questions });

    let view = project_runtime_view(&runtime);
    let questions = view["user_questions"].as_array().unwrap();
    let ids = questions
        .iter()
        .filter_map(|question| question["ask_id"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(questions.len(), RUNTIME_QUESTION_VIEW_LIMIT);
    assert!(ids.contains(&"ask-pending-old"));
    assert!(!ids.contains(&"ask-answered-5"));
    assert!(ids.contains(&"ask-answered-24"));
}

#[test]
fn project_runtime_view_limits_workers_but_keeps_actionable() {
    let mut workers = vec![json!({
        "id": "worker-running-old",
        "status": "running",
        "phase": "build",
        "summary": "Still running"
    })];
    workers.extend((0..55).map(|idx| {
        json!({
            "id": format!("worker-done-{idx}"),
            "status": "done",
            "phase": "verify",
            "summary": format!("Done {idx}")
        })
    }));
    let runtime = json!({ "workers": workers });

    let view = project_runtime_view(&runtime);
    let workers = view["workers"].as_array().unwrap();
    let ids = workers
        .iter()
        .filter_map(|worker| worker["id"].as_str())
        .collect::<Vec<_>>();

    assert_eq!(workers.len(), RUNTIME_WORKER_VIEW_LIMIT);
    assert!(ids.contains(&"worker-running-old"));
    assert!(!ids.contains(&"worker-done-5"));
    assert!(ids.contains(&"worker-done-54"));
}

// Captain Control's Preact views (Projects.js/ProjectRuntime.js) replaced
// roadmap.js as the web UI for projects and their runtime. These guards
// moved with it: both views only ever consume the sanitized `/resume` and
// `/runtime` views (see project_runtime_view.rs's safe_* builders), never
// raw project metadata.
#[test]
fn projects_web_does_not_read_legacy_raw_project_metadata() {
    let sources = [
        include_str!("../static/js/app/views/Projects.js"),
        include_str!("../static/js/app/views/ProjectRuntime.js"),
    ];

    for forbidden in [
        "project.metadata",
        "metadata.runtime",
        "metadata.launch",
        "worker.task",
        "goal.check_command",
        "goal.recovery_command",
        "env.project_root",
        "env.workspaces_dir",
        "/api/projects/environment",
        "repo.clone_url",
        "github_clone_url",
    ] {
        for source in sources {
            assert!(!source.contains(forbidden), "web reads {forbidden}");
        }
    }
}

#[test]
fn projects_web_reads_bounded_recent_runtime_transcript() {
    let api_source = include_str!("../static/js/app/api.js");
    let runtime_source = include_str!("../static/js/app/views/ProjectRuntime.js");

    assert!(
        api_source.contains("events = 80") && api_source.contains("?events=${events}"),
        "web Projects runtime fetch must pass an explicit event window"
    );
    assert!(
        runtime_source.contains("runtime.timeline"),
        "ProjectRuntime.js must render the runtime timeline"
    );
}

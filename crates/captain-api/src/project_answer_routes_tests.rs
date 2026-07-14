use super::*;
use axum::body::to_bytes;
use axum::response::IntoResponse;
use captain_kernel::CaptainKernel;
use captain_memory::project;
use captain_types::config::{DefaultModelConfig, KernelConfig};
use std::time::Instant;

fn test_state() -> (tempfile::TempDir, AppState) {
    let tmp = tempfile::tempdir().unwrap();
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    let kernel = std::sync::Arc::new(CaptainKernel::boot_with_config(config).unwrap());
    kernel.set_self_handle();
    let state = AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: std::sync::Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        ask_user_channels: dashmap::DashMap::new(),
        provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
    };
    (tmp, state)
}

async fn response_body(response: axum::response::Response) -> String {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

async fn response_json(response: axum::response::Response) -> serde_json::Value {
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn project_with_runtime(runtime: serde_json::Value) -> project::Project {
    project::Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Ship".to_string(),
        status: project::ProjectStatus::Active,
        deadline: None,
        created_at: 1,
        updated_at: 2,
        metadata: serde_json::json!({ "runtime": runtime }),
    }
}

fn pending_question_runtime() -> serde_json::Value {
    serde_json::json!({
        "status": "blocked",
        "current_phase": "build",
        "control": {"paused": true, "takeover": false},
        "timeline": [],
        "workers": [{
            "id": "worker-build",
            "phase": "build",
            "status": "blocked",
            "error": "Waiting for user"
        }],
        "worker_results": {
            "build": {
                "status": "blocked",
                "blocked": true
            }
        },
        "user_questions": [{
            "ask_id": "ask-abcdef",
            "run_id": "run-1",
            "phase": "build",
            "worker_id": "worker-build",
            "agent_id": "agent-build",
            "worker_role": "builder",
            "question": "Which runtime?",
            "status": "pending",
            "delivery": "waiting_for_user"
        }]
    })
}

#[tokio::test]
async fn answer_project_ask_rejects_invalid_project_identifier_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = answer_project_ask(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
        axum::Json(AnswerProjectAskReq {
            ask_id: "ask-1".to_string(),
            answer: "done".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = response_body(response).await;
    assert!(body.contains("project identifier is invalid"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}

#[tokio::test]
async fn answer_project_ask_missing_project_does_not_echo_identifier() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = answer_project_ask(
        axum::extract::State(state),
        axum::extract::Path("missing-project".to_string()),
        axum::Json(AnswerProjectAskReq {
            ask_id: "ask-1".to_string(),
            answer: "done".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    let body = response_body(response).await;
    assert!(body.contains("project not found"));
    assert!(!body.contains("missing-project"));
}

#[tokio::test]
async fn answer_project_ask_records_resume_when_active_worker_is_gone() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Ask resume".to_string(),
            slug: "ask-resume".to_string(),
            goal: "Resume after user answer".to_string(),
            deadline: None,
        })
        .unwrap();
    state
        .kernel
        .memory
        .project_update(
            &project.id,
            project::ProjectPatch {
                metadata: Some(serde_json::json!({ "runtime": pending_question_runtime() })),
                ..Default::default()
            },
        )
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = answer_project_ask(
        axum::extract::State(state),
        axum::extract::Path(project.slug),
        axum::Json(AnswerProjectAskReq {
            ask_id: "ask-abc".to_string(),
            answer: "Use Rust".to_string(),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = response_json(response).await;
    assert_eq!(body["delivered_to_active_worker"], false);
    assert_eq!(body["runtime_resume_pending"], true);
    assert_eq!(body["runtime"]["status"], "ready");
    assert_eq!(body["runtime"]["current_phase"], "build");
    assert_eq!(
        body["runtime"]["resume_pending"]["reason"],
        "project_ask_answered"
    );
    assert_eq!(body["runtime"]["user_questions"][0]["status"], "answered");
    assert_eq!(
        body["active_worker_error"],
        "Active worker delivery failed; answer may need runtime resume"
    );
    assert!(body["runtime"]["user_questions"][0].get("answer").is_none());
    assert_eq!(body["runtime"]["timeline"][0]["detail"], "Use Rust");
}

#[test]
fn project_runtime_payload_marks_answered_question_resume_ready() {
    let project = project_with_runtime(serde_json::json!({
        "status": "ready",
        "current_phase": "build",
        "resume_pending": {"reason": "project_ask_answered", "phase": "build"},
        "user_questions": [{"ask_id": "ask-1", "status": "answered"}],
    }));

    let (runtime, operator_status) = project_runtime_payload(&project);

    assert_eq!(runtime["resume_pending"]["reason"], "project_ask_answered");
    assert_eq!(operator_status["state"], "resume_ready");
    assert_eq!(operator_status["actions"][0]["label"], "resume_runtime");
}

#[test]
fn project_answer_success_view_omits_raw_project_runtime_and_answer() {
    let project = project_with_runtime(serde_json::json!({
        "status": "ready",
        "current_phase": "build",
        "resume_pending": {
            "reason": "project_ask_answered",
            "phase": "build",
            "answer": "stored answer secret"
        },
        "user_questions": [{
            "ask_id": "ask-1",
            "status": "answered",
            "answer": "question answer secret",
            "metadata": {"secret": "question metadata secret"}
        }],
        "workers": [{
            "id": "worker-1",
            "phase": "build",
            "status": "done",
            "prompt": "raw worker prompt secret"
        }],
        "timeline": [{
            "id": "event-1",
            "title": "Answered",
            "data": {"secret": "event data secret"}
        }]
    }));

    let view = project_answer_success_view(
        &project,
        "ask-1",
        false,
        true,
        None,
        Some("La question projet [ask-secret] n'est plus active. /Users/private ghp_secret"),
    );

    assert_eq!(view["project"]["id"], "project-1");
    assert!(view["project"].get("metadata").is_none());
    assert_eq!(view["runtime_resume_pending"], true);
    assert_eq!(
        view["active_worker_error"],
        "Active project question is no longer waiting"
    );
    assert!(view.get("answer").is_none());

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "stored answer secret",
        "question answer secret",
        "question metadata secret",
        "raw worker prompt secret",
        "event data secret",
        "ask-secret",
        "/Users/private",
        "ghp_secret",
        "metadata",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn project_answer_error_views_omit_raw_storage_details() {
    let raw = "database failed at /Users/example/.captain/projects.db with ghp_secret";

    let storage = safe_project_answer_storage_error(raw);
    let warning = safe_project_answer_runtime_warning(raw);
    let runtime = safe_project_answer_runtime_error(raw);

    assert_eq!(
        storage,
        "Project lookup failed; verify project storage availability"
    );
    assert_eq!(
        warning,
        "Answer delivered to active worker, but runtime state was not updated: Project question state could not be updated; verify project storage availability"
    );
    assert_eq!(
        runtime,
        "Project question state could not be updated; verify project storage availability"
    );

    let encoded = serde_json::json!({
        "storage": storage,
        "warning": warning,
        "runtime": runtime,
    })
    .to_string();
    for forbidden in [
        "/Users/example",
        "projects.db",
        "ghp_secret",
        "database failed",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

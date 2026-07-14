use super::*;
use axum::body::to_bytes;
use axum::response::IntoResponse;
use captain_kernel::CaptainKernel;
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

#[tokio::test]
async fn project_task_text_fields_are_normalized_before_storage() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep task text bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = create_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.id.clone()),
        axum::Json(CreateTaskReq {
            title: "  Trim task title  ".to_string(),
            description: "  Trim task description  ".to_string(),
            parent_id: None,
            priority: None,
            deadline: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let created = state
        .kernel
        .memory
        .task_list_for_project(&project.id)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(created.title, "Trim task title");
    assert_eq!(created.description, "Trim task description");

    let response = update_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path(created.id.clone()),
        axum::Json(UpdateTaskReq {
            status: None,
            title: Some("  Updated task title  ".to_string()),
            description: Some("   ".to_string()),
            priority: None,
            parent_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let stored = state.kernel.memory.task_get(&created.id).unwrap().unwrap();
    assert_eq!(stored.title, "Updated task title");
    assert_eq!(stored.description, "");
}

#[tokio::test]
async fn update_project_task_rejects_invalid_text_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep task text bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let task = state
        .kernel
        .memory
        .task_create(project_task::NewProjectTask {
            project_id: project.id,
            parent_id: None,
            title: "Stable title".to_string(),
            description: "Stable description".to_string(),
            priority: 0,
            deadline: None,
            assignee_agent_id: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);
    let secret_title = format!("{}-/Users/example/private-ghp_secret", "x".repeat(181));

    let response = update_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path(task.id.clone()),
        axum::Json(UpdateTaskReq {
            status: None,
            title: Some(secret_title),
            description: None,
            priority: None,
            parent_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("task title is too long"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state.kernel.memory.task_get(&task.id).unwrap().unwrap();
    assert_eq!(stored.title, "Stable title");
    assert_eq!(stored.description, "Stable description");
}

#[tokio::test]
async fn update_project_task_rejects_invalid_status_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep task status bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let task = state
        .kernel
        .memory
        .task_create(project_task::NewProjectTask {
            project_id: project.id,
            parent_id: None,
            title: "Safe task".to_string(),
            description: String::new(),
            priority: 0,
            deadline: None,
            assignee_agent_id: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = update_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path(task.id.clone()),
        axum::Json(UpdateTaskReq {
            status: Some("invalid-/Users/example/private-ghp_secret".to_string()),
            title: None,
            description: None,
            priority: None,
            parent_id: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("unknown project task status"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state.kernel.memory.task_get(&task.id).unwrap().unwrap();
    assert_eq!(stored.status, project_task::TaskStatus::Todo);
}

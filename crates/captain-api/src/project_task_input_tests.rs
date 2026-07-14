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

fn create_project_and_task(
    state: &AppState,
    title: &str,
) -> (project::Project, project_task::ProjectTask) {
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep task ids bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let task = state
        .kernel
        .memory
        .task_create(project_task::NewProjectTask {
            project_id: project.id.clone(),
            parent_id: None,
            title: title.to_string(),
            description: String::new(),
            priority: 0,
            deadline: None,
            assignee_agent_id: None,
        })
        .unwrap();
    (project, task)
}

#[tokio::test]
async fn list_project_tasks_trims_project_id_before_lookup() {
    let (_tmp, state) = test_state();
    let (project, _task) = create_project_and_task(&state, "Listed task");
    let state = std::sync::Arc::new(state);

    let response = list_project_tasks(
        axum::extract::State(state),
        axum::extract::Path(format!(" {} ", project.id)),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["tasks"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn create_project_task_trims_project_id_before_storage() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep task project ids bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = create_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path(format!(" {} ", project.id)),
        axum::Json(CreateTaskReq {
            title: "New task".to_string(),
            description: String::new(),
            parent_id: None,
            priority: None,
            deadline: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let stored = state
        .kernel
        .memory
        .task_list_for_project(&project.id)
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].project_id, project.id);
}

#[tokio::test]
async fn project_task_routes_resolve_slug_to_canonical_project_id() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Resolve task routes from slug".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let create = create_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.slug.clone()),
        axum::Json(CreateTaskReq {
            title: "Slug task".to_string(),
            description: String::new(),
            parent_id: None,
            priority: None,
            deadline: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(create.status(), axum::http::StatusCode::CREATED);

    let listed = list_project_tasks(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.slug.clone()),
    )
    .await
    .into_response();
    assert_eq!(listed.status(), axum::http::StatusCode::OK);
    let body = to_bytes(listed.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["tasks"].as_array().unwrap().len(), 1);

    let stored = state
        .kernel
        .memory
        .task_list_for_project(&project.id)
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].project_id, project.id);
}

#[tokio::test]
async fn list_project_tasks_rejects_invalid_project_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = list_project_tasks(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project task project id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}

#[tokio::test]
async fn create_project_task_rejects_invalid_project_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep invalid task project ids out".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = create_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
        axum::Json(CreateTaskReq {
            title: "New task".to_string(),
            description: String::new(),
            parent_id: None,
            priority: None,
            deadline: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project task project id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state
        .kernel
        .memory
        .task_list_for_project(&project.id)
        .unwrap();
    assert!(stored.is_empty());
}

#[tokio::test]
async fn update_project_task_rejects_invalid_task_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let (_project, task) = create_project_and_task(&state, "Stable task");
    let state = std::sync::Arc::new(state);

    let response = update_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
        axum::Json(UpdateTaskReq {
            status: Some("done".to_string()),
            title: Some("Changed".to_string()),
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
    assert!(body.contains("project task id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state.kernel.memory.task_get(&task.id).unwrap().unwrap();
    assert_eq!(stored.title, "Stable task");
    assert_eq!(stored.status, project_task::TaskStatus::Todo);
}

#[tokio::test]
async fn create_project_task_rejects_invalid_parent_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep parent ids bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = create_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.id.clone()),
        axum::Json(CreateTaskReq {
            title: "Child task".to_string(),
            description: String::new(),
            parent_id: Some("bad-/Users/example/private-ghp_secret".to_string()),
            priority: None,
            deadline: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project task id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state
        .kernel
        .memory
        .task_list_for_project(&project.id)
        .unwrap();
    assert!(stored.is_empty());
}

#[tokio::test]
async fn update_project_task_trims_task_and_parent_ids_before_storage() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep parent ids bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let parent = state
        .kernel
        .memory
        .task_create(project_task::NewProjectTask {
            project_id: project.id.clone(),
            parent_id: None,
            title: "Parent".to_string(),
            description: String::new(),
            priority: 0,
            deadline: None,
            assignee_agent_id: None,
        })
        .unwrap();
    let child = state
        .kernel
        .memory
        .task_create(project_task::NewProjectTask {
            project_id: project.id,
            parent_id: None,
            title: "Child".to_string(),
            description: String::new(),
            priority: 0,
            deadline: None,
            assignee_agent_id: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = update_project_task(
        axum::extract::State(state.clone()),
        axum::extract::Path(format!(" {} ", child.id)),
        axum::Json(UpdateTaskReq {
            status: None,
            title: None,
            description: None,
            priority: None,
            parent_id: Some(Some(format!(" {} ", parent.id))),
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let stored = state.kernel.memory.task_get(&child.id).unwrap().unwrap();
    assert_eq!(stored.parent_id.as_deref(), Some(parent.id.as_str()));
}

#[tokio::test]
async fn delete_project_task_missing_task_does_not_echo_task_id() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = delete_project_task(
        axum::extract::State(state),
        axum::extract::Path("missing-task-id".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project task not found"));
    assert!(!body.contains("missing-task-id"));
}

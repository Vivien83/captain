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
async fn create_milestone_normalizes_text_before_storage() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep milestone text bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = create_milestone(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.id.clone()),
        axum::Json(CreateMilestoneReq {
            name: "  Beta launch  ".to_string(),
            due_date: None,
            deliverables: vec![
                " docs ".to_string(),
                "   ".to_string(),
                "release notes".to_string(),
            ],
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::CREATED);
    let stored = state
        .kernel
        .memory
        .milestone_list_for_project(&project.id)
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].name, "Beta launch");
    assert_eq!(
        stored[0].deliverables,
        vec!["docs".to_string(), "release notes".to_string()]
    );
}

#[tokio::test]
async fn create_milestone_rejects_invalid_text_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep milestone text bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);
    let secret_deliverable = format!("{}-/Users/example/private-ghp_secret", "x".repeat(301));

    let response = create_milestone(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.id.clone()),
        axum::Json(CreateMilestoneReq {
            name: "Beta launch".to_string(),
            due_date: None,
            deliverables: vec![secret_deliverable],
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("milestone deliverable is too long"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state
        .kernel
        .memory
        .milestone_list_for_project(&project.id)
        .unwrap();
    assert!(stored.is_empty());
}

#[tokio::test]
async fn list_milestones_trims_project_id_before_lookup() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep milestone ids bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    state
        .kernel
        .memory
        .milestone_create(milestone::NewMilestone {
            project_id: project.id.clone(),
            name: "Beta".to_string(),
            due_date: None,
            deliverables: vec![],
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = list_milestones(
        axum::extract::State(state),
        axum::extract::Path(format!(" {} ", project.id)),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["milestones"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn milestone_routes_resolve_slug_to_canonical_project_id() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Resolve milestone routes from slug".to_string(),
            deadline: None,
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let create = create_milestone(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.slug.clone()),
        axum::Json(CreateMilestoneReq {
            name: "Slug milestone".to_string(),
            due_date: None,
            deliverables: vec![],
        }),
    )
    .await
    .into_response();
    assert_eq!(create.status(), axum::http::StatusCode::CREATED);

    let list = list_milestones(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.slug.clone()),
    )
    .await
    .into_response();
    assert_eq!(list.status(), axum::http::StatusCode::OK);
    let body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["milestones"].as_array().unwrap().len(), 1);

    let progress = get_milestone_progress(
        axum::extract::State(state.clone()),
        axum::extract::Path(project.slug.clone()),
    )
    .await
    .into_response();
    assert_eq!(progress.status(), axum::http::StatusCode::OK);

    let stored = state
        .kernel
        .memory
        .milestone_list_for_project(&project.id)
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].project_id, project.id);
}

#[tokio::test]
async fn create_milestone_rejects_invalid_project_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = create_milestone(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
        axum::Json(CreateMilestoneReq {
            name: "Beta launch".to_string(),
            due_date: None,
            deliverables: vec![],
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project milestone id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}

#[tokio::test]
async fn complete_milestone_rejects_invalid_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let project = state
        .kernel
        .memory
        .project_create(project::NewProject {
            name: "Demo".to_string(),
            slug: "demo".to_string(),
            goal: "Keep milestone ids bounded".to_string(),
            deadline: None,
        })
        .unwrap();
    let milestone = state
        .kernel
        .memory
        .milestone_create(milestone::NewMilestone {
            project_id: project.id,
            name: "Beta".to_string(),
            due_date: None,
            deliverables: vec![],
        })
        .unwrap();
    let state = std::sync::Arc::new(state);

    let response = complete_milestone(
        axum::extract::State(state.clone()),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project milestone id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
    let stored = state
        .kernel
        .memory
        .milestone_progress(&milestone.project_id, chrono::Utc::now().timestamp_millis())
        .unwrap();
    assert_eq!(stored.completed, 0);
}

#[tokio::test]
async fn complete_milestone_missing_milestone_does_not_echo_id() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = complete_milestone(
        axum::extract::State(state),
        axum::extract::Path("missing-milestone-id".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project milestone not found"));
    assert!(!body.contains("missing-milestone-id"));
}

#[tokio::test]
async fn milestone_progress_rejects_invalid_project_id_without_echoing_input() {
    let (_tmp, state) = test_state();
    let state = std::sync::Arc::new(state);

    let response = get_milestone_progress(
        axum::extract::State(state),
        axum::extract::Path("bad-/Users/example/private-ghp_secret".to_string()),
    )
    .await
    .into_response();

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("project milestone id"));
    assert!(!body.contains("/Users/example"));
    assert!(!body.contains("ghp_secret"));
}

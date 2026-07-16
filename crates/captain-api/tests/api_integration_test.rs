//! Real HTTP integration tests for the Captain API.
//!
//! These tests boot a real kernel, start a real axum HTTP server on a random
//! port, and hit actual endpoints with reqwest.  No mocking.
//!
//! Tests that require an LLM API call are gated behind GROQ_API_KEY.
//!
//! Run: cargo test -p captain-api --test api_integration_test -- --nocapture

use axum::Router;
use captain_api::event_webhooks;
use captain_api::middleware;
use captain_api::routes::{self, AppState};
use captain_api::ws;
use captain_kernel::CaptainKernel;
use captain_types::config::{DefaultModelConfig, KernelConfig, WebhookTriggerConfig};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

struct TestServer {
    base_url: String,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

const TEST_WEBHOOK_TOKEN: &str = "0123456789abcdef0123456789abcdef";

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Start a test server using ollama as default provider (no API key needed).
/// This lets the kernel boot without any real LLM credentials.
/// Tests that need actual LLM calls should use `start_test_server_with_llm()`.
async fn start_test_server() -> TestServer {
    start_test_server_with_provider("ollama", "test-model", "OLLAMA_API_KEY").await
}

/// Start a test server with Groq as the LLM provider (requires GROQ_API_KEY).
async fn start_test_server_with_llm() -> TestServer {
    start_test_server_with_provider("groq", "llama-3.3-70b-versatile", "GROQ_API_KEY").await
}

async fn start_test_server_with_provider(
    provider: &str,
    model: &str,
    api_key_env: &str,
) -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: api_key_env.to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };

    start_test_server_with_config(tmp, config).await
}

async fn start_test_server_with_webhook_triggers() -> TestServer {
    const TOKEN_ENV: &str = "CAPTAIN_TEST_WEBHOOK_TOKEN";
    std::env::set_var(TOKEN_ENV, TEST_WEBHOOK_TOKEN);

    let tmp = tempfile::tempdir().expect("Failed to create temp dir");
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        webhook_triggers: Some(WebhookTriggerConfig {
            enabled: true,
            token_env: TOKEN_ENV.to_string(),
            ..WebhookTriggerConfig::default()
        }),
        ..KernelConfig::default()
    };

    start_test_server_with_config(tmp, config).await
}

async fn start_test_server_with_config(tmp: tempfile::TempDir, config: KernelConfig) -> TestServer {
    let kernel = CaptainKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let state = Arc::new(AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        ask_user_channels: dashmap::DashMap::new(),
        provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
    });

    let app = Router::new()
        .route("/api/health", axum::routing::get(routes::health))
        .route("/api/status", axum::routing::get(routes::status))
        .route(
            "/api/agents",
            axum::routing::get(routes::list_agents).post(routes::spawn_agent),
        )
        .route(
            "/api/agents/{id}/message",
            axum::routing::post(routes::send_message),
        )
        .route(
            "/api/agents/{id}/session",
            axum::routing::get(routes::get_agent_session),
        )
        .route("/api/sessions", axum::routing::get(routes::list_sessions))
        .route(
            "/api/sessions/{id}",
            axum::routing::get(routes::get_session),
        )
        .route(
            "/api/agents/{id}/sessions",
            axum::routing::get(routes::list_agent_sessions).post(routes::create_agent_session),
        )
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .route(
            "/api/agents/{id}",
            axum::routing::delete(routes::kill_agent)
                .put(routes::update_agent)
                .patch(routes::patch_agent),
        )
        .route(
            "/api/triggers",
            axum::routing::get(routes::list_triggers).post(routes::create_trigger),
        )
        .route(
            "/api/triggers/{id}",
            axum::routing::delete(routes::delete_trigger).put(routes::update_trigger),
        )
        .route(
            "/api/file-triggers",
            axum::routing::get(routes::list_file_triggers).post(routes::create_file_trigger),
        )
        .route(
            "/api/file-triggers/{id}",
            axum::routing::delete(routes::delete_file_trigger).put(routes::update_file_trigger),
        )
        .route(
            "/api/events",
            axum::routing::get(event_webhooks::recent_events),
        )
        .route(
            "/api/webhooks/outbound",
            axum::routing::get(event_webhooks::outbound_webhooks),
        )
        .route(
            "/api/webhooks/outbound/test",
            axum::routing::post(event_webhooks::test_outbound_webhook),
        )
        .route(
            "/api/webhooks/outbound/endpoints",
            axum::routing::post(event_webhooks::create_outbound_webhook_endpoint),
        )
        .route(
            "/api/webhooks/outbound/endpoints/{name}",
            axum::routing::put(event_webhooks::update_outbound_webhook_endpoint)
                .delete(event_webhooks::delete_outbound_webhook_endpoint),
        )
        .route("/hooks/wake", axum::routing::post(routes::webhook_wake))
        .route("/hooks/agent", axum::routing::post(routes::webhook_agent))
        .route(
            "/api/workflows",
            axum::routing::get(routes::list_workflows).post(routes::create_workflow),
        )
        .route(
            "/api/workflows/{id}/run",
            axum::routing::post(routes::run_workflow),
        )
        .route(
            "/api/workflows/{id}/runs",
            axum::routing::get(routes::list_workflow_runs),
        )
        .route("/api/config", axum::routing::get(routes::get_config))
        .route(
            "/api/config/raw",
            axum::routing::get(routes::config_raw_get).put(routes::config_raw_put),
        )
        .route(
            "/api/config/template",
            axum::routing::get(routes::config_template_get),
        )
        .route(
            "/api/config/validate",
            axum::routing::post(routes::config_validate),
        )
        .route("/api/auth/check", axum::routing::get(routes::auth_check))
        .route("/api/shutdown", axum::routing::post(routes::shutdown))
        .layer(axum::middleware::from_fn(middleware::request_logging))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp: tmp,
    }
}

/// Manifest that uses ollama (no API key required, won't make real LLM calls).
const TEST_MANIFEST: &str = r#"
name = "test-agent"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

/// Manifest that uses Groq for real LLM tests.
const LLM_MANIFEST: &str = r#"
name = "test-agent"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_health_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    // Middleware injects x-request-id
    assert!(resp.headers().contains_key("x-request-id"));

    let body: serde_json::Value = resp.json().await.unwrap();
    // Public health endpoint returns minimal info (redacted for security)
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
    // Detailed fields should NOT appear in public health endpoint
    assert!(body["database"].is_null());
    assert!(body["agent_count"].is_null());
}

#[tokio::test]
async fn test_status_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "running");
    assert_eq!(body["agent_count"], 1); // default assistant auto-spawned
    assert!(body["uptime_seconds"].is_number());
    assert_eq!(body["default_provider"], "ollama");
    assert_eq!(body["llm_driver_ready"], true);
    assert!(body["llm_driver_error"].is_null());
    assert_eq!(body["agents"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn test_detached_session_is_globally_listed_and_loadable() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let captain = server
        .state
        .kernel
        .registry
        .find_by_name("captain")
        .expect("default Captain");
    let active_before = captain.session_id;

    let response = client
        .post(format!(
            "{}/api/agents/{}/sessions",
            server.base_url, captain.id
        ))
        .json(&serde_json::json!({
            "label": "Cross-surface restore",
            "activate": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let created: serde_json::Value = response.json().await.unwrap();
    assert_eq!(created["active"], false);
    let session_id = created["session_id"].as_str().unwrap();
    assert!(uuid::Uuid::parse_str(session_id).is_ok());
    assert_eq!(
        server
            .state
            .kernel
            .registry
            .get(captain.id)
            .unwrap()
            .session_id,
        active_before
    );

    let listed: serde_json::Value = client
        .get(format!("{}/api/sessions", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let row = listed["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_id"] == session_id)
        .expect("detached session in global catalog");
    assert_eq!(row["agent_id"], captain.id.to_string());
    assert_eq!(row["agent_name"], "captain");
    assert_eq!(row["label"], "Cross-surface restore");
    assert_eq!(row["active"], false);

    let detail: serde_json::Value = client
        .get(format!("{}/api/sessions/{session_id}", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["session_id"], session_id);
    assert_eq!(detail["agent_id"], captain.id.to_string());
    assert_eq!(detail["message_count"], 0);
}

#[tokio::test]
async fn test_spawn_list_kill_agent() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // --- Spawn ---
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "test-agent");
    let agent_id = body["agent_id"].as_str().unwrap().to_string();
    assert!(!agent_id.is_empty());

    // --- List (2 agents: default assistant + test-agent) ---
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 2);
    let test_agent = agents.iter().find(|a| a["name"] == "test-agent").unwrap();
    assert_eq!(test_agent["id"], agent_id);
    assert_eq!(test_agent["model_provider"], "ollama");

    // --- Kill ---
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, agent_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "killed");

    // --- List (only default assistant remains) ---
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["name"], "captain");
}

#[tokio::test]
async fn test_agent_session_empty() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    // Session should be empty — no messages sent yet
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["message_count"], 0);
    assert_eq!(body["messages"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_send_message_with_llm() {
    if std::env::var("GROQ_API_KEY").is_err() {
        eprintln!("GROQ_API_KEY not set, skipping LLM integration test");
        return;
    }

    let server = start_test_server_with_llm().await;
    let client = reqwest::Client::new();

    // Spawn
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": LLM_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    // Send message through the real HTTP endpoint → kernel → Groq LLM
    let resp = client
        .post(format!(
            "{}/api/agents/{}/message",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"message": "Say hello in exactly 3 words."}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let response_text = body["response"].as_str().unwrap();
    assert!(
        !response_text.is_empty(),
        "LLM response should not be empty"
    );
    assert!(body["input_tokens"].as_u64().unwrap() > 0);
    assert!(body["output_tokens"].as_u64().unwrap() > 0);

    // Session should now have messages
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    let session: serde_json::Value = resp.json().await.unwrap();
    assert!(session["message_count"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_workflow_crud() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent for workflow
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_name = body["name"].as_str().unwrap().to_string();

    // Create workflow
    let resp = client
        .post(format!("{}/api/workflows", server.base_url))
        .json(&serde_json::json!({
            "name": "test-workflow",
            "description": "Integration test workflow",
            "steps": [
                {
                    "name": "step1",
                    "agent_name": agent_name,
                    "prompt": "Echo: {{input}}",
                    "mode": "sequential",
                    "timeout_secs": 30
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let workflow_id = body["workflow_id"].as_str().unwrap().to_string();
    assert!(!workflow_id.is_empty());

    // List workflows
    let resp = client
        .get(format!("{}/api/workflows", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let workflows: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0]["name"], "test-workflow");
    assert_eq!(workflows[0]["steps"], 1);
}

#[tokio::test]
async fn test_trigger_crud() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent for trigger
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    // Create trigger (Lifecycle pattern — simplest variant)
    let resp = client
        .post(format!("{}/api/triggers", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "pattern": "lifecycle",
            "prompt_template": "Handle: {{event}}",
            "max_fires": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let trigger_id = body["trigger_id"].as_str().unwrap().to_string();
    assert_eq!(body["agent_id"], agent_id);

    // List triggers (unfiltered)
    let resp = client
        .get(format!("{}/api/triggers", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0]["agent_id"], agent_id);
    assert_eq!(triggers[0]["enabled"], true);
    assert_eq!(triggers[0]["max_fires"], 5);

    // List triggers (filtered by agent_id)
    let resp = client
        .get(format!(
            "{}/api/triggers?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(triggers.len(), 1);

    // Update trigger through the same HTTP route used by the UI.
    let resp = client
        .put(format!("{}/api/triggers/{}", server.base_url, trigger_id))
        .json(&serde_json::json!({
            "enabled": false,
            "max_fires": 2,
            "prompt_template": "Updated: {{event}}"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "updated");
    assert_eq!(body["trigger"]["enabled"], false);
    assert_eq!(body["trigger"]["max_fires"], 2);

    // Delete trigger
    let resp = client
        .delete(format!("{}/api/triggers/{}", server.base_url, trigger_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // List triggers (should be empty)
    let resp = client
        .get(format!("{}/api/triggers", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(triggers.len(), 0);
}

#[tokio::test]
async fn test_webhook_wake_fires_event_trigger_and_records_event() {
    let server = start_test_server_with_webhook_triggers().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    let resp = client
        .post(format!("{}/api/triggers", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "pattern": "all",
            "prompt_template": "Webhook smoke: {{event}}",
            "max_fires": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let trigger_id = resp.json::<serde_json::Value>().await.unwrap()["trigger_id"]
        .as_str()
        .unwrap()
        .to_string();

    let unauth = client
        .post(format!("{}/hooks/wake", server.base_url))
        .json(&serde_json::json!({
            "text": "real trigger smoke",
            "mode": "now"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    let resp = client
        .post(format!("{}/hooks/wake", server.base_url))
        .bearer_auth(TEST_WEBHOOK_TOKEN)
        .json(&serde_json::json!({
            "text": "real trigger smoke",
            "mode": "now"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "accepted");

    tokio::time::sleep(Duration::from_millis(100)).await;

    let resp = client
        .get(format!(
            "{}/api/triggers?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    let trigger = triggers
        .iter()
        .find(|item| item["id"] == trigger_id)
        .expect("created trigger should still be listed");
    assert_eq!(trigger["fire_count"], 1);

    let resp = client
        .get(format!("{}/api/events?limit=20", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let events = body["events"].as_array().unwrap();
    assert!(
        events.iter().any(|event| event["kind"] == "webhook.wake"),
        "webhook.wake should be visible in the live event stream: {events:?}"
    );
}

#[tokio::test]
async fn test_outbound_webhook_endpoint_crud_and_dry_run_via_http() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!(
            "{}/api/webhooks/outbound/endpoints",
            server.base_url
        ))
        .json(&serde_json::json!({
            "name": "n8n-smoke",
            "url": "https://example.com/captain-hook",
            "events": ["webhook.test", "project.*"],
            "secret_env": "CAPTAIN_TEST_WEBHOOK_SECRET",
            "enabled": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "created");
    assert_eq!(body["restart_required"], true);
    assert_eq!(body["endpoint"]["name"], "n8n-smoke");
    assert_eq!(body["endpoint"]["signed"], true);

    let raw = std::fs::read_to_string(server._tmp.path().join("config.toml")).unwrap();
    assert!(raw.contains("name = \"n8n-smoke\""));
    assert!(raw.contains("url = \"https://example.com/captain-hook\""));
    assert!(raw.contains("project.*"));

    let resp = client
        .post(format!("{}/api/webhooks/outbound/test", server.base_url))
        .json(&serde_json::json!({
            "url": "https://example.com/captain-hook",
            "event": "webhook.test",
            "dry_run": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "dry_run_ok");
    assert_eq!(body["event"], "webhook.test");

    let blocked = client
        .post(format!("{}/api/webhooks/outbound/test", server.base_url))
        .json(&serde_json::json!({
            "url": "http://127.0.0.1:9/captain-hook",
            "event": "webhook.test",
            "dry_run": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 400);

    let resp = client
        .put(format!(
            "{}/api/webhooks/outbound/endpoints/n8n-smoke",
            server.base_url
        ))
        .json(&serde_json::json!({
            "name": "n8n-smoke",
            "url": "https://example.com/captain-hook-v2",
            "events": ["model.fallback"],
            "enabled": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "updated");
    assert_eq!(body["endpoint"]["enabled"], false);

    let raw = std::fs::read_to_string(server._tmp.path().join("config.toml")).unwrap();
    assert!(raw.contains("url = \"https://example.com/captain-hook-v2\""));
    assert!(raw.contains("model.fallback"));

    let resp = client
        .delete(format!(
            "{}/api/webhooks/outbound/endpoints/n8n-smoke",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "deleted");

    let raw = std::fs::read_to_string(server._tmp.path().join("config.toml")).unwrap();
    assert!(!raw.contains("n8n-smoke"));
}

#[tokio::test]
async fn test_file_trigger_http_crud_and_real_file_event() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    let watched_dir = server._tmp.path().join("watched");
    std::fs::create_dir_all(&watched_dir).unwrap();

    let resp = client
        .post(format!("{}/api/file-triggers", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "paths": [watched_dir.display().to_string()],
            "events": ["any"],
            "recursive": true,
            "enabled": true,
            "debounce_ms": 200,
            "prompt_template": "File smoke {kind}: {path}"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let trigger_id = body["trigger_id"].as_str().unwrap().to_string();

    std::fs::write(watched_dir.join("input.txt"), "captain trigger smoke").unwrap();

    let mut saw_file_event = false;
    for _ in 0..30 {
        let resp = client
            .get(format!("{}/api/events?limit=50", server.base_url))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        let events = body["events"].as_array().unwrap();
        saw_file_event = events.iter().any(|event| {
            event["kind"] == "file.changed"
                && event["summary"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("input.txt")
        });
        if saw_file_event {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        saw_file_event,
        "file trigger should emit a real file.changed event"
    );

    let resp = client
        .put(format!(
            "{}/api/file-triggers/{}",
            server.base_url, trigger_id
        ))
        .json(&serde_json::json!({ "enabled": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["enabled"], false);

    let resp = client
        .get(format!(
            "{}/api/file-triggers?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0]["id"], trigger_id);
    assert_eq!(triggers[0]["enabled"], false);

    let resp = client
        .delete(format!(
            "{}/api/file-triggers/{}",
            server.base_url, trigger_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let resp = client
        .get(format!("{}/api/file-triggers", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(triggers.len(), 0);
}

#[tokio::test]
async fn test_invalid_agent_id_returns_400() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Send message to invalid ID
    let resp = client
        .post(format!("{}/api/agents/not-a-uuid/message", server.base_url))
        .json(&serde_json::json!({"message": "hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid"));

    // Kill invalid ID
    let resp = client
        .delete(format!("{}/api/agents/not-a-uuid", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Session for invalid ID
    let resp = client
        .get(format!("{}/api/agents/not-a-uuid/session", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn test_kill_nonexistent_agent_returns_404() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let fake_id = uuid::Uuid::new_v4();
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, fake_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_spawn_invalid_manifest_returns_400() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": "this is {{ not valid toml"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid manifest"));
}

#[tokio::test]
async fn test_agent_updates_reject_removed_automatic_routing() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let captain = server
        .state
        .kernel
        .registry
        .find_by_name("captain")
        .expect("default Captain");
    let url = format!("{}/api/agents/{}", server.base_url, captain.id);

    let patch = client
        .patch(&url)
        .json(&serde_json::json!({
            "routing": {"simple_model": "small", "complex_model": "large"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(patch.status(), 400);
    let patch_body: serde_json::Value = patch.json().await.unwrap();
    assert!(patch_body["error"]
        .as_str()
        .unwrap()
        .contains("model routing was removed"));

    let legacy_mode_patch = client
        .patch(&url)
        .json(&serde_json::json!({"orchestration_mode": "routing"}))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy_mode_patch.status(), 400);
    let legacy_mode_patch_body: serde_json::Value = legacy_mode_patch.json().await.unwrap();
    assert!(legacy_mode_patch_body["error"]
        .as_str()
        .unwrap()
        .contains("model routing was removed"));

    let manifest_with_routing = format!(
        "{TEST_MANIFEST}\n[routing]\nsimple_model = \"small\"\ncomplex_model = \"large\"\n"
    );
    let put = client
        .put(&url)
        .json(&serde_json::json!({"manifest_toml": manifest_with_routing}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 400);
    let put_body: serde_json::Value = put.json().await.unwrap();
    assert!(put_body["error"]
        .as_str()
        .unwrap()
        .contains("model routing was removed"));

    let manifest_with_legacy_mode =
        TEST_MANIFEST.replacen("[model]", "orchestration_mode = \"routing\"\n\n[model]", 1);
    let legacy_mode_put = client
        .put(url)
        .json(&serde_json::json!({"manifest_toml": manifest_with_legacy_mode}))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy_mode_put.status(), 400);
    let legacy_mode_put_body: serde_json::Value = legacy_mode_put.json().await.unwrap();
    assert!(legacy_mode_put_body["error"]
        .as_str()
        .unwrap()
        .contains("model routing was removed"));
}

#[tokio::test]
async fn test_request_id_header_is_uuid() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();

    let request_id = resp
        .headers()
        .get("x-request-id")
        .expect("x-request-id header should be present");
    let id_str = request_id.to_str().unwrap();
    assert!(
        uuid::Uuid::parse_str(id_str).is_ok(),
        "x-request-id should be a valid UUID, got: {}",
        id_str
    );
}

#[tokio::test]
async fn test_multiple_agents_lifecycle() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn 3 agents
    let mut ids = Vec::new();
    for i in 0..3 {
        let manifest = format!(
            r#"
name = "agent-{i}"
version = "0.1.0"
description = "Multi-agent test {i}"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Agent {i}."

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#
        );

        let resp = client
            .post(format!("{}/api/agents", server.base_url))
            .json(&serde_json::json!({"manifest_toml": manifest}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        ids.push(body["agent_id"].as_str().unwrap().to_string());
    }

    // List should show 4 (3 spawned + default assistant)
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 4);

    // Status should agree
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["agent_count"], 4);

    // Kill one
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, ids[1]))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // List should show 3 (2 spawned + default assistant)
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 3);

    // Kill the rest
    for id in [&ids[0], &ids[2]] {
        client
            .delete(format!("{}/api/agents/{}", server.base_url, id))
            .send()
            .await
            .unwrap();
    }

    // List should have only default assistant
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 1);
}

// ---------------------------------------------------------------------------
// Auth integration tests
// ---------------------------------------------------------------------------

/// Start a test server with Bearer-token authentication enabled.
async fn start_test_server_with_auth(api_key: &str) -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };

    let kernel = CaptainKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let state = Arc::new(AppState {
        kernel,
        started_at: Instant::now(),
        peer_registry: None,
        bridge_manager: tokio::sync::Mutex::new(None),
        channels_config: tokio::sync::RwLock::new(Default::default()),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        ask_user_channels: dashmap::DashMap::new(),
        provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
    });

    let api_key = state.kernel.config.api_key.trim().to_string();
    let auth_state = middleware::AuthState {
        api_key: api_key.clone(),
        home_dir: state.kernel.config.home_dir.clone(),
        fallback_auth: state.kernel.config.auth.clone(),
    };

    let app = Router::new()
        .route("/api/health", axum::routing::get(routes::health))
        .route("/api/status", axum::routing::get(routes::status))
        .route(
            "/api/agents",
            axum::routing::get(routes::list_agents).post(routes::spawn_agent),
        )
        .route(
            "/api/agents/{id}/message",
            axum::routing::post(routes::send_message),
        )
        .route(
            "/api/agents/{id}/session",
            axum::routing::get(routes::get_agent_session),
        )
        .route("/api/agents/{id}/ws", axum::routing::get(ws::agent_ws))
        .route(
            "/api/agents/{id}",
            axum::routing::delete(routes::kill_agent),
        )
        .route(
            "/api/triggers",
            axum::routing::get(routes::list_triggers).post(routes::create_trigger),
        )
        .route(
            "/api/triggers/{id}",
            axum::routing::delete(routes::delete_trigger),
        )
        .route(
            "/api/workflows",
            axum::routing::get(routes::list_workflows).post(routes::create_workflow),
        )
        .route(
            "/api/workflows/{id}/run",
            axum::routing::post(routes::run_workflow),
        )
        .route(
            "/api/workflows/{id}/runs",
            axum::routing::get(routes::list_workflow_runs),
        )
        .route("/api/config", axum::routing::get(routes::get_config))
        .route(
            "/api/config/raw",
            axum::routing::get(routes::config_raw_get).put(routes::config_raw_put),
        )
        .route(
            "/api/config/template",
            axum::routing::get(routes::config_template_get),
        )
        .route(
            "/api/config/validate",
            axum::routing::post(routes::config_validate),
        )
        .route("/api/auth/check", axum::routing::get(routes::auth_check))
        .route("/api/shutdown", axum::routing::post(routes::shutdown))
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .layer(axum::middleware::from_fn(middleware::request_logging))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp: tmp,
    }
}

#[tokio::test]
async fn test_auth_health_is_public() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // /api/health should be accessible without auth
    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_auth_rejects_no_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Protected endpoint without auth header → 401
    // Note: /api/status is public, so use a protected endpoint.
    let resp = client
        .get(format!("{}/api/commands", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Missing"));
}

#[tokio::test]
async fn test_auth_rejects_wrong_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Wrong bearer token → 401
    // Note: /api/status is public, so use a protected endpoint.
    let resp = client
        .get(format!("{}/api/commands", server.base_url))
        .header("authorization", "Bearer wrong-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid"));
}

#[tokio::test]
async fn test_auth_accepts_correct_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Correct bearer token → 200
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .header("authorization", "Bearer secret-key-123")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "running");
}

#[tokio::test]
async fn test_config_requires_auth_but_auth_check_is_public() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    let check = client
        .get(format!("{}/api/auth/check", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(check.status(), 200);

    let unauth = client
        .get(format!("{}/api/config", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), 401);

    let unauth_template = client
        .get(format!("{}/api/config/template", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth_template.status(), 401);

    let authed = client
        .get(format!("{}/api/config", server.base_url))
        .header("authorization", "Bearer secret-key-123")
        .send()
        .await
        .unwrap();
    assert_eq!(authed.status(), 200);

    let valid = client
        .post(format!("{}/api/config/validate", server.base_url))
        .header("authorization", "Bearer secret-key-123")
        .json(&serde_json::json!({"content": "log_level = \"info\"\n"}))
        .send()
        .await
        .unwrap();
    assert_eq!(valid.status(), 200);
}

#[tokio::test]
async fn test_auth_disabled_when_no_key() {
    // Empty API key = auth disabled
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Protected endpoint accessible without auth when no key is configured
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

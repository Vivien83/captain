//! Native event and outbound webhook API surface.
//!
//! Inbound hooks wake Captain; outbound hooks let external systems subscribe to
//! Captain lifecycle events without relying on the model to call a tool.

use crate::routes::AppState;
use crate::ssrf_pin::resolve_pinned_socket_addr;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{extract::Path, extract::State, Json};
use captain_types::config::{OutboundWebhookEndpoint, OutboundWebhooksConfig};
use captain_types::event::{ChatStreamEvent, Event, EventPayload, LifecycleEvent, SystemEvent};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::time::Duration;
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug)]
struct PreparedWebhookDelivery {
    body: Vec<u8>,
    signature: Option<String>,
    client: reqwest::Client,
    attempts: u8,
}

struct WebhookAttemptFailure {
    http_status: Option<u16>,
    error: String,
}

pub fn spawn_outbound_webhook_dispatcher(state: Arc<AppState>) {
    let config = state.kernel.config.outbound_webhooks.clone();
    if !config.enabled || config.endpoints.iter().all(|endpoint| !endpoint.enabled) {
        return;
    }

    let mut rx = state.kernel.event_bus.subscribe_all();
    tokio::spawn(async move {
        loop {
            let Ok(event) = rx.recv().await else {
                continue;
            };
            let event_kind = event_kind(&event);
            if event_kind.starts_with("webhook.delivery.") {
                continue;
            }
            for endpoint in matching_endpoints(&config, &event_kind) {
                let event = event.clone();
                let config = config.clone();
                let event_kind = event_kind.clone();
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    if let Err(err) =
                        deliver_event(&state, &config, &endpoint, &event_kind, &event).await
                    {
                        tracing::warn!(
                            endpoint = endpoint.name,
                            event_kind,
                            error = %err,
                            "outbound webhook delivery failed"
                        );
                    }
                });
            }
        }
    });
}

pub async fn recent_events(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params
        .get("limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100)
        .min(500);
    let events = state.kernel.event_bus.history(limit).await;
    let rows = events
        .iter()
        .rev()
        .map(event_summary)
        .collect::<Vec<serde_json::Value>>();
    Json(serde_json::json!({ "events": rows }))
}

pub async fn outbound_webhooks(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = &state.kernel.config.outbound_webhooks;
    let endpoints = config
        .endpoints
        .iter()
        .map(|endpoint| {
            serde_json::json!({
                "name": endpoint.name,
                "url": endpoint.url,
                "enabled": endpoint.enabled,
                "events": endpoint.events,
                "signed": !endpoint.secret_env.trim().is_empty(),
                "secret_env": redact_env_name(&endpoint.secret_env),
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({
        "enabled": config.enabled,
        "timeout_secs": config.timeout_secs,
        "max_attempts": config.max_attempts,
        "endpoints": endpoints,
        "restart_required_for_config_changes": true,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpsertOutboundEndpointReq {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub secret_env: String,
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

pub async fn create_outbound_webhook_endpoint(
    State(state): State<Arc<AppState>>,
    Json(req): Json<UpsertOutboundEndpointReq>,
) -> impl IntoResponse {
    match upsert_endpoint_in_config(&state, None, req, false) {
        Ok(endpoint) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "status": "created",
                "endpoint": endpoint_response(&endpoint),
                "restart_required": true,
            })),
        ),
        Err(err) => config_edit_error(err),
    }
}

pub async fn update_outbound_webhook_endpoint(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<UpsertOutboundEndpointReq>,
) -> impl IntoResponse {
    match upsert_endpoint_in_config(&state, Some(&name), req, true) {
        Ok(endpoint) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "updated",
                "endpoint": endpoint_response(&endpoint),
                "restart_required": true,
            })),
        ),
        Err(err) => config_edit_error(err),
    }
}

pub async fn delete_outbound_webhook_endpoint(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match delete_endpoint_from_config(&state, &name) {
        Ok(true) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "deleted",
                "endpoint": name,
                "restart_required": true,
            })),
        ),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "outbound webhook endpoint not found" })),
        ),
        Err(err) => config_edit_error(err),
    }
}

#[derive(Debug, Deserialize)]
pub struct OutboundWebhookTestReq {
    pub url: String,
    #[serde(default)]
    pub secret_env: String,
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
}

pub async fn test_outbound_webhook(Json(req): Json<OutboundWebhookTestReq>) -> impl IntoResponse {
    let event_kind = req.event.unwrap_or_else(|| "webhook.test".to_string());
    let endpoint = OutboundWebhookEndpoint {
        name: "manual-test".to_string(),
        url: req.url,
        secret_env: req.secret_env,
        events: vec![event_kind.clone()],
        enabled: true,
    };
    if let Err(err) = validate_public_webhook_url(&endpoint.url) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": err })),
        );
    }

    let payload = serde_json::json!({
        "source": "captain",
        "event": event_kind,
        "test": true,
    });
    let body = match serde_json::to_vec(&payload) {
        Ok(body) => body,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err.to_string() })),
            );
        }
    };
    let signature = signature_header(&endpoint.secret_env, &body).ok();

    if req.dry_run {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "dry_run_ok",
                "event": event_kind,
                "signed": signature.is_some(),
                "url": endpoint.url,
            })),
        );
    }

    let pinned = match resolve_pinned_socket_addr(&endpoint.url, false).await {
        Ok(pinned) => pinned,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": err })),
            );
        }
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&pinned.host, pinned.addr)
        .build();
    let Ok(client) = client else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": "failed to create HTTP client" })),
        );
    };
    let mut request = client
        .post(&endpoint.url)
        .header("content-type", "application/json")
        .header("x-captain-event", &event_kind)
        .body(body);
    if let Some(signature) = signature {
        request = request.header("x-captain-signature", signature);
    }
    match request.send().await {
        Ok(response) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "sent",
                "http_status": response.status().as_u16(),
            })),
        ),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": err.to_string() })),
        ),
    }
}

async fn deliver_event(
    state: &AppState,
    config: &OutboundWebhooksConfig,
    endpoint: &OutboundWebhookEndpoint,
    event_kind: &str,
    event: &Event,
) -> Result<(), String> {
    let prepared = prepare_webhook_delivery(config, endpoint, event_kind, event).await?;
    let mut last_err = None;
    for attempt in 1..=prepared.attempts {
        publish_webhook_step(
            state,
            "webhook.delivery.attempt",
            endpoint,
            event_kind,
            attempt,
            None,
            None,
        )
        .await;
        tracing::info!(
            endpoint = endpoint.name,
            event_kind,
            attempt,
            "outbound webhook delivery attempt"
        );
        match send_webhook_attempt(&prepared, endpoint, event_kind, attempt).await {
            Ok(status) => {
                publish_webhook_success(state, endpoint, event_kind, attempt, status).await;
                return Ok(());
            }
            Err(failure) => {
                publish_webhook_retry(state, endpoint, event_kind, attempt, &failure).await;
                last_err = Some(failure.error);
            }
        }
        tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
    }
    let err = last_err.unwrap_or_else(|| "unknown delivery error".to_string());
    publish_webhook_step(
        state,
        "webhook.delivery.failed",
        endpoint,
        event_kind,
        prepared.attempts,
        None,
        Some(&err),
    )
    .await;
    Err(err)
}

async fn prepare_webhook_delivery(
    config: &OutboundWebhooksConfig,
    endpoint: &OutboundWebhookEndpoint,
    event_kind: &str,
    event: &Event,
) -> Result<PreparedWebhookDelivery, String> {
    validate_public_webhook_url(&endpoint.url)?;
    // Outbound event webhooks have no local-testing escape hatch — see
    // validate_public_webhook_url — so this always pins with allow_local:
    // false, same as the validation above.
    let pinned = resolve_pinned_socket_addr(&endpoint.url, false).await?;
    let payload = serde_json::json!({
        "source": "captain",
        "event": event_kind,
        "captain_event": event,
    });
    let body = serde_json::to_vec(&payload).map_err(|err| err.to_string())?;
    let signature = signature_header(&endpoint.secret_env, &body).ok();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(config.timeout_secs.max(1)))
        .redirect(reqwest::redirect::Policy::none())
        .resolve(&pinned.host, pinned.addr)
        .build()
        .map_err(|err| err.to_string())?;
    Ok(PreparedWebhookDelivery {
        body,
        signature,
        client,
        attempts: config.max_attempts.clamp(1, 8),
    })
}

async fn send_webhook_attempt(
    prepared: &PreparedWebhookDelivery,
    endpoint: &OutboundWebhookEndpoint,
    event_kind: &str,
    attempt: u8,
) -> Result<u16, WebhookAttemptFailure> {
    let mut request = prepared
        .client
        .post(&endpoint.url)
        .header("content-type", "application/json")
        .header("x-captain-event", event_kind)
        .header("x-captain-attempt", attempt.to_string())
        .body(prepared.body.clone());
    if let Some(signature) = &prepared.signature {
        request = request.header("x-captain-signature", signature);
    }
    match request.send().await {
        Ok(response) if response.status().is_success() => Ok(response.status().as_u16()),
        Ok(response) => {
            let status = response.status().as_u16();
            Err(WebhookAttemptFailure {
                http_status: Some(status),
                error: format!("HTTP {}", response.status()),
            })
        }
        Err(err) => Err(WebhookAttemptFailure {
            http_status: None,
            error: err.to_string(),
        }),
    }
}

async fn publish_webhook_success(
    state: &AppState,
    endpoint: &OutboundWebhookEndpoint,
    event_kind: &str,
    attempt: u8,
    status: u16,
) {
    publish_webhook_step(
        state,
        "webhook.delivery.succeeded",
        endpoint,
        event_kind,
        attempt,
        Some(status),
        None,
    )
    .await;
    tracing::info!(
        endpoint = endpoint.name,
        event_kind,
        attempt,
        status,
        "outbound webhook delivery succeeded"
    );
}

async fn publish_webhook_retry(
    state: &AppState,
    endpoint: &OutboundWebhookEndpoint,
    event_kind: &str,
    attempt: u8,
    failure: &WebhookAttemptFailure,
) {
    publish_webhook_step(
        state,
        "webhook.delivery.retry",
        endpoint,
        event_kind,
        attempt,
        failure.http_status,
        Some(&failure.error),
    )
    .await;
    tracing::warn!(
        endpoint = endpoint.name,
        event_kind,
        attempt,
        error = %failure.error,
        "outbound webhook delivery attempt failed"
    );
}

fn matching_endpoints(
    config: &OutboundWebhooksConfig,
    event_kind: &str,
) -> Vec<OutboundWebhookEndpoint> {
    config
        .endpoints
        .iter()
        .filter(|endpoint| endpoint.enabled && endpoint_matches(endpoint, event_kind))
        .cloned()
        .collect()
}

fn endpoint_matches(endpoint: &OutboundWebhookEndpoint, event_kind: &str) -> bool {
    endpoint.events.is_empty()
        || endpoint.events.iter().any(|pattern| {
            let pattern = pattern.trim();
            pattern == "*"
                || pattern == event_kind
                || event_kind.starts_with(pattern.trim_end_matches('*'))
        })
}

async fn publish_webhook_step(
    state: &AppState,
    step: &str,
    endpoint: &OutboundWebhookEndpoint,
    event_kind: &str,
    attempt: u8,
    http_status: Option<u16>,
    error: Option<&str>,
) {
    let payload = serde_json::json!({
        "event": step,
        "endpoint": endpoint.name,
        "target_event": event_kind,
        "attempt": attempt,
        "http_status": http_status,
        "error": error,
    });
    if let Ok(bytes) = serde_json::to_vec(&payload) {
        state
            .kernel
            .publish_event(Event::new(
                captain_types::agent::AgentId::new(),
                captain_types::event::EventTarget::Broadcast,
                EventPayload::Custom(bytes),
            ))
            .await;
    }
}

fn event_summary(event: &Event) -> serde_json::Value {
    serde_json::json!({
        "id": event.id.to_string(),
        "timestamp": event.timestamp.to_rfc3339(),
        "source": event.source.to_string(),
        "target": serde_json::to_value(&event.target).unwrap_or_default(),
        "kind": event_kind(event),
        "summary": event_text(event),
    })
}

fn event_kind(event: &Event) -> String {
    match &event.payload {
        EventPayload::Lifecycle(LifecycleEvent::Spawned { .. }) => "agent.spawned",
        EventPayload::Lifecycle(LifecycleEvent::Started { .. }) => "agent.started",
        EventPayload::Lifecycle(LifecycleEvent::Suspended { .. }) => "agent.suspended",
        EventPayload::Lifecycle(LifecycleEvent::Resumed { .. }) => "agent.resumed",
        EventPayload::Lifecycle(LifecycleEvent::Terminated { .. }) => "agent.terminated",
        EventPayload::Lifecycle(LifecycleEvent::Crashed { .. }) => "agent.crashed",
        EventPayload::System(SystemEvent::KernelStarted) => "kernel.started",
        EventPayload::System(SystemEvent::KernelStopping) => "kernel.stopping",
        EventPayload::System(SystemEvent::IntegrationConfigured { .. }) => "integration.configured",
        EventPayload::System(SystemEvent::FileChanged { .. }) => "file.changed",
        EventPayload::System(SystemEvent::TriggerThrottled { .. }) => "trigger.throttled",
        EventPayload::System(SystemEvent::HealthCheck { .. }) => "health.check",
        EventPayload::System(SystemEvent::HealthCheckFailed { .. }) => "health.failed",
        EventPayload::System(SystemEvent::QuotaWarning { .. }) => "quota.warning",
        EventPayload::System(SystemEvent::QuotaEnforced { .. }) => "quota.enforced",
        EventPayload::System(SystemEvent::ModelRouted { .. }) => "model.routed",
        EventPayload::System(SystemEvent::UserAction { .. }) => "user.action",
        EventPayload::ChatStream(ChatStreamEvent::MemoryQueued { .. }) => "learning.memory_queued",
        EventPayload::ChatStream(ChatStreamEvent::MemoryStored { .. }) => "learning.memory_stored",
        EventPayload::ChatStream(ChatStreamEvent::SkillProposalQueued { .. }) => {
            "learning.skill_proposal_queued"
        }
        EventPayload::ChatStream(ChatStreamEvent::SkillRefinementQueued { .. }) => {
            "learning.skill_refinement_queued"
        }
        EventPayload::ChatStream(ChatStreamEvent::ProjectAskUser { .. }) => "project.ask_user",
        EventPayload::ChatStream(ChatStreamEvent::ToolStart { .. }) => "tool.started",
        EventPayload::ChatStream(ChatStreamEvent::ToolEnd { .. }) => "tool.ended",
        EventPayload::ChatStream(ChatStreamEvent::Phase { phase, .. })
            if phase == "model_fallback" =>
        {
            "model.fallback"
        }
        EventPayload::ChatStream(_) => "chat.event",
        EventPayload::Message(_) => "agent.message",
        EventPayload::ToolResult(_) => "tool.result",
        EventPayload::MemoryUpdate(_) => "memory.updated",
        EventPayload::Network(_) => "network.event",
        EventPayload::ToolRun(_) => "tool_run.status_changed",
        EventPayload::Custom(data) => {
            return custom_event_field(data).unwrap_or_else(|| "custom".to_string());
        }
    }
    .to_string()
}

fn event_text(event: &Event) -> String {
    match &event.payload {
        EventPayload::Lifecycle(LifecycleEvent::Spawned { name, .. }) => {
            format!("Agent spawned: {name}")
        }
        EventPayload::Lifecycle(LifecycleEvent::Terminated { reason, .. }) => {
            format!("Agent terminated: {reason}")
        }
        EventPayload::Lifecycle(LifecycleEvent::Crashed { error, .. }) => {
            format!("Agent crashed: {error}")
        }
        EventPayload::System(SystemEvent::IntegrationConfigured { name }) => {
            format!("Integration configured: {name}")
        }
        EventPayload::System(SystemEvent::FileChanged { path, kind, .. }) => {
            format!("File {}: {}", kind.as_str(), path.display())
        }
        EventPayload::ChatStream(ChatStreamEvent::MemoryQueued {
            subject, object, ..
        }) => {
            format!("Learning queued: {subject} -> {object}")
        }
        EventPayload::ChatStream(ChatStreamEvent::SkillProposalQueued { name, .. }) => {
            format!("Skill proposal queued: {name}")
        }
        EventPayload::ChatStream(ChatStreamEvent::SkillRefinementQueued { skill, .. }) => {
            format!("Skill refinement queued: {skill}")
        }
        EventPayload::Custom(data) => {
            custom_event_field(data).unwrap_or_else(|| "custom".to_string())
        }
        EventPayload::ToolRun(run) => {
            format!(
                "Tool run {} ({}): {}",
                run.run_id, run.tool_name, run.status
            )
        }
        _ => event_kind(event),
    }
}

fn custom_event_field(data: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(data).ok()?;
    let raw = value
        .get("event")
        .or_else(|| value.get("type"))
        .and_then(|v| v.as_str())?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn signature_header(secret_env: &str, body: &[u8]) -> Result<String, String> {
    let secret_env = secret_env.trim();
    if secret_env.is_empty() {
        return Err("no secret env configured".to_string());
    }
    let secret = std::env::var(secret_env).map_err(|_| "secret env is not set".to_string())?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|err| err.to_string())?;
    mac.update(body);
    Ok(format!(
        "sha256={}",
        hex::encode(mac.finalize().into_bytes())
    ))
}

fn redact_env_name(value: &str) -> String {
    if value.trim().is_empty() {
        String::new()
    } else {
        value.to_string()
    }
}

fn default_true() -> bool {
    true
}

fn endpoint_response(endpoint: &OutboundWebhookEndpoint) -> serde_json::Value {
    serde_json::json!({
        "name": endpoint.name,
        "url": endpoint.url,
        "enabled": endpoint.enabled,
        "events": endpoint.events,
        "signed": !endpoint.secret_env.trim().is_empty(),
        "secret_env": redact_env_name(&endpoint.secret_env),
    })
}

fn config_edit_error(err: String) -> (StatusCode, Json<serde_json::Value>) {
    let status = if err.contains("not found") {
        StatusCode::NOT_FOUND
    } else if err.contains("already exists") || err.contains("required") || err.contains("invalid")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, Json(serde_json::json!({ "error": err })))
}

fn upsert_endpoint_in_config(
    state: &AppState,
    existing_name: Option<&str>,
    req: UpsertOutboundEndpointReq,
    must_exist: bool,
) -> Result<OutboundWebhookEndpoint, String> {
    let endpoint = normalize_endpoint(req)?;
    let config_path = state.kernel.config.home_dir.join("config.toml");
    edit_outbound_config(&config_path, |endpoints| {
        let target = existing_name.unwrap_or(&endpoint.name);
        let existing_idx = endpoints
            .iter()
            .position(|item| item.name.eq_ignore_ascii_case(target));
        if must_exist && existing_idx.is_none() {
            return Err("outbound webhook endpoint not found".to_string());
        }
        if existing_name.is_none() && existing_idx.is_some() {
            return Err("outbound webhook endpoint already exists".to_string());
        }
        if let Some(idx) = existing_idx {
            endpoints[idx] = endpoint.clone();
        } else {
            endpoints.push(endpoint.clone());
        }
        Ok(())
    })?;
    Ok(endpoint)
}

fn delete_endpoint_from_config(state: &AppState, name: &str) -> Result<bool, String> {
    let config_path = state.kernel.config.home_dir.join("config.toml");
    let mut removed = false;
    edit_outbound_config(&config_path, |endpoints| {
        let before = endpoints.len();
        endpoints.retain(|item| !item.name.eq_ignore_ascii_case(name));
        removed = endpoints.len() != before;
        Ok(())
    })?;
    Ok(removed)
}

fn normalize_endpoint(req: UpsertOutboundEndpointReq) -> Result<OutboundWebhookEndpoint, String> {
    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err("endpoint name is required".to_string());
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(
            "endpoint name can only contain letters, numbers, '.', '_' and '-'".to_string(),
        );
    }
    let url = req.url.trim().to_string();
    validate_public_webhook_url(&url)?;
    let events = req
        .events
        .into_iter()
        .flat_map(|item| {
            item.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    Ok(OutboundWebhookEndpoint {
        name,
        url,
        secret_env: req.secret_env.trim().to_string(),
        events: if events.is_empty() {
            vec!["*".to_string()]
        } else {
            events
        },
        enabled: req.enabled,
    })
}

fn edit_outbound_config<F>(config_path: &FsPath, mut edit: F) -> Result<(), String>
where
    F: FnMut(&mut Vec<OutboundWebhookEndpoint>) -> Result<(), String>,
{
    let raw = if config_path.exists() {
        std::fs::read_to_string(config_path)
            .map_err(|e| format!("read {}: {e}", config_path.display()))?
    } else {
        String::new()
    };
    let mut doc: DocumentMut = raw.parse().map_err(|e| format!("parse TOML: {e}"))?;
    let mut endpoints = read_endpoint_tables(&doc)?;
    edit(&mut endpoints)?;
    write_endpoint_tables(&mut doc, &endpoints)?;
    captain_types::durable_fs::atomic_write(config_path, doc.to_string().as_bytes())
        .map_err(|e| format!("persist config.toml: {e}"))?;
    Ok(())
}

fn read_endpoint_tables(doc: &DocumentMut) -> Result<Vec<OutboundWebhookEndpoint>, String> {
    let Some(item) = doc.get("outbound_webhooks") else {
        return Ok(Vec::new());
    };
    let table = item
        .as_table()
        .ok_or_else(|| "outbound_webhooks is not a TOML table".to_string())?;
    let Some(endpoints) = table.get("endpoints") else {
        return Ok(Vec::new());
    };
    if let Some(array) = endpoints.as_array() {
        if array.is_empty() {
            return Ok(Vec::new());
        }
    }
    let aot = endpoints.as_array_of_tables().ok_or_else(|| {
        "outbound_webhooks.endpoints must be an empty array or an array of tables".to_string()
    })?;
    aot.iter().map(endpoint_from_table).collect()
}

fn endpoint_from_table(table: &Table) -> Result<OutboundWebhookEndpoint, String> {
    let name = table
        .get("name")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let url = table
        .get("url")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let secret_env = table
        .get("secret_env")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    let events = table
        .get("events")
        .and_then(|item| item.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec!["*".to_string()]);
    let enabled = table
        .get("enabled")
        .and_then(|item| item.as_bool())
        .unwrap_or(true);
    Ok(OutboundWebhookEndpoint {
        name,
        url,
        secret_env,
        events,
        enabled,
    })
}

fn write_endpoint_tables(
    doc: &mut DocumentMut,
    endpoints: &[OutboundWebhookEndpoint],
) -> Result<(), String> {
    if !doc.contains_key("outbound_webhooks") {
        let mut table = Table::new();
        table.set_implicit(false);
        doc.insert("outbound_webhooks", Item::Table(table));
    }
    let outbound = doc["outbound_webhooks"]
        .as_table_mut()
        .ok_or_else(|| "outbound_webhooks is not a TOML table".to_string())?;
    if !outbound.contains_key("enabled") {
        outbound.insert("enabled", value(!endpoints.is_empty()));
    }
    if !outbound.contains_key("timeout_secs") {
        outbound.insert("timeout_secs", value(10));
    }
    if !outbound.contains_key("max_attempts") {
        outbound.insert("max_attempts", value(3));
    }
    let mut aot = ArrayOfTables::new();
    for endpoint in endpoints {
        let mut table = Table::new();
        table.insert("name", value(endpoint.name.clone()));
        table.insert("url", value(endpoint.url.clone()));
        table.insert("secret_env", value(endpoint.secret_env.clone()));
        let mut events = Array::default();
        for event in &endpoint.events {
            events.push(event.clone());
        }
        table.insert("events", value(events));
        table.insert("enabled", value(endpoint.enabled));
        aot.push(table);
    }
    outbound.insert("endpoints", Item::ArrayOfTables(aot));
    Ok(())
}

/// Outbound event webhooks have no local-testing escape hatch (unlike
/// agent-API callbacks), so this always validates with `allow_local: false`.
/// See `captain_types::ssrf_guard` for the shared SSRF check this delegates
/// to — outbound event webhooks, agent-API egress callbacks, and
/// agent-API provisioning-time validation all share the same guard so a
/// future fix lands once instead of drifting across three copies.
pub(crate) fn validate_public_webhook_url(url: &str) -> Result<(), String> {
    captain_types::ssrf_guard::validate_outbound_callback_url(url, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::AgentId;
    use captain_types::event::{EventTarget, FileEventKind, TriggerId};

    #[test]
    fn endpoint_wildcard_and_prefix_matching() {
        let endpoint = OutboundWebhookEndpoint {
            events: vec!["project.*".to_string(), "model.fallback".to_string()],
            ..Default::default()
        };
        assert!(endpoint_matches(&endpoint, "project.created"));
        assert!(endpoint_matches(&endpoint, "model.fallback"));
        assert!(!endpoint_matches(&endpoint, "agent.spawned"));
    }

    #[test]
    fn public_url_guard_rejects_local_and_private() {
        assert!(validate_public_webhook_url("http://localhost:8080/hook").is_err());
        assert!(validate_public_webhook_url("http://127.0.0.1/hook").is_err());
        assert!(validate_public_webhook_url("http://192.168.1.5/hook").is_err());
        assert!(validate_public_webhook_url("https://example.com/hook").is_ok());
    }

    #[tokio::test]
    async fn prepare_webhook_delivery_clamps_attempts_and_signs() {
        const SECRET_ENV: &str = "CAPTAIN_TEST_WEBHOOK_SECRET_PREPARE";
        std::env::set_var(SECRET_ENV, "test-secret");
        let config = OutboundWebhooksConfig {
            timeout_secs: 0,
            max_attempts: 99,
            ..Default::default()
        };
        // An IP literal, not a domain: resolving it is instant and local
        // (no network I/O), keeping this test hermetic while still
        // exercising the same DNS-pinning code path as a real domain.
        let endpoint = OutboundWebhookEndpoint {
            name: "ops".to_string(),
            url: "https://1.1.1.1/hook".to_string(),
            secret_env: SECRET_ENV.to_string(),
            events: vec!["kernel.started".to_string()],
            enabled: true,
        };
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::KernelStarted),
        );

        let prepared = prepare_webhook_delivery(&config, &endpoint, "kernel.started", &event)
            .await
            .unwrap();

        assert_eq!(prepared.attempts, 8);
        assert!(prepared
            .signature
            .as_deref()
            .is_some_and(|signature| signature.starts_with("sha256=")));
        let body = String::from_utf8(prepared.body).unwrap();
        assert!(body.contains("\"event\":\"kernel.started\""));
        std::env::remove_var(SECRET_ENV);
    }

    /// Regression for the DNS-resolution gap: outbound event webhooks share
    /// the pinning code path with agent-API egress, and always validate
    /// with allow_local: false (no escape hatch exists for this feature).
    #[tokio::test]
    async fn prepare_webhook_delivery_rejects_loopback_target() {
        const SECRET_ENV: &str = "CAPTAIN_TEST_WEBHOOK_SECRET_LOOPBACK";
        std::env::set_var(SECRET_ENV, "test-secret");
        let config = OutboundWebhooksConfig::default();
        let endpoint = OutboundWebhookEndpoint {
            name: "ops".to_string(),
            url: "https://127.0.0.1/hook".to_string(),
            secret_env: SECRET_ENV.to_string(),
            events: vec!["kernel.started".to_string()],
            enabled: true,
        };
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::KernelStarted),
        );

        let err = prepare_webhook_delivery(&config, &endpoint, "kernel.started", &event)
            .await
            .unwrap_err();

        std::env::remove_var(SECRET_ENV);
        assert!(err.contains("loopback"), "got: {err}");
    }

    #[test]
    fn event_kind_maps_file_change() {
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::System(SystemEvent::FileChanged {
                trigger_id: TriggerId::new(),
                path: "/tmp/input.txt".into(),
                kind: FileEventKind::Modify,
                previous_path: None,
            }),
        );
        assert_eq!(event_kind(&event), "file.changed");
    }

    #[test]
    fn event_kind_maps_project_ask_user() {
        let agent_id = AgentId::new();
        let event = Event::new(
            agent_id,
            EventTarget::Broadcast,
            EventPayload::ChatStream(ChatStreamEvent::ProjectAskUser {
                agent_id,
                ask_id: "ask-42".to_string(),
                project_id: "project-1".to_string(),
                project_slug: "demo".to_string(),
                project_name: "Demo".to_string(),
                phase: "plan".to_string(),
                worker_role: "Planner".to_string(),
                question: "Quel chemin prendre ?".to_string(),
                options: Some(vec!["A".to_string(), "B".to_string()]),
            }),
        );
        assert_eq!(event_kind(&event), "project.ask_user");
    }

    #[test]
    fn custom_event_kind_uses_json_event_name() {
        let event = Event::new(
            AgentId::new(),
            EventTarget::Broadcast,
            EventPayload::Custom(br#"{"event":"project.created"}"#.to_vec()),
        );
        assert_eq!(event_kind(&event), "project.created");
        assert_eq!(event_text(&event), "project.created");
    }

    #[test]
    fn endpoint_config_crud_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "language = \"fr\"\n\n[outbound_webhooks]\ntimeout_secs = 7\n",
        )
        .unwrap();

        edit_outbound_config(&path, |endpoints| {
            endpoints.push(OutboundWebhookEndpoint {
                name: "n8n".to_string(),
                url: "https://example.com/hook".to_string(),
                secret_env: "N8N_SECRET".to_string(),
                events: vec!["project.*".to_string()],
                enabled: true,
            });
            Ok(())
        })
        .unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("language = \"fr\""));
        assert!(raw.contains("timeout_secs = 7"));
        assert!(raw.contains("[[outbound_webhooks.endpoints]]"));
        let doc: DocumentMut = raw.parse().unwrap();
        let endpoints = read_endpoint_tables(&doc).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].name, "n8n");
    }

    #[test]
    fn endpoint_config_accepts_default_empty_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[outbound_webhooks]\nenabled = false\nendpoints = []\n",
        )
        .unwrap();

        edit_outbound_config(&path, |endpoints| {
            assert!(endpoints.is_empty());
            endpoints.push(OutboundWebhookEndpoint {
                name: "audit".to_string(),
                url: "https://example.com/audit".to_string(),
                secret_env: String::new(),
                events: vec!["*".to_string()],
                enabled: false,
            });
            Ok(())
        })
        .unwrap();

        let doc: DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
        let endpoints = read_endpoint_tables(&doc).unwrap();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].name, "audit");
    }
}

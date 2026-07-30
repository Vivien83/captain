use crate::state::AppState;
use axum::{
    extract::{Query, State},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use std::{collections::HashMap, convert::Infallible, sync::Arc, time::Duration};
use tokio_stream::wrappers::ReceiverStream;

/// GET /api/audit/recent - Get recent audit log entries.
pub async fn audit_recent(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let n: usize = params
        .get("n")
        .or_else(|| params.get("limit"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .min(1000);

    let entries = state.kernel.audit_log.recent(n);
    let tip = state.kernel.audit_log.tip_hash();
    let integrity = state.kernel.audit_log.integrity_status();

    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "epoch": e.epoch,
                "hash_version": e.hash_version,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": e.action.to_string(),
                "detail": e.detail,
                "outcome": e.outcome,
                "prev_hash": e.prev_hash,
                "hash": e.hash,
            })
        })
        .collect();

    Json(serde_json::json!({
        "entries": items,
        "total": state.kernel.audit_log.len(),
        "tip_hash": tip,
        "integrity": integrity,
    }))
}

/// GET /api/audit/verify - Verify the audit chain integrity.
pub async fn audit_verify(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let integrity = state.kernel.audit_log.integrity_status();
    match state.kernel.audit_log.verify_integrity() {
        Ok(()) => {
            if integrity.entry_count == 0 {
                Json(serde_json::json!({
                    "valid": true,
                    "status": "healthy",
                    "message": "Audit hash chain is empty and ready",
                    "entries": 0,
                    "warning": "Audit log is empty - no events have been recorded yet",
                    "tip_hash": integrity.tip_hash,
                    "active_epoch": integrity.active_epoch,
                    "active_epoch_valid": integrity.active_epoch_valid,
                    "invalid_epochs": integrity.invalid_epochs,
                }))
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "status": "healthy",
                    "message": format!(
                        "Audit hash chain verified in epoch {}",
                        integrity.active_epoch
                    ),
                    "entries": integrity.entry_count,
                    "tip_hash": integrity.tip_hash,
                    "active_epoch": integrity.active_epoch,
                    "active_epoch_valid": integrity.active_epoch_valid,
                    "invalid_epochs": integrity.invalid_epochs,
                }))
            }
        }
        Err(msg) => Json(serde_json::json!({
            "valid": false,
            "status": "degraded",
            "message": "Audit integrity is degraded; the active recovery epoch remains observable",
            "error": msg,
            "entries": integrity.entry_count,
            "tip_hash": integrity.tip_hash,
            "active_epoch": integrity.active_epoch,
            "active_epoch_valid": integrity.active_epoch_valid,
            "invalid_epochs": integrity.invalid_epochs,
        })),
    }
}

/// GET /api/logs/stream - SSE endpoint for real-time audit log streaming.
pub async fn logs_stream(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> axum::response::Response {
    let level_filter = params.get("level").cloned().unwrap_or_default();
    let text_filter = params
        .get("filter")
        .cloned()
        .unwrap_or_default()
        .to_lowercase();

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(256);

    tokio::spawn(async move {
        let mut last_seq: u64 = 0;
        let mut first_poll = true;

        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;

            let entries = state.kernel.audit_log.recent(200);

            for entry in &entries {
                if !first_poll && entry.seq <= last_seq {
                    continue;
                }

                let action_str = entry.action.to_string();

                if !level_filter.is_empty() {
                    let classified = classify_audit_level(&action_str);
                    if classified != level_filter {
                        continue;
                    }
                }

                if !text_filter.is_empty() {
                    let haystack = format!("{} {} {}", action_str, entry.detail, entry.agent_id)
                        .to_lowercase();
                    if !haystack.contains(&text_filter) {
                        continue;
                    }
                }

                let json = serde_json::json!({
                    "seq": entry.seq,
                    "epoch": entry.epoch,
                    "timestamp": entry.timestamp,
                    "agent_id": entry.agent_id,
                    "action": action_str,
                    "detail": entry.detail,
                    "outcome": entry.outcome,
                    "hash": entry.hash,
                });
                let data = serde_json::to_string(&json).unwrap_or_default();
                if tx.send(Ok(Event::default().data(data))).await.is_err() {
                    return;
                }
            }

            if let Some(last) = entries.last() {
                last_seq = last.seq;
            }
            first_poll = false;
        }
    });

    let rx_stream = ReceiverStream::new(rx);
    Sse::new(rx_stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

fn classify_audit_level(action: &str) -> &'static str {
    let a = action.to_lowercase();
    if a.contains("error") || a.contains("fail") || a.contains("crash") || a.contains("denied") {
        "error"
    } else if a.contains("warn") || a.contains("block") || a.contains("kill") {
        "warn"
    } else {
        "info"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use captain_kernel::CaptainKernel;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;
    use tower::ServiceExt;

    #[tokio::test]
    async fn recent_endpoint_exposes_versioned_entries_and_integrity() {
        let temp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: temp.path().to_path_buf(),
            data_dir: temp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel
            .audit_log
            .record(
                "captain",
                captain_runtime::audit::AuditAction::ConfigChange,
                "api envelope proof",
                "ok",
            )
            .unwrap();
        let state = Arc::new(AppState {
            kernel: Arc::clone(&kernel),
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        });
        let app = Router::new()
            .route("/api/audit/recent", get(audit_recent))
            .with_state(state);

        let response = app
            .oneshot(
                Request::get("/api/audit/recent?n=200")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let payload: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let entry = payload["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["detail"] == "api envelope proof")
            .expect("recorded entry");

        assert_eq!(entry["epoch"], 0);
        assert_eq!(entry["hash_version"], 2);
        assert_eq!(entry["agent_id"], "captain");
        assert_eq!(entry["action"], "ConfigChange");
        assert_eq!(entry["prev_hash"].as_str().unwrap().len(), 64);
        assert_eq!(payload["integrity"]["valid"], true);
        assert_eq!(payload["integrity"]["active_epoch"], 0);
        kernel.shutdown();
    }
}

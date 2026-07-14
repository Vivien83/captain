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
        .and_then(|v| v.parse().ok())
        .unwrap_or(50)
        .min(1000);

    let entries = state.kernel.audit_log.recent(n);
    let tip = state.kernel.audit_log.tip_hash();

    let items: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": format!("{:?}", e.action),
                "detail": e.detail,
                "outcome": e.outcome,
                "hash": e.hash,
            })
        })
        .collect();

    Json(serde_json::json!({
        "entries": items,
        "total": state.kernel.audit_log.len(),
        "tip_hash": tip,
    }))
}

/// GET /api/audit/verify - Verify the audit chain integrity.
pub async fn audit_verify(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let entry_count = state.kernel.audit_log.len();
    match state.kernel.audit_log.verify_integrity() {
        Ok(()) => {
            if entry_count == 0 {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": 0,
                    "warning": "Audit log is empty - no events have been recorded yet",
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
            } else {
                Json(serde_json::json!({
                    "valid": true,
                    "entries": entry_count,
                    "tip_hash": state.kernel.audit_log.tip_hash(),
                }))
            }
        }
        Err(msg) => Json(serde_json::json!({
            "valid": false,
            "error": msg,
            "entries": entry_count,
        })),
    }
}

/// POST /api/audit/repair - Repair the Merkle hash chain.
pub async fn audit_repair(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let repaired = state.kernel.audit_log.repair_chain();
    tracing::info!(repaired = repaired, "Audit chain repaired");
    let valid = state.kernel.audit_log.verify_integrity().is_ok();
    Json(serde_json::json!({
        "repaired": repaired,
        "valid": valid,
        "entries": state.kernel.audit_log.len(),
        "tip_hash": state.kernel.audit_log.tip_hash(),
    }))
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

                let action_str = format!("{:?}", entry.action);

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

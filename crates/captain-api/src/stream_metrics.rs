//! Operator-safe latency metrics for streaming chat surfaces.

use captain_runtime::llm_driver::StreamEvent;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct StreamMetricHandle {
    stream_id: u64,
    started_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StreamMetricRecord {
    stream_id: u64,
    agent_id: String,
    surface: String,
    started_at: DateTime<Utc>,
    first_signal_ms: Option<u64>,
    first_signal_kind: Option<String>,
    first_token_ms: Option<u64>,
    total_ms: Option<u64>,
    status: StreamMetricStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamMetricStatus {
    Active,
    Completed,
}

#[derive(Debug, Default)]
struct StreamMetricsState {
    next_id: u64,
    active: HashMap<u64, StreamMetricRecord>,
    last: Option<StreamMetricRecord>,
    completed_streams: u64,
}

static STREAM_METRICS: OnceLock<Mutex<StreamMetricsState>> = OnceLock::new();

impl StreamMetricHandle {
    pub(crate) fn start(agent_id: impl Into<String>, surface: impl Into<String>) -> Self {
        let started_at = Instant::now();
        let stream_id = record_stream_started(agent_id.into(), surface.into());
        Self {
            stream_id,
            started_at,
        }
    }

    pub(crate) fn observe(&self, event: &StreamEvent) {
        record_stream_event(self.stream_id, event, self.started_at.elapsed());
    }

    pub(crate) fn finish(&self) {
        record_stream_finished(self.stream_id, self.started_at.elapsed());
    }
}

impl Drop for StreamMetricHandle {
    fn drop(&mut self) {
        record_stream_finished(self.stream_id, self.started_at.elapsed());
    }
}

pub(crate) fn status_json() -> serde_json::Value {
    let state = stream_metrics_state().lock().expect("stream metrics lock");
    let mut active: Vec<_> = state.active.values().cloned().collect();
    active.sort_by_key(|record| record.stream_id);
    serde_json::json!({
        "active": active.len(),
        "completed": state.completed_streams,
        "last": state.last.as_ref().map(record_json),
        "active_streams": active.iter().map(record_json).collect::<Vec<_>>(),
    })
}

pub(crate) fn first_signal_kind(event: &StreamEvent) -> Option<&'static str> {
    match event {
        StreamEvent::TextDelta { text } if !text.is_empty() => Some("text"),
        StreamEvent::ToolUseStart { .. } | StreamEvent::ToolUseEnd { .. } => Some("tool"),
        StreamEvent::ToolExecutionResult { .. } => Some("tool_result"),
        StreamEvent::ToolOutputDelta {
            stream: "progress",
            chunk,
            ..
        } if !chunk.trim().is_empty() => Some("tool_progress"),
        StreamEvent::IntermediateMessage { content } if !content.trim().is_empty() => {
            Some("intermediate")
        }
        StreamEvent::PhaseChange { phase, detail }
            if phase == "model_fallback"
                && detail
                    .as_deref()
                    .map(|value| !value.trim().is_empty())
                    .unwrap_or(false) =>
        {
            Some("phase")
        }
        _ => None,
    }
}

pub(crate) fn is_first_token_event(event: &StreamEvent) -> bool {
    matches!(event, StreamEvent::TextDelta { text } if !text.is_empty())
}

fn stream_metrics_state() -> &'static Mutex<StreamMetricsState> {
    STREAM_METRICS.get_or_init(|| {
        Mutex::new(StreamMetricsState {
            next_id: 1,
            ..Default::default()
        })
    })
}

fn record_stream_started(agent_id: String, surface: String) -> u64 {
    let mut state = stream_metrics_state().lock().expect("stream metrics lock");
    let stream_id = state.next_id.max(1);
    state.next_id = stream_id.saturating_add(1);
    state.active.insert(
        stream_id,
        StreamMetricRecord {
            stream_id,
            agent_id,
            surface,
            started_at: Utc::now(),
            first_signal_ms: None,
            first_signal_kind: None,
            first_token_ms: None,
            total_ms: None,
            status: StreamMetricStatus::Active,
        },
    );
    stream_id
}

fn record_stream_event(stream_id: u64, event: &StreamEvent, elapsed: Duration) {
    let mut state = stream_metrics_state().lock().expect("stream metrics lock");
    let Some(record) = state.active.get_mut(&stream_id) else {
        return;
    };
    let elapsed_ms = duration_ms(elapsed);
    if record.first_signal_ms.is_none() {
        if let Some(kind) = first_signal_kind(event) {
            record.first_signal_ms = Some(elapsed_ms);
            record.first_signal_kind = Some(kind.to_string());
        }
    }
    if record.first_token_ms.is_none() && is_first_token_event(event) {
        record.first_token_ms = Some(elapsed_ms);
    }
}

fn record_stream_finished(stream_id: u64, elapsed: Duration) {
    let mut state = stream_metrics_state().lock().expect("stream metrics lock");
    let Some(mut record) = state.active.remove(&stream_id) else {
        return;
    };
    record.total_ms = Some(duration_ms(elapsed));
    record.status = StreamMetricStatus::Completed;
    state.completed_streams = state.completed_streams.saturating_add(1);
    state.last = Some(record);
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn record_json(record: &StreamMetricRecord) -> serde_json::Value {
    serde_json::json!({
        "stream_id": record.stream_id,
        "agent_id": &record.agent_id,
        "surface": &record.surface,
        "started_at": record.started_at.to_rfc3339(),
        "first_signal_ms": record.first_signal_ms,
        "first_signal_kind": record.first_signal_kind.as_deref(),
        "first_token_ms": record.first_token_ms,
        "total_ms": record.total_ms,
        "status": match record.status {
            StreamMetricStatus::Active => "active",
            StreamMetricStatus::Completed => "completed",
        },
    })
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    let mut state = stream_metrics_state().lock().expect("stream metrics lock");
    *state = StreamMetricsState {
        next_id: 1,
        ..Default::default()
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::{StopReason, TokenUsage};
    use std::thread;

    #[test]
    fn stream_metrics_record_first_signal_token_and_total() {
        reset_for_tests();
        let handle = StreamMetricHandle::start("agent-1", "web");
        thread::sleep(Duration::from_millis(1));
        handle.observe(&StreamEvent::ToolOutputDelta {
            tool_use_id: "tool-1".to_string(),
            stream: "progress",
            chunk: "working".to_string(),
        });
        handle.observe(&StreamEvent::TextDelta {
            text: "hello".to_string(),
        });
        handle.finish();

        let status = status_json();
        assert_eq!(status["active"], serde_json::json!(0));
        assert_eq!(status["completed"], serde_json::json!(1));
        assert_eq!(status["last"]["agent_id"], serde_json::json!("agent-1"));
        assert_eq!(status["last"]["surface"], serde_json::json!("web"));
        assert_eq!(
            status["last"]["first_signal_kind"],
            serde_json::json!("tool_progress")
        );
        assert!(status["last"]["first_signal_ms"].as_u64().is_some());
        assert!(status["last"]["first_token_ms"].as_u64().is_some());
        assert!(status["last"]["total_ms"].as_u64().is_some());
    }

    #[test]
    fn stream_metrics_classify_visible_events_only() {
        assert_eq!(
            first_signal_kind(&StreamEvent::TextDelta {
                text: String::new()
            }),
            None
        );
        assert_eq!(
            first_signal_kind(&StreamEvent::ToolOutputDelta {
                tool_use_id: "tool-1".to_string(),
                stream: "stdout",
                chunk: "raw".to_string(),
            }),
            None
        );
        assert_eq!(
            first_signal_kind(&StreamEvent::ToolOutputDelta {
                tool_use_id: "tool-1".to_string(),
                stream: "progress",
                chunk: "50%".to_string(),
            }),
            Some("tool_progress")
        );
        assert!(!is_first_token_event(&StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
        }));
    }
}

use captain_runtime::llm_driver::StreamEvent;
use captain_types::message::{StopReason, TokenUsage};

#[derive(Default)]
pub(crate) struct DaemonStreamState {
    input_tokens: u64,
    output_tokens: u64,
}

impl DaemonStreamState {
    pub(crate) fn total_usage(&self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            ..Default::default()
        }
    }
}

pub(crate) fn daemon_stream_events_from_sse_line(
    line: &str,
    state: &mut DaemonStreamState,
) -> Vec<StreamEvent> {
    if line.is_empty() || line.starts_with("event:") {
        return Vec::new();
    }
    let Some(data) = line.strip_prefix("data: ") else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
        return Vec::new();
    };

    if let Some(event) = typed_daemon_stream_event(&json) {
        return vec![event];
    }

    let mut events = Vec::new();
    if let Some(content) = json.get("content").and_then(|value| value.as_str()) {
        events.push(StreamEvent::TextDelta {
            text: content.to_string(),
        });
    }
    if let Some(event) = legacy_tool_event(&json) {
        events.push(event);
    }
    if json.get("done").and_then(|value| value.as_bool()) == Some(true) {
        let usage = json.get("usage").cloned().unwrap_or_default();
        state.input_tokens += usage
            .get("input_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        state.output_tokens += usage
            .get("output_tokens")
            .and_then(|value| value.as_u64())
            .unwrap_or(0);
        events.push(StreamEvent::ContentComplete {
            stop_reason: StopReason::EndTurn,
            usage: state.total_usage(),
        });
    }
    events
}

fn typed_daemon_stream_event(json: &serde_json::Value) -> Option<StreamEvent> {
    let kind = json.get("type").and_then(|value| value.as_str())?;
    match kind {
        "tool_start" => {
            let tool = json.get("tool").and_then(|value| value.as_str())?;
            Some(StreamEvent::ToolUseStart {
                id: tool_start_id(json),
                name: tool.to_string(),
            })
        }
        "tool_end" => {
            let tool = json.get("tool").and_then(|value| value.as_str())?;
            Some(StreamEvent::ToolUseEnd {
                id: json
                    .get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                name: tool.to_string(),
                input: json.get("input").cloned().unwrap_or_default(),
            })
        }
        "tool_result" => {
            let tool = json.get("tool").and_then(|value| value.as_str())?;
            Some(StreamEvent::ToolExecutionResult {
                tool_use_id: tool_id(json),
                name: tool.to_string(),
                result_preview: json
                    .get("result")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string(),
                is_error: json
                    .get("is_error")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
            })
        }
        "tool_output_delta" => Some(StreamEvent::ToolOutputDelta {
            tool_use_id: tool_id(json),
            stream: daemon_tool_output_stream(json),
            chunk: json
                .get("chunk")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
        }),
        // Same shape as ws.rs's web relay — without this arm the daemon/TUI
        // surface never learns a question is pending and the tool call
        // blocks until the 300s ask_user timeout.
        "ask_user" => Some(StreamEvent::AskUser {
            question: json
                .get("question")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string(),
            options: json.get("options").and_then(|value| {
                value.as_array().map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
            }),
        }),
        _ => None,
    }
}

fn legacy_tool_event(json: &serde_json::Value) -> Option<StreamEvent> {
    let tool = json.get("tool").and_then(|value| value.as_str())?;
    if json.get("input").is_none() {
        Some(StreamEvent::ToolUseStart {
            id: String::new(),
            name: tool.to_string(),
        })
    } else {
        Some(StreamEvent::ToolUseEnd {
            id: String::new(),
            name: tool.to_string(),
            input: json["input"].clone(),
        })
    }
}

fn tool_id(json: &serde_json::Value) -> String {
    json.get("tool_use_id")
        .or_else(|| json.get("id"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn tool_start_id(json: &serde_json::Value) -> String {
    json.get("id")
        .or_else(|| json.get("tool_use_id"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn daemon_tool_output_stream(json: &serde_json::Value) -> &'static str {
    match json.get("stream").and_then(|value| value.as_str()) {
        Some("stderr") => "stderr",
        Some("progress") => "progress",
        _ => "stdout",
    }
}

#[cfg(test)]
#[path = "event_stream/tests.rs"]
mod tests;

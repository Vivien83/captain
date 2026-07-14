use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize)]
pub(super) struct BrowserNetworkEvent {
    sequence: u64,
    captured_at_ms: u64,
    event: String,
    url: Option<String>,
    request_method: Option<String>,
    status: Option<u64>,
    mime_type: Option<String>,
    error_text: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BrowserConsoleEvent {
    sequence: u64,
    captured_at_ms: u64,
    event: String,
    level: Option<String>,
    text: Option<String>,
    url: Option<String>,
    line: Option<u64>,
    column: Option<u64>,
}

pub(super) fn parse_network_event(
    json: &serde_json::Value,
    sequence: u64,
) -> Option<BrowserNetworkEvent> {
    let event = json.get("method")?.as_str()?;
    if !event.starts_with("Network.") {
        return None;
    }
    let params = json.get("params").unwrap_or(&serde_json::Value::Null);
    let now_ms = now_ms();

    match event {
        "Network.requestWillBeSent" => {
            let request = params.get("request").unwrap_or(&serde_json::Value::Null);
            Some(BrowserNetworkEvent {
                sequence,
                captured_at_ms: now_ms,
                event: event.to_string(),
                url: request
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                request_method: request
                    .get("method")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                status: None,
                mime_type: None,
                error_text: None,
            })
        }
        "Network.responseReceived" => {
            let response = params.get("response").unwrap_or(&serde_json::Value::Null);
            Some(BrowserNetworkEvent {
                sequence,
                captured_at_ms: now_ms,
                event: event.to_string(),
                url: response
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                request_method: None,
                status: response.get("status").and_then(|v| v.as_u64()),
                mime_type: response
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                error_text: None,
            })
        }
        "Network.loadingFailed" => Some(BrowserNetworkEvent {
            sequence,
            captured_at_ms: now_ms,
            event: event.to_string(),
            url: None,
            request_method: None,
            status: None,
            mime_type: None,
            error_text: params
                .get("errorText")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        }),
        _ => None,
    }
}

pub(super) fn parse_console_event(
    json: &serde_json::Value,
    sequence: u64,
) -> Option<BrowserConsoleEvent> {
    let event = json.get("method")?.as_str()?;
    let params = json.get("params").unwrap_or(&serde_json::Value::Null);
    let now_ms = now_ms();

    match event {
        "Runtime.consoleAPICalled" => console_api_called(params, sequence, now_ms, event),
        "Runtime.exceptionThrown" => exception_thrown(params, sequence, now_ms, event),
        "Log.entryAdded" => log_entry_added(params, sequence, now_ms, event),
        _ => None,
    }
}

fn console_api_called(
    params: &serde_json::Value,
    sequence: u64,
    captured_at_ms: u64,
    event: &str,
) -> Option<BrowserConsoleEvent> {
    let args = params
        .get("args")
        .and_then(|v| v.as_array())
        .map(|args| {
            args.iter()
                .filter_map(|arg| {
                    arg.get("value")
                        .and_then(|v| v.as_str())
                        .or_else(|| arg.get("description").and_then(|v| v.as_str()))
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|s| !s.is_empty());
    Some(BrowserConsoleEvent {
        sequence,
        captured_at_ms,
        event: event.to_string(),
        level: params
            .get("type")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        text: args,
        url: first_stack_frame_str(params, "url").map(str::to_string),
        line: first_stack_frame_u64(params, "lineNumber"),
        column: first_stack_frame_u64(params, "columnNumber"),
    })
}

fn exception_thrown(
    params: &serde_json::Value,
    sequence: u64,
    captured_at_ms: u64,
    event: &str,
) -> Option<BrowserConsoleEvent> {
    let details = params
        .get("exceptionDetails")
        .unwrap_or(&serde_json::Value::Null);
    Some(BrowserConsoleEvent {
        sequence,
        captured_at_ms,
        event: event.to_string(),
        level: Some("error".to_string()),
        text: details
            .get("exception")
            .and_then(|e| e.get("description"))
            .and_then(|v| v.as_str())
            .or_else(|| details.get("text").and_then(|v| v.as_str()))
            .map(str::to_string),
        url: details
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        line: details.get("lineNumber").and_then(|v| v.as_u64()),
        column: details.get("columnNumber").and_then(|v| v.as_u64()),
    })
}

fn log_entry_added(
    params: &serde_json::Value,
    sequence: u64,
    captured_at_ms: u64,
    event: &str,
) -> Option<BrowserConsoleEvent> {
    let entry = params.get("entry").unwrap_or(&serde_json::Value::Null);
    Some(BrowserConsoleEvent {
        sequence,
        captured_at_ms,
        event: event.to_string(),
        level: entry
            .get("level")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        text: entry
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        url: entry
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        line: entry.get("lineNumber").and_then(|v| v.as_u64()),
        column: None,
    })
}

fn first_stack_frame(params: &serde_json::Value) -> Option<&serde_json::Value> {
    params
        .get("stackTrace")
        .and_then(|s| s.get("callFrames"))
        .and_then(|v| v.as_array())
        .and_then(|frames| frames.first())
}

fn first_stack_frame_str<'a>(params: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    first_stack_frame(params)
        .and_then(|frame| frame.get(field))
        .and_then(|v| v.as_str())
}

fn first_stack_frame_u64(params: &serde_json::Value, field: &str) -> Option<u64> {
    first_stack_frame(params)
        .and_then(|frame| frame.get(field))
        .and_then(|v| v.as_u64())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_event_reads_stack_numeric_location() {
        let event = parse_console_event(
            &serde_json::json!({
                "method": "Runtime.consoleAPICalled",
                "params": {
                    "type": "log",
                    "args": [{"value": "hello"}, {"description": "world"}],
                    "stackTrace": {
                        "callFrames": [{
                            "url": "https://example.test/app.js",
                            "lineNumber": 12,
                            "columnNumber": 7
                        }]
                    }
                }
            }),
            42,
        )
        .expect("console event parsed");

        assert_eq!(event.sequence, 42);
        assert_eq!(event.level.as_deref(), Some("log"));
        assert_eq!(event.text.as_deref(), Some("hello world"));
        assert_eq!(event.url.as_deref(), Some("https://example.test/app.js"));
        assert_eq!(event.line, Some(12));
        assert_eq!(event.column, Some(7));
    }

    #[test]
    fn network_event_ignores_non_network_methods() {
        let event = parse_network_event(
            &serde_json::json!({
                "method": "Runtime.consoleAPICalled",
                "params": {}
            }),
            1,
        );

        assert!(event.is_none());
    }
}

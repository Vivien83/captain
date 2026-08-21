use serde_json::Value;

const MAX_LABEL_BYTES: usize = 120;
const MAX_MESSAGE_BYTES: usize = 128 * 1024;
const MAX_HISTORY_MESSAGES: usize = 400;
const MAX_SESSIONS: usize = 200;
const MAX_OPTIONS: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AgentInfo {
    pub id: String,
    pub name: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionInfo {
    pub id: String,
    pub label: String,
    pub message_count: u64,
    pub active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum StreamSignal {
    Text(String),
    ToolStarted(String),
    ToolFinished {
        name: String,
        failed: bool,
    },
    Phase(String),
    AskUser {
        question: String,
        options: Vec<String>,
    },
    Done,
}

pub(super) fn captain_agent(body: &Value) -> Option<AgentInfo> {
    body.as_array()?
        .iter()
        .filter_map(agent_from_value)
        .min_by_key(|agent| {
            if agent.name.eq_ignore_ascii_case("captain") {
                0
            } else {
                1
            }
        })
}

fn agent_from_value(value: &Value) -> Option<AgentInfo> {
    let id = safe_identifier(value.get("id")?)?;
    let name = safe_single_line(value.get("name"), MAX_LABEL_BYTES)?;
    let model = safe_single_line(value.get("model_name"), MAX_LABEL_BYTES)
        .unwrap_or_else(|| "model unavailable".to_string());
    Some(AgentInfo { id, name, model })
}

pub(super) fn sessions_from_body(body: &Value) -> Vec<SessionInfo> {
    body.get("sessions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(MAX_SESSIONS)
        .filter_map(session_from_value)
        .collect()
}

pub(super) fn session_from_value(value: &Value) -> Option<SessionInfo> {
    let id = safe_identifier(value.get("session_id")?)?;
    let label = safe_single_line(value.get("label"), MAX_LABEL_BYTES)
        .unwrap_or_else(|| format!("Session {}", &id[..id.len().min(8)]));
    Some(SessionInfo {
        id,
        label,
        message_count: value
            .get("message_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        active: value
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

pub(super) fn messages_from_body(body: &Value) -> Vec<ChatMessage> {
    body.get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .take(MAX_HISTORY_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|value| {
            let role = match value.get("role").and_then(Value::as_str) {
                Some("user") => MessageRole::User,
                Some("assistant") => MessageRole::Assistant,
                Some("system") => MessageRole::System,
                _ => return None,
            };
            let content = safe_multiline(value.get("content"), MAX_MESSAGE_BYTES)?;
            Some(ChatMessage { role, content })
        })
        .collect()
}

pub(super) fn stream_signal(body: &Value) -> Option<StreamSignal> {
    if body.get("done").and_then(Value::as_bool) == Some(true) {
        return Some(StreamSignal::Done);
    }
    if let Some(content) = safe_multiline(body.get("content"), MAX_MESSAGE_BYTES) {
        return Some(StreamSignal::Text(content));
    }
    match body.get("type").and_then(Value::as_str) {
        Some("tool_start") => {
            safe_single_line(body.get("tool"), MAX_LABEL_BYTES).map(StreamSignal::ToolStarted)
        }
        Some("tool_result") => {
            let name = safe_single_line(body.get("tool"), MAX_LABEL_BYTES)
                .unwrap_or_else(|| "tool".to_string());
            let failed = body
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Some(StreamSignal::ToolFinished { name, failed })
        }
        Some("ask_user") => {
            let question = safe_multiline(body.get("question"), MAX_MESSAGE_BYTES)?;
            let options = body
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .take(MAX_OPTIONS)
                .filter_map(|option| safe_single_line(Some(option), MAX_LABEL_BYTES))
                .collect();
            Some(StreamSignal::AskUser { question, options })
        }
        _ => safe_single_line(body.get("phase"), MAX_LABEL_BYTES).map(StreamSignal::Phase),
    }
}

pub(super) fn safe_identifier(value: &Value) -> Option<String> {
    let value = value.as_str()?.trim();
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    Some(value.to_string())
}

fn safe_single_line(value: Option<&Value>, max_bytes: usize) -> Option<String> {
    let value = value?.as_str()?.trim();
    if value.is_empty() || value.len() > max_bytes || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.to_string())
}

fn safe_multiline(value: Option<&Value>, max_bytes: usize) -> Option<String> {
    let value = value?.as_str()?.trim();
    if value.is_empty() || value.len() > max_bytes {
        return None;
    }
    let sanitized = value
        .chars()
        .map(|character| {
            if character == '\n' || character == '\t' || !character.is_control() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>();
    Some(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_projection_prefers_captain_and_bounds_remote_text() {
        let body = serde_json::json!([
            {"id":"worker-1","name":"Worker","model_name":"small"},
            {"id":"captain-1","name":"captain","model_name":"gpt-5.6-sol"},
            {"id":"bad/path","name":"Bad","model_name":"bad"}
        ]);
        let agent = captain_agent(&body).unwrap();
        assert_eq!(agent.id, "captain-1");
        assert_eq!(agent.model, "gpt-5.6-sol");
    }

    #[test]
    fn sessions_and_history_drop_invalid_or_unbounded_rows() {
        let sessions = sessions_from_body(&serde_json::json!({"sessions":[
            {"session_id":"session-1","label":"Projet","message_count":4,"active":true},
            {"session_id":"../../bad","label":"Bad"}
        ]}));
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].active);

        let messages = messages_from_body(&serde_json::json!({"messages":[
            {"role":"user","content":"hello\u{0000}world"},
            {"role":"tool","content":"private tool input"},
            {"role":"assistant","content":"done"}
        ]}));
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "hello world");
    }

    #[test]
    fn stream_projection_keeps_operator_signals_without_tool_payloads() {
        assert_eq!(
            stream_signal(&serde_json::json!({
                "type":"tool_result",
                "tool":"shell_exec",
                "result":"secret raw output",
                "is_error":false
            })),
            Some(StreamSignal::ToolFinished {
                name: "shell_exec".to_string(),
                failed: false,
            })
        );
        assert_eq!(
            stream_signal(&serde_json::json!({"content":"hello","done":false})),
            Some(StreamSignal::Text("hello".to_string()))
        );
    }
}

//! Runtime bridge between an owning agent turn and the durable workflow
//! episode store. Tool payloads are reduced to stable, redacted shapes before
//! they cross the persistence boundary.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use captain_capspec::{reviewed_effect, Effect};
use captain_memory::workflow_learning::{
    NewWorkflowEpisode, NewWorkflowEpisodeStep, WorkflowEpisodeStatus, WorkflowEpisodeStore,
    WorkflowStepOutcome, WorkflowStepStatus,
};
use captain_memory::MemorySubstrate;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::tool::ToolCall;
use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tracing::warn;

#[derive(Clone)]
pub(crate) struct WorkflowEpisodeContext {
    inner: Arc<WorkflowEpisodeInner>,
}

struct WorkflowEpisodeInner {
    store: WorkflowEpisodeStore,
    episode_id: String,
    next_ordinal: AtomicU32,
    dependency_frontier: Mutex<Vec<String>>,
    dependency_overrides: Mutex<HashMap<String, Vec<String>>>,
    deferred_frontier_tools: Mutex<HashSet<String>>,
    verification_hints: Mutex<HashMap<String, Option<String>>>,
}

struct EpisodeCompletionGuard {
    context: WorkflowEpisodeContext,
    completed: bool,
}

impl Drop for EpisodeCompletionGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.context.finish_as(
                WorkflowEpisodeStatus::Uncertain,
                Some("turn_future_dropped"),
            );
        }
    }
}

tokio::task_local! {
    static WORKFLOW_EPISODE: WorkflowEpisodeContext;
}

impl WorkflowEpisodeContext {
    pub(crate) fn begin(
        memory: &MemorySubstrate,
        agent_id: &str,
        session_id: &str,
        user_message: &str,
        origin_channel: Option<&str>,
        workspace_root: Option<&Path>,
    ) -> CaptainResult<Self> {
        let store = WorkflowEpisodeStore::new(memory.usage_conn());
        let episode_id = uuid::Uuid::new_v4().to_string();
        let (intent_redacted, secret_detected) = redact_intent(user_message);
        let intent_fingerprint = stable_hash(intent_redacted.as_bytes());
        let workspace_scope = workspace_root.map(|path| {
            let lexical = path.to_string_lossy();
            format!("workspace:{}", &stable_hash(lexical.as_bytes())[..16])
        });
        let project_id =
            crate::active_project::global().and_then(|registry| registry.get(agent_id));
        let now = chrono::Utc::now().timestamp_millis();
        let authoritative_id = store.begin_episode(&NewWorkflowEpisode {
            id: episode_id,
            session_id: session_id.to_string(),
            turn_id: uuid::Uuid::new_v4().to_string(),
            agent_id: agent_id.to_string(),
            origin_channel: origin_channel.map(str::to_string),
            project_id,
            workspace_scope,
            intent_redacted,
            intent_fingerprint,
            secret_detected,
            explicit_reuse_request: explicitly_requests_reuse(user_message),
            started_at_unix_ms: now,
        })?;
        Ok(Self {
            inner: Arc::new(WorkflowEpisodeInner {
                store,
                episode_id: authoritative_id,
                next_ordinal: AtomicU32::new(0),
                dependency_frontier: Mutex::new(Vec::new()),
                dependency_overrides: Mutex::new(HashMap::new()),
                deferred_frontier_tools: Mutex::new(HashSet::new()),
                verification_hints: Mutex::new(HashMap::new()),
            }),
        })
    }

    fn finish<T>(&self, result: &CaptainResult<T>) {
        let (status, reason) = match result {
            Ok(_) => (WorkflowEpisodeStatus::Succeeded, None),
            Err(CaptainError::ShuttingDown) => {
                (WorkflowEpisodeStatus::Stopped, Some("runtime_shutdown"))
            }
            Err(error) => (WorkflowEpisodeStatus::Failed, Some(error_class(error))),
        };
        self.finish_as(status, reason);
    }

    fn finish_as(&self, status: WorkflowEpisodeStatus, reason: Option<&str>) {
        if let Err(error) = self.inner.store.finish_episode(
            &self.inner.episode_id,
            status,
            reason,
            chrono::Utc::now().timestamp_millis(),
        ) {
            warn!(
                episode_id = %self.inner.episode_id,
                error = %error,
                "Failed to close workflow learning episode"
            );
        }
    }

    fn dependencies(&self) -> Vec<String> {
        self.inner
            .dependency_frontier
            .lock()
            .map(|frontier| frontier.clone())
            .unwrap_or_default()
    }

    fn advance_frontier(&self, tool_use_ids: Vec<String>) {
        if let Ok(mut frontier) = self.inner.dependency_frontier.lock() {
            *frontier = sorted_unique(tool_use_ids);
        }
    }
}

pub(crate) async fn run_in_workflow_episode<F, T>(
    context: Option<WorkflowEpisodeContext>,
    future: F,
) -> CaptainResult<T>
where
    F: Future<Output = CaptainResult<T>>,
{
    let Some(context) = context else {
        return future.await;
    };
    let mut guard = EpisodeCompletionGuard {
        context: context.clone(),
        completed: false,
    };
    let result = WORKFLOW_EPISODE.scope(context, future).await;
    guard.context.finish(&result);
    guard.completed = true;
    result
}

pub(crate) fn begin_episode_best_effort(
    memory: &MemorySubstrate,
    agent_id: &str,
    session_id: &str,
    user_message: &str,
    origin_channel: Option<&str>,
    workspace_root: Option<&Path>,
) -> Option<WorkflowEpisodeContext> {
    match WorkflowEpisodeContext::begin(
        memory,
        agent_id,
        session_id,
        user_message,
        origin_channel,
        workspace_root,
    ) {
        Ok(context) => Some(context),
        Err(error) => {
            warn!(error = %error, "Failed to begin workflow learning episode");
            None
        }
    }
}

pub(crate) fn record_tool_started(tool_use_id: &str, tool_name: &str, input: &Value) {
    let Ok(context) = WORKFLOW_EPISODE.try_with(Clone::clone) else {
        return;
    };
    let dependencies = context
        .inner
        .dependency_overrides
        .lock()
        .ok()
        .and_then(|mut overrides| overrides.remove(tool_use_id))
        .unwrap_or_else(|| context.dependencies());
    let (input_shape, secret_detected) = normalize_input_shape(input);
    let input_shape_json = serde_json::to_string(&input_shape).unwrap_or_else(|_| "null".into());
    let effect_class = effect_name(reviewed_effect(tool_name));
    if let Ok(mut hints) = context.inner.verification_hints.lock() {
        hints.insert(
            tool_use_id.to_string(),
            verification_hint(tool_name, reviewed_effect(tool_name), &input_shape),
        );
    }
    let step = NewWorkflowEpisodeStep {
        episode_id: context.inner.episode_id.clone(),
        tool_use_id: tool_use_id.to_string(),
        ordinal: context.inner.next_ordinal.fetch_add(1, Ordering::Relaxed),
        tool_name: tool_name.to_string(),
        dependency_ids_json: serde_json::to_string(&sorted_unique(dependencies))
            .unwrap_or_else(|_| "[]".to_string()),
        input_fingerprint: stable_hash(input_shape_json.as_bytes()),
        input_shape_json,
        effect_class: effect_class.to_string(),
        secret_detected,
        started_at_unix_ms: chrono::Utc::now().timestamp_millis(),
    };
    if let Err(error) = context.inner.store.begin_step(&step) {
        warn!(
            episode_id = %context.inner.episode_id,
            tool_use_id,
            tool_name,
            error = %error,
            "Failed to record workflow tool start"
        );
    }
}

pub(crate) fn record_tool_finished(
    tool_use_id: &str,
    tool_name: &str,
    is_error: bool,
    retry_count: u32,
    output_class: &str,
) {
    let Ok(context) = WORKFLOW_EPISODE.try_with(Clone::clone) else {
        return;
    };
    let verification_marker = context
        .inner
        .verification_hints
        .lock()
        .ok()
        .and_then(|mut hints| hints.remove(tool_use_id).flatten())
        .filter(|_| !is_error);
    let outcome = WorkflowStepOutcome {
        status: if is_error {
            WorkflowStepStatus::Failed
        } else {
            WorkflowStepStatus::Succeeded
        },
        output_class: Some(output_class.to_string()),
        verification_marker,
        retry_count,
        completed_at_unix_ms: chrono::Utc::now().timestamp_millis(),
    };
    if let Err(error) =
        context
            .inner
            .store
            .finish_step(&context.inner.episode_id, tool_use_id, &outcome)
    {
        warn!(
            episode_id = %context.inner.episode_id,
            tool_use_id,
            tool_name,
            error = %error,
            "Failed to record workflow tool completion"
        );
    }

    let deferred = context
        .inner
        .deferred_frontier_tools
        .lock()
        .map(|mut tools| tools.remove(tool_use_id))
        .unwrap_or(false);
    if !deferred {
        context.advance_frontier(vec![tool_use_id.to_string()]);
    }
}

pub(crate) fn record_terminal_tool_attempt(
    tool_call: &ToolCall,
    is_error: bool,
    output_class: &str,
) {
    record_tool_started(&tool_call.id, &tool_call.name, &tool_call.input);
    record_tool_finished(&tool_call.id, &tool_call.name, is_error, 0, output_class);
}

pub(crate) fn register_parallel_tool_dependencies(tool_use_id: &str, dependency_ids: Vec<String>) {
    let Ok(context) = WORKFLOW_EPISODE.try_with(Clone::clone) else {
        return;
    };
    if let Ok(mut overrides) = context.inner.dependency_overrides.lock() {
        overrides.insert(tool_use_id.to_string(), sorted_unique(dependency_ids));
    }
    if let Ok(mut tools) = context.inner.deferred_frontier_tools.lock() {
        tools.insert(tool_use_id.to_string());
    };
}

pub(crate) fn current_dependency_frontier() -> Vec<String> {
    WORKFLOW_EPISODE
        .try_with(WorkflowEpisodeContext::dependencies)
        .unwrap_or_default()
}

pub(crate) fn advance_dependency_frontier(tool_use_ids: Vec<String>) {
    if let Ok(context) = WORKFLOW_EPISODE.try_with(Clone::clone) {
        context.advance_frontier(tool_use_ids);
    }
}

fn normalize_input_shape(input: &Value) -> (Value, bool) {
    normalize_value(input, None)
}

fn normalize_value(value: &Value, key: Option<&str>) -> (Value, bool) {
    match value {
        Value::Null => (Value::Null, false),
        Value::Bool(value) => (Value::Bool(*value), false),
        Value::Number(_) => (Value::String("<number>".to_string()), false),
        Value::String(value) => normalize_string(value, key),
        Value::Array(values) => {
            let mut sensitive = false;
            let mut shapes = BTreeSet::new();
            for value in values {
                let (shape, found_sensitive) = normalize_value(value, key);
                sensitive |= found_sensitive;
                if let Ok(serialized) = serde_json::to_string(&shape) {
                    shapes.insert(serialized);
                }
            }
            let items = shapes
                .into_iter()
                .filter_map(|shape| serde_json::from_str(&shape).ok())
                .collect();
            let mut normalized = Map::new();
            normalized.insert("$array".to_string(), Value::Array(items));
            (Value::Object(normalized), sensitive)
        }
        Value::Object(values) => {
            let mut sensitive = false;
            let mut normalized = Map::new();
            let mut keys: Vec<&String> = values.keys().collect();
            keys.sort_unstable();
            for key in keys {
                let normalized_key = key.to_ascii_lowercase();
                let (shape, found_sensitive) = normalize_value(&values[key], Some(&normalized_key));
                sensitive |= found_sensitive;
                normalized.insert(normalized_key, shape);
            }
            (Value::Object(normalized), sensitive)
        }
    }
}

fn normalize_string(value: &str, key: Option<&str>) -> (Value, bool) {
    let key = key.unwrap_or_default();
    if sensitive_key(key)
        || crate::memory_policy::scan_for_secrets(value).is_some()
        || crate::pii_filter::detect_pii(value).is_some()
    {
        return (Value::String("<sensitive>".to_string()), true);
    }
    let class = if key == "command" {
        normalize_command_template(value)
    } else if key == "code" || key == "script" {
        "<command>".to_string()
    } else if key.contains("path") || key.contains("file") || Path::new(value).is_absolute() {
        "<path>".to_string()
    } else if key.contains("url") || key.contains("webhook") || looks_like_url(value) {
        "<url>".to_string()
    } else if key == "host" || key.ends_with("_host") || looks_like_ip(value) {
        "<host>".to_string()
    } else if matches!(
        key,
        "query" | "message" | "content" | "text" | "prompt" | "body"
    ) {
        "<text>".to_string()
    } else if key == "id" || key.ends_with("_id") || uuid::Uuid::parse_str(value).is_ok() {
        "<id>".to_string()
    } else if enum_key(key) && is_safe_enum(value) {
        format!("enum:{}", value.to_ascii_lowercase())
    } else {
        "<string>".to_string()
    };
    (Value::String(class), false)
}

fn normalize_command_template(command: &str) -> String {
    let Some(tokens) = shlex::split(command) else {
        return "<command>".to_string();
    };
    if tokens.is_empty() {
        return "<command>".to_string();
    }

    let mut normalized = Vec::new();
    let mut executable_position = true;
    for token in tokens.iter().take(32) {
        let value = if shell_operator(token) {
            executable_position = true;
            token.clone()
        } else if executable_position {
            executable_position = false;
            let basename = Path::new(token)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(token);
            if safe_command_word(basename) {
                basename.to_ascii_lowercase()
            } else {
                "<executable>".to_string()
            }
        } else if looks_like_url(token) {
            "<url>".to_string()
        } else if Path::new(token).is_absolute()
            || token.starts_with("./")
            || token.starts_with("../")
        {
            "<path>".to_string()
        } else if token.parse::<f64>().is_ok() {
            "<number>".to_string()
        } else if let Some((flag, _)) = token.split_once('=') {
            if flag.starts_with('-') && safe_command_word(flag) {
                format!("{}=<value>", flag.to_ascii_lowercase())
            } else {
                "<env>=<value>".to_string()
            }
        } else if token.starts_with('-') && safe_command_word(token) {
            token.to_ascii_lowercase()
        } else if reusable_command_word(token) {
            token.to_ascii_lowercase()
        } else {
            "<arg>".to_string()
        };
        normalized.push(value);
    }
    if tokens.len() > 32 {
        normalized.push("<more>".to_string());
    }
    format!("command:{}", normalized.join(" "))
}

fn shell_operator(value: &str) -> bool {
    matches!(value, "&&" | "||" | "|" | ";" | "&")
}

fn safe_command_word(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'+'))
}

fn reusable_command_word(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "add"
            | "build"
            | "cancel"
            | "check"
            | "checkout"
            | "clean"
            | "clone"
            | "commit"
            | "compose"
            | "create"
            | "delete"
            | "diff"
            | "doctor"
            | "down"
            | "exec"
            | "fetch"
            | "fmt"
            | "health"
            | "inspect"
            | "install"
            | "list"
            | "log"
            | "pull"
            | "push"
            | "remove"
            | "restart"
            | "restore"
            | "run"
            | "show"
            | "start"
            | "status"
            | "stop"
            | "switch"
            | "tag"
            | "test"
            | "uninstall"
            | "up"
            | "update"
            | "verify"
    )
}

fn redact_intent(message: &str) -> (String, bool) {
    let sensitive = crate::memory_policy::scan_for_secrets(message).is_some()
        || crate::pii_filter::detect_pii(message).is_some();
    if sensitive {
        return ("<redacted:sensitive-intent>".to_string(), true);
    }
    let mut redacted = url_regex().replace_all(message, "<url>").to_string();
    redacted = absolute_path_regex()
        .replace_all(&redacted, "<path>")
        .to_string();
    redacted = uuid_regex().replace_all(&redacted, "<id>").to_string();
    let compact = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    (truncate_chars(&compact, 512), false)
}

fn explicitly_requests_reuse(message: &str) -> bool {
    let normalized = fold_ascii(message);
    [
        "make this reusable",
        "save this workflow",
        "create a skill",
        "create a capability",
        "cree un skill",
        "cree une competence",
        "cree une capacite",
        "rends cette procedure reutilisable",
        "memorise cette procedure",
        "ajoute cette fonctionnalite",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn error_class(error: &CaptainError) -> &'static str {
    match error {
        CaptainError::AgentNotFound(_) => "agent_not_found",
        CaptainError::AgentAlreadyExists(_) => "agent_exists",
        CaptainError::CapabilityDenied(_) | CaptainError::AuthDenied(_) => "authority_denied",
        CaptainError::QuotaExceeded(_) => "quota_exceeded",
        CaptainError::InvalidState { .. } => "invalid_state",
        CaptainError::SessionNotFound(_) => "session_not_found",
        CaptainError::Memory(_) => "memory_failure",
        CaptainError::ToolExecution { .. } => "tool_failure",
        CaptainError::LlmDriver(_) => "provider_failure",
        CaptainError::Config(_) | CaptainError::ManifestParse(_) => "configuration_failure",
        CaptainError::Sandbox(_) => "sandbox_failure",
        CaptainError::Network(_) => "network_failure",
        CaptainError::Serialization(_) => "serialization_failure",
        CaptainError::MaxIterationsExceeded(_) => "max_iterations",
        CaptainError::ShuttingDown => "runtime_shutdown",
        CaptainError::Io(_) => "io_failure",
        CaptainError::Internal(_) => "internal_failure",
        CaptainError::MeteringError(_) => "metering_failure",
        CaptainError::InvalidInput(_) => "invalid_input",
    }
}

fn effect_name(effect: Effect) -> &'static str {
    match effect {
        Effect::Read => "read",
        Effect::Write => "write",
        Effect::External => "external",
        Effect::Destructive => "destructive",
    }
}

fn verification_hint(tool_name: &str, effect: Effect, input_shape: &Value) -> Option<String> {
    match effect {
        Effect::Read => Some("result_received".to_string()),
        Effect::Write | Effect::External => Some("operation_confirmed".to_string()),
        Effect::Destructive if destructive_verification_shape(tool_name, input_shape) => {
            Some("command_verified".to_string())
        }
        Effect::Destructive => None,
    }
}

fn destructive_verification_shape(tool_name: &str, input_shape: &Value) -> bool {
    if matches!(tool_name, "cargo" | "npm" | "pip") {
        return input_shape
            .get("subcommand")
            .and_then(Value::as_str)
            .and_then(|value| value.strip_prefix("enum:"))
            .is_some_and(verification_command_word);
    }
    if !matches!(
        tool_name,
        "shell_exec" | "execute_code" | "ssh_exec" | "docker_exec" | "docker_run"
    ) {
        return false;
    }
    let Some(template) = input_shape.get("command").and_then(Value::as_str) else {
        return false;
    };
    let final_command = template
        .strip_prefix("command:")
        .unwrap_or(template)
        .split(|character| matches!(character, '&' | '|' | ';'))
        .filter(|segment| !segment.trim().is_empty())
        .next_back()
        .unwrap_or_default();
    let words = final_command.split_whitespace().collect::<Vec<_>>();
    words.first().is_some_and(|word| read_only_command(word))
        || words.iter().any(|word| verification_command_word(word))
}

fn verification_command_word(value: &str) -> bool {
    matches!(
        value,
        "audit"
            | "check"
            | "diff"
            | "doctor"
            | "freeze"
            | "health"
            | "inspect"
            | "list"
            | "log"
            | "outdated"
            | "show"
            | "status"
            | "test"
            | "tree"
            | "verify"
            | "version"
    )
}

fn read_only_command(value: &str) -> bool {
    matches!(
        value,
        "cat"
            | "date"
            | "df"
            | "free"
            | "head"
            | "ls"
            | "ps"
            | "pwd"
            | "ss"
            | "stat"
            | "tail"
            | "uname"
            | "uptime"
            | "wc"
            | "whoami"
    )
}

fn sorted_unique(mut values: Vec<String>) -> Vec<String> {
    values.sort_unstable();
    values.dedup();
    values
}

fn sensitive_key(key: &str) -> bool {
    [
        "secret",
        "password",
        "passwd",
        "token",
        "api_key",
        "apikey",
        "authorization",
        "cookie",
        "credential",
        "private_key",
    ]
    .iter()
    .any(|part| key.contains(part))
}

fn enum_key(key: &str) -> bool {
    matches!(
        key,
        "action"
            | "format"
            | "language"
            | "level"
            | "method"
            | "mode"
            | "model"
            | "provider"
            | "status"
            | "subcommand"
            | "type"
    )
}

fn is_safe_enum(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 48
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn looks_like_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn looks_like_ip(value: &str) -> bool {
    value.parse::<std::net::IpAddr>().is_ok()
}

fn stable_hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn fold_ascii(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| match character {
            'à' | 'á' | 'â' | 'ä' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ô' | 'ö' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'ÿ' => 'y',
            other => other,
        })
        .collect()
}

fn url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"https?://[^\s]+").expect("valid URL regex"))
}

fn absolute_path_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?:/[A-Za-z0-9._~ -]+){2,}").expect("valid absolute path regex")
    })
}

fn uuid_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",
        )
        .expect("valid UUID regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_shape_is_order_independent_and_never_keeps_credentials() {
        let first = serde_json::json!({
            "mode": "FAST",
            "api_key": "sk-this-must-never-survive",
            "path": "/Users/alex/private/file.txt",
            "count": 12
        });
        let second = serde_json::json!({
            "count": 99,
            "path": "/another/machine/file.txt",
            "api_key": "different-secret",
            "mode": "fast"
        });
        let (first_shape, first_sensitive) = normalize_input_shape(&first);
        let (second_shape, second_sensitive) = normalize_input_shape(&second);
        assert_eq!(first_shape, second_shape);
        assert!(first_sensitive && second_sensitive);
        let serialized = serde_json::to_string(&first_shape).unwrap();
        assert!(!serialized.contains("sk-this"));
        assert!(!serialized.contains("/Users"));
        assert!(serialized.contains("<sensitive>"));
        assert!(serialized.contains("<path>"));
    }

    #[test]
    fn intent_redaction_is_bounded_and_marks_sensitive_content() {
        let (redacted, sensitive) = redact_intent("use token=ghp_abcdefghijklmnopqrstuvwxyz123456");
        assert!(sensitive);
        assert_eq!(redacted, "<redacted:sensitive-intent>");

        let (redacted, sensitive) = redact_intent(
            "Inspect https://example.com and /Users/alex/project/private/file.txt now",
        );
        assert!(!sensitive);
        assert!(!redacted.contains("example.com"));
        assert!(!redacted.contains("/Users"));
    }

    #[test]
    fn command_shape_keeps_only_reusable_operations() {
        let (shape, sensitive) = normalize_input_shape(&serde_json::json!({
            "command": "/usr/local/bin/cargo test --workspace && git checkout confidential-feature"
        }));
        assert!(!sensitive);
        assert_eq!(
            shape["command"],
            "command:cargo test --workspace && git checkout <arg>"
        );

        let (shape, sensitive) = normalize_input_shape(&serde_json::json!({
            "command": "curl https://private.example.test/api > /tmp/result"
        }));
        assert!(!sensitive);
        let serialized = serde_json::to_string(&shape).unwrap();
        assert!(serialized.contains("command:curl <url>"), "{serialized}");
        assert!(!serialized.contains("private.example.test"));
        assert!(!serialized.contains("/tmp/result"));

        let (shape, sensitive) = normalize_input_shape(&serde_json::json!({
            "subcommand": "test",
            "args": ["--workspace", "private-crate"]
        }));
        assert!(!sensitive);
        assert_eq!(shape["subcommand"], "enum:test");
        assert!(!serde_json::to_string(&shape)
            .unwrap()
            .contains("private-crate"));
    }

    #[test]
    fn destructive_verification_requires_a_real_control_command() {
        let (verified, _) = normalize_input_shape(&serde_json::json!({
            "command": "cargo build --release && cargo test --workspace"
        }));
        assert!(destructive_verification_shape("shell_exec", &verified));

        let (inspection, _) = normalize_input_shape(&serde_json::json!({
            "command": "uptime"
        }));
        assert!(destructive_verification_shape("ssh_exec", &inspection));

        let (mutation, _) = normalize_input_shape(&serde_json::json!({
            "command": "git checkout production"
        }));
        assert!(!destructive_verification_shape("shell_exec", &mutation));

        let (package_test, _) = normalize_input_shape(&serde_json::json!({
            "subcommand": "test",
            "args": []
        }));
        assert!(destructive_verification_shape("cargo", &package_test));
        let (package_install, _) = normalize_input_shape(&serde_json::json!({
            "subcommand": "install",
            "args": []
        }));
        assert!(!destructive_verification_shape("cargo", &package_install));
    }

    #[test]
    fn explicit_reuse_request_accepts_french_accents_without_guessing_generic_work() {
        assert!(explicitly_requests_reuse(
            "Crée une capacité réutilisable pour cette procédure"
        ));
        assert!(!explicitly_requests_reuse("Vérifie simplement le VPS"));
    }

    #[tokio::test]
    async fn parallel_dependency_scope_preserves_the_previous_frontier() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let context = WorkflowEpisodeContext::begin(
            &memory,
            "captain",
            "session-a",
            "inspect services",
            Some("telegram"),
            None,
        )
        .unwrap();
        let episode_id = context.inner.episode_id.clone();
        let store = context.inner.store.clone();

        let result: CaptainResult<()> = run_in_workflow_episode(Some(context), async {
            record_tool_started("root", "file_read", &serde_json::json!({"path": "/tmp/a"}));
            record_tool_finished("root", "file_read", false, 0, "success");
            let dependencies = current_dependency_frontier();
            register_parallel_tool_dependencies("peer-a", dependencies.clone());
            register_parallel_tool_dependencies("peer-b", dependencies);
            record_tool_started("peer-a", "web_search", &serde_json::json!({"query": "a"}));
            record_tool_finished("peer-a", "web_search", false, 0, "success");
            record_tool_started("peer-b", "web_search", &serde_json::json!({"query": "b"}));
            record_tool_finished("peer-b", "web_search", false, 0, "success");
            advance_dependency_frontier(vec!["peer-a".into(), "peer-b".into()]);
            Ok(())
        })
        .await;
        result.unwrap();

        let steps = store.list_steps(&episode_id).unwrap();
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[1].dependency_ids_json, r#"["root"]"#);
        assert_eq!(steps[2].dependency_ids_json, r#"["root"]"#);
        assert_eq!(
            store.get_episode(&episode_id).unwrap().unwrap().status,
            "succeeded"
        );
    }

    #[tokio::test]
    async fn real_tool_runner_records_success_and_dispatch_rejection() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let context = WorkflowEpisodeContext::begin(
            &memory,
            "captain",
            "session-tool-runner",
            "check the time then reject an unknown capability",
            Some("cli"),
            None,
        )
        .unwrap();
        let episode_id = context.inner.episode_id.clone();
        let store = context.inner.store.clone();

        run_in_workflow_episode(Some(context), async {
            let success = crate::tool_runner::execute_tool(
                "tool-success",
                "system_time",
                &serde_json::json!({}),
                None,
                None,
                Some("captain"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await;
            assert!(!success.is_error);

            let rejected = crate::tool_runner::execute_tool(
                "tool-rejected",
                "definitely_unknown_tool",
                &serde_json::json!({"token": "must-not-survive"}),
                None,
                None,
                Some("captain"),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await;
            assert!(rejected.is_error);
            Ok(())
        })
        .await
        .unwrap();

        let episode = store.get_episode(&episode_id).unwrap().unwrap();
        assert_eq!(episode.tool_attempt_count, 2);
        assert_eq!(episode.success_count, 1);
        assert_eq!(episode.failure_count, 1);
        assert!(episode.has_secret_input);
        let steps = store.list_steps(&episode_id).unwrap();
        assert_eq!(steps[0].status, "succeeded");
        assert_eq!(steps[1].status, "failed");
        assert!(!steps[1].input_shape_json.contains("must-not-survive"));
    }

    #[tokio::test]
    async fn pre_dispatch_rejection_is_a_terminal_attempt() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let context = WorkflowEpisodeContext::begin(
            &memory,
            "captain",
            "session-pre-dispatch",
            "reject an unsafe operation",
            Some("web"),
            None,
        )
        .unwrap();
        let episode_id = context.inner.episode_id.clone();
        let store = context.inner.store.clone();
        let tool_call = ToolCall {
            id: "blocked-tool".to_string(),
            name: "shell_exec".to_string(),
            input: serde_json::json!({"command": "unsafe", "token": "must-not-survive"}),
        };

        run_in_workflow_episode(Some(context), async {
            record_terminal_tool_attempt(&tool_call, true, "policy_blocked");
            Ok(())
        })
        .await
        .unwrap();

        let episode = store.get_episode(&episode_id).unwrap().unwrap();
        assert_eq!(episode.tool_attempt_count, 1);
        assert_eq!(episode.failure_count, 1);
        let step = &store.list_steps(&episode_id).unwrap()[0];
        assert_eq!(step.status, "failed");
        assert_eq!(step.output_class.as_deref(), Some("policy_blocked"));
        assert!(!step.input_shape_json.contains("must-not-survive"));
    }

    #[tokio::test]
    async fn verification_markers_keep_confirmed_work_separate_from_unverified_mutation() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let confirmed = WorkflowEpisodeContext::begin(
            &memory,
            "captain",
            "session-confirmed",
            "write a generated document",
            Some("cli"),
            None,
        )
        .unwrap();
        let confirmed_id = confirmed.inner.episode_id.clone();
        let store = confirmed.inner.store.clone();
        run_in_workflow_episode(Some(confirmed), async {
            record_tool_started(
                "write",
                "file_write",
                &serde_json::json!({"path": "/tmp/report", "content": "body"}),
            );
            record_tool_finished("write", "file_write", false, 0, "tool_success");
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            !store
                .get_episode(&confirmed_id)
                .unwrap()
                .unwrap()
                .has_unverified_mutation
        );
        assert_eq!(
            store.list_steps(&confirmed_id).unwrap()[0]
                .verification_marker
                .as_deref(),
            Some("operation_confirmed")
        );

        let unverified = WorkflowEpisodeContext::begin(
            &memory,
            "captain",
            "session-unverified",
            "change the active branch",
            Some("cli"),
            None,
        )
        .unwrap();
        let unverified_id = unverified.inner.episode_id.clone();
        run_in_workflow_episode(Some(unverified), async {
            record_tool_started(
                "shell",
                "shell_exec",
                &serde_json::json!({"command": "git checkout production"}),
            );
            record_tool_finished("shell", "shell_exec", false, 0, "tool_success");
            Ok(())
        })
        .await
        .unwrap();
        assert!(
            store
                .get_episode(&unverified_id)
                .unwrap()
                .unwrap()
                .has_unverified_mutation
        );
        assert!(store.list_steps(&unverified_id).unwrap()[0]
            .verification_marker
            .is_none());
    }

    #[tokio::test]
    async fn recorded_verified_integration_passes_the_explicit_analysis_path() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let context = WorkflowEpisodeContext::begin(
            &memory,
            "captain",
            "session-analysis",
            "Make this reusable: check the production server health",
            Some("telegram"),
            None,
        )
        .unwrap();
        let store = context.inner.store.clone();
        run_in_workflow_episode(Some(context), async {
            record_tool_started(
                "health",
                "ssh_health_check",
                &serde_json::json!({"host": "production-server"}),
            );
            record_tool_finished("health", "ssh_health_check", false, 0, "tool_success");
            Ok(())
        })
        .await
        .unwrap();

        let batch = crate::workflow_learning_analysis::analyze_workflow_evidence(
            store.list_pending_evidence(10).unwrap(),
            &Default::default(),
        );
        assert_eq!(batch.groups.len(), 1);
        assert_eq!(
            batch.groups[0].classification,
            crate::workflow_learning_analysis::WorkflowClassification::Capspec
        );
        assert!(batch.groups[0].eligible, "{:?}", batch.groups[0].reasons);
    }

    #[tokio::test]
    async fn dropping_a_live_turn_closes_it_as_uncertain() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let context = WorkflowEpisodeContext::begin(
            &memory,
            "captain",
            "session-cancelled",
            "run a long operation",
            Some("web"),
            None,
        )
        .unwrap();
        let episode_id = context.inner.episode_id.clone();
        let store = context.inner.store.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();

        let handle = tokio::spawn(run_in_workflow_episode(Some(context), async move {
            record_tool_started(
                "tool-long",
                "shell_exec",
                &serde_json::json!({"command": "sleep"}),
            );
            let _ = started_tx.send(());
            std::future::pending::<CaptainResult<()>>().await
        }));
        started_rx.await.unwrap();
        handle.abort();
        let _ = handle.await;

        let episode = store.get_episode(&episode_id).unwrap().unwrap();
        assert_eq!(episode.status, "uncertain");
        assert_eq!(
            episode.failure_reason.as_deref(),
            Some("turn_future_dropped")
        );
        assert_eq!(
            store.list_steps(&episode_id).unwrap()[0].status,
            "interrupted"
        );
    }
}

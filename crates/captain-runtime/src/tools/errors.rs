//! Structured tool error rendering.

/// Format a tool error with a retry suggestion appended when the analyzer
/// produced something actionable. The raw error stays first so the LLM can
/// still grep for keywords it knows.
pub(crate) fn render_error_with_suggestion(
    tool_name: &str,
    err: &str,
    suggestion: &crate::retry_transformer::RetryTransform,
) -> String {
    use crate::retry_transformer::RetryTransform;
    let hint: Option<String> = match suggestion {
        RetryTransform::None => recovery_docs_hint(tool_name),
        RetryTransform::Retry {
            attempt_cap,
            base_delay,
        } => Some(format!(
            "Retry suggestion: transient network error. Retry up to {attempt_cap}x with ~{}ms backoff.",
            base_delay.as_millis()
        )),
        RetryTransform::RetryWithTimeout { new_timeout } => Some(format!(
            "Retry suggestion: operation timed out. Retry with timeout >= {}s.",
            new_timeout.as_secs()
        )),
        RetryTransform::SuggestSudo { command, .. } => Some(format!(
            "Retry suggestion (requires user approval): run `{command}` to retry with elevated privileges."
        )),
        RetryTransform::SuggestInstall {
            package,
            install_cmd,
            ..
        } => Some(format!(
            "Retry suggestion (requires user approval): `{package}` is missing. Install with `{install_cmd}` then retry."
        )),
    };
    let structured = classify_tool_error(tool_name, err, suggestion).to_json(tool_name);
    match hint {
        Some(hint) => {
            format!("Error: {err}\n\n[tool_error]\n{structured}\n[/tool_error]\n\n{hint}")
        }
        None => format!("Error: {err}\n\n[tool_error]\n{structured}\n[/tool_error]"),
    }
}

struct StructuredToolError {
    code: &'static str,
    retryable: bool,
    severity: &'static str,
    next_action: &'static str,
}

impl StructuredToolError {
    fn to_json(&self, tool_name: &str) -> String {
        serde_json::json!({
            "code": self.code,
            "tool": tool_name,
            "retryable": self.retryable,
            "severity": self.severity,
            "next_action": self.next_action,
            "docs_query": format!("{tool_name} error recovery"),
        })
        .to_string()
    }
}

fn classify_tool_error(
    tool_name: &str,
    err: &str,
    suggestion: &crate::retry_transformer::RetryTransform,
) -> StructuredToolError {
    let lower = err.to_ascii_lowercase();
    if let Some(error) = classify_policy_error(tool_name, &lower) {
        return error;
    }
    classify_recoverable_error(&lower, suggestion)
}

fn classify_policy_error(tool_name: &str, lower: &str) -> Option<StructuredToolError> {
    if contains_any(
        lower,
        &[
            "security blocked",
            "secret literal",
            "raw secret",
            "taint violation",
        ],
    ) {
        return Some(security_blocked_error());
    }
    if contains_any(lower, &["permission denied", "allowed_tools", "approval"]) {
        return Some(permission_denied_error());
    }
    if contains_any(lower, &["missing", "required", "invalid", "must be"]) {
        return Some(invalid_tool_input_error());
    }
    if contains_any(lower, &["credential", "api key", "token"])
        || matches!(tool_name, "secret_read" | "secret_write" | "config_setup")
    {
        return Some(credential_unavailable_error());
    }
    None
}

fn classify_recoverable_error(
    lower: &str,
    suggestion: &crate::retry_transformer::RetryTransform,
) -> StructuredToolError {
    use crate::retry_transformer::RetryTransform;

    match suggestion {
        RetryTransform::Retry { .. } | RetryTransform::RetryWithTimeout { .. } => {
            transient_failure_error()
        }
        RetryTransform::SuggestSudo { .. } => privilege_required_error(),
        RetryTransform::SuggestInstall { .. } => dependency_missing_error(),
        RetryTransform::None => classify_error_without_retry_hint(lower),
    }
}

fn classify_error_without_retry_hint(lower: &str) -> StructuredToolError {
    if contains_any(lower, &["not found", "no such file", "unknown"]) {
        return target_not_found_error();
    }
    if contains_any(lower, &["timeout", "timed out"]) {
        return timeout_error();
    }
    tool_failed_error()
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn security_blocked_error() -> StructuredToolError {
    StructuredToolError {
        code: "security_blocked",
        retryable: false,
        severity: "high",
        next_action: "Do not retry with raw secret material. Store secrets with secret_write, then use a native integration, config_setup, or a skill env_inject reference.",
    }
}

fn permission_denied_error() -> StructuredToolError {
    StructuredToolError {
        code: "permission_denied",
        retryable: false,
        severity: "medium",
        next_action: "Use capability_search/tool_search/captain_docs to choose an allowed tool, or request explicit user approval when the operation needs it.",
    }
}

fn invalid_tool_input_error() -> StructuredToolError {
    StructuredToolError {
        code: "invalid_tool_input",
        retryable: false,
        severity: "low",
        next_action:
            "Fix the tool input according to the schema, then retry once with corrected parameters.",
    }
}

fn credential_unavailable_error() -> StructuredToolError {
    StructuredToolError {
        code: "credential_unavailable",
        retryable: false,
        severity: "medium",
        next_action: "Check for the credential with secret_read; if absent, ask the user for it or persist the provided value with secret_write before retrying.",
    }
}

fn transient_failure_error() -> StructuredToolError {
    StructuredToolError {
        code: "transient_failure",
        retryable: true,
        severity: "low",
        next_action: "Retry with backoff or a longer timeout, then inspect logs/status if the second attempt fails.",
    }
}

fn privilege_required_error() -> StructuredToolError {
    StructuredToolError {
        code: "privilege_required",
        retryable: false,
        severity: "medium",
        next_action: "Ask for explicit user approval before retrying with elevated privileges.",
    }
}

fn dependency_missing_error() -> StructuredToolError {
    StructuredToolError {
        code: "dependency_missing",
        retryable: false,
        severity: "medium",
        next_action: "Ask for approval to install the missing dependency, then retry and verify the tool is available.",
    }
}

fn target_not_found_error() -> StructuredToolError {
    StructuredToolError {
        code: "target_not_found",
        retryable: false,
        severity: "low",
        next_action: "List or inspect the available targets before retrying; do not guess identifiers or paths.",
    }
}

fn timeout_error() -> StructuredToolError {
    StructuredToolError {
        code: "timeout",
        retryable: true,
        severity: "low",
        next_action: "Retry with a longer timeout if the operation is still relevant; otherwise inspect service health first.",
    }
}

fn tool_failed_error() -> StructuredToolError {
    StructuredToolError {
        code: "tool_failed",
        retryable: false,
        severity: "medium",
        next_action: "Read captain_docs for the tool family, inspect the current state, then retry with a justified correction.",
    }
}

fn recovery_docs_hint(tool_name: &str) -> Option<String> {
    if matches!(tool_name, "captain_docs" | "tool_search" | "ask_user") {
        return None;
    }
    Some(format!(
        "Recovery hint: if the next action is unclear, call \
         captain_docs({{\"query\":\"{tool_name} error recovery\"}}) before asking the user or giving up."
    ))
}

/// Tools that are safe to retry on transient errors.
pub(crate) fn is_retryable_tool(name: &str) -> bool {
    matches!(
        name,
        "memory_save"
            | "memory_store"
            | "memory_recall"
            | "cron_create"
            | "cron_list"
            | "cron_update"
            | "cron_cancel"
            | "file_trigger_list"
            | "file_trigger_set_enabled"
            | "knowledge_query"
            | "knowledge_add_entity"
            | "knowledge_add_relation"
            | "config_read"
            | "secret_read"
    )
}

pub(crate) fn is_write_tool_that_must_not_be_masked(name: &str) -> bool {
    matches!(
        name,
        "memory_save"
            | "memory_store"
            | "cron_create"
            | "cron_update"
            | "cron_cancel"
            | "knowledge_add_entity"
            | "knowledge_add_relation"
            | "model_switch_apply"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_policy_keeps_validation_visible_for_write_tools() {
        assert!(is_retryable_tool("memory_save"));
        assert!(is_write_tool_that_must_not_be_masked("memory_save"));
        assert!(is_retryable_tool("secret_read"));
        assert!(!is_write_tool_that_must_not_be_masked("secret_read"));
        assert!(!is_retryable_tool("shell_exec"));
    }

    #[test]
    fn meta_tools_do_not_self_loop_to_docs() {
        assert!(recovery_docs_hint("captain_docs").is_none());
        assert!(recovery_docs_hint("tool_search").is_none());
        assert!(recovery_docs_hint("file_read").is_some());
    }

    #[test]
    fn classifier_keeps_policy_errors_ahead_of_retry_hints() {
        let error = classify_tool_error(
            "shell_exec",
            "Security blocked: approval required for raw secret literal",
            &crate::retry_transformer::RetryTransform::Retry {
                attempt_cap: 2,
                base_delay: std::time::Duration::from_millis(250),
            },
        );

        assert_eq!(error.code, "security_blocked");
        assert!(!error.retryable);
        assert_eq!(error.severity, "high");
    }

    #[test]
    fn classifier_keeps_credential_tools_explicit() {
        let error = classify_tool_error(
            "secret_read",
            "lookup failed",
            &crate::retry_transformer::RetryTransform::None,
        );

        assert_eq!(error.code, "credential_unavailable");
        assert_eq!(error.severity, "medium");
        assert!(error.next_action.contains("secret_read"));
    }

    #[test]
    fn classifier_distinguishes_recoverable_none_cases() {
        let missing = classify_tool_error(
            "file_read",
            "No such file or directory",
            &crate::retry_transformer::RetryTransform::None,
        );
        let timeout = classify_tool_error(
            "web_fetch",
            "operation timed out",
            &crate::retry_transformer::RetryTransform::None,
        );
        let generic = classify_tool_error(
            "browser_click",
            "element detached",
            &crate::retry_transformer::RetryTransform::None,
        );

        assert_eq!(missing.code, "target_not_found");
        assert_eq!(timeout.code, "timeout");
        assert!(timeout.retryable);
        assert_eq!(generic.code, "tool_failed");
    }
}

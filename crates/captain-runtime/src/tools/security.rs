//! Security guardrails applied at tool execution boundaries.

use captain_types::taint::{TaintLabel, TaintSink, TaintedValue};
use std::collections::HashSet;
use tracing::warn;

/// Check if a shell command should be blocked by taint tracking.
pub(crate) fn check_taint_shell_exec(command: &str) -> Option<String> {
    if let Some(reason) = crate::subprocess_sandbox::contains_shell_metacharacters(command) {
        return Some(format!("Shell metacharacter injection blocked: {reason}"));
    }

    let suspicious_patterns = ["curl ", "wget ", "| sh", "| bash", "base64 -d", "eval "];
    for pattern in &suspicious_patterns {
        if command.contains(pattern) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::ExternalNetwork);
            let tainted = TaintedValue::new(command, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::shell_exec()) {
                warn!(command = crate::str_utils::safe_truncate_str(command, 80), %violation, "Shell taint check failed");
                return Some(violation.to_string());
            }
        }
    }
    None
}

/// Check if a URL should be blocked by taint tracking before network fetch.
pub(crate) fn check_taint_net_fetch(url: &str) -> Option<String> {
    let exfil_patterns = [
        "api_key=",
        "apikey=",
        "token=",
        "secret=",
        "password=",
        "Authorization:",
    ];
    for pattern in &exfil_patterns {
        if url.to_lowercase().contains(&pattern.to_lowercase()) {
            let mut labels = HashSet::new();
            labels.insert(TaintLabel::Secret);
            let tainted = TaintedValue::new(url, labels, "llm_tool_call");
            if let Err(violation) = tainted.check_sink(&TaintSink::net_fetch()) {
                warn!(url = crate::str_utils::safe_truncate_str(url, 80), %violation, "Net fetch taint check failed");
                return Some(violation.to_string());
            }
        }
    }
    None
}

/// Check browser batch navigation steps for secret-bearing URLs.
pub(crate) fn check_taint_browser_batch(input: &serde_json::Value) -> Option<String> {
    let steps = input.get("steps")?.as_array()?;
    for step in steps {
        let action = step
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if matches!(action.as_str(), "navigate" | "browser_navigate") {
            if let Some(url) = step.get("url").and_then(|v| v.as_str()) {
                if let Some(violation) = check_taint_net_fetch(url) {
                    return Some(violation);
                }
            }
        }
    }
    None
}

/// Block LLM-controlled sink content that contains literal secrets.
pub(crate) fn ensure_no_secret_literal(
    tool_name: &str,
    field: &str,
    text: &str,
) -> Result<(), String> {
    if let Some(kind) = crate::memory_policy::scan_for_secrets(text) {
        return Err(format!(
            "Security blocked: `{tool_name}.{field}` contains a literal secret-looking value \
             ({kind}). Do not write, execute, log, or retransmit raw API keys/tokens/passwords. \
             Recovery: store new credentials with `secret_write`, verify existing credentials \
             with `secret_read` only for masked presence, then use a native integration or a \
             skill with `[requirements.env_inject]` so the vault injects the value at runtime. \
             Generated files/scripts/commands may contain only env-var references such as \
             `GEMINI_API_KEY`, never the raw key."
        ));
    }
    Ok(())
}

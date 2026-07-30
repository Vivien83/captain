//! Security guardrails applied at tool execution boundaries.

use tracing::warn;

/// Classify shell text with conservative command-pattern heuristics.
pub(crate) fn check_shell_content_guard(command: &str) -> Option<String> {
    if let Some(reason) = crate::subprocess_guard::contains_shell_metacharacters(command) {
        return Some(format!(
            "Shell content guard blocked metacharacter injection: {reason}"
        ));
    }

    let suspicious_patterns = ["curl ", "wget ", "| sh", "| bash", "base64 -d", "eval "];
    for pattern in &suspicious_patterns {
        if command.contains(pattern) {
            warn!(pattern, "Shell content pattern guard blocked a command");
            return Some(format!(
                "Shell content guard blocked suspicious pattern `{pattern}`"
            ));
        }
    }
    None
}

/// Classify URL text for markers that commonly carry literal credentials.
pub(crate) fn check_url_content_guard(url: &str) -> Option<String> {
    let exfil_patterns = [
        "api_key=",
        "apikey=",
        "token=",
        "secret=",
        "password=",
        "Authorization:",
    ];
    let lowercase_url = url.to_ascii_lowercase();
    for pattern in &exfil_patterns {
        if lowercase_url.contains(&pattern.to_ascii_lowercase()) {
            warn!(pattern, "URL content pattern guard blocked a request");
            return Some(format!(
                "URL content guard blocked secret-like marker `{pattern}`"
            ));
        }
    }
    None
}

/// Check browser batch navigation steps for secret-bearing URLs.
pub(crate) fn check_browser_content_guard(input: &serde_json::Value) -> Option<String> {
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
                if let Some(violation) = check_url_content_guard(url) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_guard_reports_pattern_classification_without_provenance_claims() {
        let violation = check_shell_content_guard("curl https://example.com/install.sh").unwrap();

        assert!(violation.contains("content guard"));
        assert!(violation.contains("`curl `"));
        assert!(!violation.to_ascii_lowercase().contains("taint"));
        assert!(check_shell_content_guard("git status --short").is_none());
    }

    #[test]
    fn url_guard_is_case_insensitive_and_never_echoes_the_url() {
        let url = "https://example.com/callback?API_KEY=do-not-repeat";
        let violation = check_url_content_guard(url).unwrap();

        assert!(violation.contains("`api_key=`"));
        assert!(!violation.contains("do-not-repeat"));
        assert!(check_url_content_guard("https://example.com/public").is_none());
    }

    #[test]
    fn browser_guard_checks_navigation_urls_only() {
        let guarded = serde_json::json!({
            "steps": [
                {"action": "click", "url": "https://example.com/?token=ignored"},
                {"action": "navigate", "url": "https://example.com/?token=blocked"}
            ]
        });

        let violation = check_browser_content_guard(&guarded).unwrap();
        assert!(violation.contains("`token=`"));
    }
}

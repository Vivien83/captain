//! Deterministic failure classification for command tools composed by CapSpec.

use captain_types::tool::ToolResult;

/// Command tools historically return a readable transcript even when the
/// child exits unsuccessfully. That is useful to an interactive model, but a
/// deterministic CapSpec step must not commit a non-zero exit as success.
pub fn normalize_capspec_tool_result(tool_name: &str, mut result: ToolResult) -> ToolResult {
    if result.is_error {
        return result;
    }
    if command_exit_code(tool_name, &result.content).is_some_and(|code| code != 0) {
        result.is_error = true;
    }
    result
}

fn command_exit_code(tool_name: &str, content: &str) -> Option<i64> {
    match tool_name {
        "shell_exec" | "cargo" | "npm" | "pip" => prefixed_exit_code(content),
        "execute_code" => structured_exit_code(content),
        "ssh_exec" | "ssh_health_check" => remote_exit_code(content),
        _ => None,
    }
}

fn prefixed_exit_code(content: &str) -> Option<i64> {
    content
        .lines()
        .next()?
        .strip_prefix("Exit code: ")?
        .trim()
        .parse()
        .ok()
}

fn structured_exit_code(content: &str) -> Option<i64> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get("exit_code")?
        .as_i64()
}

fn remote_exit_code(content: &str) -> Option<i64> {
    content
        .strip_prefix("[exit ")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::ContentBlock;

    fn result(content: &str) -> ToolResult {
        ToolResult {
            tool_use_id: "capspec:test".to_string(),
            content: content.to_string(),
            is_error: false,
            transient_content: Vec::<ContentBlock>::new(),
        }
    }

    #[test]
    fn package_nonzero_exit_is_a_capspec_error() {
        let result = normalize_capspec_tool_result(
            "cargo",
            result("Exit code: 101\n\nSTDOUT:\n\nSTDERR:\ncompile failed"),
        );
        assert!(result.is_error);
        assert!(result.content.contains("compile failed"));
    }

    #[test]
    fn successful_command_results_remain_successful() {
        assert!(
            !normalize_capspec_tool_result(
                "shell_exec",
                result("Exit code: 0\n\nSTDOUT:\nok\nSTDERR:\n")
            )
            .is_error
        );
        assert!(
            !normalize_capspec_tool_result(
                "execute_code",
                result(r#"{"exit_code":0,"stdout":"ok"}"#)
            )
            .is_error
        );
    }

    #[test]
    fn structured_and_remote_nonzero_exits_are_errors() {
        assert!(
            normalize_capspec_tool_result(
                "execute_code",
                result(r#"{"exit_code":2,"stderr":"bad input"}"#)
            )
            .is_error
        );
        assert!(
            normalize_capspec_tool_result(
                "ssh_exec",
                result("[exit 7 on root@example:22, 10ms]\n--- stdout ---\n")
            )
            .is_error
        );
    }

    #[test]
    fn unknown_or_still_running_results_are_not_guessed() {
        assert!(!normalize_capspec_tool_result("file_read", result("Exit code: 9")).is_error);
        assert!(
            !normalize_capspec_tool_result(
                "ssh_exec",
                result("[exit ? on root@example:22, timeout_mode=review_window]")
            )
            .is_error
        );
    }
}

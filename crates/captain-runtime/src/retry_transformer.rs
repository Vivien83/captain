//! Failure-pattern-aware retry suggestions (v3.10h).
//!
//! When a tool call fails, a human operator usually knows what to try
//! next: `Permission denied` → maybe sudo; `command not found` → maybe
//! install; `connection refused` → retry after a short backoff;
//! `timeout` → retry with a longer deadline. This module encodes those
//! moves as [`RetryTransform`] variants so the agent loop can surface
//! a concrete next-step to the user (with confirmation when the move
//! escalates privileges or installs packages) instead of just
//! bubbling the raw error back.
//!
//! The classifier is a pure function over the captured error message.
//! Wiring into `tool_runner.rs` is deferred; this module provides the
//! primitives with tests so the integration can be a small patch.

use std::time::Duration;

/// A suggested transformation for a failed tool call.
///
/// `SuggestSudo` and `SuggestInstall` are flagged `requires_approval`
/// because they escalate privilege or mutate the host — the agent
/// should surface an inline confirmation before applying them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryTransform {
    /// No match — let the error bubble up verbatim.
    None,
    /// Retry identically after an exponential backoff.
    Retry {
        attempt_cap: u32,
        base_delay: Duration,
    },
    /// Retry, but with a longer timeout. Use for transient network
    /// lag or slow-starting containers.
    RetryWithTimeout { new_timeout: Duration },
    /// Propose re-running the command under `sudo`. Never auto-applied.
    SuggestSudo {
        command: String,
        requires_approval: bool,
    },
    /// Propose installing the missing binary. Heuristic: pick the
    /// package manager likely to be present on the host.
    SuggestInstall {
        package: String,
        install_cmd: String,
        requires_approval: bool,
    },
}

impl RetryTransform {
    pub fn requires_approval(&self) -> bool {
        matches!(
            self,
            RetryTransform::SuggestSudo {
                requires_approval: true,
                ..
            } | RetryTransform::SuggestInstall {
                requires_approval: true,
                ..
            }
        )
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            RetryTransform::None => "none",
            RetryTransform::Retry { .. } => "retry",
            RetryTransform::RetryWithTimeout { .. } => "retry_with_timeout",
            RetryTransform::SuggestSudo { .. } => "suggest_sudo",
            RetryTransform::SuggestInstall { .. } => "suggest_install",
        }
    }
}

/// Context passed to [`analyze`] so the same error text can produce
/// different suggestions depending on what ran and on which host.
#[derive(Debug, Clone, Default)]
pub struct ErrorContext<'a> {
    /// The tool that failed — used to scope certain suggestions.
    pub tool_name: &'a str,
    /// For `shell_exec` / `execute_code`: the raw command line. Used
    /// to compose `sudo <cmd>` and to grep the missing binary name.
    pub command: Option<&'a str>,
    /// Host OS hint: "macos", "linux", "windows", or "unknown".
    /// Picks brew vs apt vs choco for install suggestions.
    pub host_os: &'a str,
}

/// Classify `stderr` (or equivalent) into a retry suggestion.
pub fn analyze(stderr: &str, ctx: &ErrorContext) -> RetryTransform {
    let hay = stderr.to_ascii_lowercase();

    // ORDER MATTERS: "permission denied" would also match "permission"
    // alone, so we keep the most specific rules first.

    if hay.contains("permission denied") || hay.contains("operation not permitted") {
        let base = ctx.command.unwrap_or("").trim();
        if !base.is_empty() && !base.starts_with("sudo ") {
            return RetryTransform::SuggestSudo {
                command: format!("sudo {base}"),
                requires_approval: true,
            };
        }
        return RetryTransform::None;
    }

    if let Some(pkg) = extract_missing_command(&hay) {
        let install_cmd = match ctx.host_os {
            "macos" => format!("brew install {pkg}"),
            "linux" => format!("sudo apt install -y {pkg}"),
            "windows" => format!("choco install {pkg}"),
            _ => format!("install {pkg}"),
        };
        return RetryTransform::SuggestInstall {
            package: pkg,
            install_cmd,
            requires_approval: true,
        };
    }

    if hay.contains("connection refused")
        || hay.contains("econnrefused")
        || hay.contains("connection reset")
    {
        return RetryTransform::Retry {
            attempt_cap: 3,
            base_delay: Duration::from_millis(500),
        };
    }

    if hay.contains("timed out") || hay.contains("timeout") || hay.contains("deadline exceeded") {
        return RetryTransform::RetryWithTimeout {
            new_timeout: Duration::from_secs(60),
        };
    }

    RetryTransform::None
}

/// Parse a shell stderr looking for `bash: foo: command not found`-style
/// output and return the missing command name. Returns `None` when the
/// pattern is absent or the token is suspicious (contains whitespace,
/// path separators, etc.).
fn extract_missing_command(hay_lower: &str) -> Option<String> {
    const NEEDLE: &str = "command not found";
    let idx = hay_lower.find(NEEDLE)?;

    // Pattern examples covered:
    //   bash: foo: command not found
    //   zsh: command not found: foo
    //   sh: 1: foo: not found
    // We try both "word before NEEDLE between colons" and "word after
    // NEEDLE after a colon" to cover both shells.

    // Strategy 1: last token before NEEDLE, enclosed by ": ".
    let before = &hay_lower[..idx].trim_end_matches(": ");
    if let Some(last_colon) = before.rfind(": ") {
        let candidate = before[last_colon + 2..].trim();
        if looks_like_command(candidate) {
            return Some(candidate.to_string());
        }
    }

    // Strategy 2: first token after NEEDLE, past a colon.
    let after = &hay_lower[idx + NEEDLE.len()..];
    if let Some(colon) = after.find(':') {
        let candidate = after[colon + 1..]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim();
        if looks_like_command(candidate) {
            return Some(candidate.to_string());
        }
    }

    None
}

fn looks_like_command(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && s.len() <= 64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(cmd: Option<&'static str>, os: &'static str) -> ErrorContext<'static> {
        ErrorContext {
            tool_name: "shell_exec",
            command: cmd,
            host_os: os,
        }
    }

    #[test]
    fn permission_denied_suggests_sudo() {
        let t = analyze(
            "cat: /etc/shadow: Permission denied",
            &ctx(Some("cat /etc/shadow"), "linux"),
        );
        match t {
            RetryTransform::SuggestSudo {
                command,
                requires_approval,
            } => {
                assert_eq!(command, "sudo cat /etc/shadow");
                assert!(requires_approval);
            }
            other => panic!("expected SuggestSudo, got {other:?}"),
        }
    }

    #[test]
    fn permission_denied_without_command_is_noop() {
        let t = analyze("permission denied", &ctx(None, "linux"));
        assert_eq!(t, RetryTransform::None);
    }

    #[test]
    fn already_sudo_is_not_re_suggested() {
        let t = analyze(
            "Permission denied",
            &ctx(Some("sudo rm /etc/hosts"), "linux"),
        );
        assert_eq!(t, RetryTransform::None);
    }

    #[test]
    fn command_not_found_bash_suggests_install_macos() {
        let t = analyze(
            "bash: ripgrep: command not found",
            &ctx(Some("ripgrep"), "macos"),
        );
        match t {
            RetryTransform::SuggestInstall {
                package,
                install_cmd,
                ..
            } => {
                assert_eq!(package, "ripgrep");
                assert_eq!(install_cmd, "brew install ripgrep");
            }
            other => panic!("expected SuggestInstall, got {other:?}"),
        }
    }

    #[test]
    fn command_not_found_zsh_suggests_install_linux() {
        let t = analyze(
            "zsh: command not found: jq",
            &ctx(Some("jq .name"), "linux"),
        );
        match t {
            RetryTransform::SuggestInstall {
                package,
                install_cmd,
                ..
            } => {
                assert_eq!(package, "jq");
                assert_eq!(install_cmd, "sudo apt install -y jq");
            }
            other => panic!("expected SuggestInstall, got {other:?}"),
        }
    }

    #[test]
    fn connection_refused_triggers_retry() {
        let t = analyze(
            "curl: (7) Failed to connect to localhost port 5432: Connection refused",
            &ctx(None, "linux"),
        );
        match t {
            RetryTransform::Retry { attempt_cap, .. } => assert_eq!(attempt_cap, 3),
            other => panic!("expected Retry, got {other:?}"),
        }
    }

    #[test]
    fn timeout_triggers_retry_with_timeout() {
        let t = analyze("operation timed out after 5s", &ctx(None, "linux"));
        match t {
            RetryTransform::RetryWithTimeout { new_timeout } => {
                assert_eq!(new_timeout, Duration::from_secs(60));
            }
            other => panic!("expected RetryWithTimeout, got {other:?}"),
        }
    }

    #[test]
    fn unknown_error_returns_none() {
        let t = analyze("something went wrong", &ctx(None, "linux"));
        assert_eq!(t, RetryTransform::None);
    }

    #[test]
    fn requires_approval_flags_escalations() {
        assert!(analyze("Permission denied", &ctx(Some("ls /root"), "linux")).requires_approval());
        assert!(
            analyze("bash: ripgrep: command not found", &ctx(None, "macos")).requires_approval()
        );
        assert!(!analyze("operation timed out", &ctx(None, "linux")).requires_approval());
    }

    #[test]
    fn kind_label_is_stable() {
        assert_eq!(RetryTransform::None.kind_label(), "none");
        assert_eq!(
            RetryTransform::Retry {
                attempt_cap: 1,
                base_delay: Duration::from_millis(1),
            }
            .kind_label(),
            "retry"
        );
    }

    #[test]
    fn looks_like_command_rejects_garbage() {
        assert!(looks_like_command("rg"));
        assert!(looks_like_command("python3"));
        assert!(!looks_like_command(""));
        assert!(!looks_like_command("foo bar"));
        assert!(!looks_like_command("/usr/bin/rg"));
        assert!(!looks_like_command(&"x".repeat(100)));
    }
}

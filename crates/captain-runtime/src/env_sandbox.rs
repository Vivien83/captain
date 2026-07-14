//! Hardened env propagation for child processes spawned by Captain.
//!
//! Captain's daemon loads API keys (OPENAI_API_KEY, GROQ_API_KEY,
//! SLACK_TOKEN, …) from secrets.env into its own process env. Without
//! `env_clear`, every subprocess we spawn — `execute_code` snippets,
//! `process_start` long-runners, `skill_execute` runners — inherits that
//! env and an LLM-supplied script can exfiltrate the secrets in one
//! `os.environ['OPENAI_API_KEY']` call.
//!
//! `apply_minimal_env` strips the inherited env and re-attaches the bare
//! minimum the interpreter / binary needs to actually run:
//! - `PATH` so the binary resolves on `bash`, `node`, `python`, etc.
//! - `HOME` for `$HOME`-based caches (pip, node, …)
//! - `LANG` for stdout encoding (matters on macOS more than Linux)
//! - `USER` because some tools crash without it
//!
//! Anything outside this whitelist must be passed explicitly as a CLI
//! argument or set inside the snippet itself — never inherited.

/// Strip the inherited environment from `cmd` and re-attach only the four
/// keys the interpreter needs to run. Calling this BEFORE `spawn()` is what
/// stops a child process from reading Captain's secrets.
pub fn apply_minimal_env(cmd: &mut tokio::process::Command) {
    cmd.env_clear();
    for key in MINIMAL_ENV_WHITELIST {
        if let Ok(value) = std::env::var(key) {
            cmd.env(key, value);
        }
    }
}

/// Same as `apply_minimal_env` for the blocking `std::process::Command`.
/// Some call sites still go through std (older code, sync paths); this
/// keeps the whitelist defined in exactly one place.
pub fn apply_minimal_env_std(cmd: &mut std::process::Command) {
    cmd.env_clear();
    for key in MINIMAL_ENV_WHITELIST {
        if let Ok(value) = std::env::var(key) {
            cmd.env(key, value);
        }
    }
}

/// The deliberate, minimal env whitelist. Any addition is a real security
/// decision — keep this list short.
pub const MINIMAL_ENV_WHITELIST: &[&str] = &["PATH", "HOME", "LANG", "USER"];

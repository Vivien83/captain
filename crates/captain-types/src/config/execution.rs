//! Execution and subprocess safety configuration.

use serde::{Deserialize, Serialize};

/// Q.9 — High-level Captain security profile, chosen at `captain setup`
/// (or via `/security` later). Independent of `ExecSecurityMode` (which
/// controls *how* shell commands are sandboxed once allowed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CriticalMode {
    /// **Open** (default) — Captain runs everything. If a hyper-critical
    /// pattern is detected (rm -rf /, dd of=/dev/, DROP DATABASE, etc.),
    /// the user gets a one-shot approval modal. Never asked twice for
    /// the same flow.
    #[default]
    #[serde(alias = "default")]
    Open,
    /// **Safe** — hyper-critical patterns are blocked outright (no modal).
    Safe,
    /// **Paranoid** — every shell-affecting tool requires approval.
    Paranoid,
}

/// Shell/exec security mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecSecurityMode {
    /// Block all shell execution.
    #[serde(alias = "none", alias = "disabled")]
    Deny,
    /// Only allow commands in safe_bins or allowed_commands.
    #[serde(alias = "restricted")]
    Allowlist,
    /// Allow all commands except those in blocked_commands.
    #[default]
    #[serde(alias = "allow", alias = "all", alias = "unrestricted")]
    Full,
}

/// Shell/exec security policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecPolicy {
    /// Security mode: "deny" blocks all, "allowlist" only allows listed,
    /// "full" allows all except blocked_commands.
    pub mode: ExecSecurityMode,
    /// Commands that bypass allowlist (stdin-only utilities).
    pub safe_bins: Vec<String>,
    /// Global command allowlist (when mode = allowlist).
    pub allowed_commands: Vec<String>,
    /// Commands blocked in Full mode (dangerous operations).
    #[serde(default = "default_blocked_commands")]
    pub blocked_commands: Vec<String>,
    /// Max execution timeout in seconds. Default: 30.
    pub timeout_secs: u64,
    /// Max output size in bytes. Default: 100KB.
    pub max_output_bytes: usize,
    /// No-output idle timeout in seconds. When > 0, kills processes that
    /// produce no stdout/stderr output for this duration. Default: 30.
    #[serde(default = "default_no_output_timeout")]
    pub no_output_timeout_secs: u64,
    /// Q.9 — High-level Captain security profile. Default: Open.
    #[serde(default)]
    pub critical_mode: CriticalMode,
}

fn default_no_output_timeout() -> u64 {
    30
}

/// Default dangerous-command blocklist (v3.8j).
///
/// Hardened from the original 13 patterns to 60+ by merging conservative
/// `tools/approval.py::DANGEROUS_PATTERNS` with Captain self-protection
/// and data-destruction guards. Documented in `.hora/patterns.yaml`.
fn default_blocked_commands() -> Vec<String> {
    [
        // destructive_fs
        "rm -rf /",
        "rm -rf /*",
        "rm -rf ~",
        "rm -rf $HOME",
        "rm -rf --no-preserve-root",
        "mkfs",
        "mkfs.ext4",
        "mkfs.xfs",
        "dd if=",
        "dd of=/dev/",
        "> /dev/sda",
        "> /dev/nvme",
        "shred -u /",
        "wipefs",
        "mv / /dev/null",
        // fork_bomb
        ":(){ :|:&};:",
        ":(){:|:&};:",
        "while true; do fork; done",
        "perl -e 'fork while 1'",
        // system_control
        "shutdown",
        "reboot",
        "halt",
        "poweroff",
        "init 0",
        "init 6",
        "systemctl poweroff",
        "systemctl reboot",
        // privilege_escalation
        "sudo su -",
        "chmod -R 777 /",
        "chmod 4755",
        "chown -R root",
        "setcap cap_setuid",
        "usermod -aG sudo",
        // credential_exfil
        "cat .env",
        "cat ~/.ssh/id_",
        "cat ~/.aws/credentials",
        "cat /etc/shadow",
        "cat /etc/passwd | curl",
        "curl -d @.env",
        "wget --post-file=.env",
        // self_termination
        "pkill -f captain",
        "pkill captain",
        "killall captain",
        "kill -9 1",
        "rm -rf ~/.captain",
        // db_destructive
        "DROP DATABASE",
        "DROP SCHEMA",
        "DROP TABLE IF EXISTS users",
        "TRUNCATE TABLE",
        "DELETE FROM users",
        "DELETE FROM accounts",
        // git_destructive
        "git push --force origin main",
        "git push -f origin main",
        "git push --force origin master",
        "git reset --hard HEAD~",
        "git clean -fdx",
        "git branch -D main",
        "git branch -D master",
        // unsafe_pipes
        "curl | sh",
        "curl | bash",
        "wget -O - | sh",
        "wget -O - | bash",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

impl Default for ExecPolicy {
    fn default() -> Self {
        Self {
            mode: ExecSecurityMode::default(),
            safe_bins: vec![
                "sleep", "true", "false", "cat", "sort", "uniq", "cut", "tr", "head", "tail", "wc",
                "date", "echo", "printf", "basename", "dirname", "pwd", "env",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            allowed_commands: Vec::new(),
            blocked_commands: default_blocked_commands(),
            timeout_secs: 30,
            max_output_bytes: 100 * 1024,
            no_output_timeout_secs: default_no_output_timeout(),
            critical_mode: CriticalMode::default(),
        }
    }
}

/// Reason a subprocess was terminated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationReason {
    /// Process exited normally.
    Exited(i32),
    /// Absolute timeout exceeded.
    AbsoluteTimeout,
    /// No output timeout exceeded.
    NoOutputTimeout,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v3.8j — default blocklist covers categories beyond the original 13 patterns.
    /// Regression guard: any removal should be intentional.
    #[test]
    fn default_blocked_commands_cover_runtime_risk_categories() {
        let blocked = default_blocked_commands();
        assert!(
            blocked.len() >= 50,
            "expected >= 50 blocked patterns after v3.8j port, got {}",
            blocked.len()
        );
        // Spot-check one entry per category.
        let must_contain = [
            "rm -rf /",                     // destructive_fs
            ":(){ :|:&};:",                 // fork_bomb
            "shutdown",                     // system_control
            "chmod -R 777 /",               // privilege_escalation
            "cat .env",                     // credential_exfil
            "pkill -f captain",             // self_termination
            "DROP DATABASE",                // db_destructive
            "git push --force origin main", // git_destructive
            "curl | sh",                    // unsafe_pipes
        ];
        for needle in must_contain {
            assert!(
                blocked.iter().any(|p| p == needle),
                "blocklist missing expected pattern: {needle}"
            );
        }
    }

    #[test]
    fn exec_policy_default_keeps_safe_runtime_limits() {
        let policy = ExecPolicy::default();
        assert_eq!(policy.mode, ExecSecurityMode::Full);
        assert_eq!(policy.timeout_secs, 30);
        assert_eq!(policy.max_output_bytes, 100 * 1024);
        assert_eq!(policy.no_output_timeout_secs, 30);
        assert_eq!(policy.critical_mode, CriticalMode::Open);
        assert!(policy.safe_bins.iter().any(|bin| bin == "cat"));
        assert!(policy.blocked_commands.iter().any(|cmd| cmd == "rm -rf /"));
    }

    #[test]
    fn exec_policy_deserializes_missing_idle_timeout_with_default() {
        let policy: ExecPolicy = toml::from_str(
            r#"
mode = "deny"
safe_bins = []
allowed_commands = []
blocked_commands = []
timeout_secs = 5
max_output_bytes = 1024
"#,
        )
        .unwrap();

        assert_eq!(policy.mode, ExecSecurityMode::Deny);
        assert_eq!(policy.no_output_timeout_secs, 30);
        assert_eq!(policy.critical_mode, CriticalMode::Open);
    }
}

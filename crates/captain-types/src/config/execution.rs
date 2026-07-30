//! Execution and subprocess safety configuration.

use serde::{Deserialize, Serialize};

/// Native shell/program execution runs as a child of the Captain host process.
pub const HOST_EXECUTION_BACKEND: &str = "host_process";
/// The host boundary clears and reconstructs the child environment. It does not
/// create an operating-system isolation boundary.
pub const HOST_EXECUTION_ISOLATION_LEVEL: &str = "environment_scrub";
/// Host subprocesses do not use namespaces, seccomp, Landlock, chroot, or a
/// container. Isolated WASM and Docker execution are separate explicit tools.
pub const HOST_EXECUTION_OS_ISOLATED: bool = false;
/// Dangerous-command recognition is a normalized lexical guard, not a shell
/// proof or an adversarial-code sandbox.
pub const DANGEROUS_COMMAND_GUARD_LEVEL: &str = "normalized_lexical_heuristic";

/// Q.9 — High-level Captain security profile, chosen at `captain setup`
/// (or via `/security` later). Independent of `ExecSecurityMode` (which
/// controls which shell commands may run once reviewed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CriticalMode {
    /// **Open** — a detected hyper-critical command may proceed after a
    /// content-bound, one-shot operator approval.
    Open,
    /// **Safe** (default) — hyper-critical commands are blocked outright.
    #[serde(alias = "default")]
    Safe,
    /// **Paranoid** — every shell-affecting tool requires approval.
    Paranoid,
}

impl Default for CriticalMode {
    fn default() -> Self {
        Self::Safe
    }
}

impl CriticalMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Safe => "safe",
            Self::Paranoid => "paranoid",
        }
    }
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

impl ExecSecurityMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Deny => "deny",
            Self::Allowlist => "allowlist",
            Self::Full => "full",
        }
    }
}

/// Honest, machine-readable posture for host subprocess execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HostExecutionPosture {
    pub backend: &'static str,
    pub isolation_level: &'static str,
    pub os_isolation: bool,
    pub environment_scrub: bool,
    pub dangerous_command_guard: &'static str,
    pub policy_mode: ExecSecurityMode,
    pub critical_mode: CriticalMode,
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
    /// Q.9 — High-level Captain security profile. Default: Safe.
    #[serde(default)]
    pub critical_mode: CriticalMode,
}

impl ExecPolicy {
    pub const fn host_execution_posture(&self) -> HostExecutionPosture {
        HostExecutionPosture {
            backend: HOST_EXECUTION_BACKEND,
            isolation_level: HOST_EXECUTION_ISOLATION_LEVEL,
            os_isolation: HOST_EXECUTION_OS_ISOLATED,
            environment_scrub: true,
            dangerous_command_guard: DANGEROUS_COMMAND_GUARD_LEVEL,
            policy_mode: self.mode,
            critical_mode: self.critical_mode,
        }
    }
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
    fn exec_policy_default_keeps_autonomy_with_a_safe_critical_floor() {
        let policy = ExecPolicy::default();
        assert_eq!(policy.mode, ExecSecurityMode::Full);
        assert_eq!(policy.timeout_secs, 30);
        assert_eq!(policy.max_output_bytes, 100 * 1024);
        assert_eq!(policy.no_output_timeout_secs, 30);
        assert_eq!(policy.critical_mode, CriticalMode::Safe);
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
        assert_eq!(policy.critical_mode, CriticalMode::Safe);
    }

    #[test]
    fn host_execution_posture_never_claims_os_isolation() {
        let posture = ExecPolicy::default().host_execution_posture();
        assert_eq!(posture.backend, "host_process");
        assert_eq!(posture.isolation_level, "environment_scrub");
        assert!(!posture.os_isolation);
        assert!(posture.environment_scrub);
        assert_eq!(
            posture.dangerous_command_guard,
            "normalized_lexical_heuristic"
        );
        assert_eq!(posture.policy_mode, ExecSecurityMode::Full);
        assert_eq!(posture.critical_mode, CriticalMode::Safe);
    }
}

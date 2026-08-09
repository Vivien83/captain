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
/// Captain never selects an isolation backend from host capabilities. The
/// operator or agent must name an explicit Docker tool or WASM agent.
pub const ISOLATION_ROUTING_MODE: &str = "explicit_only";
pub const EXPLICIT_ISOLATION_BACKENDS: &[&str] = &["docker_exec", "wasm_agent"];

/// Deployment posture for agent-controlled execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionProfile {
    /// Single-user workstation. Host execution follows the configured policy
    /// and remains explicitly non-isolated at the operating-system level.
    #[default]
    PersonalWorkstation,
    /// Remotely operated daemon. Host execution is constrained to allowlist
    /// semantics even if a legacy policy still says `full`.
    RemoteOperator,
    /// Untrusted workloads may not spawn agent-controlled host processes.
    /// Docker and WASM remain separate, explicit execution rails.
    UntrustedExecution,
}

impl ExecutionProfile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PersonalWorkstation => "personal_workstation",
            Self::RemoteOperator => "remote_operator",
            Self::UntrustedExecution => "untrusted_execution",
        }
    }

    pub const fn restriction_rank(self) -> u8 {
        match self {
            Self::PersonalWorkstation => 0,
            Self::RemoteOperator => 1,
            Self::UntrustedExecution => 2,
        }
    }

    pub const fn stricter(self, other: Self) -> Self {
        if self.restriction_rank() >= other.restriction_rank() {
            self
        } else {
            other
        }
    }
}

/// Q.9 — High-level Captain security profile, chosen at `captain setup`
/// (or via `/security` later). Independent of `ExecSecurityMode` (which
/// controls which shell commands may run once reviewed).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CriticalMode {
    /// **Open** — a detected hyper-critical command may proceed after a
    /// content-bound, one-shot operator approval.
    Open,
    /// **Safe** (default) — hyper-critical commands are blocked outright.
    #[default]
    #[serde(alias = "default")]
    Safe,
    /// **Paranoid** — every shell-affecting tool requires approval.
    Paranoid,
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
    #[default]
    #[serde(alias = "restricted")]
    Allowlist,
    /// Allow all commands except those in blocked_commands.
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
    pub profile: ExecutionProfile,
    pub backend: &'static str,
    pub isolation_level: &'static str,
    pub os_isolation: bool,
    pub environment_scrub: bool,
    pub dangerous_command_guard: &'static str,
    pub configured_policy_mode: ExecSecurityMode,
    pub policy_mode: ExecSecurityMode,
    pub critical_mode: CriticalMode,
    pub host_execution_allowed: bool,
    pub isolation_routing: &'static str,
    pub explicit_isolation_backends: &'static [&'static str],
}

/// Shell/exec security policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExecPolicy {
    /// Deployment posture. Profiles constrain, but never silently broaden,
    /// the command policy configured below.
    #[serde(default)]
    pub profile: ExecutionProfile,
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
    /// Intersect two execution policies without granting authority held by
    /// only one side. This is used when a per-agent policy meets the daemon
    /// deployment boundary.
    pub fn intersect(&self, other: &Self) -> Self {
        let profile = self.profile.stricter(other.profile);
        let mode = stricter_exec_mode(self.mode, other.mode);
        let effective_mode = effective_mode_for(profile, mode);
        let self_effective_mode = self.effective_mode();
        let other_effective_mode = other.effective_mode();
        let safe_bins = policy_allow_values(
            effective_mode,
            self_effective_mode,
            &self.safe_bins,
            other_effective_mode,
            &other.safe_bins,
        );
        let allowed_commands = policy_allow_values(
            effective_mode,
            self_effective_mode,
            &self.allowed_commands,
            other_effective_mode,
            &other.allowed_commands,
        );
        let mut blocked_commands = self.blocked_commands.clone();
        blocked_commands.extend(other.blocked_commands.iter().cloned());
        blocked_commands.sort();
        blocked_commands.dedup();
        Self {
            profile,
            mode,
            safe_bins,
            allowed_commands,
            blocked_commands,
            timeout_secs: strict_positive_limit(self.timeout_secs, other.timeout_secs),
            max_output_bytes: self.max_output_bytes.min(other.max_output_bytes),
            no_output_timeout_secs: strict_positive_limit(
                self.no_output_timeout_secs,
                other.no_output_timeout_secs,
            ),
            critical_mode: stricter_critical_mode(self.critical_mode, other.critical_mode),
        }
    }

    /// Effective host policy after applying the profile's non-bypassable floor.
    pub const fn effective_mode(&self) -> ExecSecurityMode {
        effective_mode_for(self.profile, self.mode)
    }

    pub const fn host_execution_posture(&self) -> HostExecutionPosture {
        let effective_mode = self.effective_mode();
        HostExecutionPosture {
            profile: self.profile,
            backend: HOST_EXECUTION_BACKEND,
            isolation_level: HOST_EXECUTION_ISOLATION_LEVEL,
            os_isolation: HOST_EXECUTION_OS_ISOLATED,
            environment_scrub: true,
            dangerous_command_guard: DANGEROUS_COMMAND_GUARD_LEVEL,
            configured_policy_mode: self.mode,
            policy_mode: effective_mode,
            critical_mode: self.critical_mode,
            host_execution_allowed: !matches!(effective_mode, ExecSecurityMode::Deny),
            isolation_routing: ISOLATION_ROUTING_MODE,
            explicit_isolation_backends: EXPLICIT_ISOLATION_BACKENDS,
        }
    }
}

const fn effective_mode_for(profile: ExecutionProfile, mode: ExecSecurityMode) -> ExecSecurityMode {
    match profile {
        ExecutionProfile::PersonalWorkstation => mode,
        ExecutionProfile::RemoteOperator => match mode {
            ExecSecurityMode::Deny => ExecSecurityMode::Deny,
            ExecSecurityMode::Allowlist | ExecSecurityMode::Full => ExecSecurityMode::Allowlist,
        },
        ExecutionProfile::UntrustedExecution => ExecSecurityMode::Deny,
    }
}

fn stricter_exec_mode(left: ExecSecurityMode, right: ExecSecurityMode) -> ExecSecurityMode {
    if left == ExecSecurityMode::Deny || right == ExecSecurityMode::Deny {
        ExecSecurityMode::Deny
    } else if left == ExecSecurityMode::Allowlist || right == ExecSecurityMode::Allowlist {
        ExecSecurityMode::Allowlist
    } else {
        ExecSecurityMode::Full
    }
}

fn policy_allow_values(
    result_mode: ExecSecurityMode,
    left_mode: ExecSecurityMode,
    left: &[String],
    right_mode: ExecSecurityMode,
    right: &[String],
) -> Vec<String> {
    if result_mode != ExecSecurityMode::Allowlist {
        return Vec::new();
    }
    match (
        left_mode == ExecSecurityMode::Allowlist,
        right_mode == ExecSecurityMode::Allowlist,
    ) {
        (true, true) => left
            .iter()
            .filter(|value| right.contains(value))
            .cloned()
            .collect(),
        (true, false) => left.to_vec(),
        (false, true) => right.to_vec(),
        (false, false) => Vec::new(),
    }
}

fn strict_positive_limit(left: u64, right: u64) -> u64 {
    match (left, right) {
        (0, value) | (value, 0) => value,
        _ => left.min(right),
    }
}

fn stricter_critical_mode(left: CriticalMode, right: CriticalMode) -> CriticalMode {
    use CriticalMode::{Open, Paranoid, Safe};
    match (left, right) {
        (Paranoid, _) | (_, Paranoid) => Paranoid,
        (Safe, _) | (_, Safe) => Safe,
        _ => Open,
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
            profile: ExecutionProfile::default(),
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
    fn exec_policy_default_is_allowlisted_and_explicitly_non_isolated() {
        let policy = ExecPolicy::default();
        assert_eq!(policy.profile, ExecutionProfile::PersonalWorkstation);
        assert_eq!(policy.mode, ExecSecurityMode::Allowlist);
        assert_eq!(policy.effective_mode(), ExecSecurityMode::Allowlist);
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
        assert_eq!(posture.profile, ExecutionProfile::PersonalWorkstation);
        assert_eq!(posture.backend, "host_process");
        assert_eq!(posture.isolation_level, "environment_scrub");
        assert!(!posture.os_isolation);
        assert!(posture.environment_scrub);
        assert_eq!(
            posture.dangerous_command_guard,
            "normalized_lexical_heuristic"
        );
        assert_eq!(posture.configured_policy_mode, ExecSecurityMode::Allowlist);
        assert_eq!(posture.policy_mode, ExecSecurityMode::Allowlist);
        assert_eq!(posture.critical_mode, CriticalMode::Safe);
        assert!(posture.host_execution_allowed);
        assert_eq!(posture.isolation_routing, "explicit_only");
        assert_eq!(
            posture.explicit_isolation_backends,
            ["docker_exec", "wasm_agent"]
        );
    }

    #[test]
    fn execution_profiles_only_restrict_configured_host_authority() {
        let full = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        assert_eq!(full.effective_mode(), ExecSecurityMode::Full);

        let remote = ExecPolicy {
            profile: ExecutionProfile::RemoteOperator,
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        assert_eq!(remote.effective_mode(), ExecSecurityMode::Allowlist);

        let untrusted = ExecPolicy {
            profile: ExecutionProfile::UntrustedExecution,
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        assert_eq!(untrusted.effective_mode(), ExecSecurityMode::Deny);
        assert!(!untrusted.host_execution_posture().host_execution_allowed);
    }

    #[test]
    fn execution_profile_serde_uses_human_readable_names() {
        for (raw, expected) in [
            (
                "personal_workstation",
                ExecutionProfile::PersonalWorkstation,
            ),
            ("remote_operator", ExecutionProfile::RemoteOperator),
            ("untrusted_execution", ExecutionProfile::UntrustedExecution),
        ] {
            let policy: ExecPolicy =
                toml::from_str(&format!("profile = \"{raw}\"\nmode = \"full\"\n"))
                    .expect("execution profile should deserialize");
            assert_eq!(policy.profile, expected);
        }
    }

    #[test]
    fn policy_intersection_never_broadens_daemon_authority() {
        let mut agent = ExecPolicy {
            mode: ExecSecurityMode::Full,
            timeout_secs: 60,
            max_output_bytes: 2000,
            no_output_timeout_secs: 0,
            critical_mode: CriticalMode::Open,
            ..ExecPolicy::default()
        };
        agent.blocked_commands = vec!["agent-block".to_string()];
        let daemon = ExecPolicy {
            profile: ExecutionProfile::RemoteOperator,
            mode: ExecSecurityMode::Allowlist,
            safe_bins: vec!["echo".to_string()],
            allowed_commands: vec!["cargo test".to_string()],
            blocked_commands: vec!["daemon-block".to_string()],
            timeout_secs: 20,
            max_output_bytes: 1000,
            no_output_timeout_secs: 10,
            critical_mode: CriticalMode::Safe,
        };

        let effective = agent.intersect(&daemon);

        assert_eq!(effective.profile, ExecutionProfile::RemoteOperator);
        assert_eq!(effective.mode, ExecSecurityMode::Allowlist);
        assert_eq!(effective.effective_mode(), ExecSecurityMode::Allowlist);
        assert_eq!(effective.safe_bins, ["echo"]);
        assert_eq!(effective.allowed_commands, ["cargo test"]);
        assert_eq!(effective.blocked_commands, ["agent-block", "daemon-block"]);
        assert_eq!(effective.timeout_secs, 20);
        assert_eq!(effective.max_output_bytes, 1000);
        assert_eq!(effective.no_output_timeout_secs, 10);
        assert_eq!(effective.critical_mode, CriticalMode::Safe);
    }

    #[test]
    fn policy_intersection_preserves_global_deny() {
        let agent = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        let daemon = ExecPolicy {
            mode: ExecSecurityMode::Deny,
            ..ExecPolicy::default()
        };

        assert_eq!(
            agent.intersect(&daemon).effective_mode(),
            ExecSecurityMode::Deny
        );
    }

    #[test]
    fn remote_profile_retains_only_shared_latent_allowlist_entries() {
        let mut agent = ExecPolicy {
            profile: ExecutionProfile::RemoteOperator,
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        agent.safe_bins = vec!["echo".to_string(), "cat".to_string()];
        let mut daemon = ExecPolicy {
            profile: ExecutionProfile::RemoteOperator,
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        daemon.safe_bins = vec!["echo".to_string(), "date".to_string()];

        let effective = agent.intersect(&daemon);

        assert_eq!(effective.mode, ExecSecurityMode::Full);
        assert_eq!(effective.effective_mode(), ExecSecurityMode::Allowlist);
        assert_eq!(effective.safe_bins, ["echo"]);
    }
}

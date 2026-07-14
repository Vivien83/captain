//! Docker sandbox configuration for runtime tool execution.

use serde::{Deserialize, Serialize};

/// Docker container sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DockerSandboxConfig {
    /// Enable Docker sandbox. Default: false.
    pub enabled: bool,
    /// Docker image for exec sandbox. Default: "python:3.12-slim".
    pub image: String,
    /// Container name prefix. Default: "captain-sandbox".
    pub container_prefix: String,
    /// Working directory inside container. Default: "/workspace".
    pub workdir: String,
    /// Network mode: "none", "bridge", or custom. Default: "none".
    pub network: String,
    /// Memory limit (e.g., "256m", "1g"). Default: "512m".
    pub memory_limit: String,
    /// CPU limit (e.g., 0.5, 1.0, 2.0). Default: 1.0.
    pub cpu_limit: f64,
    /// Max execution time in seconds. Default: 60.
    pub timeout_secs: u64,
    /// Read-only root filesystem. Default: true.
    pub read_only_root: bool,
    /// Additional capabilities to add. Default: empty (drop all).
    pub cap_add: Vec<String>,
    /// tmpfs mounts. Default: ["/tmp:size=64m"].
    pub tmpfs: Vec<String>,
    /// PID limit. Default: 100.
    pub pids_limit: u32,
    /// Docker sandbox mode: off, non_main, all. Default: off.
    #[serde(default)]
    pub mode: DockerSandboxMode,
    /// Container lifecycle scope. Default: session.
    #[serde(default)]
    pub scope: DockerScope,
    /// Cooldown before reusing a released container (seconds). Default: 300.
    #[serde(default = "default_reuse_cool_secs")]
    pub reuse_cool_secs: u64,
    /// Idle timeout - destroy containers after N seconds of inactivity. Default: 86400 (24h).
    #[serde(default = "default_docker_idle_timeout")]
    pub idle_timeout_secs: u64,
    /// Maximum age before forced destruction (seconds). Default: 604800 (7 days).
    #[serde(default = "default_docker_max_age")]
    pub max_age_secs: u64,
    /// Paths blocked from bind mounting.
    #[serde(default)]
    pub blocked_mounts: Vec<String>,
}

fn default_reuse_cool_secs() -> u64 {
    300
}

fn default_docker_idle_timeout() -> u64 {
    86400
}

fn default_docker_max_age() -> u64 {
    604800
}

impl Default for DockerSandboxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            image: "python:3.12-slim".to_string(),
            container_prefix: "captain-sandbox".to_string(),
            workdir: "/workspace".to_string(),
            network: "none".to_string(),
            memory_limit: "512m".to_string(),
            cpu_limit: 1.0,
            timeout_secs: 60,
            read_only_root: true,
            cap_add: Vec::new(),
            tmpfs: vec!["/tmp:size=64m".to_string()],
            pids_limit: 100,
            mode: DockerSandboxMode::Off,
            scope: DockerScope::Session,
            reuse_cool_secs: default_reuse_cool_secs(),
            idle_timeout_secs: default_docker_idle_timeout(),
            max_age_secs: default_docker_max_age(),
            blocked_mounts: Vec::new(),
        }
    }
}

/// Docker sandbox activation mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerSandboxMode {
    /// Docker sandbox disabled.
    #[default]
    Off,
    /// Only use Docker for non-main agents.
    NonMain,
    /// Use Docker for all agents.
    All,
}

/// Docker container lifecycle scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DockerScope {
    /// Container per session (destroyed when session ends).
    #[default]
    Session,
    /// Container per agent (reused across sessions).
    Agent,
    /// Shared container pool.
    Shared,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_sandbox_defaults_keep_runtime_limits() {
        let config = DockerSandboxConfig::default();

        assert!(!config.enabled);
        assert_eq!(config.image, "python:3.12-slim");
        assert_eq!(config.container_prefix, "captain-sandbox");
        assert_eq!(config.workdir, "/workspace");
        assert_eq!(config.network, "none");
        assert_eq!(config.memory_limit, "512m");
        assert_eq!(config.cpu_limit, 1.0);
        assert_eq!(config.timeout_secs, 60);
        assert!(config.read_only_root);
        assert_eq!(config.tmpfs, vec!["/tmp:size=64m"]);
        assert_eq!(config.pids_limit, 100);
        assert_eq!(config.mode, DockerSandboxMode::Off);
        assert_eq!(config.scope, DockerScope::Session);
        assert_eq!(config.reuse_cool_secs, 300);
        assert_eq!(config.idle_timeout_secs, 86400);
        assert_eq!(config.max_age_secs, 604800);
        assert!(config.blocked_mounts.is_empty());
    }

    #[test]
    fn docker_sandbox_deserializes_missing_maturity_fields_with_defaults() {
        let config: DockerSandboxConfig = toml::from_str(
            r#"
enabled = true
image = "python:3.12-slim"
container_prefix = "captain-sandbox"
workdir = "/workspace"
network = "none"
memory_limit = "512m"
cpu_limit = 1.0
timeout_secs = 60
read_only_root = true
cap_add = []
tmpfs = ["/tmp:size=64m"]
pids_limit = 100
"#,
        )
        .unwrap();

        assert_eq!(config.mode, DockerSandboxMode::Off);
        assert_eq!(config.scope, DockerScope::Session);
        assert_eq!(config.reuse_cool_secs, 300);
        assert_eq!(config.idle_timeout_secs, 86400);
        assert_eq!(config.max_age_secs, 604800);
        assert!(config.blocked_mounts.is_empty());
    }

    #[test]
    fn docker_sandbox_serde_accepts_snake_case_modes() {
        let config: DockerSandboxConfig = toml::from_str(
            r#"
mode = "non_main"
scope = "shared"
"#,
        )
        .unwrap();

        assert_eq!(config.mode, DockerSandboxMode::NonMain);
        assert_eq!(config.scope, DockerScope::Shared);
    }
}

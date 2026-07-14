use super::*;

#[test]
fn test_sanitize_container_name_valid() {
    let result = sanitize_container_name("captain-sandbox-abc123").unwrap();
    assert_eq!(result, "captain-sandbox-abc123");
}

#[test]
fn test_sanitize_container_name_special_chars() {
    let result = sanitize_container_name("test;rm -rf /").unwrap();
    assert!(!result.contains(';'));
    assert!(!result.contains(' '));
}

#[test]
fn test_sanitize_container_name_empty() {
    assert!(sanitize_container_name("").is_err());
}

#[test]
fn test_sanitize_container_name_too_long() {
    let long = "a".repeat(100);
    assert!(sanitize_container_name(&long).is_err());
}

#[test]
fn test_validate_image_name_valid() {
    assert!(validate_image_name("python:3.12-slim").is_ok());
    assert!(validate_image_name("ubuntu:22.04").is_ok());
    assert!(validate_image_name("registry.example.com/my-image:latest").is_ok());
}

#[test]
fn test_validate_image_name_empty() {
    assert!(validate_image_name("").is_err());
}

#[test]
fn test_validate_image_name_invalid() {
    assert!(validate_image_name("image;rm -rf /").is_err());
    assert!(validate_image_name("image`whoami`").is_err());
    assert!(validate_image_name("image$(id)").is_err());
}

#[test]
fn test_validate_command_valid() {
    assert!(validate_command("python script.py").is_ok());
    assert!(validate_command("ls -la /workspace").is_ok());
}

#[test]
fn test_validate_command_pipe_blocked() {
    assert!(validate_command("echo hello | grep h").is_err());
}

#[test]
fn test_validate_command_empty() {
    assert!(validate_command("").is_err());
}

#[test]
fn test_validate_command_backticks() {
    assert!(validate_command("echo `whoami`").is_err());
}

#[test]
fn test_validate_command_dollar_paren() {
    assert!(validate_command("echo $(id)").is_err());
}

#[test]
fn test_validate_command_dollar_brace() {
    assert!(validate_command("echo ${HOME}").is_err());
}

#[tokio::test]
async fn test_docker_available() {
    let _ = is_docker_available().await;
}

#[test]
fn test_config_defaults() {
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
    assert!(config.cap_add.is_empty());
    assert_eq!(config.tmpfs, vec!["/tmp:size=64m"]);
    assert_eq!(config.pids_limit, 100);
}

#[test]
fn test_exec_result_fields() {
    let result = ExecResult {
        stdout: "hello".to_string(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.stdout, "hello");
}

#[test]
fn test_container_pool_empty() {
    let pool = ContainerPool::new();
    assert!(pool.is_empty());
    assert_eq!(pool.len(), 0);
}

#[test]
fn test_container_pool_release_acquire() {
    let pool = ContainerPool::new();
    let container = SandboxContainer {
        container_id: "test123".to_string(),
        agent_id: "agent1".to_string(),
        created_at: chrono::Utc::now(),
    };
    pool.release(container, 12345);
    assert_eq!(pool.len(), 1);

    let acquired = pool.acquire(12345, 0);
    assert!(acquired.is_some());
    assert_eq!(acquired.unwrap().container_id, "test123");
    assert!(pool.is_empty());
}

#[test]
fn test_container_pool_hash_mismatch() {
    let pool = ContainerPool::new();
    let container = SandboxContainer {
        container_id: "test123".to_string(),
        agent_id: "agent1".to_string(),
        created_at: chrono::Utc::now(),
    };
    pool.release(container, 12345);

    let acquired = pool.acquire(99999, 0);
    assert!(acquired.is_none());
}

#[test]
fn test_validate_bind_mount_valid() {
    assert!(validate_bind_mount("/home/user/workspace", &[]).is_ok());
    assert!(validate_bind_mount("/tmp/sandbox", &[]).is_ok());
}

#[test]
fn test_validate_bind_mount_non_absolute() {
    assert!(validate_bind_mount("relative/path", &[]).is_err());
}

#[test]
fn test_validate_bind_mount_blocked_paths() {
    assert!(validate_bind_mount("/etc/passwd", &[]).is_err());
    assert!(validate_bind_mount("/proc/self", &[]).is_err());
    assert!(validate_bind_mount("/sys/kernel", &[]).is_err());
    assert!(validate_bind_mount("/var/run/docker.sock", &[]).is_err());
}

#[test]
fn test_validate_bind_mount_traversal() {
    assert!(validate_bind_mount("/home/user/../etc/passwd", &[]).is_err());
}

#[test]
fn test_validate_bind_mount_custom_blocked() {
    let blocked = vec!["/data/secrets".to_string()];
    assert!(validate_bind_mount("/data/secrets/vault", &blocked).is_err());
    assert!(validate_bind_mount("/data/public", &blocked).is_ok());
}

#[test]
fn test_config_hash_deterministic() {
    let c1 = DockerSandboxConfig::default();
    let c2 = DockerSandboxConfig::default();
    assert_eq!(config_hash(&c1), config_hash(&c2));
}

#[test]
fn test_config_hash_different_images() {
    let c1 = DockerSandboxConfig::default();
    let c2 = DockerSandboxConfig {
        image: "node:20-slim".to_string(),
        ..Default::default()
    };
    assert_ne!(config_hash(&c1), config_hash(&c2));
}

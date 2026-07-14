use super::*;

fn test_state(capabilities: Vec<Capability>) -> GuestState {
    GuestState {
        capabilities,
        kernel: None,
        agent_id: "test-agent".to_string(),
        tokio_handle: tokio::runtime::Handle::current(),
    }
}

#[tokio::test]
async fn test_time_now_always_allowed() {
    let result = host_time_now();
    assert!(result.get("ok").is_some());
    let ts = result["ok"].as_u64().unwrap();
    assert!(ts > 1_700_000_000);
}

#[tokio::test]
async fn test_fs_read_denied_no_capability() {
    let state = test_state(vec![]);
    let result = host_fs_read(&state, &json!({"path": "/etc/passwd"}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("denied"));
}

#[tokio::test]
async fn test_fs_write_denied_no_capability() {
    let state = test_state(vec![]);
    let result = host_fs_write(&state, &json!({"path": "/tmp/test", "content": "hello"}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("denied"));
}

#[tokio::test]
async fn test_fs_read_granted_wildcard() {
    let state = test_state(vec![Capability::FileRead("*".to_string())]);
    let result = host_fs_read(&state, &json!({"path": "Cargo.toml"}));
    if let Some(err) = result.get("error") {
        let msg = err.as_str().unwrap_or("");
        assert!(
            !msg.contains("denied"),
            "Should not be capability-denied: {msg}"
        );
    }
}

#[tokio::test]
async fn test_shell_exec_denied() {
    let state = test_state(vec![]);
    let result = host_shell_exec(&state, &json!({"command": "ls"}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("denied"));
}

#[tokio::test]
async fn test_env_read_denied() {
    let state = test_state(vec![]);
    let result = host_env_read(&state, &json!({"name": "HOME"}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("denied"));
}

#[tokio::test]
async fn test_env_read_granted() {
    let state = test_state(vec![Capability::EnvRead("PATH".to_string())]);
    let result = host_env_read(&state, &json!({"name": "PATH"}));
    assert!(result.get("ok").is_some(), "Expected ok: {:?}", result);
}

#[tokio::test]
async fn test_kv_get_no_kernel() {
    let state = test_state(vec![Capability::MemoryRead("*".to_string())]);
    let result = host_kv_get(&state, &json!({"key": "test"}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("kernel"));
}

#[tokio::test]
async fn test_agent_send_denied() {
    let state = test_state(vec![]);
    let result = host_agent_send(&state, &json!({"target": "some-agent", "message": "hello"}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("denied"));
}

#[tokio::test]
async fn test_agent_spawn_denied() {
    let state = test_state(vec![]);
    let result = host_agent_spawn(&state, &json!({"manifest": "name = 'test'"}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("denied"));
}

#[tokio::test]
async fn test_dispatch_unknown_method() {
    let state = test_state(vec![]);
    let result = dispatch(&state, "bogus_method", &json!({}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("Unknown"));
}

#[tokio::test]
async fn test_missing_params() {
    let state = test_state(vec![Capability::FileRead("*".to_string())]);
    let result = host_fs_read(&state, &json!({}));
    let err = result["error"].as_str().unwrap();
    assert!(err.contains("Missing"));
}

#[test]
fn test_safe_resolve_path_traversal() {
    assert!(safe_resolve_path("../etc/passwd").is_err());
    assert!(safe_resolve_path("/tmp/../../etc/passwd").is_err());
    assert!(safe_resolve_path("foo/../bar").is_err());
}

#[test]
fn test_safe_resolve_parent_traversal() {
    assert!(safe_resolve_parent("../malicious.txt").is_err());
    assert!(safe_resolve_parent("/tmp/../../etc/shadow").is_err());
}

#[test]
fn test_ssrf_private_ips_blocked() {
    assert!(is_ssrf_target("http://127.0.0.1:8080/secret").is_err());
    assert!(is_ssrf_target("http://localhost:3000/api").is_err());
    assert!(is_ssrf_target("http://169.254.169.254/metadata").is_err());
    assert!(is_ssrf_target("http://metadata.google.internal/v1/instance").is_err());
}

#[test]
fn test_ssrf_public_ips_allowed() {
    assert!(is_ssrf_target("https://api.openai.com/v1/chat").is_ok());
    assert!(is_ssrf_target("https://google.com").is_ok());
}

#[test]
fn test_ssrf_scheme_validation() {
    assert!(is_ssrf_target("file:///etc/passwd").is_err());
    assert!(is_ssrf_target("gopher://evil.com").is_err());
    assert!(is_ssrf_target("ftp://example.com").is_err());
}

#[test]
fn test_is_private_ip() {
    use std::net::IpAddr;
    assert!(is_private_ip(&"10.0.0.1".parse::<IpAddr>().unwrap()));
    assert!(is_private_ip(&"172.16.0.1".parse::<IpAddr>().unwrap()));
    assert!(is_private_ip(&"192.168.1.1".parse::<IpAddr>().unwrap()));
    assert!(is_private_ip(&"169.254.169.254".parse::<IpAddr>().unwrap()));
    assert!(!is_private_ip(&"8.8.8.8".parse::<IpAddr>().unwrap()));
    assert!(!is_private_ip(&"1.1.1.1".parse::<IpAddr>().unwrap()));
}

#[test]
fn test_extract_host_from_url() {
    assert_eq!(
        extract_host_from_url("https://api.openai.com/v1/chat"),
        "api.openai.com:443"
    );
    assert_eq!(
        extract_host_from_url("http://localhost:8080/api"),
        "localhost:8080"
    );
    assert_eq!(
        extract_host_from_url("http://example.com"),
        "example.com:80"
    );
}

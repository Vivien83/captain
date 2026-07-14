use super::*;

/// Minimal echo module: returns input JSON unchanged.
const ECHO_WAT: &str = r#"
    (module
        (memory (export "memory") 1)
        (global $bump (mut i32) (i32.const 1024))

        (func (export "alloc") (param $size i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $size)))
            (local.get $ptr)
        )

        (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
            ;; Echo: return the input as-is
            (i64.or
                (i64.shl
                    (i64.extend_i32_u (local.get $ptr))
                    (i64.const 32)
                )
                (i64.extend_i32_u (local.get $len))
            )
        )
    )
"#;

/// Module with infinite loop to test fuel exhaustion.
const INFINITE_LOOP_WAT: &str = r#"
    (module
        (memory (export "memory") 1)
        (global $bump (mut i32) (i32.const 1024))

        (func (export "alloc") (param $size i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $size)))
            (local.get $ptr)
        )

        (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
            (loop $inf
                (br $inf)
            )
            (i64.const 0)
        )
    )
"#;

/// Proxy module: forwards input to host_call and returns the response.
const HOST_CALL_PROXY_WAT: &str = r#"
    (module
        (import "captain" "host_call" (func $host_call (param i32 i32) (result i64)))
        (memory (export "memory") 2)
        (global $bump (mut i32) (i32.const 1024))

        (func (export "alloc") (param $size i32) (result i32)
            (local $ptr i32)
            (local.set $ptr (global.get $bump))
            (global.set $bump (i32.add (global.get $bump) (local.get $size)))
            (local.get $ptr)
        )

        (func (export "execute") (param $input_ptr i32) (param $input_len i32) (result i64)
            (call $host_call (local.get $input_ptr) (local.get $input_len))
        )
    )
"#;

#[test]
fn test_sandbox_config_default() {
    let config = SandboxConfig::default();
    assert_eq!(config.fuel_limit, 1_000_000);
    assert_eq!(config.max_memory_bytes, 16 * 1024 * 1024);
    assert!(config.capabilities.is_empty());
}

#[test]
fn test_sandbox_engine_creation() {
    let sandbox = WasmSandbox::new().unwrap();
    drop(sandbox);
}

#[test]
fn test_guest_result_pack_round_trips_pointer_and_length() {
    let packed = pack_guest_result(1234, 5678);
    let unpacked = unpack_guest_result(packed);

    assert_eq!(unpacked.ptr, 1234);
    assert_eq!(unpacked.len, 5678);
}

#[test]
fn test_host_call_request_defaults_missing_fields() {
    let request = parse_host_call_request(br#"{"params":{"x":1}}"#).unwrap();
    assert_eq!(request.method, "");
    assert_eq!(request.params, serde_json::json!({"x": 1}));

    let empty = parse_host_call_request(b"{}").unwrap();
    assert_eq!(empty.method, "");
    assert_eq!(empty.params, serde_json::Value::Null);
}

#[tokio::test]
async fn test_echo_module() {
    let sandbox = WasmSandbox::new().unwrap();
    let input = serde_json::json!({"hello": "world", "num": 42});
    let config = SandboxConfig::default();

    let result = sandbox
        .execute(
            ECHO_WAT.as_bytes(),
            input.clone(),
            config,
            None,
            "test-agent",
        )
        .await
        .unwrap();

    assert_eq!(result.output, input);
    assert!(result.fuel_consumed > 0);
}

#[tokio::test]
async fn test_fuel_exhaustion() {
    let sandbox = WasmSandbox::new().unwrap();
    let input = serde_json::json!({});
    let config = SandboxConfig {
        fuel_limit: 10_000,
        ..Default::default()
    };

    let err = sandbox
        .execute(
            INFINITE_LOOP_WAT.as_bytes(),
            input,
            config,
            None,
            "test-agent",
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, SandboxError::FuelExhausted),
        "Expected FuelExhausted, got: {err}"
    );
}

#[tokio::test]
async fn test_host_call_time_now() {
    let sandbox = WasmSandbox::new().unwrap();
    let input = serde_json::json!({"method": "time_now", "params": {}});
    let config = SandboxConfig::default();

    let result = sandbox
        .execute(
            HOST_CALL_PROXY_WAT.as_bytes(),
            input,
            config,
            None,
            "test-agent",
        )
        .await
        .unwrap();

    assert!(
        result.output.get("ok").is_some(),
        "Expected ok field: {:?}",
        result.output
    );
    let ts = result.output["ok"].as_u64().unwrap();
    assert!(ts > 1_700_000_000, "Timestamp looks too small: {ts}");
}

#[tokio::test]
async fn test_host_call_capability_denied() {
    let sandbox = WasmSandbox::new().unwrap();
    let input = serde_json::json!({
        "method": "fs_read",
        "params": {"path": "/etc/passwd"}
    });
    let config = SandboxConfig {
        capabilities: vec![],
        ..Default::default()
    };

    let result = sandbox
        .execute(
            HOST_CALL_PROXY_WAT.as_bytes(),
            input,
            config,
            None,
            "test-agent",
        )
        .await
        .unwrap();

    let err_msg = result.output["error"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("denied"),
        "Expected capability denied, got: {err_msg}"
    );
}

#[tokio::test]
async fn test_host_call_unknown_method() {
    let sandbox = WasmSandbox::new().unwrap();
    let input = serde_json::json!({"method": "nonexistent_method", "params": {}});
    let config = SandboxConfig::default();

    let result = sandbox
        .execute(
            HOST_CALL_PROXY_WAT.as_bytes(),
            input,
            config,
            None,
            "test-agent",
        )
        .await
        .unwrap();

    let err_msg = result.output["error"].as_str().unwrap_or("");
    assert!(
        err_msg.contains("Unknown"),
        "Expected unknown method error, got: {err_msg}"
    );
}

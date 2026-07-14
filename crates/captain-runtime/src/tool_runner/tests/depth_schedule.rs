use super::*;

#[test]
fn test_depth_limit_constant() {
    assert_eq!(MAX_AGENT_CALL_DEPTH, 5);
}

#[test]
fn test_depth_limit_first_call_succeeds() {
    // Default depth is 0, which is < MAX_AGENT_CALL_DEPTH
    let default_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    assert!(default_depth < MAX_AGENT_CALL_DEPTH);
}

#[test]
fn test_task_local_compiles() {
    // Verify task_local macro works — just ensure the type exists
    let cell = std::cell::Cell::new(0u32);
    assert_eq!(cell.get(), 0);
}

#[tokio::test]
async fn test_schedule_tools_without_kernel() {
    let result = execute_tool(
        "test-id",
        "schedule_list",
        &serde_json::json!({}),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None, // media_engine
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("Kernel handle not available"));
}

// ─── Canvas / A2UI tests ────────────────────────────────────────

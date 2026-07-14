use super::*;

async fn execute_contract_tool(
    tool_name: &str,
    input: &serde_json::Value,
    allowed_tools: Option<&[String]>,
) -> ToolResult {
    execute_tool(
        "test-id",
        tool_name,
        input,
        None,
        allowed_tools,
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
    .await
}

#[derive(Default)]
struct DenyApprovalKernel {
    summaries: std::sync::Mutex<Vec<String>>,
}

impl DenyApprovalKernel {
    fn summaries(&self) -> Vec<String> {
        self.summaries.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::kernel_handle::KernelHandle for DenyApprovalKernel {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        Err("not implemented".to_string())
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
        Err("not implemented".to_string())
    }

    fn list_agents(&self) -> Vec<crate::kernel_handle::AgentInfo> {
        Vec::new()
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
        Err("not implemented".to_string())
    }

    fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    fn requires_approval(&self, tool_name: &str) -> bool {
        matches!(tool_name, "file_write" | "shell_exec")
    }

    async fn request_approval(
        &self,
        _agent_id: &str,
        _tool_name: &str,
        action_summary: &str,
    ) -> Result<bool, String> {
        self.summaries
            .lock()
            .unwrap()
            .push(action_summary.to_string());
        Ok(false)
    }

    fn find_agents(&self, _query: &str) -> Vec<crate::kernel_handle::AgentInfo> {
        Vec::new()
    }

    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, String> {
        Err("not implemented".to_string())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
        Ok(())
    }
}

async fn execute_with_kernel(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: &std::sync::Arc<DenyApprovalKernel>,
    workspace_root: Option<&std::path::Path>,
) -> ToolResult {
    let kernel_dyn: std::sync::Arc<dyn crate::kernel_handle::KernelHandle> = kernel.clone();
    execute_tool(
        "test-id",
        tool_name,
        input,
        Some(&kernel_dyn),
        None,
        Some("captain"),
        None,
        None,
        None,
        None,
        None,
        workspace_root,
        None,
        None,
        None,
        None,
        None,
    )
    .await
}

#[tokio::test]
async fn test_file_read_missing() {
    let bad_path = std::env::temp_dir()
        .join("captain_test_nonexistent_99999")
        .join("file.txt");
    let result = execute_contract_tool(
        "file_read",
        &serde_json::json!({"path": bad_path.to_str().unwrap()}),
        None,
    )
    .await;
    assert!(
        result.is_error,
        "Expected error but got: {}",
        result.content
    );
}

#[tokio::test]
async fn approval_preview_blocks_file_write_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("guarded.txt");
    std::fs::write(&path, "old\n").unwrap();
    let kernel = std::sync::Arc::new(DenyApprovalKernel::default());

    let result = execute_with_kernel(
        "file_write",
        &serde_json::json!({
            "path": "guarded.txt",
            "content": "new\n"
        }),
        &kernel,
        Some(dir.path()),
    )
    .await;

    assert!(result.is_error);
    assert!(result.content.contains("requires human approval"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");

    let summaries = kernel.summaries();
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert!(summary.contains("Approval preview"));
    assert!(summary.contains("no file written yet"));
    assert!(summary.contains("Diff before write"));
    assert!(summary.contains("-old"));
    assert!(summary.contains("+new"));
}

#[tokio::test]
async fn approval_preview_blocks_shell_exec_before_run() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("should-not-exist");
    let kernel = std::sync::Arc::new(DenyApprovalKernel::default());

    let result = execute_with_kernel(
        "shell_exec",
        &serde_json::json!({
            "command": format!("touch {}", marker.display())
        }),
        &kernel,
        Some(dir.path()),
    )
    .await;

    assert!(result.is_error);
    assert!(
        !marker.exists(),
        "shell command must not run before approval"
    );

    let summaries = kernel.summaries();
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert!(summary.contains("no command executed yet"));
    assert!(summary.contains("Command list before run"));
    assert!(summary.contains("touch"));
}

#[tokio::test]
async fn test_file_read_path_traversal_blocked() {
    let result = execute_contract_tool(
        "file_read",
        &serde_json::json!({"path": "../../etc/passwd"}),
        None,
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("traversal"));
}

#[tokio::test]
async fn test_file_write_path_traversal_blocked() {
    let result = execute_contract_tool(
        "file_write",
        &serde_json::json!({"path": "../../../tmp/evil.txt", "content": "pwned"}),
        None,
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("traversal"));
}

#[tokio::test]
async fn test_file_list_path_traversal_blocked() {
    let result = execute_contract_tool(
        "file_list",
        &serde_json::json!({"path": "/foo/../../etc"}),
        None,
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("traversal"));
}

#[tokio::test]
async fn test_web_search() {
    let result = execute_contract_tool(
        "web_search",
        &serde_json::json!({"query": "rust programming"}),
        None,
    )
    .await;
    // web_search now attempts a real fetch; may succeed or fail depending on network.
    assert!(!result.tool_use_id.is_empty());
}

#[tokio::test]
async fn test_unknown_tool() {
    let result = execute_contract_tool("nonexistent_tool", &serde_json::json!({}), None).await;
    assert!(result.is_error);
    assert!(result.content.contains("Unknown tool"));
}

#[tokio::test]
async fn test_agent_tools_without_kernel() {
    let result = execute_contract_tool("agent_list", &serde_json::json!({}), None).await;
    assert!(result.is_error);
    assert!(result.content.contains("Kernel handle not available"));
}

#[tokio::test]
async fn subagent_lineage_depth_denies_admin_tools_at_execution() {
    let result = with_agent_lineage_depth(
        1,
        execute_contract_tool(
            "cron_create",
            &serde_json::json!({"name":"x","schedule":"daily","prompt":"x"}),
            None,
        ),
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("sub-agent depth"),
        "expected sub-agent depth denial, got: {}",
        result.content
    );
}

/// B.4 — A canonical tool name absent from `allowed_tools` must be
/// denied. Companion test to `_aliased_denied` covering the no-alias
/// path: the LLM calls a real tool name that simply isn't in policy.
#[tokio::test]
async fn test_capability_enforcement_denied() {
    let allowed = vec!["file_read".to_string(), "file_list".to_string()];
    let result = execute_contract_tool(
        "shell_exec",
        &serde_json::json!({"command": "echo test"}),
        Some(&allowed),
    )
    .await;
    assert!(result.is_error, "denied tool must surface as is_error=true");
    assert!(
        result.content.contains("Permission denied"),
        "denial must mention the policy, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_capability_enforcement_allowed() {
    let allowed = vec!["file_read".to_string()];
    let bad_path = std::env::temp_dir()
        .join("captain_test_nonexistent_12345")
        .join("file.txt");
    let result = execute_contract_tool(
        "file_read",
        &serde_json::json!({"path": bad_path.to_str().unwrap()}),
        Some(&allowed),
    )
    .await;
    assert!(
        result.is_error,
        "Expected error but got: {}",
        result.content
    );
    assert!(
        result.content.contains("Failed to read")
            || result.content.contains("not found")
            || result.content.contains("No such file"),
        "Unexpected error: {}",
        result.content
    );
}

#[tokio::test]
async fn test_capability_enforcement_aliased_tool_name() {
    let allowed = vec![
        "file_read".to_string(),
        "file_write".to_string(),
        "file_list".to_string(),
        "shell_exec".to_string(),
    ];
    // A filesystem-root path (was "/nonexistent/file.txt") made this test
    // environment-dependent: the assertion below is a proxy for "the
    // capability layer let the request through" (that layer's own denial
    // also reads "Permission denied" — see the _denied test below), but
    // creating a directory directly under `/` is itself denied by the OS
    // on CI runners (though not on a macOS dev machine), producing the
    // exact same substring for an unrelated reason and defeating the
    // proxy. A temp-dir path can't hit that OS permission boundary on any
    // platform, so a match here can only mean the capability layer.
    let bad_path = std::env::temp_dir()
        .join("captain_test_nonexistent_67890")
        .join("file.txt");
    let result = execute_contract_tool(
        "fs-write",
        &serde_json::json!({"path": bad_path.to_str().unwrap(), "content": "hello"}),
        Some(&allowed),
    )
    .await;
    assert!(
        !result.content.contains("Permission denied"),
        "fs-write should normalize to file_write and pass capability check, got: {}",
        result.content
    );
}

/// B.4 — When `allowed_tools` is supplied AND the (post-normalization)
/// tool name is not in the list, execute_tool must short-circuit with a
/// "Permission denied" ToolResult. Without this, an LLM-supplied tool
/// outside the agent's grants ran anyway.
#[tokio::test]
async fn test_capability_enforcement_aliased_denied() {
    let allowed = vec!["file_read".to_string()];
    let result = execute_contract_tool(
        "fs-write",
        &serde_json::json!({"path": "/tmp/test.txt", "content": "hello"}),
        Some(&allowed),
    )
    .await;
    assert!(
        result.is_error,
        "denied tool must surface as is_error=true, got: {:?}",
        result
    );
    assert!(
        result.content.contains("Permission denied")
            || result.content.contains("not in")
            || result.content.contains("allowed_tools"),
        "denial message must mention the policy, got: {}",
        result.content
    );
}

/// B.4 — Passing `None` for allowed_tools is the explicit "no policy
/// configured" path used by tests, embedded uses, and migration of
/// agents that have never opted into capability lists. It must keep
/// behaving as a full bypass — the absence of a policy is not a denial.
#[tokio::test]
async fn test_capability_enforcement_passthrough_when_none() {
    let result = execute_contract_tool(
        "file_read",
        &serde_json::json!({"path": "/nonexistent/file.txt"}),
        None,
    )
    .await;
    assert!(
        !result.content.contains("Permission denied"),
        "None must bypass the capability check, got: {}",
        result.content
    );
}

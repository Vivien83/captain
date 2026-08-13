use super::*;
use crate::execution_routing::{with_turn_execution_context, RemoteToolExecutionRequest};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RemoteRoutingKernel {
    requests: Mutex<Vec<RemoteToolExecutionRequest>>,
    approval_requests: Mutex<usize>,
}

#[async_trait::async_trait]
impl crate::kernel_handle::KernelHandle for RemoteRoutingKernel {
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

    async fn execute_remote_tool(
        &self,
        request: RemoteToolExecutionRequest,
    ) -> Result<ToolResult, String> {
        let tool_use_id = request.tool_use_id.clone();
        self.requests.lock().unwrap().push(request);
        Ok(ToolResult {
            tool_use_id,
            content: "remote-result".to_string(),
            is_error: false,
            transient_content: Vec::new(),
        })
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
        tool_name == "file_write"
    }

    async fn request_approval(
        &self,
        _agent_id: &str,
        _tool_name: &str,
        _action_summary: &str,
        _action_digest: &str,
    ) -> Result<captain_types::approval::ApprovalOutcome, String> {
        *self.approval_requests.lock().unwrap() += 1;
        Ok(captain_types::approval::ApprovalDecision::Denied.into())
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

    fn email_accounts(&self) -> Result<Vec<captain_types::email::GmailAccountSummary>, String> {
        Ok(Vec::new())
    }

    fn email_automation_rules(
        &self,
        _request: captain_types::email_automation::GmailAutomationRuleQuery,
    ) -> Result<Vec<captain_types::email_automation::GmailAutomationRuleView>, String> {
        Ok(Vec::new())
    }
}

async fn execute_with_route(
    kernel: &Arc<RemoteRoutingKernel>,
    context: crate::execution_routing::TurnExecutionContext,
    tool_name: &str,
    input: serde_json::Value,
    workspace_root: Option<&std::path::Path>,
) -> ToolResult {
    let kernel_dyn: Arc<dyn crate::kernel_handle::KernelHandle> = kernel.clone();
    with_turn_execution_context(
        context,
        execute_tool(
            "tool-call-1",
            tool_name,
            &input,
            Some(&kernel_dyn),
            None,
            Some("agent-1"),
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
        ),
    )
    .await
}

#[tokio::test]
async fn node_route_dispatches_supported_tool_without_local_mutation_or_double_approval() {
    let dir = tempfile::tempdir().unwrap();
    let destination = dir.path().join("must-not-exist.txt");
    let kernel = Arc::new(RemoteRoutingKernel::default());
    let result = execute_with_route(
        &kernel,
        crate::execution_routing::TurnExecutionContext::node("session-1", "node-1", "workspace-1"),
        "file_write",
        serde_json::json!({"path": "must-not-exist.txt", "content": "local-write"}),
        Some(dir.path()),
    )
    .await;

    assert!(!result.is_error);
    assert_eq!(result.content, "remote-result");
    assert!(!destination.exists());
    assert_eq!(*kernel.approval_requests.lock().unwrap(), 0);
    let requests = kernel.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].scope_id, "session-1");
    assert_eq!(requests[0].device_id, "node-1");
    assert_eq!(requests[0].workspace_id, "workspace-1");
    assert_eq!(requests[0].tool_name, "file_write");
    assert!(format!("{:?}", requests[0]).contains("[REDACTED]"));
    assert!(!format!("{:?}", requests[0]).contains("local-write"));
}

#[tokio::test]
async fn hub_route_keeps_supported_tool_on_the_local_dispatcher() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("local.txt");
    std::fs::write(&source, "local-result").unwrap();
    let kernel = Arc::new(RemoteRoutingKernel::default());
    let result = execute_with_route(
        &kernel,
        crate::execution_routing::TurnExecutionContext::hub("session-1"),
        "file_read",
        serde_json::json!({"path": source}),
        Some(dir.path()),
    )
    .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("local-result"));
    assert!(kernel.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn node_route_keeps_hub_only_tools_on_the_hub() {
    let kernel = Arc::new(RemoteRoutingKernel::default());
    let result = execute_with_route(
        &kernel,
        crate::execution_routing::TurnExecutionContext::node("session-1", "node-1", "workspace-1"),
        "system_time",
        serde_json::json!({}),
        None,
    )
    .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(kernel.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn paired_client_can_use_a_local_tool_only_on_the_selected_node() {
    let kernel = Arc::new(RemoteRoutingKernel::default());
    let remote = with_origin_channel(
        Some("paired-client:chat".to_string()),
        execute_with_route(
            &kernel,
            crate::execution_routing::TurnExecutionContext::node(
                "session-node",
                "node-1",
                "workspace-1",
            ),
            "file_read",
            serde_json::json!({"path": "README.md"}),
            None,
        ),
    )
    .await;
    assert!(!remote.is_error, "{}", remote.content);
    assert_eq!(kernel.requests.lock().unwrap().len(), 1);

    let denied = with_origin_channel(
        Some("paired-client:chat".to_string()),
        execute_with_route(
            &kernel,
            crate::execution_routing::TurnExecutionContext::hub("session-hub"),
            "file_read",
            serde_json::json!({"path": "README.md"}),
            None,
        ),
    )
    .await;
    assert!(denied.is_error);
    assert_eq!(kernel.requests.lock().unwrap().len(), 1);
}

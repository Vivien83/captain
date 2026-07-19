use super::*;
use captain_capspec::{CapabilityExecutor, CapabilityRegistry, CapabilityRunStatus};
use std::path::Path;

const READ_NOTE: &str = r#"
format = 1
name = "read-note"
description = "Read the project note through the central ToolRunner."
output = "{{steps.read.output}}"

[permissions]
tools = ["file_read"]
read_paths = ["note.txt"]

[[steps]]
id = "read"
tool = "file_read"
with = { path = "note.txt" }
"#;

struct CapSpecKernel {
    registry: Arc<CapabilityRegistry>,
    executor: Arc<CapabilityExecutor>,
    blocked_tools: Vec<String>,
}

#[async_trait::async_trait]
impl crate::kernel_handle::KernelHandle for CapSpecKernel {
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

    fn capspec_executor_for_workspace(
        &self,
        _workspace: Option<&Path>,
    ) -> Result<Option<Arc<CapabilityExecutor>>, String> {
        Ok(Some(Arc::clone(&self.executor)))
    }

    fn capspec_tool_definitions(
        &self,
        workspace: Option<&Path>,
    ) -> Result<Vec<ToolDefinition>, String> {
        self.registry
            .active_capabilities(workspace)
            .map(|capabilities| {
                capabilities
                    .into_iter()
                    .map(|capability| capability.tool_definition())
                    .collect()
            })
            .map_err(|error| error.to_string())
    }

    fn tool_is_blocked_for_agent(&self, _caller_agent_id: Option<&str>, tool_name: &str) -> bool {
        self.blocked_tools
            .iter()
            .any(|blocked| blocked == tool_name)
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    workspace: std::path::PathBuf,
    kernel: Arc<dyn crate::kernel_handle::KernelHandle>,
    executor: Arc<CapabilityExecutor>,
}

impl Fixture {
    fn new() -> Self {
        Self::with_blocked_tools(Vec::new())
    }

    fn with_blocked_tools(blocked_tools: Vec<String>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let capability_root = temp.path().join("capabilities");
        let database = temp.path().join("capabilities.db");
        let key = temp.path().join("capabilities.key");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("note.txt"), "native-capability\n").unwrap();
        let registry = Arc::new(CapabilityRegistry::open(&capability_root, &database).unwrap());
        std::fs::write(capability_root.join("read-note.captain"), READ_NOTE).unwrap();
        registry.reload_global().unwrap();
        let executor =
            Arc::new(CapabilityExecutor::open(Arc::clone(&registry), &database, &key).unwrap());
        let kernel: Arc<dyn crate::kernel_handle::KernelHandle> = Arc::new(CapSpecKernel {
            registry,
            executor: Arc::clone(&executor),
            blocked_tools,
        });
        Self {
            _temp: temp,
            workspace,
            kernel,
            executor,
        }
    }

    async fn execute(&self, allowed_tools: &[String]) -> ToolResult {
        execute_tool(
            "capspec-test",
            "cap_read_note",
            &serde_json::json!({}),
            Some(&self.kernel),
            Some(allowed_tools),
            Some("captain"),
            None,
            None,
            None,
            None,
            None,
            Some(&self.workspace),
            None,
            None,
            None,
            None,
            None,
        )
        .await
    }
}

#[tokio::test]
async fn capspec_dispatch_reenters_the_central_tool_runner() {
    let fixture = Fixture::new();
    let result = fixture
        .execute(&["cap_read_note".to_string(), "file_read".to_string()])
        .await;
    assert!(!result.is_error, "{}", result.content);
    let payload: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(payload["output"], "native-capability\n");
    assert!(payload["source_hash"].as_str().is_some());
    let run_id = payload["run_id"].as_str().unwrap();
    let run = fixture.executor.run(run_id).unwrap();
    assert_eq!(run.status, CapabilityRunStatus::Succeeded);
    assert_eq!(run.nodes[0].tool_name, "file_read");
    assert_eq!(run.nodes[0].attempts, 1);
}

#[tokio::test]
async fn capspec_prefix_never_falls_through_without_a_kernel() {
    let result = execute_tool(
        "capspec-no-kernel",
        "cap_missing",
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
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    assert!(result.is_error);
    assert!(result
        .content
        .contains("CapSpec tools require the Captain kernel"));
}

#[tokio::test]
async fn capspec_steps_cannot_exceed_the_callers_primitive_grants() {
    let fixture = Fixture::new();
    let result = fixture.execute(&["cap_read_note".to_string()]).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("allowed_tools"),
        "{}",
        result.content
    );
    assert!(
        fixture.executor.list_runs(1).unwrap().is_empty(),
        "an authority preflight denial must not create a durable run"
    );
}

#[tokio::test]
async fn capspec_steps_cannot_bypass_the_callers_hard_blocklist() {
    let fixture = Fixture::with_blocked_tools(vec!["file_read".to_string()]);
    let result = fixture
        .execute(&["cap_read_note".to_string(), "file_read".to_string()])
        .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("tool_blocklist"),
        "{}",
        result.content
    );
    assert!(
        fixture.executor.list_runs(1).unwrap().is_empty(),
        "a hard-blocklist preflight denial must not create a durable run"
    );
}

#[tokio::test]
async fn capability_search_surfaces_active_capfiles_with_their_schema() {
    let fixture = Fixture::new();
    let raw = tool_capability_search(
        &serde_json::json!({
            "query": "select:cap_read_note",
            "sources": ["capspec"]
        }),
        None,
        None,
        Some(&fixture.kernel),
        Some(&fixture.workspace),
    )
    .await
    .unwrap();
    let response: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(response["searched_sources"], serde_json::json!(["capfile"]));
    assert_eq!(response["results"][0]["source"], "capfile_tool");
    assert_eq!(response["results"][0]["name"], "cap_read_note");
    assert_eq!(response["results"][0]["status"], "active_native");
    assert!(response["results"][0]["input_schema"].is_object());
}

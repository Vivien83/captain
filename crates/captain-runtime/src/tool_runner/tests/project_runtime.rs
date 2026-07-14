use super::*;
use crate::kernel_handle::{AgentInfo, KernelHandle};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

async fn execute_project_tool(
    tool_name: &str,
    input: &serde_json::Value,
    kernel: Arc<dyn KernelHandle>,
) -> ToolResult {
    execute_tool(
        "project-test",
        tool_name,
        input,
        Some(&kernel),
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
    .await
}

#[derive(Default)]
struct ProjectToolStub {
    calls: AtomicUsize,
    last_project_id: Mutex<Option<String>>,
    last_parent_id: Mutex<Option<String>>,
}

#[async_trait::async_trait]
impl KernelHandle for ProjectToolStub {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        Err("stub".into())
    }

    async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
        Err("stub".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        Vec::new()
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        Vec::new()
    }

    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, String> {
        Err("stub".into())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
        Ok(())
    }

    fn project_create(
        &self,
        _name: &str,
        slug: &str,
        _goal: &str,
        _deadline: Option<i64>,
    ) -> Result<serde_json::Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({ "slug": slug }))
    }

    fn project_task_create(
        &self,
        project_id: &str,
        title: &str,
        description: &str,
        parent_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_project_id.lock().unwrap() = Some(project_id.to_string());
        *self.last_parent_id.lock().unwrap() = parent_id.map(str::to_string);
        Ok(serde_json::json!({
            "project_id": project_id,
            "title": title,
            "description": description,
            "parent_id": parent_id,
        }))
    }

    fn project_task_update_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some(serde_json::json!({ "id": id, "status": status })))
    }

    fn checkpoint_save(
        &self,
        project_id: &str,
        summary: &str,
        state: serde_json::Value,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({
            "project_id": project_id,
            "summary": summary,
            "state": state,
            "session_id": session_id,
        }))
    }
}

#[tokio::test]
async fn project_task_create_normalizes_ids_and_text_before_kernel() {
    let stub = Arc::new(ProjectToolStub::default());
    let kernel: Arc<dyn KernelHandle> = stub.clone();

    let result = execute_project_tool(
        "project_task_create",
        &serde_json::json!({
            "project_id": " project-1 ",
            "title": "  Build slice  ",
            "description": "  Verify runtime boundary  ",
            "parent_id": " parent-1 ",
        }),
        kernel,
    )
    .await;

    assert!(!result.is_error, "unexpected error: {}", result.content);
    assert_eq!(
        stub.last_project_id.lock().unwrap().as_deref(),
        Some("project-1")
    );
    assert_eq!(
        stub.last_parent_id.lock().unwrap().as_deref(),
        Some("parent-1")
    );
}

#[tokio::test]
async fn project_task_create_rejects_invalid_project_id_without_kernel_call() {
    let stub = Arc::new(ProjectToolStub::default());
    let kernel: Arc<dyn KernelHandle> = stub.clone();

    let result = execute_project_tool(
        "project_task_create",
        &serde_json::json!({
            "project_id": "bad-../private/leaky-fragment",
            "title": "Build slice",
        }),
        kernel,
    )
    .await;

    assert!(result.is_error);
    assert!(result.content.contains("project tool id"));
    assert!(!result.content.contains("../private"));
    assert!(!result.content.contains("leaky-fragment"));
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn project_task_update_rejects_invalid_status_without_echoing_input() {
    let stub = Arc::new(ProjectToolStub::default());
    let kernel: Arc<dyn KernelHandle> = stub.clone();

    let result = execute_project_tool(
        "project_task_update",
        &serde_json::json!({
            "id": "task-1",
            "status": "bad-../private/leaky-fragment",
        }),
        kernel,
    )
    .await;

    assert!(result.is_error);
    assert!(result.content.contains("project task status"));
    assert!(!result.content.contains("../private"));
    assert!(!result.content.contains("leaky-fragment"));
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn checkpoint_save_rejects_invalid_project_id_without_kernel_call() {
    let stub = Arc::new(ProjectToolStub::default());
    let kernel: Arc<dyn KernelHandle> = stub.clone();

    let result = execute_project_tool(
        "checkpoint_save",
        &serde_json::json!({
            "project_id": "bad-../private/leaky-fragment",
            "summary": "Reached verify",
            "state": {},
        }),
        kernel,
    )
    .await;

    assert!(result.is_error);
    assert!(result.content.contains("project tool id"));
    assert!(!result.content.contains("../private"));
    assert!(!result.content.contains("leaky-fragment"));
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn project_create_rejects_invalid_slug_without_kernel_call() {
    let stub = Arc::new(ProjectToolStub::default());
    let kernel: Arc<dyn KernelHandle> = stub.clone();

    let result = execute_project_tool(
        "project_create",
        &serde_json::json!({
            "name": "Demo",
            "slug": "bad-../private/leaky-fragment",
        }),
        kernel,
    )
    .await;

    assert!(result.is_error);
    assert!(result.content.contains("project tool slug"));
    assert!(!result.content.contains("../private"));
    assert!(!result.content.contains("leaky-fragment"));
    assert_eq!(stub.calls.load(Ordering::SeqCst), 0);
}

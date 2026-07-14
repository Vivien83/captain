use super::*;
use crate::kernel_handle::AgentInfo;
use async_trait::async_trait;
use captain_types::agent::AgentManifest;
use captain_types::message::TokenUsage;
use std::sync::{Arc, Mutex};

struct QuotaKernel {
    quota_result: Result<(), String>,
    checked_agents: Mutex<Vec<String>>,
}

impl QuotaKernel {
    fn failing(message: &str) -> Self {
        Self {
            quota_result: Err(message.to_string()),
            checked_agents: Mutex::new(Vec::new()),
        }
    }

    fn checked_agents(&self) -> Vec<String> {
        self.checked_agents.lock().unwrap().clone()
    }
}

#[async_trait]
impl KernelHandle for QuotaKernel {
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

    fn list_agents(&self) -> Vec<AgentInfo> {
        Vec::new()
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
        Err("not implemented".to_string())
    }

    fn check_agent_quota(&self, agent_id: &str) -> Result<(), String> {
        self.checked_agents
            .lock()
            .unwrap()
            .push(agent_id.to_string());
        self.quota_result.clone()
    }

    fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
        Err("not implemented".to_string())
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
        Err("not implemented".to_string())
    }

    async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
        Ok(())
    }
}

fn manifest() -> AgentManifest {
    AgentManifest {
        name: "captain".to_string(),
        ..Default::default()
    }
}

fn usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 10,
        output_tokens: 5,
        ..Default::default()
    }
}

#[test]
fn quota_check_skips_initial_iteration() {
    let manifest = manifest();
    let kernel = Arc::new(QuotaKernel::failing("limit"));
    let kernel_ref: Arc<dyn KernelHandle> = kernel.clone();

    let result = check_mid_loop_quota(&manifest, Some(&kernel_ref), 0, &usage(), &[]);

    assert!(result.is_none());
    assert!(kernel.checked_agents().is_empty());
}

#[test]
fn quota_check_returns_agent_loop_result_after_first_iteration() {
    let manifest = manifest();
    let kernel: Arc<dyn KernelHandle> = Arc::new(QuotaKernel::failing("daily limit"));
    let recorded = vec![ToolCallRecord {
        tool_name: "shell_exec".to_string(),
        reason: "Run a shell command needed for the current task.".to_string(),
        is_error: false,
        duration_ms: 12,
        input_summary: "{\"cmd\":\"true\"}".to_string(),
        output_summary: "ok".to_string(),
    }];

    let result = check_mid_loop_quota(&manifest, Some(&kernel), 3, &usage(), &recorded).unwrap();

    assert!(result
        .response
        .contains("Quota exceeded after 3 iterations: daily limit"));
    assert!(result.response.contains("captain agent caps captain"));
    assert_eq!(result.iterations, 3);
    assert_eq!(result.total_usage.input_tokens, 10);
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].tool_name, "shell_exec");
}

#[tokio::test]
async fn streaming_quota_breaks_and_emits_phase_event() {
    let manifest = manifest();
    let kernel: Arc<dyn KernelHandle> = Arc::new(QuotaKernel::failing("daily limit"));
    let (stream_tx, mut stream_rx) = mpsc::channel(2);

    let should_break = streaming_quota_should_break(&manifest, Some(&kernel), 2, &stream_tx).await;

    assert!(should_break);
    let event = stream_rx.recv().await.expect("quota event");
    assert!(matches!(
        event,
        StreamEvent::PhaseChange {
            phase,
            detail: Some(detail),
        } if phase == "quota_exceeded" && detail.contains("captain agent caps captain")
    ));
}

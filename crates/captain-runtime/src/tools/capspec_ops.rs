use crate::kernel_handle::{CapSpecForgeRequest, KernelHandle};
use crate::tools::require_kernel;
use std::path::Path;
use std::sync::Arc;

pub(crate) fn tool_capability_forge(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let request: CapSpecForgeRequest = serde_json::from_value(input.clone())
        .map_err(|error| format!("Invalid capability_forge input: {error}"))?;
    let result = require_kernel(kernel)?.capspec_forge(&request, workspace, caller_agent_id)?;
    serde_json::to_string_pretty(&result)
        .map_err(|error| format!("Cannot render Captain Forge result: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_handle::{AgentInfo, CapSpecForgeAction};
    use async_trait::async_trait;

    struct ForgeKernel;

    #[async_trait]
    impl KernelHandle for ForgeKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), String> {
            unreachable!()
        }

        async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
            unreachable!()
        }

        fn list_agents(&self) -> Vec<AgentInfo> {
            Vec::new()
        }

        fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
            unreachable!()
        }

        fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
            unreachable!()
        }

        fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
            unreachable!()
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
            unreachable!()
        }

        async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
            unreachable!()
        }

        async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
            unreachable!()
        }

        fn capspec_forge(
            &self,
            request: &CapSpecForgeRequest,
            _workspace: Option<&Path>,
            _caller_agent_id: Option<&str>,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({"action": request.action}))
        }
    }

    #[test]
    fn forge_rejects_unknown_and_operator_only_actions() {
        let kernel: Arc<dyn KernelHandle> = Arc::new(ForgeKernel);
        for action in ["approve", "reject", "rollback", "delete"] {
            let error = tool_capability_forge(
                &serde_json::json!({"action": action}),
                Some(&kernel),
                None,
                Some("captain"),
            )
            .unwrap_err();
            assert!(error.contains("unknown variant"), "{error}");
        }
    }

    #[test]
    fn forge_dispatches_a_typed_safe_action() {
        let kernel: Arc<dyn KernelHandle> = Arc::new(ForgeKernel);
        let output = tool_capability_forge(
            &serde_json::json!({"action": "list"}),
            Some(&kernel),
            None,
            Some("captain"),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(value["action"], serde_json::json!(CapSpecForgeAction::List));
    }
}

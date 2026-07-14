use crate::error::{KernelError, KernelResult};
use crate::workflow::{
    StepAgent, Workflow, WorkflowId, WorkflowRunId, WorkflowStepContext, WorkflowStepTimeoutPolicy,
};

use super::CaptainKernel;
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::AgentId;
use captain_types::error::CaptainError;
use std::sync::Arc;

impl CaptainKernel {
    /// Register a workflow definition.
    pub async fn register_workflow(&self, workflow: Workflow) -> WorkflowId {
        self.workflows.register(workflow).await
    }

    /// Run a workflow pipeline end-to-end.
    pub async fn run_workflow(
        &self,
        workflow_id: WorkflowId,
        input: String,
    ) -> KernelResult<(WorkflowRunId, String)> {
        let run_id = self
            .workflows
            .create_run(workflow_id, input)
            .await
            .ok_or_else(|| {
                KernelError::Captain(CaptainError::Internal("Workflow not found".to_string()))
            })?;

        // Agent resolver: looks up by name or ID in the registry
        let resolver = |agent_ref: &StepAgent| -> Option<(AgentId, String)> {
            match agent_ref {
                StepAgent::ById { id } => {
                    let agent_id: AgentId = id.parse().ok()?;
                    let entry = self.registry.get(agent_id)?;
                    Some((agent_id, entry.name.clone()))
                }
                StepAgent::ByName { name } => {
                    let entry = self.registry.find_by_name(name)?;
                    Some((entry.id, entry.name.clone()))
                }
            }
        };

        // Message sender: sends to agent and returns (output, in_tokens, out_tokens)
        let send_message = |agent_id: AgentId, message: String| async move {
            self.send_message(agent_id, &message)
                .await
                .map(|r| {
                    (
                        r.response,
                        r.total_usage.input_tokens,
                        r.total_usage.output_tokens,
                    )
                })
                .map_err(|e| format!("{e}"))
        };

        // SECURITY: Global workflow timeout to prevent runaway execution.
        const MAX_WORKFLOW_SECS: u64 = 3600; // 1 hour

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(MAX_WORKFLOW_SECS),
            self.workflows.execute_run(run_id, resolver, send_message),
        )
        .await
        .map_err(|_| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Workflow timed out after {MAX_WORKFLOW_SECS}s"
            )))
        })?
        .map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!("Workflow failed: {e}")))
        })?;

        Ok((run_id, output))
    }

    /// Run a workflow for cron using inactivity-based agent step guards.
    pub async fn run_workflow_with_inactivity_timeout(
        self: &Arc<Self>,
        workflow_id: WorkflowId,
        input: String,
        inactivity_limit: std::time::Duration,
    ) -> KernelResult<(WorkflowRunId, String)> {
        let run_id = self
            .workflows
            .create_run(workflow_id, input)
            .await
            .ok_or_else(|| {
                KernelError::Captain(CaptainError::Internal("Workflow not found".to_string()))
            })?;

        let resolver = |agent_ref: &StepAgent| -> Option<(AgentId, String)> {
            match agent_ref {
                StepAgent::ById { id } => {
                    let agent_id: AgentId = id.parse().ok()?;
                    let entry = self.registry.get(agent_id)?;
                    Some((agent_id, entry.name.clone()))
                }
                StepAgent::ByName { name } => {
                    let entry = self.registry.find_by_name(name)?;
                    Some((entry.id, entry.name.clone()))
                }
            }
        };

        let kernel = Arc::clone(self);
        let send_message =
            move |agent_id: AgentId, message: String, _context: WorkflowStepContext| {
                let kernel = Arc::clone(&kernel);
                async move {
                    let kh: Arc<dyn KernelHandle> = kernel.clone();
                    crate::cron_agent_turn::run_agent_turn_with_inactivity_timeout(
                        &kernel,
                        agent_id,
                        &message,
                        kh,
                        inactivity_limit,
                    )
                    .await
                    .map(|r| {
                        (
                            r.response,
                            r.total_usage.input_tokens,
                            r.total_usage.output_tokens,
                        )
                    })
                    .map_err(|e| e.to_string())
                }
            };

        let output = self
            .workflows
            .execute_run_with_step_timeout_policy(
                run_id,
                resolver,
                send_message,
                WorkflowStepTimeoutPolicy::CallerManaged,
            )
            .await
            .map_err(|e| {
                KernelError::Captain(CaptainError::Internal(format!("Workflow failed: {e}")))
            })?;

        Ok((run_id, output))
    }

    /// Auto-load workflow definitions from a directory.
    ///
    /// Scans the given directory for `.json` files, deserializes each as a
    /// `Workflow`, and registers it. Invalid files are skipped with a warning.
    pub async fn load_workflows_from_dir(&self, dir: &std::path::Path) -> usize {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(path = ?dir, error = %e, "Failed to read workflows directory");
                }
                return 0;
            }
        };

        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(path = ?path, error = %e, "Failed to read workflow file");
                    continue;
                }
            };
            match serde_json::from_str::<Workflow>(&content) {
                Ok(wf) => {
                    let name = wf.name.clone();
                    let wf_id = self.register_workflow(wf).await;
                    tracing::info!(path = ?path, id = %wf_id, name = %name, "Auto-loaded workflow");
                    count += 1;
                }
                Err(e) => {
                    tracing::warn!(path = ?path, error = %e, "Invalid workflow JSON, skipping");
                }
            }
        }
        count
    }
}

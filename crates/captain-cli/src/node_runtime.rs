//! Composition adapter between the durable Node worker and Captain Runtime.

use captain_node::{
    AuthorizedNodeRun, NodeRunCancellation, NodeToolDriver, NodeToolExecutionOutput,
    NodeToolReview, NodeWorkerError,
};
use captain_runtime::node_tool_runtime::{
    execute_local_node_tool, review_local_node_tool, LocalNodeToolEffect, LocalNodeToolExecution,
    LocalNodeToolOutput, LocalNodeToolRejection,
};
use captain_types::config::ExecPolicy;
use captain_wire::{hub_protocol::RunRejection, RunEffect, RunLease};
use futures::future::BoxFuture;
use std::{fmt, sync::Arc};

pub(crate) struct CliNodeToolDriver {
    exec_policy: ExecPolicy,
}

impl CliNodeToolDriver {
    pub(crate) fn new(exec_policy: ExecPolicy) -> Self {
        Self { exec_policy }
    }
}

impl NodeToolDriver for CliNodeToolDriver {
    fn review(&self, lease: &RunLease) -> Result<NodeToolReview, RunRejection> {
        let runtime = review_local_node_tool(&lease.tool_name, &lease.input, &self.exec_policy)
            .map_err(|rejection| runtime_rejection(lease, rejection))?;
        let reviewed = captain_node::NodeReviewedTool::new(
            runtime.tool_name(),
            runtime.family(),
            wire_effect(runtime.effect()),
        )
        .map_err(|_| fixed_rejection(lease, "runtime_review_contract_invalid", false))?;
        NodeToolReview::new(
            reviewed,
            runtime.action_digest(),
            runtime.approval_required(),
            runtime.risk_level(),
            runtime.approval_summary(),
        )
        .map_err(|_| fixed_rejection(lease, "runtime_review_contract_invalid", false))
    }

    fn execute(
        self: Arc<Self>,
        run: AuthorizedNodeRun,
        approved_action_digest: Option<String>,
        cancellation: NodeRunCancellation,
    ) -> BoxFuture<'static, NodeToolExecutionOutput> {
        Box::pin(async move {
            let lease = run.lease().clone();
            let workspace_root = run.workspace_root().to_path_buf();
            let tool_use_id = format!("node-{}-{}", lease.run_id, lease.attempt);
            let execution = execute_local_node_tool(LocalNodeToolExecution {
                tool_use_id: &tool_use_id,
                tool_name: &lease.tool_name,
                input: &lease.input,
                workspace_id: &lease.workspace_id,
                workspace_root: &workspace_root,
                exec_policy: &self.exec_policy,
                approved_action_digest: approved_action_digest.as_deref(),
            });
            tokio::pin!(execution);
            tokio::select! {
                result = &mut execution => match result {
                    Ok(output) => node_output(output),
                    Err(rejection) => safe_failure(&format!(
                        "Local Node runtime rejected execution ({}).",
                        rejection.code()
                    )),
                },
                () = cancellation.requested() => {
                    safe_failure("Local Node execution received a cancellation request.")
                }
            }
        })
    }
}

impl fmt::Debug for CliNodeToolDriver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CliNodeToolDriver")
            .field("execution_profile", &self.exec_policy.profile)
            .field("execution_mode", &self.exec_policy.mode)
            .field(
                "allowlist_entries",
                &self.exec_policy.allowed_commands.len(),
            )
            .finish()
    }
}

fn wire_effect(effect: LocalNodeToolEffect) -> RunEffect {
    match effect {
        LocalNodeToolEffect::ReadOnly => RunEffect::ReadOnly,
        LocalNodeToolEffect::LocalMutation => RunEffect::LocalMutation,
        LocalNodeToolEffect::ExternalEffect => RunEffect::ExternalEffect,
    }
}

fn runtime_rejection(lease: &RunLease, rejection: LocalNodeToolRejection) -> RunRejection {
    RunRejection {
        run_id: lease.run_id.clone(),
        attempt: lease.attempt,
        code: rejection.code().to_string(),
        message: rejection.message().to_string(),
        retryable: rejection.is_retryable(),
        path_policy_applied: true,
    }
}

fn fixed_rejection(lease: &RunLease, code: &str, retryable: bool) -> RunRejection {
    RunRejection {
        run_id: lease.run_id.clone(),
        attempt: lease.attempt,
        code: code.to_string(),
        message: "The local Runtime review did not satisfy the Node contract".to_string(),
        retryable,
        path_policy_applied: true,
    }
}

fn node_output(output: LocalNodeToolOutput) -> NodeToolExecutionOutput {
    let (succeeded, content, total_output_bytes, capped, redacted) = output.into_parts();
    NodeToolExecutionOutput::new(succeeded, content, total_output_bytes, capped, redacted)
        .unwrap_or_else(|_| safe_failure("Local Node output failed its final wire contract."))
}

fn safe_failure(message: &str) -> NodeToolExecutionOutput {
    NodeToolExecutionOutput::new(false, message, message.len() as u64, false, false).unwrap_or_else(
        |error| match error {
            NodeWorkerError::DriverContract => NodeToolExecutionOutput::new(
                false,
                "Local Node execution failed safely.",
                35,
                false,
                false,
            )
            .expect("fixed Node failure is contract-safe"),
            _ => unreachable!("output construction only returns driver contract errors"),
        },
    )
}

#[cfg(test)]
#[path = "node_runtime_tests.rs"]
mod tests;

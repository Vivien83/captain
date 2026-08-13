//! Resolve the durable execution target exactly once for each LLM turn.

use crate::error::{KernelError, KernelResult};
use captain_memory::execution_targets::ExecutionTargetScope;
use captain_runtime::execution_routing::TurnExecutionContext;
use captain_types::{
    agent::{AgentId, AgentManifest, SessionId},
    error::CaptainError,
};
use captain_wire::ExecutionTarget;

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn resolve_turn_execution_context(
        &self,
        agent_id: AgentId,
        manifest: &AgentManifest,
        session_id: SessionId,
    ) -> KernelResult<TurnExecutionContext> {
        let project_id = self.resolve_turn_project_id(agent_id, manifest)?;
        let project_target = project_id
            .as_deref()
            .map(|project_id| {
                self.memory
                    .execution_targets()
                    .get(ExecutionTargetScope::Project, project_id)
            })
            .transpose()
            .map_err(execution_target_error)?
            .flatten()
            .map(|binding| binding.target);
        let session_scope_id = session_id.to_string();
        let session_target = self
            .memory
            .execution_targets()
            .get(ExecutionTargetScope::Session, &session_scope_id)
            .map_err(execution_target_error)?
            .map(|binding| binding.target);

        Ok(resolve_context(
            session_scope_id,
            project_target,
            session_target,
        ))
    }

    fn resolve_turn_project_id(
        &self,
        agent_id: AgentId,
        manifest: &AgentManifest,
    ) -> KernelResult<Option<String>> {
        if let Some(project_id) = manifest
            .metadata
            .get("project_id")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|project_id| !project_id.is_empty())
        {
            return Ok(Some(project_id.to_string()));
        }
        let Some(slug) = captain_runtime::active_project::global()
            .and_then(|registry| registry.get(&agent_id.to_string()))
        else {
            return Ok(None);
        };
        self.memory
            .project_find_by_slug(&slug)
            .map(|project| project.map(|project| project.id))
            .map_err(|_| {
                KernelError::Captain(CaptainError::Memory(
                    "active project execution target could not be resolved".to_string(),
                ))
            })
    }
}

fn resolve_context(
    session_scope_id: String,
    project_target: Option<ExecutionTarget>,
    session_target: Option<ExecutionTarget>,
) -> TurnExecutionContext {
    match project_target
        .or(session_target)
        .unwrap_or(ExecutionTarget::Auto)
    {
        ExecutionTarget::Auto | ExecutionTarget::Hub => TurnExecutionContext::hub(session_scope_id),
        ExecutionTarget::Node {
            device_id,
            workspace_id,
        } => TurnExecutionContext::node(session_scope_id, device_id, workspace_id),
    }
}

fn execution_target_error(
    _error: captain_memory::execution_targets::ExecutionTargetStoreError,
) -> KernelError {
    KernelError::Captain(CaptainError::Memory(
        "durable execution target state is unavailable".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_runtime::execution_routing::ResolvedExecutionTarget;

    #[test]
    fn missing_and_auto_targets_resolve_conservatively_to_hub() {
        for target in [None, Some(ExecutionTarget::Auto)] {
            assert_eq!(
                resolve_context("session-1".to_string(), None, target),
                TurnExecutionContext::hub("session-1")
            );
        }
    }

    #[test]
    fn explicit_node_target_preserves_only_logical_identifiers() {
        let context = resolve_context(
            "session-1".to_string(),
            None,
            Some(ExecutionTarget::Node {
                device_id: "node-1".to_string(),
                workspace_id: "workspace-1".to_string(),
            }),
        );
        assert_eq!(context.scope_id, "session-1");
        assert_eq!(
            context.target,
            ResolvedExecutionTarget::Node {
                device_id: "node-1".to_string(),
                workspace_id: "workspace-1".to_string(),
            }
        );
    }

    #[test]
    fn explicit_project_binding_overrides_the_session_binding() {
        let context = resolve_context(
            "session-1".to_string(),
            Some(ExecutionTarget::Hub),
            Some(ExecutionTarget::Node {
                device_id: "node-1".to_string(),
                workspace_id: "workspace-1".to_string(),
            }),
        );
        assert_eq!(context, TurnExecutionContext::hub("session-1"));
    }
}

//! Task-local execution routing resolved once for an agent turn.

use std::future::Future;

/// Concrete execution target for a running turn. `Auto` is resolved by the
/// Kernel before this context is installed, so tool dispatch never guesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedExecutionTarget {
    Hub,
    Node {
        device_id: String,
        workspace_id: String,
    },
}

/// Stable execution context shared by every tool call in one agent turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnExecutionContext {
    pub scope_id: String,
    pub target: ResolvedExecutionTarget,
}

impl TurnExecutionContext {
    pub fn hub(scope_id: impl Into<String>) -> Self {
        Self {
            scope_id: scope_id.into(),
            target: ResolvedExecutionTarget::Hub,
        }
    }

    pub fn node(
        scope_id: impl Into<String>,
        device_id: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> Self {
        Self {
            scope_id: scope_id.into(),
            target: ResolvedExecutionTarget::Node {
                device_id: device_id.into(),
                workspace_id: workspace_id.into(),
            },
        }
    }
}

/// Input passed across the runtime/Kernel boundary for one Node tool run.
#[derive(Clone, PartialEq)]
pub struct RemoteToolExecutionRequest {
    pub scope_id: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub caller_agent_id: String,
    pub device_id: String,
    pub workspace_id: String,
}

impl std::fmt::Debug for RemoteToolExecutionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RemoteToolExecutionRequest")
            .field("scope_id", &self.scope_id)
            .field("tool_use_id", &self.tool_use_id)
            .field("tool_name", &self.tool_name)
            .field("input", &"[REDACTED]")
            .field("caller_agent_id", &self.caller_agent_id)
            .field("device_id", &self.device_id)
            .field("workspace_id", &self.workspace_id)
            .finish()
    }
}

tokio::task_local! {
    static TURN_EXECUTION_CONTEXT: TurnExecutionContext;
}

pub async fn with_turn_execution_context<F, T>(context: TurnExecutionContext, future: F) -> T
where
    F: Future<Output = T>,
{
    TURN_EXECUTION_CONTEXT.scope(context, future).await
}

pub fn current_turn_execution_context() -> Option<TurnExecutionContext> {
    TURN_EXECUTION_CONTEXT.try_with(Clone::clone).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn context_is_scoped_and_restored() {
        assert!(current_turn_execution_context().is_none());

        let observed = with_turn_execution_context(
            TurnExecutionContext::node("session-1", "node-1", "workspace-1"),
            async { current_turn_execution_context() },
        )
        .await;

        assert_eq!(
            observed,
            Some(TurnExecutionContext::node(
                "session-1",
                "node-1",
                "workspace-1"
            ))
        );
        assert!(current_turn_execution_context().is_none());
    }

    #[tokio::test]
    async fn nested_context_restores_the_outer_route() {
        let outer = TurnExecutionContext::hub("session-outer");
        with_turn_execution_context(outer.clone(), async {
            with_turn_execution_context(
                TurnExecutionContext::node("session-inner", "node-1", "workspace-1"),
                async {
                    assert!(matches!(
                        current_turn_execution_context().map(|context| context.target),
                        Some(ResolvedExecutionTarget::Node { .. })
                    ));
                },
            )
            .await;
            assert_eq!(current_turn_execution_context(), Some(outer));
        })
        .await;
    }
}

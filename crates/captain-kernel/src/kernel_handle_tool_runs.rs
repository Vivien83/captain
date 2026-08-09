//! Kernel authority for operator-triggered Live Run mutations.
//!
//! Read-only projections can use the process-wide registry directly. A cancel
//! request is different: every accepted or rejected attempt must cross one
//! auditable kernel boundary, including in-process TUI operation.

use captain_runtime::{
    audit::AuditAction,
    tool_runs::{global_registry, ToolRunCancelError, ToolRunSnapshot},
};

use super::CaptainKernel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRunOperatorSurface {
    Api,
    Tui,
}

impl ToolRunOperatorSurface {
    fn audit_actor(self) -> &'static str {
        match self {
            Self::Api => "operator:api",
            Self::Tui => "operator:tui",
        }
    }
}

impl CaptainKernel {
    pub fn operator_cancel_tool_run(
        &self,
        surface: ToolRunOperatorSurface,
        run_id: &str,
    ) -> Result<ToolRunSnapshot, ToolRunCancelError> {
        let result = global_registry().cancel_cancellable(run_id);
        let outcome = match &result {
            Ok(_) => "cancelled",
            Err(ToolRunCancelError::NotFound) => "not_found",
            Err(ToolRunCancelError::NotActive { .. }) => "not_active",
            Err(ToolRunCancelError::NotCancellable) => "not_cancellable",
        };
        self.audit_log.record_or_alert(
            surface.audit_actor(),
            AuditAction::ToolInvoke,
            format!("tool_run_cancel run_id={run_id}"),
            outcome,
        );
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{DefaultModelConfig, KernelConfig};

    #[tokio::test]
    async fn tui_cancellation_aborts_the_task_and_uses_the_fixed_audit_actor() {
        let temp = tempfile::tempdir().unwrap();
        let kernel = CaptainKernel::boot_with_config(KernelConfig {
            home_dir: temp.path().join("home"),
            data_dir: temp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        })
        .unwrap();
        let registry = global_registry();
        let run_id = registry.start("shell_exec", None, None, true, None);
        let task = tokio::spawn(std::future::pending::<()>());
        registry.attach_abort_handle(&run_id, task.abort_handle());

        let cancelled = kernel
            .operator_cancel_tool_run(ToolRunOperatorSurface::Tui, &run_id)
            .unwrap();

        assert_eq!(
            cancelled.status,
            captain_runtime::tool_runs::ToolRunStatus::Cancelled
        );
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(kernel.audit_log.recent(10).iter().any(|entry| {
            entry.agent_id == "operator:tui"
                && entry.action == AuditAction::ToolInvoke
                && entry.detail == format!("tool_run_cancel run_id={run_id}")
                && entry.outcome == "cancelled"
        }));
        kernel.shutdown();
    }
}

use crate::executor_scope::{rendered_permissions, validate_step_scope};
use crate::run_store::{RunStore, StoredRun};
use crate::{
    render_template, CapabilityExecution, CapabilityExecutionContext, CapabilityInvocation,
    CapabilityInvocationResult, CapabilityNodeStatus, CapabilityRegistry, CapabilityResumeContext,
    CapabilityRunStatus, CapabilityRunView, CapabilityToolInvoker, CompiledCapability,
    CompiledStep, Effect, ExecutorError, Idempotency, PermissionSet, ResolvedCapability,
    TemplateContext, UncertainNodeExpectation, UncertainResolution, UncertainResolutionReceipt,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

#[path = "executor_drive.rs"]
mod drive;
#[path = "executor_step.rs"]
mod step;

use step::{bounded_text, ensure_payload_size};

#[derive(Clone)]
pub struct CapabilityExecutor {
    registry: Arc<CapabilityRegistry>,
    store: Arc<Mutex<RunStore>>,
    active_runs: Arc<Mutex<BTreeSet<String>>>,
}

impl CapabilityExecutor {
    pub fn open(
        registry: Arc<CapabilityRegistry>,
        database_path: &Path,
        key_path: &Path,
    ) -> Result<Self, ExecutorError> {
        Ok(Self {
            registry,
            store: Arc::new(Mutex::new(RunStore::open(database_path, key_path)?)),
            active_runs: Arc::new(Mutex::new(BTreeSet::new())),
        })
    }

    pub async fn execute_tool(
        &self,
        tool_name: &str,
        input: Value,
        context: CapabilityExecutionContext,
        invoker: &dyn CapabilityToolInvoker,
    ) -> Result<CapabilityExecution, ExecutorError> {
        let workspace = context.workspace.as_deref().map(Path::new);
        let resolved = self
            .registry
            .resolved_by_tool(tool_name, workspace)?
            .ok_or_else(|| ExecutorError::CapabilityUnavailable(tool_name.to_string()))?;
        self.execute_resolved(resolved, input, context, invoker)
            .await
    }

    pub async fn execute_resolved(
        &self,
        resolved: ResolvedCapability,
        input: Value,
        mut context: CapabilityExecutionContext,
        invoker: &dyn CapabilityToolInvoker,
    ) -> Result<CapabilityExecution, ExecutorError> {
        ensure_payload_size(&input)?;
        normalize_execution_workspace(&resolved, &mut context)?;
        let input = resolved
            .compiled
            .validate_input(&input)
            .map_err(ExecutorError::InvalidInput)?;
        let run_id = uuid::Uuid::new_v4().to_string();
        self.lock_store()?
            .create_run(&run_id, &resolved, &input, &context)?;
        self.drive_with_deadline(&run_id, resolved.compiled, invoker)
            .await
    }

    pub async fn resume(
        &self,
        run_id: &str,
        invoker: &dyn CapabilityToolInvoker,
    ) -> Result<CapabilityExecution, ExecutorError> {
        let run = self.lock_store()?.load_run(run_id)?;
        match run.view.status {
            CapabilityRunStatus::Succeeded => {
                return Ok(CapabilityExecution {
                    run_id: run_id.to_string(),
                    source_hash: run.view.source_hash,
                    output: run.output.unwrap_or(Value::Null),
                    completed_nodes: run
                        .nodes
                        .iter()
                        .filter(|node| node.status == CapabilityNodeStatus::Succeeded)
                        .count(),
                });
            }
            CapabilityRunStatus::Pending
            | CapabilityRunStatus::Interrupted
            | CapabilityRunStatus::Running => {}
            CapabilityRunStatus::WaitingDecision => {
                let node_id = run
                    .nodes
                    .iter()
                    .find(|node| node.status == CapabilityNodeStatus::Uncertain)
                    .map(|node| node.step_id.clone())
                    .unwrap_or_else(|| "unknown".to_string());
                return Err(ExecutorError::WaitingDecision {
                    run_id: run_id.to_string(),
                    node_id,
                });
            }
            status => {
                return Err(ExecutorError::NotResumable {
                    run_id: run_id.to_string(),
                    status,
                });
            }
        }
        let compiled = self.pinned_revision(&run)?;
        self.drive_with_deadline(run_id, compiled, invoker).await
    }

    pub async fn resolve_uncertain(
        &self,
        run_id: &str,
        node_id: &str,
        resolution: UncertainResolution,
        invoker: &dyn CapabilityToolInvoker,
    ) -> Result<CapabilityExecution, ExecutorError> {
        let run = self.lock_store()?.load_run(run_id)?;
        let node = run
            .nodes
            .iter()
            .find(|node| node.step_id == node_id)
            .ok_or_else(|| ExecutorError::NodeNotFound {
                run_id: run_id.to_string(),
                node_id: node_id.to_string(),
            })?;
        if node.status != CapabilityNodeStatus::Uncertain {
            return Err(ExecutorError::InvalidState(format!(
                "node '{node_id}' is {:?}, not uncertain",
                node.status
            )));
        }
        let expectation = UncertainNodeExpectation {
            tool_use_id: node.tool_use_id.clone().ok_or_else(|| {
                ExecutorError::InvalidState(format!(
                    "uncertain node '{node_id}' has no tool use identity"
                ))
            })?,
            attempt: node.attempts,
        };
        let receipt = self.apply_uncertain_resolution(run_id, node_id, &expectation, resolution)?;
        if receipt.resume_required {
            self.resume(run_id, invoker).await
        } else {
            Err(ExecutorError::RunFailed {
                run_id: run_id.to_string(),
                message: "operator marked uncertain node as failed".to_string(),
            })
        }
    }

    pub fn apply_uncertain_resolution(
        &self,
        run_id: &str,
        node_id: &str,
        expectation: &UncertainNodeExpectation,
        resolution: UncertainResolution,
    ) -> Result<UncertainResolutionReceipt, ExecutorError> {
        if let UncertainResolution::ConfirmSucceeded { output } = &resolution {
            ensure_payload_size(output)?;
        }
        let resolution = match resolution {
            UncertainResolution::MarkFailed { reason } => UncertainResolution::MarkFailed {
                reason: bounded_text(reason),
            },
            other => other,
        };
        if !matches!(resolution, UncertainResolution::MarkFailed { .. }) {
            let run = self.lock_store()?.load_run(run_id)?;
            self.pinned_revision(&run)?;
        }
        self.lock_store()?
            .resolve_uncertain(run_id, node_id, expectation, &resolution)?;
        Ok(UncertainResolutionReceipt {
            run: self.run(run_id)?,
            resume_required: !matches!(resolution, UncertainResolution::MarkFailed { .. }),
        })
    }

    pub fn resume_context(&self, run_id: &str) -> Result<CapabilityResumeContext, ExecutorError> {
        let run = self.lock_store()?.load_run(run_id)?;
        let compiled = self.pinned_revision(&run)?;
        Ok(CapabilityResumeContext {
            run: run.view,
            execution: run.execution,
            required_tools: compiled.permissions.tools.clone(),
        })
    }

    pub fn required_tools(
        &self,
        tool_name: &str,
        workspace: Option<&Path>,
    ) -> Result<Vec<String>, ExecutorError> {
        let resolved = self
            .registry
            .resolved_by_tool(tool_name, workspace)?
            .ok_or_else(|| ExecutorError::CapabilityUnavailable(tool_name.to_string()))?;
        Ok(resolved.compiled.permissions.tools.clone())
    }

    pub fn run(&self, run_id: &str) -> Result<CapabilityRunView, ExecutorError> {
        self.lock_store()?.load_view(run_id)
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<CapabilityRunView>, ExecutorError> {
        self.lock_store()?.list_runs(limit)
    }

    /// List only runs that currently require an exact operator decision.
    /// This avoids hiding an older uncertain run behind unrelated recent runs.
    pub fn list_waiting_runs(&self, limit: usize) -> Result<Vec<CapabilityRunView>, ExecutorError> {
        self.lock_store()?.list_waiting_runs(limit)
    }

    /// Return durable operator-authorized resumes that still need dispatch.
    pub fn list_operator_resume_run_ids(&self, limit: usize) -> Result<Vec<String>, ExecutorError> {
        self.lock_store()?.list_operator_resume_run_ids(limit)
    }

    /// Claim or recover an operator-authorized resume without granting new authority.
    pub fn claim_operator_resume(&self, run_id: &str) -> Result<bool, ExecutorError> {
        self.lock_store()?.claim_operator_resume(run_id)
    }

    /// Put a claimed resume back in the durable queue after a preflight failure.
    pub fn release_operator_resume(&self, run_id: &str) -> Result<(), ExecutorError> {
        self.lock_store()?.release_operator_resume(run_id)
    }

    /// Acknowledge a normally settled resume attempt.
    pub fn finish_operator_resume(&self, run_id: &str) -> Result<(), ExecutorError> {
        self.lock_store()?.finish_operator_resume(run_id)
    }

    pub fn is_run_active(&self, run_id: &str) -> Result<bool, ExecutorError> {
        Ok(self
            .active_runs
            .lock()
            .map_err(|_| ExecutorError::Poisoned)?
            .contains(run_id))
    }

    fn pinned_revision(&self, run: &StoredRun) -> Result<Arc<CompiledCapability>, ExecutorError> {
        self.registry
            .compiled_revision(
                &run.view.scope,
                &run.view.capability_name,
                &run.view.source_hash,
            )?
            .ok_or_else(|| ExecutorError::RevisionUnavailable {
                run_id: run.view.run_id.clone(),
                source_hash: run.view.source_hash.clone(),
            })
    }

    fn fail_node(&self, run_id: &str, step_id: &str, message: &str) -> Result<(), ExecutorError> {
        let mut store = self.lock_store()?;
        store.mark_node_failed(run_id, step_id, message)?;
        store.set_run_status(run_id, CapabilityRunStatus::Failed, None, Some(message))
    }

    fn lock_store(&self) -> Result<MutexGuard<'_, RunStore>, ExecutorError> {
        self.store.lock().map_err(|_| ExecutorError::Poisoned)
    }

    fn claim_run(&self, run_id: &str) -> Result<ActiveRunLease, ExecutorError> {
        let mut active_runs = self
            .active_runs
            .lock()
            .map_err(|_| ExecutorError::Poisoned)?;
        if !active_runs.insert(run_id.to_string()) {
            return Err(ExecutorError::RunAlreadyExecuting(run_id.to_string()));
        }
        Ok(ActiveRunLease {
            run_id: run_id.to_string(),
            active_runs: Arc::clone(&self.active_runs),
            store: Arc::clone(&self.store),
            armed: true,
        })
    }
}

struct ActiveRunLease {
    run_id: String,
    active_runs: Arc<Mutex<BTreeSet<String>>>,
    store: Arc<Mutex<RunStore>>,
    armed: bool,
}

impl ActiveRunLease {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ActiveRunLease {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(mut store) = self.store.lock() {
                let _ = store.interrupt_run(
                    &self.run_id,
                    "CapSpec execution future was cancelled before a durable result",
                );
            }
        }
        if let Ok(mut active_runs) = self.active_runs.lock() {
            active_runs.remove(&self.run_id);
        }
    }
}

fn normalize_execution_workspace(
    resolved: &ResolvedCapability,
    context: &mut CapabilityExecutionContext,
) -> Result<(), ExecutorError> {
    match &resolved.scope {
        crate::CapabilityScope::Global => {
            if let Some(workspace) = context.workspace.as_deref() {
                let canonical = Path::new(workspace).canonicalize().map_err(|error| {
                    ExecutorError::WorkspaceMismatch(format!("{workspace}: {error}"))
                })?;
                context.workspace = Some(canonical.to_string_lossy().into_owned());
            }
            Ok(())
        }
        crate::CapabilityScope::Project(expected) => {
            let workspace = context.workspace.as_deref().ok_or_else(|| {
                ExecutorError::WorkspaceMismatch(format!(
                    "project capability requires workspace {}",
                    expected.display()
                ))
            })?;
            let actual = Path::new(workspace).canonicalize().map_err(|error| {
                ExecutorError::WorkspaceMismatch(format!("{workspace}: {error}"))
            })?;
            if &actual != expected {
                return Err(ExecutorError::WorkspaceMismatch(format!(
                    "expected {}, got {}",
                    expected.display(),
                    actual.display()
                )));
            }
            context.workspace = Some(actual.to_string_lossy().into_owned());
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;

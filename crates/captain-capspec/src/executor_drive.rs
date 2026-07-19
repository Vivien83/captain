use super::*;
use futures::{stream, StreamExt};

impl CapabilityExecutor {
    pub(super) async fn drive_with_deadline(
        &self,
        run_id: &str,
        capability: Arc<CompiledCapability>,
        invoker: &dyn CapabilityToolInvoker,
    ) -> Result<CapabilityExecution, ExecutorError> {
        let mut lease = self.claim_run(run_id)?;
        let timeout = Duration::from_secs(capability.policy.timeout_secs);
        let result =
            match tokio::time::timeout(timeout, self.drive_run(run_id, &capability, invoker)).await
            {
                Ok(result) => result,
                Err(_) => {
                    let reason = format!(
                        "capability exceeded its {} second deadline",
                        capability.policy.timeout_secs
                    );
                    let status = self.lock_store()?.interrupt_run(run_id, &reason)?;
                    if status == CapabilityRunStatus::WaitingDecision {
                        let node_id = self
                            .lock_store()?
                            .load_run(run_id)?
                            .nodes
                            .into_iter()
                            .find(|node| node.status == CapabilityNodeStatus::Uncertain)
                            .map(|node| node.step_id)
                            .unwrap_or_else(|| "unknown".to_string());
                        Err(ExecutorError::WaitingDecision {
                            run_id: run_id.to_string(),
                            node_id,
                        })
                    } else {
                        Err(ExecutorError::RunInterrupted {
                            run_id: run_id.to_string(),
                            message: reason,
                        })
                    }
                }
            };
        lease.disarm();
        result
    }

    async fn drive_run(
        &self,
        run_id: &str,
        capability: &CompiledCapability,
        invoker: &dyn CapabilityToolInvoker,
    ) -> Result<CapabilityExecution, ExecutorError> {
        loop {
            let run = self.lock_store()?.load_run(run_id)?;
            let outputs = succeeded_outputs(&run);
            if let Some(node) = run
                .nodes
                .iter()
                .find(|node| node.status == CapabilityNodeStatus::Uncertain)
            {
                self.lock_store()?.set_run_status(
                    run_id,
                    CapabilityRunStatus::WaitingDecision,
                    None,
                    node.error.as_deref(),
                )?;
                return Err(ExecutorError::WaitingDecision {
                    run_id: run_id.to_string(),
                    node_id: node.step_id.clone(),
                });
            }
            if let Some(node) = run
                .nodes
                .iter()
                .find(|node| node.status == CapabilityNodeStatus::Failed)
            {
                let message = node
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("step '{}' failed", node.step_id));
                self.lock_store()?.set_run_status(
                    run_id,
                    CapabilityRunStatus::Failed,
                    None,
                    Some(&message),
                )?;
                return Err(ExecutorError::RunFailed {
                    run_id: run_id.to_string(),
                    message,
                });
            }
            if run
                .nodes
                .iter()
                .all(|node| node.status == CapabilityNodeStatus::Succeeded)
            {
                let output = render_template(
                    &capability.output,
                    &TemplateContext {
                        run_id,
                        input: &run.input,
                        step_outputs: &outputs,
                    },
                )?;
                ensure_payload_size(&output)?;
                self.lock_store()?.set_run_status(
                    run_id,
                    CapabilityRunStatus::Succeeded,
                    Some(&output),
                    None,
                )?;
                return Ok(CapabilityExecution {
                    run_id: run_id.to_string(),
                    source_hash: capability.source_hash.clone(),
                    output,
                    completed_nodes: run.nodes.len(),
                });
            }

            let ready = ready_steps(capability, &run);
            if ready.is_empty() {
                let message = "no runnable node remains; persisted DAG state is inconsistent";
                self.lock_store()?.set_run_status(
                    run_id,
                    CapabilityRunStatus::Failed,
                    None,
                    Some(message),
                )?;
                return Err(ExecutorError::RunFailed {
                    run_id: run_id.to_string(),
                    message: message.to_string(),
                });
            }
            let workspace = run.view.workspace.as_deref().map(PathBuf::from);
            let permissions = rendered_permissions(capability, run_id, &run.input)?;
            if is_parallel_read(ready[0], invoker) {
                self.execute_read_batch(
                    run_id,
                    capability,
                    invoker,
                    &run,
                    &outputs,
                    &permissions,
                    workspace.as_deref(),
                    ready,
                )
                .await?;
            } else {
                self.execute_step(
                    run_id,
                    capability,
                    ready[0],
                    &run,
                    &outputs,
                    &permissions,
                    workspace.as_deref(),
                    invoker,
                )
                .await?;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_read_batch(
        &self,
        run_id: &str,
        capability: &CompiledCapability,
        invoker: &dyn CapabilityToolInvoker,
        run: &StoredRun,
        outputs: &BTreeMap<String, Value>,
        permissions: &PermissionSet,
        workspace: Option<&Path>,
        ready: Vec<&CompiledStep>,
    ) -> Result<(), ExecutorError> {
        let batch: Vec<CompiledStep> = ready
            .into_iter()
            .take_while(|step| is_parallel_read(step, invoker))
            .take(capability.policy.max_parallel)
            .cloned()
            .collect();
        let results = stream::iter(batch.into_iter().map(move |step| async move {
            self.execute_step(
                run_id,
                capability,
                &step,
                run,
                outputs,
                permissions,
                workspace,
                invoker,
            )
            .await
        }))
        .buffer_unordered(capability.policy.max_parallel)
        .collect::<Vec<_>>()
        .await;
        let mut first_error = None;
        for result in results {
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            self.lock_store()?.refresh_run_status(run_id, None)?;
            return Err(error);
        }
        Ok(())
    }
}

fn ready_steps<'a>(capability: &'a CompiledCapability, run: &StoredRun) -> Vec<&'a CompiledStep> {
    capability
        .steps
        .iter()
        .filter(|step| {
            let pending = run.nodes.iter().any(|node| {
                node.step_id == step.id && node.status == CapabilityNodeStatus::Pending
            });
            pending
                && step.needs.iter().all(|need| {
                    run.nodes.iter().any(|node| {
                        node.step_id == *need && node.status == CapabilityNodeStatus::Succeeded
                    })
                })
        })
        .collect()
}

fn succeeded_outputs(run: &StoredRun) -> BTreeMap<String, Value> {
    run.nodes
        .iter()
        .filter_map(|node| {
            (node.status == CapabilityNodeStatus::Succeeded)
                .then(|| {
                    node.output
                        .clone()
                        .map(|output| (node.step_id.clone(), output))
                })
                .flatten()
        })
        .collect()
}

fn is_parallel_read(step: &CompiledStep, invoker: &dyn CapabilityToolInvoker) -> bool {
    step.effect == Effect::Read && invoker.reviewed_effect(&step.tool) == Effect::Read
}

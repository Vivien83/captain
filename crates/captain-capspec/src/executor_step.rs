use super::*;

pub(super) const MAX_RUNTIME_PAYLOAD_BYTES: usize = 4 * 1024 * 1024;

impl CapabilityExecutor {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_step(
        &self,
        run_id: &str,
        capability: &CompiledCapability,
        step: &CompiledStep,
        run: &StoredRun,
        outputs: &BTreeMap<String, Value>,
        permissions: &PermissionSet,
        workspace: Option<&Path>,
        invoker: &dyn CapabilityToolInvoker,
    ) -> Result<(), ExecutorError> {
        let context = TemplateContext {
            run_id,
            input: &run.input,
            step_outputs: outputs,
        };
        let input = match render_template(&step.input, &context) {
            Ok(input) => input,
            Err(error) => {
                self.fail_node(run_id, &step.id, &error.to_string())?;
                return Err(error.into());
            }
        };
        if let Err(error) = ensure_payload_size(&input) {
            self.fail_node(run_id, &step.id, &error.to_string())?;
            return Err(error);
        }
        if let Err(error) = validate_step_scope(step, &input, permissions, workspace) {
            self.fail_node(run_id, &step.id, &error.to_string())?;
            return Err(error);
        }
        let idempotency_key = match step
            .idempotency_key
            .as_ref()
            .map(|template| render_idempotency_key(template, &context))
            .transpose()
        {
            Ok(key) => key,
            Err(error) => {
                self.fail_node(run_id, &step.id, &error.to_string())?;
                return Err(error);
            }
        };
        let supports_key =
            step.idempotency == Idempotency::Keyed && invoker.supports_idempotency(&step.tool);
        let replay_safe = step.idempotency == Idempotency::Safe || supports_key;
        let prior_node = run.nodes.iter().find(|node| node.step_id == step.id);
        let prior_attempts = prior_node.map(|node| node.attempts).unwrap_or(0);
        let operator_retry_permit = prior_node.is_some_and(|node| node.operator_retry_permit);
        if prior_attempts >= step.retry.max_attempts && !operator_retry_permit {
            let message = format!(
                "step '{}' exhausted {} attempts",
                step.id, step.retry.max_attempts
            );
            self.lock_store()?
                .mark_node_failed(run_id, &step.id, &message)?;
            return Ok(());
        }

        let mut next_attempt = prior_attempts.saturating_add(1);
        loop {
            let tool_use_id = format!("capspec:{run_id}:{}:{next_attempt}", step.id);
            let attempt = self.lock_store()?.mark_node_running(
                run_id,
                &step.id,
                &input,
                idempotency_key.as_deref(),
                &tool_use_id,
                replay_safe,
            )?;
            let invocation = CapabilityInvocation {
                run_id: run_id.to_string(),
                source_hash: capability.source_hash.clone(),
                step_id: step.id.clone(),
                tool_use_id,
                tool_name: step.tool.clone(),
                input: input.clone(),
                attempt,
                idempotency_key: idempotency_key.clone(),
            };
            let result = tokio::time::timeout(
                Duration::from_secs(step.timeout_secs),
                invoker.invoke(invocation),
            )
            .await;
            match result {
                Ok(result) if !result.is_error => {
                    let output = match parse_tool_output(result) {
                        Ok(output) => output,
                        Err(error) => {
                            self.fail_node(run_id, &step.id, &error.to_string())?;
                            return Err(error);
                        }
                    };
                    if let Err(error) = ensure_payload_size(&output) {
                        self.fail_node(run_id, &step.id, &error.to_string())?;
                        return Err(error);
                    }
                    self.lock_store()?
                        .mark_node_succeeded(run_id, &step.id, &output)?;
                    return Ok(());
                }
                Ok(result) => {
                    if replay_safe && attempt < step.retry.max_attempts {
                        tokio::time::sleep(Duration::from_millis(step.retry.backoff_ms)).await;
                        next_attempt = attempt.saturating_add(1);
                        continue;
                    }
                    let message = if step.idempotency == Idempotency::Keyed && !supports_key {
                        format!(
                            "{}; retry suppressed because '{}' does not prove keyed idempotency",
                            result.content, step.tool
                        )
                    } else {
                        result.content
                    };
                    self.lock_store()?.mark_node_failed(
                        run_id,
                        &step.id,
                        &bounded_text(message),
                    )?;
                    return Ok(());
                }
                Err(_) if replay_safe && attempt < step.retry.max_attempts => {
                    tokio::time::sleep(Duration::from_millis(step.retry.backoff_ms)).await;
                    next_attempt = attempt.saturating_add(1);
                }
                Err(_) if replay_safe => {
                    let message = format!(
                        "step '{}' exceeded its {} second deadline",
                        step.id, step.timeout_secs
                    );
                    self.lock_store()?
                        .mark_node_failed(run_id, &step.id, &message)?;
                    return Ok(());
                }
                Err(_) => {
                    let message = format!(
                        "step '{}' outcome is uncertain after its {} second deadline",
                        step.id, step.timeout_secs
                    );
                    self.lock_store()?
                        .mark_node_uncertain(run_id, &step.id, &message)?;
                    return Err(ExecutorError::WaitingDecision {
                        run_id: run_id.to_string(),
                        node_id: step.id.clone(),
                    });
                }
            }
        }
    }
}

fn render_idempotency_key(
    template: &str,
    context: &TemplateContext<'_>,
) -> Result<String, ExecutorError> {
    let rendered = render_template(&Value::String(template.to_string()), context)?;
    match rendered {
        Value::String(value) => Ok(value),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        _ => Err(ExecutorError::InvalidState(
            "idempotency key must render to a scalar".to_string(),
        )),
    }
}

fn parse_tool_output(result: CapabilityInvocationResult) -> Result<Value, ExecutorError> {
    if result.content.len() > MAX_RUNTIME_PAYLOAD_BYTES {
        return Err(ExecutorError::PayloadTooLarge {
            actual: result.content.len(),
            limit: MAX_RUNTIME_PAYLOAD_BYTES,
        });
    }
    Ok(serde_json::from_str(&result.content).unwrap_or(Value::String(result.content)))
}

pub(super) fn ensure_payload_size(value: &Value) -> Result<(), ExecutorError> {
    let actual = serde_json::to_vec(value)?.len();
    if actual > MAX_RUNTIME_PAYLOAD_BYTES {
        Err(ExecutorError::PayloadTooLarge {
            actual,
            limit: MAX_RUNTIME_PAYLOAD_BYTES,
        })
    } else {
        Ok(())
    }
}

pub(super) fn bounded_text(value: String) -> String {
    if value.len() <= MAX_RUNTIME_PAYLOAD_BYTES {
        return value;
    }
    format!(
        "tool response exceeded the {} byte CapSpec state limit",
        MAX_RUNTIME_PAYLOAD_BYTES
    )
}

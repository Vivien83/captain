use super::*;

impl RunStore {
    pub(crate) fn set_run_status(
        &mut self,
        run_id: &str,
        status: CapabilityRunStatus,
        output: Option<&Value>,
        error: Option<&str>,
    ) -> Result<(), ExecutorError> {
        let output_blob = output
            .map(|value| self.seal_json(&format!("run:{run_id}:output"), value))
            .transpose()?;
        let error_blob = error
            .map(|value| self.seal_text(&format!("run:{run_id}:error"), value))
            .transpose()?;
        let changed = self.connection.execute(
            "UPDATE capspec_runs
             SET status = ?2, output_blob = COALESCE(?3, output_blob),
                 error_blob = ?4,
                 operator_resume_state = CASE
                     WHEN ?2 IN ('succeeded', 'failed', 'waiting_decision') THEN 'none'
                     ELSE operator_resume_state
                 END,
                 updated_at = ?5
             WHERE run_id = ?1",
            params![
                run_id,
                status.as_storage(),
                output_blob,
                error_blob,
                crate::store::now(),
            ],
        )?;
        ensure_run_changed(changed, run_id)
    }

    pub(crate) fn mark_node_running(
        &mut self,
        run_id: &str,
        step_id: &str,
        input: &Value,
        idempotency_key: Option<&str>,
        tool_use_id: &str,
        replay_safe: bool,
    ) -> Result<u32, ExecutorError> {
        let input_blob = self.seal_json(&format!("run:{run_id}:node:{step_id}:input"), input)?;
        let key_blob = idempotency_key
            .map(|value| self.seal_text(&format!("run:{run_id}:node:{step_id}:key"), value))
            .transpose()?;
        let transaction = self.connection.transaction()?;
        let attempts: u32 = transaction
            .query_row(
                "SELECT attempts FROM capspec_run_nodes WHERE run_id = ?1 AND step_id = ?2",
                params![run_id, step_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| ExecutorError::NodeNotFound {
                run_id: run_id.to_string(),
                node_id: step_id.to_string(),
            })?;
        let next_attempt = attempts.saturating_add(1);
        transaction.execute(
            "UPDATE capspec_run_nodes
             SET status = ?3, attempts = ?4, replay_safe = ?5, input_blob = ?6,
                 idempotency_key_blob = ?7, tool_use_id = ?8, started_at = ?9,
                 finished_at = NULL, output_blob = NULL, error_blob = NULL,
                 operator_retry_permit = 0
             WHERE run_id = ?1 AND step_id = ?2",
            params![
                run_id,
                step_id,
                CapabilityNodeStatus::Running.as_storage(),
                next_attempt,
                replay_safe,
                input_blob,
                key_blob,
                tool_use_id,
                crate::store::now(),
            ],
        )?;
        transaction.execute(
            "UPDATE capspec_runs
             SET status = ?2, operator_resume_state = 'none', updated_at = ?3
             WHERE run_id = ?1",
            params![
                run_id,
                CapabilityRunStatus::Running.as_storage(),
                crate::store::now(),
            ],
        )?;
        transaction.commit()?;
        Ok(next_attempt)
    }

    pub(crate) fn mark_node_succeeded(
        &mut self,
        run_id: &str,
        step_id: &str,
        output: &Value,
    ) -> Result<(), ExecutorError> {
        let blob = self.seal_json(&format!("run:{run_id}:node:{step_id}:output"), output)?;
        self.update_node_terminal(
            run_id,
            step_id,
            CapabilityNodeStatus::Succeeded,
            Some(blob),
            None,
        )
    }

    pub(crate) fn mark_node_failed(
        &mut self,
        run_id: &str,
        step_id: &str,
        error: &str,
    ) -> Result<(), ExecutorError> {
        let blob = self.seal_text(&format!("run:{run_id}:node:{step_id}:error"), error)?;
        self.update_node_terminal(
            run_id,
            step_id,
            CapabilityNodeStatus::Failed,
            None,
            Some(blob),
        )
    }

    pub(crate) fn mark_node_uncertain(
        &mut self,
        run_id: &str,
        step_id: &str,
        error: &str,
    ) -> Result<(), ExecutorError> {
        let blob = self.seal_text(&format!("run:{run_id}:node:{step_id}:error"), error)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "UPDATE capspec_run_nodes SET status = ?3, error_blob = ?4,
                    finished_at = ?5 WHERE run_id = ?1 AND step_id = ?2",
            params![
                run_id,
                step_id,
                CapabilityNodeStatus::Uncertain.as_storage(),
                blob,
                crate::store::now(),
            ],
        )?;
        transaction.execute(
            "UPDATE capspec_runs SET status = ?2, updated_at = ?3 WHERE run_id = ?1",
            params![
                run_id,
                CapabilityRunStatus::WaitingDecision.as_storage(),
                crate::store::now(),
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn reset_node_pending(
        &mut self,
        run_id: &str,
        step_id: &str,
    ) -> Result<(), ExecutorError> {
        let changed = self.connection.execute(
            "UPDATE capspec_run_nodes SET status = ?3,
                    attempts = MAX(attempts - 1, 0), error_blob = NULL,
                    finished_at = NULL WHERE run_id = ?1 AND step_id = ?2",
            params![run_id, step_id, CapabilityNodeStatus::Pending.as_storage()],
        )?;
        ensure_node_changed(changed, run_id, step_id)
    }

    pub(crate) fn resolve_uncertain(
        &mut self,
        run_id: &str,
        step_id: &str,
        expectation: &crate::UncertainNodeExpectation,
        resolution: &crate::UncertainResolution,
    ) -> Result<CapabilityRunStatus, ExecutorError> {
        let output_blob = match resolution {
            crate::UncertainResolution::ConfirmSucceeded { output } => {
                Some(self.seal_json(&format!("run:{run_id}:node:{step_id}:output"), output)?)
            }
            _ => None,
        };
        let node_error_blob = match resolution {
            crate::UncertainResolution::MarkFailed { reason } => {
                Some(self.seal_text(&format!("run:{run_id}:node:{step_id}:error"), reason)?)
            }
            _ => None,
        };
        let run_error_blob = match resolution {
            crate::UncertainResolution::MarkFailed { reason } => {
                Some(self.seal_text(&format!("run:{run_id}:error"), reason)?)
            }
            _ => None,
        };
        let (status, finished_at) = match resolution {
            crate::UncertainResolution::ConfirmSucceeded { .. } => {
                ("succeeded", Some(crate::store::now()))
            }
            crate::UncertainResolution::Retry => ("pending", None),
            crate::UncertainResolution::MarkFailed { .. } => ("failed", Some(crate::store::now())),
        };
        let transaction = self.connection.transaction()?;
        let changed = transaction.execute(
            "UPDATE capspec_run_nodes
             SET status = ?5, output_blob = ?6, error_blob = ?7, finished_at = ?8,
                 operator_retry_permit = ?9
             WHERE run_id = ?1 AND step_id = ?2 AND status = 'uncertain'
               AND tool_use_id = ?3 AND attempts = ?4",
            params![
                run_id,
                step_id,
                expectation.tool_use_id,
                expectation.attempt,
                status,
                output_blob,
                node_error_blob,
                finished_at,
                matches!(resolution, crate::UncertainResolution::Retry),
            ],
        )?;
        if changed != 1 {
            let actual = transaction
                .query_row(
                    "SELECT status, attempts, tool_use_id FROM capspec_run_nodes
                     WHERE run_id = ?1 AND step_id = ?2",
                    params![run_id, step_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, u32>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()?
                .ok_or_else(|| ExecutorError::NodeNotFound {
                    run_id: run_id.to_string(),
                    node_id: step_id.to_string(),
                })?;
            return Err(ExecutorError::StaleUncertainDecision {
                run_id: run_id.to_string(),
                node_id: step_id.to_string(),
                expected_tool_use_id: expectation.tool_use_id.clone(),
                expected_attempt: expectation.attempt,
                actual_tool_use_id: actual.2,
                actual_attempt: actual.1,
                actual_status: CapabilityNodeStatus::from_storage(&actual.0)?,
            });
        }
        let run_status = if matches!(resolution, crate::UncertainResolution::MarkFailed { .. }) {
            CapabilityRunStatus::Failed
        } else {
            refreshed_status(&transaction, run_id)?
        };
        let operator_resume_state =
            if matches!(resolution, crate::UncertainResolution::MarkFailed { .. }) {
                "none"
            } else {
                "requested"
            };
        transaction.execute(
            "UPDATE capspec_runs
             SET status = ?2, error_blob = ?3, operator_resume_state = ?4,
                 updated_at = ?5 WHERE run_id = ?1",
            params![
                run_id,
                run_status.as_storage(),
                run_error_blob,
                operator_resume_state,
                crate::store::now(),
            ],
        )?;
        transaction.commit()?;
        Ok(run_status)
    }

    pub(crate) fn claim_operator_resume(&mut self, run_id: &str) -> Result<bool, ExecutorError> {
        let changed = self.connection.execute(
            "UPDATE capspec_runs SET operator_resume_state = 'in_progress'
             WHERE run_id = ?1
               AND operator_resume_state IN ('requested', 'in_progress')
               AND status IN ('pending', 'running', 'interrupted')",
            [run_id],
        )?;
        Ok(changed == 1)
    }

    pub(crate) fn release_operator_resume(&mut self, run_id: &str) -> Result<(), ExecutorError> {
        let changed = self.connection.execute(
            "UPDATE capspec_runs
             SET operator_resume_state = CASE
                 WHEN status IN ('pending', 'running', 'interrupted') THEN 'requested'
                 ELSE 'none'
             END
             WHERE run_id = ?1 AND operator_resume_state = 'in_progress'",
            [run_id],
        )?;
        if changed == 0 {
            self.load_view(run_id)?;
        }
        Ok(())
    }

    pub(crate) fn finish_operator_resume(&mut self, run_id: &str) -> Result<(), ExecutorError> {
        let changed = self.connection.execute(
            "UPDATE capspec_runs SET operator_resume_state = 'none'
             WHERE run_id = ?1 AND operator_resume_state != 'none'",
            [run_id],
        )?;
        if changed == 0 {
            self.load_view(run_id)?;
        }
        Ok(())
    }

    fn update_node_terminal(
        &mut self,
        run_id: &str,
        step_id: &str,
        status: CapabilityNodeStatus,
        output_blob: Option<Vec<u8>>,
        error_blob: Option<Vec<u8>>,
    ) -> Result<(), ExecutorError> {
        let changed = self.connection.execute(
            "UPDATE capspec_run_nodes
             SET status = ?3, output_blob = ?4, error_blob = ?5, finished_at = ?6
             WHERE run_id = ?1 AND step_id = ?2",
            params![
                run_id,
                step_id,
                status.as_storage(),
                output_blob,
                error_blob,
                crate::store::now(),
            ],
        )?;
        ensure_node_changed(changed, run_id, step_id)
    }
}

fn refreshed_status(
    transaction: &Transaction<'_>,
    run_id: &str,
) -> Result<CapabilityRunStatus, ExecutorError> {
    let (uncertain, failed, pending, running): (usize, usize, usize, usize) = transaction
        .query_row(
            "SELECT
                SUM(CASE WHEN status = 'uncertain' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END),
                SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END)
             FROM capspec_run_nodes WHERE run_id = ?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    Ok(if uncertain > 0 {
        CapabilityRunStatus::WaitingDecision
    } else if failed > 0 {
        CapabilityRunStatus::Failed
    } else if running > 0 {
        CapabilityRunStatus::Running
    } else if pending > 0 {
        CapabilityRunStatus::Interrupted
    } else {
        CapabilityRunStatus::Interrupted
    })
}

fn ensure_run_changed(changed: usize, run_id: &str) -> Result<(), ExecutorError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(ExecutorError::RunNotFound(run_id.to_string()))
    }
}

fn ensure_node_changed(changed: usize, run_id: &str, step_id: &str) -> Result<(), ExecutorError> {
    if changed == 1 {
        Ok(())
    } else {
        Err(ExecutorError::NodeNotFound {
            run_id: run_id.to_string(),
            node_id: step_id.to_string(),
        })
    }
}
